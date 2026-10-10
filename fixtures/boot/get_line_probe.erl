-module(get_line_probe).
-export([main/0, wait/0, reader/1]).

%% Read lines with io:get_line/2 until the end of input, then write each
%% result: the character codes of a line, or eof.
main() ->
    Lines = read([]),
    io:put_chars([lists:join(" ", [show(Line) || Line <- Lines]), "\n"]).

read(Acc) ->
    case io:get_line(standard_io, "> ") of
        eof -> lists:reverse([eof | Acc]);
        Line -> read([Line | Acc])
    end.

%% A reader process waits for a line, and the probe writes "tick" after a
%% timer of 100 ms, before the line arrives.
wait() ->
    Reader = spawn(?MODULE, reader, [self()]),
    receive after 100 -> ok end,
    io:put_chars("tick\n"),
    receive {Reader, Line} -> io:put_chars(["line ", show(Line), "\n"]) end.

reader(Parent) ->
    Parent ! {self(), io:get_line(standard_io, "")}.

show(eof) ->
    "eof";
show(Line) when is_list(Line) ->
    ["[", lists:join(",", [integer_to_list(Char) || Char <- Line]), "]"].
