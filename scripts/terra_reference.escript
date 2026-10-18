#!/usr/bin/env escript
%% Run the Terra Counter scenario of docs/terra-target.md in a pseudo-terminal, check each screen, the exit status, and the terminal restore, and write a report.
%% Usage: escript scripts/terra_reference.escript <output directory> -- <command>...
%% The macOS script(1) utility opens the pseudo-terminal. Its stdin and stdout are the pipes of an Erlang port, so the harness sends the keys and reads the bytes that the program writes to the terminal.
%% A shell in the pseudo-terminal sets 80 columns and 24 rows, writes `stty -g` and `stty -a` before and after the command, and writes the exit status of the command. A background loop of the shell sets 30 rows and 100 columns when the harness writes the `resize` file, so that the resize reaches the program as a `SIGWINCH`.
%% A small screen model applies the control sequences of the output, as a terminal does. The checks read the visible screen, not the bytes.
%% Exit status: 0 when every check passes, 1 when a check fails, 2 for a usage error.
-mode(compile).

-define(COLUMNS, 80).
-define(ROWS, 24).
%% The bound of the first screen, in milliseconds. The command can build the example before it starts, and the first build downloads and compiles the Terra sources.
-define(START_TIMEOUT, 600000).
%% The bound of each other step, in milliseconds. A step that does not see its screen in its bound fails the run.
-define(STEP_TIMEOUT, 10000).
-define(SCRIPT, "/usr/bin/script").

main([Out, "--" | [_ | _] = Command]) ->
    ok = filelib:ensure_path(Out),
    Port = open(Out, Command),
    {Steps, Screen, Status} = run_steps(Port, Out, steps(), new_screen()),
    ok = file:write_file(filename:join(Out, "transcript.bin"), maps:get(transcript, Screen)),
    Report = report(Out, Command, Steps, Screen, Status),
    ok = file:write_file(filename:join(Out, "report.txt"), Report),
    io:put_chars(Report),
    case lists:all(fun({_, Result}) -> Result =:= pass end, checks(Out, Steps, Screen, Status)) of
        true -> halt(0);
        false -> halt(1)
    end;
main(_) ->
    io:format(standard_error, "usage: terra_reference.escript <output directory> -- <command>...~n", []),
    halt(2).

%% The steps of the Counter scenario: the action, the texts that the visible screen must show, and the bound. Each action goes only after the screen of the step before it.
%% `resize` sets the pseudo-terminal to 30 rows and 100 columns. `{send, Bytes, Wait}` sends the bytes and waits, so the input flush timer of Terra fires before the next step reads its screen.
steps() ->
    [
        {start, none, ["Count: 0", "j/k to change, q to quit"], ?START_TIMEOUT},
        {resize, resize, ["Count: 0"], ?STEP_TIMEOUT},
        {flush, {send, "\e[", 500}, ["Count: 0"], ?STEP_TIMEOUT},
        {increment, "j", ["Count: 1"], ?STEP_TIMEOUT},
        {increment, "j", ["Count: 2"], ?STEP_TIMEOUT},
        {decrement, "k", ["Count: 1"], ?STEP_TIMEOUT},
        {quit, "q", exit, ?STEP_TIMEOUT}
    ].

open(Out, Command) ->
    Shell =
        "out=$1; shift; "
        "rm -f \"$out/resize\" \"$out/stty-resize-ok\" \"$out/stty-resize-size.txt\" \"$out/stty-resize.g\"; "
        "(wait=0; while [ ! -f \"$out/resize\" ] && [ \"$wait\" -lt 12000 ]; do /bin/sleep 0.05; wait=$((wait + 1)); done; "
        "[ -f \"$out/resize\" ] && { stty rows 30 cols 100 < /dev/tty && stty size < /dev/tty > \"$out/stty-resize-size.txt\" && stty -g < /dev/tty > \"$out/stty-resize.g\" && : > \"$out/stty-resize-ok\"; }) & "
        "stty rows 24 cols 80 && stty -g > \"$out/stty-before.g\" && stty -a > \"$out/stty-before.txt\" || exit 125; "
        "\"$@\"; echo $? > \"$out/status\"; "
        "stty -g > \"$out/stty-after.g\"; stty -a > \"$out/stty-after.txt\"",
    Args = ["-q", "/dev/null", "/bin/sh", "-c", Shell, "sh", filename:absname(Out) | Command],
    Env = [{"TERM", "xterm-256color"}, {"LANG", "en_US.UTF-8"}, {"LC_ALL", "en_US.UTF-8"}],
    open_port({spawn_executable, ?SCRIPT}, [binary, stream, exit_status, use_stdio, {args, Args}, {env, Env}]).

run_steps(Port, Out, Steps, Screen) ->
    run_steps(Port, Out, Steps, Screen, []).

run_steps(Port, Out, [{Name, Action, Expect, Bound} | Steps], Screen0, Done) ->
    case act(Port, Out, Action, Screen0) of
        {failed, Screen} ->
            kill(Port),
            {lists:reverse([{Name, Action, Expect, act_failed, visible(Screen)} | Done]), Screen, act_failed};
        {exit, Status, Screen} ->
            {lists:reverse([{Name, Action, Expect, early_exit, visible(Screen)} | Done]), Screen, Status};
        {ok, Screen1} ->
            run_await(Port, Out, Name, Action, Expect, Bound, Steps, Screen1, Done)
    end;
run_steps(_Port, _Out, [], Screen, Done) ->
    {lists:reverse(Done), Screen, none}.

run_await(Port, Out, Name, Action, Expect, Bound, Steps, Screen1, Done) ->
    Deadline = erlang:monotonic_time(millisecond) + Bound,
    case await(Port, Expect, Screen1, Deadline) of
        {ok, Screen} ->
            Step = {Name, Action, Expect, pass, visible(Screen)},
            run_steps(Port, Out, Steps, Screen, [Step | Done]);
        {exit, Status, Screen} when Expect =:= exit ->
            {lists:reverse([{Name, Action, Expect, pass, visible(Screen)} | Done]), Screen, Status};
        {exit, Status, Screen} ->
            {lists:reverse([{Name, Action, Expect, early_exit, visible(Screen)} | Done]), Screen, Status};
        {timeout, Screen} ->
            kill(Port),
            {lists:reverse([{Name, Action, Expect, timeout, visible(Screen)} | Done]), Screen, timeout}
    end.

%% Run the action of a step: write the `resize` file that the shell loop of the
%% pseudo-terminal waits for and follow the new size in the screen model, send
%% bytes and wait, send a key, or do nothing.
act(_Port, _Out, none, Screen) ->
    {ok, Screen};
act(_Port, Out, resize, Screen) ->
    ok = file:write_file(filename:join(Out, "resize"), <<"1">>),
    case await_resize(Out, erlang:monotonic_time(millisecond) + ?STEP_TIMEOUT) of
        ok -> {ok, Screen#{rows := 30, cols := 100}};
        failed -> {failed, Screen}
    end;
act(Port, _Out, {send, Bytes, Wait}, Screen) ->
    true = port_command(Port, Bytes),
    wait(Port, erlang:monotonic_time(millisecond) + Wait, Screen);
act(Port, _Out, Key, Screen) ->
    true = port_command(Port, Key),
    {ok, Screen}.

%% Read the output of the port until the deadline, so that the program can
%% write and exit while the step waits. Return the exit status when the
%% program ends first.
wait(Port, Deadline, Screen) ->
    Timeout = Deadline - erlang:monotonic_time(millisecond),
    case Timeout =< 0 of
        true ->
            {ok, Screen};
        false ->
            receive
                {Port, {data, Bytes}} -> wait(Port, Deadline, feed(Bytes, Screen));
                {Port, {exit_status, Status}} -> {exit, Status, Screen}
            after Timeout -> {ok, Screen}
            end
    end.

%% Wait until the shell of the pseudo-terminal writes the size of the
%% resized terminal and a marker, and accept only 30 rows and 100 columns.
await_resize(Out, Deadline) ->
    Marker = filename:join(Out, "stty-resize-ok"),
    case filelib:is_regular(Marker) of
        true ->
            case file:read_file(filename:join(Out, "stty-resize-size.txt")) of
                {ok, <<"30 100\n">>} -> ok;
                _ -> failed
            end;
        false ->
            case erlang:monotonic_time(millisecond) >= Deadline of
                true -> failed;
                false -> timer:sleep(20), await_resize(Out, Deadline)
            end
    end.

%% Kill the process of the pseudo-terminal, which closes the terminal and
%% sends `SIGHUP` to the program, so that a run that times out does not
%% keep the harness alive.
kill(Port) ->
    case erlang:port_info(Port, os_pid) of
        {os_pid, Pid} -> os:cmd("kill -KILL " ++ integer_to_list(Pid));
        undefined -> ok
    end,
    %% The kill can close the port before this call.
    try
        port_close(Port)
    catch
        error:badarg -> ok
    end,
    ok.

%% The step accepts only the output that it receives before the deadline. After the deadline, queued or continuous output does not make the step pass.
await(Port, Expect, Screen, Deadline) ->
    Timeout = Deadline - erlang:monotonic_time(millisecond),
    case Expect =/= exit andalso shows(Screen, Expect) of
        true ->
            {ok, Screen};
        false when Timeout =< 0 ->
            {timeout, Screen};
        false ->
            receive
                {Port, {data, Bytes}} -> await(Port, Expect, feed(Bytes, Screen), Deadline);
                {Port, {exit_status, Status}} -> {exit, Status, Screen}
            after Timeout -> {timeout, Screen}
            end
    end.

shows(Screen, Texts) ->
    Text = unicode:characters_to_binary(lists:join($\n, visible(Screen))),
    lists:all(fun(Want) -> binary:match(Text, unicode:characters_to_binary(Want)) =/= nomatch end, Texts).

%% Checks after the run. The exit status of script(1) and the status file must be 0, the output must have the result `{:ok, 1}`, and the terminal must be as before the command: the main screen, a visible cursor, default attributes, and the same `stty -g` settings.
%% The result can go to the alternate screen: Terra sends its result to the caller before its terminal owner restores the terminal, so the order of the two writes is a race ("Code loading" in docs/erts-differences.md). The report gives the screen that shows the result after the run.
checks(Out, Steps, Screen, Status) ->
    [
        {"every step shows its screen", result(lists:all(fun({_, _, _, R, _}) -> R =:= pass end, Steps))},
        {"script(1) exit status is 0", result(Status =:= 0)},
        {"command exit status is 0", result(read(Out, "status") =:= <<"0\n">>)},
        {"output has {:ok, 1}", result(binary:match(maps:get(transcript, Screen), <<"{:ok, 1}">>) =/= nomatch)},
        {"main screen is active", result(maps:get(active, Screen) =:= main)},
        {"cursor is visible", result(maps:get(cursor_visible, Screen))},
        {"attributes are reset", result(maps:get(sgr, Screen) =:= default)},
        {"stty -g after equals stty -g before", result(read(Out, "stty-before.g") =/= missing andalso read(Out, "stty-before.g") =:= read(Out, "stty-after.g"))}
    ].

result(true) -> pass;
result(false) -> fail.

read(Out, Name) ->
    case file:read_file(filename:join(Out, Name)) of
        {ok, Bin} -> Bin;
        {error, _} -> missing
    end.

report(Out, Command, Steps, Screen, Status) ->
    unicode:characters_to_binary([
        "# Terra Counter reference run\n\n",
        "command: ", lists:join(" ", Command), "\n",
        "pseudo-terminal: ", ?SCRIPT, " -q /dev/null, ", integer_to_list(?COLUMNS), "x", integer_to_list(?ROWS),
        " (100x30 after the resize), TERM=xterm-256color, LANG=LC_ALL=en_US.UTF-8\n",
        "otp: ", erlang:system_info(otp_release), ", erts ", erlang:system_info(version), "\n",
        "os: ", os_version(), "\n",
        "script(1) exit status: ", io_lib:format("~tp", [Status]), "\n",
        "command exit status: ", string:trim(binary_to_list(as_text(read(Out, "status")))), "\n\n",
        [step_report(Step) || Step <- Steps],
        "## Final screen (", atom_to_list(maps:get(active, Screen)), ")\n\n", screen_text(visible(Screen)), "\n",
        "## Terminal state after the run\n\n",
        io_lib:format("main screen shows {:ok, 1}: ~p~ncursor visible: ~p~nattributes: ~p~nprivate modes: ~p~nunknown sequences: ~tp~n~n",
                      [maps:get(active, Screen) =:= main andalso shows(Screen, ["{:ok, 1}"]), maps:get(cursor_visible, Screen), maps:get(sgr, Screen), maps:get(modes, Screen), lists:usort(maps:get(unknown, Screen))]),
        "## stty -a before\n\n", as_text(read(Out, "stty-before.txt")), "\n",
        "## stty -a after\n\n", as_text(read(Out, "stty-after.txt")), "\n",
        "## Checks\n\n",
        [io_lib:format("~ts: ~p~n", [Name, R]) || {Name, R} <- checks(Out, Steps, Screen, Status)]
    ]).

step_report({Name, Key, Expect, Result, Visible}) ->
    [io_lib:format("## Step ~p, key ~tp, expect ~tp: ~p~n~n", [Name, Key, Expect, Result]), screen_text(Visible), "\n"].

%% The rows up to the last row that is not empty, indented.
screen_text(Rows) ->
    Shown = lists:reverse(lists:dropwhile(fun(Row) -> Row =:= [] end, lists:reverse(Rows))),
    [["    ", Row, "\n"] || Row <- Shown].

as_text(missing) -> <<"(missing)\n">>;
as_text(Bin) -> Bin.

os_version() ->
    string:trim(os:cmd("sw_vers -productVersion")) ++ " " ++ string:trim(os:cmd("uname -m")).

%% Screen model. A grid of ?ROWS rows and ?COLUMNS columns for the main screen and for the alternate screen of mode 1049. Cells hold code points; a missing cell is a space.

new_screen() ->
    #{main => #{}, alt => #{}, active => main, row => 1, col => 1, saved => {1, 1},
      rows => ?ROWS, cols => ?COLUMNS,
      cursor_visible => true, sgr => default, modes => #{}, unknown => [], pending => <<>>, transcript => <<>>}.

feed(Bytes, #{pending := Pending, transcript := Transcript} = Screen) ->
    parse(<<Pending/binary, Bytes/binary>>, Screen#{pending := <<>>, transcript := <<Transcript/binary, Bytes/binary>>}).

parse(<<>>, Screen) ->
    Screen;
parse(<<27, $[, Rest/binary>> = Bin, Screen) ->
    case csi(Rest, <<>>) of
        {Params, Final, Tail} -> parse(Tail, control(Params, Final, Screen));
        incomplete -> Screen#{pending := Bin}
    end;
parse(<<27, $(, _, Rest/binary>>, Screen) ->
    parse(Rest, Screen);
parse(<<27, $7, Rest/binary>>, #{row := Row, col := Col} = Screen) ->
    parse(Rest, Screen#{saved := {Row, Col}});
parse(<<27, $8, Rest/binary>>, #{saved := {Row, Col}} = Screen) ->
    parse(Rest, Screen#{row := Row, col := Col});
parse(<<27, C, Rest/binary>>, Screen) when C =/= $(, C =/= $[ ->
    parse(Rest, unknown([27, C], Screen));
parse(<<27, _/binary>> = Bin, Screen) ->
    Screen#{pending := Bin};
parse(<<$\r, Rest/binary>>, Screen) ->
    parse(Rest, Screen#{col := 1});
parse(<<$\n, Rest/binary>>, Screen) ->
    parse(Rest, line_feed(Screen));
parse(<<$\b, Rest/binary>>, #{col := Col} = Screen) ->
    parse(Rest, Screen#{col := max(1, min(Col, cols(Screen)) - 1)});
parse(<<$\t, Rest/binary>>, #{col := Col} = Screen) ->
    parse(Rest, Screen#{col := min(cols(Screen), ((Col - 1) div 8 + 1) * 8 + 1)});
parse(<<C, Rest/binary>>, Screen) when C < 32; C =:= 127 ->
    parse(Rest, Screen);
parse(<<C/utf8, Rest/binary>>, Screen) ->
    parse(Rest, put_char(C, Screen));
parse(Bin, Screen) when byte_size(Bin) < 4 ->
    Screen#{pending := Bin};
parse(<<_, Rest/binary>>, Screen) ->
    parse(Rest, put_char(16#FFFD, Screen)).

%% A control sequence: parameter bytes 0x30-0x3F, intermediate bytes 0x20-0x2F, and one final byte 0x40-0x7E.
csi(<<C, Rest/binary>>, Acc) when C >= 16#20, C =< 16#3F -> csi(Rest, <<Acc/binary, C>>);
csi(<<C, Rest/binary>>, Acc) when C >= 16#40, C =< 16#7E -> {Acc, C, Rest};
csi(<<>>, _Acc) -> incomplete;
csi(<<C, Rest/binary>>, Acc) -> {Acc, C, Rest}.

control(<<"?", Modes/binary>>, Final, Screen) when Final =:= $h; Final =:= $l ->
    lists:foldl(fun(Mode, S) -> private_mode(Mode, Final =:= $h, S) end, Screen, numbers(Modes, 0));
control(Params, $H, Screen) -> cursor_position(Params, Screen);
control(Params, $f, Screen) -> cursor_position(Params, Screen);
control(Params, $A, #{row := Row} = Screen) -> Screen#{row := max(1, Row - hd(numbers(Params, 1)))};
control(Params, $B, #{row := Row} = Screen) -> Screen#{row := min(rows(Screen), Row + hd(numbers(Params, 1)))};
control(Params, $C, #{col := Col} = Screen) -> Screen#{col := min(cols(Screen), Col + hd(numbers(Params, 1)))};
control(Params, $D, #{col := Col} = Screen) -> Screen#{col := max(1, min(Col, cols(Screen)) - hd(numbers(Params, 1)))};
control(Params, $G, Screen) -> Screen#{col := clamp(hd(numbers(Params, 1)), cols(Screen))};
control(Params, $d, Screen) -> Screen#{row := clamp(hd(numbers(Params, 1)), rows(Screen))};
control(Params, $J, Screen) -> erase_display(hd(numbers(Params, 0)), Screen);
control(Params, $K, Screen) -> erase_line(hd(numbers(Params, 0)), Screen);
control(Params, $m, Screen) -> Screen#{sgr := lists:foldl(fun sgr/2, maps:get(sgr, Screen), numbers(Params, 0))};
control(Params, Final, Screen) -> unknown([27, $[, Params, Final], Screen).

%% Code 0 resets the attributes; any other code sets one.
sgr(0, _) -> default;
sgr(_, _) -> styled.

private_mode(1049, true, #{row := Row, col := Col} = Screen) ->
    Screen#{active := alt, alt := #{}, saved := {Row, Col}, modes := maps:put(1049, true, maps:get(modes, Screen))};
private_mode(1049, false, #{saved := {Row, Col}} = Screen) ->
    Screen#{active := main, row := Row, col := Col, modes := maps:put(1049, false, maps:get(modes, Screen))};
private_mode(25, Set, Screen) ->
    Screen#{cursor_visible := Set};
private_mode(Mode, Set, Screen) ->
    Screen#{modes := maps:put(Mode, Set, maps:get(modes, Screen))}.

cursor_position(Params, Screen) ->
    [Row, Col | _] = numbers(Params, 1) ++ [1],
    Screen#{row := clamp(Row, rows(Screen)), col := clamp(Col, cols(Screen))}.

%% The numeric parameters, separated by ';'. An empty parameter takes the default.
numbers(Params, Default) ->
    [number(Field, Default) || Field <- binary:split(Params, <<";">>, [global])].

number(Field, Default) ->
    try binary_to_integer(Field) catch error:badarg -> Default end.

clamp(N, Max) -> max(1, min(N, Max)).

erase_display(0, #{row := Row, col := Col} = Screen) -> erase(fun({R, C}) -> R > Row orelse (R =:= Row andalso C >= Col) end, Screen);
erase_display(1, #{row := Row, col := Col} = Screen) -> erase(fun({R, C}) -> R < Row orelse (R =:= Row andalso C =< Col) end, Screen);
erase_display(_, Screen) -> erase(fun(_) -> true end, Screen).

erase_line(0, #{row := Row, col := Col} = Screen) -> erase(fun({R, C}) -> R =:= Row andalso C >= Col end, Screen);
erase_line(1, #{row := Row, col := Col} = Screen) -> erase(fun({R, C}) -> R =:= Row andalso C =< Col end, Screen);
erase_line(_, #{row := Row} = Screen) -> erase(fun({R, _}) -> R =:= Row end, Screen).

erase(Erased, #{active := Active} = Screen) ->
    Screen#{Active := maps:filter(fun(Cell, _) -> not Erased(Cell) end, maps:get(Active, Screen))}.

%% A character in the column after the last wraps to the next line first, as a terminal with auto-wrap does.
put_char(C, #{col := Col, cols := Cols} = Screen) when Col > Cols ->
    put_char(C, (line_feed(Screen))#{col := 1});
put_char(C, #{active := Active, row := Row, col := Col} = Screen) ->
    Screen#{Active := maps:put({Row, Col}, C, maps:get(Active, Screen)), col := Col + 1}.

line_feed(#{row := Row, rows := Rows} = Screen) when Row < Rows ->
    Screen#{row := Row + 1};
line_feed(#{active := Active} = Screen) ->
    Grid = maps:get(Active, Screen),
    Screen#{Active := maps:from_list([{{R - 1, C}, V} || {{R, C}, V} <- maps:to_list(Grid), R > 1])}.

unknown(Sequence, #{unknown := Unknown} = Screen) ->
    Screen#{unknown := [iolist_to_binary(io_lib:format("~tp", [iolist_to_binary(Sequence)])) | Unknown]}.

%% The rows of the active screen, without trailing spaces.
visible(#{active := Active} = Screen) ->
    Grid = maps:get(Active, Screen),
    [string:trim([maps:get({Row, Col}, Grid, $\s) || Col <- lists:seq(1, cols(Screen))], trailing) || Row <- lists:seq(1, rows(Screen))].

cols(Screen) -> maps:get(cols, Screen).
rows(Screen) -> maps:get(rows, Screen).
