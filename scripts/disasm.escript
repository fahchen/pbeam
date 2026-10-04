#!/usr/bin/env escript
%% Disassemble a module or BEAM file with beam_disasm and print the selected functions without line instructions.
%% Usage: escript scripts/disasm.escript <module or file.beam> [name/arity]...

main([Target | Functions]) ->
    Selected = [parse_function(Function) || Function <- Functions],
    Path = resolve(Target),
    Code =
        case beam_disasm:file(Path) of
            {beam_file, _Module, _Exports, _Attributes, _CompileInfo, AllCode} -> AllCode;
            {error, beam_lib, Reason} -> fail("~ts: ~tp~n", [Path, Reason])
        end,
    Output = [
        io_lib:format("%% ~ts~n", [Path])
        | [format_function(Function) || Function <- select(Selected, Code)]
    ],
    write_stdout(unicode:characters_to_binary(Output));
main([]) ->
    fail("usage: disasm.escript <module or file.beam> [name/arity]...~n", []).

parse_function(Function) ->
    case string:split(Function, "/", trailing) of
        [Name, Arity] when Name =/= "" ->
            try list_to_integer(Arity) of
                N when N >= 0 -> {list_to_atom(Name), N};
                _ -> fail_function(Function)
            catch
                error:badarg -> fail_function(Function)
            end;
        _ ->
            fail_function(Function)
    end.

fail_function(Function) ->
    fail("function ~ts: use the form name/arity~n", [Function]).

resolve(Target) ->
    case filename:extension(Target) of
        ".beam" -> Target;
        _ -> which(Target, code:which(list_to_atom(Target)))
    end.

%% code:which/1 returns preloaded for a preloaded module (OTP-29.1.1 lib/kernel/src/code.erl:1773); its BEAM file is in the erts ebin directory.
which(Module, preloaded) ->
    filename:join([code:lib_dir(erts), "ebin", Module ++ ".beam"]);
which(Module, non_existing) ->
    fail("module ~ts: not found in the code path~n", [Module]);
which(_Module, Path) when is_list(Path) ->
    Path;
which(Module, Other) ->
    fail("module ~ts: no BEAM file (~p)~n", [Module, Other]).

select([], Code) ->
    Code;
select(Selected, Code) ->
    [find(NameArity, Code) || NameArity <- Selected].

find({Name, Arity}, Code) ->
    case [Function || {function, N, A, _, _} = Function <- Code, N =:= Name, A =:= Arity] of
        [Function] -> Function;
        [] -> fail("function ~tw/~b: not in the module~n", [Name, Arity])
    end.

format_function({function, Name, Arity, Entry, Instructions}) ->
    [
        io_lib:format("~tw/~b entry ~b~n", [Name, Arity, Entry])
        | [io_lib:format("    ~1000tp~n", [I]) || I <- Instructions, not is_line(I)]
    ].

%% A raw write returns {error, epipe} when a reader such as head closes the pipe early.
%% io:put_chars/1 writes an epipe error report and exits with an exception when the output is larger than the pipe buffer.
write_stdout(Binary) ->
    {ok, Stdout} = file:open("/dev/stdout", [append, raw, binary]),
    case file:write(Stdout, Binary) of
        ok -> ok;
        {error, epipe} -> ok;
        {error, Reason} -> fail("stdout: ~p~n", [Reason])
    end.

is_line({line, _}) -> true;
is_line(_) -> false.

fail(Format, Args) ->
    io:format(standard_error, Format, Args),
    halt(1).
