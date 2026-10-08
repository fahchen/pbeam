#!/usr/bin/env escript
%% Compile Erlang fixtures and compare each result with the committed BEAM file in all chunks except CInf and Dbgi.
%% Usage: escript scripts/fixture_beam.escript [--write] [fixtures/<dir>/<name>.erl]...

main(["--write" | Sources]) ->
    run(write, sources(Sources));
main(Sources) ->
    run(verify, sources(Sources)).

%% With no source, use all fixtures under the current directory, which is the repository root when mise runs the task.
sources([]) ->
    case filelib:wildcard("fixtures/*/*.erl") of
        [] -> fail("no fixtures/*/*.erl under ~ts~n", [element(2, file:get_cwd())]);
        Sources -> Sources
    end;
sources(Sources) ->
    Sources.

run(Mode, Sources) ->
    Results = [{Source, fixture(Mode, Source)} || Source <- Sources],
    Same = length([same || {_, same} <- Results]),
    io:format("~b of ~b fixtures same~n", [Same, length(Sources)]),
    case Mode =:= verify andalso Same < length(Sources) orelse lists:keymember(error, 2, Results) of
        true -> halt(1);
        false -> ok
    end.

fixture(Mode, Source) ->
    Beam = filename:rootname(Source) ++ ".beam",
    Committed = committed(Beam),
    Status =
        case compile:file(Source, [binary, report, warnings_as_errors | options(Committed)]) of
            {ok, _Module, Binary} -> {compare(Binary, Committed), Binary};
            error -> error
        end,
    report(Mode, Source, Beam, Status).

committed(Beam) ->
    case file:read_file(Beam) of
        {ok, Binary} -> Binary;
        {error, enoent} -> none
    end.

%% A committed BEAM file records its compile options in CInf; a new fixture gets the options that docs/hello-world-target.md uses for erlc.
options(none) ->
    [no_line_info];
options(Committed) ->
    {ok, {_Module, [{compile_info, Info}]}} = beam_lib:chunks(Committed, [compile_info]),
    case lists:keyfind(options, 1, Info) of
        {options, Options} -> Options;
        false -> debug_info_options(Committed)
    end.

%% The deterministic option removes options from CInf (OTP-29.1.1 lib/compiler/src/compile.erl:2762-2766).
%% Dbgi keeps the options that change code generation, deterministic included (compile.erl:2636-2653, compile.erl:2667-2693).
debug_info_options(Committed) ->
    {ok, {_Module, [{debug_info, {debug_info_v1, erl_abstract_code, {_Abstract, Options}}}]}} =
        beam_lib:chunks(Committed, [debug_info]),
    Options.

%% CInf holds the absolute source path (OTP-29.1.1 lib/compiler/src/compile.erl:2756-2758).
%% Dbgi holds the {cwd, _} build directory that erlc adds (compile.erl:2639, compile.erl:2677-2678, compile.erl:3169).
%% beam_lib:cmp/2 ignores only CInf (lib/stdlib/src/beam_lib.erl:374), so this script compares the chunks itself.
compare(_Binary, none) ->
    new;
compare(Binary, Committed) ->
    New = chunks(Binary),
    Old = chunks(Committed),
    Ids = lists:usort(proplists:get_keys(New) ++ proplists:get_keys(Old)),
    case [Id || Id <- Ids, lists:keyfind(Id, 1, New) =/= lists:keyfind(Id, 1, Old)] of
        [] -> same;
        Differs -> {differs, Differs}
    end.

chunks(Binary) ->
    {ok, _Module, Chunks} = beam_lib:all_chunks(Binary),
    [Chunk || {Id, _} = Chunk <- Chunks, Id =/= "CInf", Id =/= "Dbgi"].

report(_Mode, Source, _Beam, error) ->
    io:format("error ~ts~n", [Source]),
    error;
report(_Mode, _Source, _Beam, {same, _Binary}) ->
    same;
report(verify, Source, _Beam, {Status, _Binary}) ->
    io:format("~ts ~ts~n", [status(Status), Source]),
    Status;
report(write, _Source, Beam, {Status, Binary}) ->
    ok = file:write_file(Beam, Binary),
    io:format("wrote ~ts (~ts)~n", [Beam, status(Status)]),
    Status.

status(new) -> "new";
status({differs, Ids}) -> ["differs ", lists:join(",", Ids)].

fail(Format, Args) ->
    io:format(standard_error, Format, Args),
    halt(1).
