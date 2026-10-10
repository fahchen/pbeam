#!/usr/bin/env escript
%% Summarize the static candidates and the exercised operations of the Terra Counter reference run.
%% Usage: escript scripts/terra_inventory.escript <calls file> <BEAM list>
%% The calls file is the output of terra_calls:run/2 (scripts/terra_calls.erl). The BEAM list is target/examples/terra_counter/pbeam.beams, the modules that the example calls directly or through other modules (scripts/example_beams.exs).
%% Static candidates come from the BEAM files of the list: the BIFs in the imports chunks and in the bif and gc_bif instructions, the NIF stubs, and the instructions. Exercised operations come from the call counts: the modules with a called function, the called BIFs and NIFs, and the instructions of the called functions.
%% Run it on the pinned OTP of the reference run: erlang:is_builtin/3 and beam_disasm come from this OTP.
-mode(compile).

%% beam_disasm shows the float arithmetic instructions as {bif, Op, ...} (lib/compiler/src/beam_disasm.erl:1062-1076 of OTP-29.1.1). They are instructions, not BIFs.
-define(FLOAT_OP(F), (F =:= fadd orelse F =:= fsub orelse F =:= fmul orelse F =:= fdiv orelse F =:= fnegate)).

main([CallsFile, ListFile]) ->
    {ok, [{result, Result} | Terms]} = file:consult(CallsFile),
    {ok, ListBin} = file:read_file(ListFile),
    List = [binary_to_list(Line) || Line <- binary:split(ListBin, <<"\n">>, [global, trim_all])],
    Static = maps:from_list([{module(Path), {Path, functions(Path)}} || Path <- List]),
    Loaded = maps:from_list([{M, Path} || {module, M, Path} <- Terms]),
    Calls = [{M, F, A, N} || {call, M, F, A, N} <- Terms],
    CalledModules = lists:usort([M || {M, _, _, _} <- Calls]),
    Code = fun(M, F, A) -> maps:get({F, A}, code(M, Static, Loaded), []) end,
    CalledBifs = [Call || {M, F, A, _} = Call <- Calls, erlang:is_builtin(M, F, A)],
    CalledNifs = [Call || {M, F, A, _} = Call <- Calls, not erlang:is_builtin(M, F, A), is_nif_stub(Code(M, F, A))],
    CalledOps = lists:usort(lists:append([opcodes(Code(M, F, A)) || {M, F, A, _} <- Calls])),
    StaticFunctions = [{M, F, A, Is} || {M, {_, Fs}} <- maps:to_list(Static), {{F, A}, Is} <- maps:to_list(Fs)],
    StaticBifs = lists:usort([MFA || {_, _, _, Is} <- StaticFunctions, {call, MFA} <- bifs(Is)]),
    InstructionBifs = lists:usort([MFA || {_, _, _, Is} <- StaticFunctions, {instruction, MFA} <- bifs(Is)]),
    StaticNifs = lists:sort([{M, F, A} || {M, F, A, Is} <- StaticFunctions, not erlang:is_builtin(M, F, A), is_nif_stub(Is)]),
    StaticOps = lists:usort(lists:append([opcodes(Is) || {_, _, _, Is} <- StaticFunctions])),
    CalledBifSet = [{M, F, A} || {M, F, A, _} <- CalledBifs],
    CalledNifSet = [{M, F, A} || {M, F, A, _} <- CalledNifs],
    NotInList = [M || M <- CalledModules, not maps:is_key(M, Static)],
    io:put_chars(unicode:characters_to_binary([
        "# Terra Counter call inventory\n\n",
        io_lib:format("result of TerraCounter.main/0: ~tp~n", [Result]),
        io_lib:format("calls file: ~ts~nBEAM list: ~ts~n~n", [CallsFile, ListFile]),
        "## Static candidates (modules of the BEAM list)\n\n",
        io_lib:format("modules: ~p; functions: ~p; BIFs in external calls: ~p; BIFs in bif and gc_bif instructions: ~p; NIF stubs: ~p; instructions: ~p~n~n", [maps:size(Static), length(StaticFunctions), length(StaticBifs), length(InstructionBifs), length(StaticNifs), length(StaticOps)]),
        origins("modules by origin", [{M, P} || {M, {P, _}} <- maps:to_list(Static)]),
        "\n### NIF stubs\n\n", [[mfa(MFA), called(MFA, CalledNifSet), "\n"] || MFA <- StaticNifs],
        "\n### Instructions\n\n", words(StaticOps), "\n",
        "\n### BIFs in external calls that the run does not call\n\n", words([mfa(MFA) || MFA <- StaticBifs -- CalledBifSet]), "\n",
        "\n### BIFs in bif and gc_bif instructions (the call counters do not count them)\n\n", words([mfa(MFA) || MFA <- InstructionBifs]), "\n",
        "\n## Exercised operations (call counts of the run)\n\n",
        io_lib:format("modules with a called function: ~p; called functions: ~p; called BIFs: ~p; called NIFs: ~p; instructions of the called functions: ~p~n~n", [length(CalledModules), length(Calls), length(CalledBifs), length(CalledNifs), length(CalledOps)]),
        origins("modules with a called function, by origin", [{M, maps:get(M, Loaded, unknown)} || M <- CalledModules]),
        "\n### Called modules that the BEAM list does not have\n\n", words(NotInList), "\n",
        "\n### Called BIFs (count)\n\n", [[mfa({M, F, A}), io_lib:format(" ~p~n", [N])] || {M, F, A, N} <- CalledBifs],
        "\n### Called NIFs (count)\n\n", [[mfa({M, F, A}), io_lib:format(" ~p~n", [N])] || {M, F, A, N} <- CalledNifs],
        "\n### Instructions of the called functions\n\n", words(CalledOps), "\n",
        "\n### Instructions of the BEAM list that no called function has\n\n", words(StaticOps -- CalledOps), "\n"
    ]));
main(_) ->
    io:format(standard_error, "usage: terra_inventory.escript <calls file> <BEAM list>~n", []),
    halt(2).

module(Path) -> list_to_atom(filename:basename(Path, ".beam")).

%% The functions of a module: from the BEAM list, or else from the path of the run. A preloaded module has no path in code:all_loaded/0; its BEAM file is in the erts ebin directory.
code(M, Static, Loaded) ->
    case Static of
        #{M := {_, Functions}} -> Functions;
        #{} ->
            case maps:get(M, Loaded, non_existing) of
                preloaded -> functions(filename:join([code:lib_dir(erts), "ebin", atom_to_list(M) ++ ".beam"]));
                Path when is_list(Path) -> functions(Path);
                _ -> #{}
            end
    end.

functions(Path) ->
    case beam_disasm:file(Path) of
        {beam_file, _, _, _, _, Functions} -> maps:from_list([{{F, A}, Is} || {function, F, A, _, Is} <- Functions]);
        {error, beam_lib, _} -> #{}
    end.

%% The BIFs of a function: {call, MFA} for an external call to a BIF, and {instruction, MFA} for a bif or gc_bif instruction.
bifs(Instructions) ->
    lists:append([bif(I) || I <- Instructions]).

bif({call_ext, _, {extfunc, M, F, A}}) -> builtin(M, F, A);
bif({call_ext_only, _, {extfunc, M, F, A}}) -> builtin(M, F, A);
bif({call_ext_last, _, {extfunc, M, F, A}, _}) -> builtin(M, F, A);
bif({bif, F, _, _, _}) when ?FLOAT_OP(F) -> [];
bif({bif, F, _, Args, _}) -> [{instruction, {erlang, F, length(Args)}}];
bif({gc_bif, F, _, _, Args, _}) -> [{instruction, {erlang, F, length(Args)}}];
bif(_) -> [].

builtin(M, F, A) ->
    case erlang:is_builtin(M, F, A) of
        true -> [{call, {M, F, A}}];
        false -> []
    end.

%% A NIF stub is a function whose BEAM body calls erlang:nif_error/1 or erlang:nif_error/2.
is_nif_stub(Instructions) ->
    lists:any(fun nif_error_call/1, Instructions).

nif_error_call({call_ext, _, {extfunc, erlang, nif_error, _}}) -> true;
nif_error_call({call_ext_only, _, {extfunc, erlang, nif_error, _}}) -> true;
nif_error_call({call_ext_last, _, {extfunc, erlang, nif_error, _}, _}) -> true;
nif_error_call(_) -> false.

%% The instruction names of a function, without line and label. A test instruction gives the name of the test, for example is_ne, which is the BEAM opcode. A bs_match instruction gives one name for each command, for example bs_match:integer. A bs_create_bin instruction gives one name for each segment type, for example bs_create_bin:append.
opcodes(Instructions) ->
    lists:usort(lists:append([opcode(I) || I <- Instructions])).

opcode({line, _}) -> [];
opcode({label, _}) -> [];
opcode({bif, F, _, _, _}) when ?FLOAT_OP(F) -> [F];
opcode({test, Name, _, _}) -> [Name];
opcode({test, Name, _, _, _}) -> [Name];
opcode({test, Name, _, _, _, _}) -> [Name];
opcode({bs_match, _, _, {commands, Commands}}) -> [list_to_atom("bs_match:" ++ atom_to_list(element(1, C))) || C <- Commands];
opcode({bs_create_bin, _, _, _, _, _, {list, Segments}}) -> [list_to_atom("bs_create_bin:" ++ atom_to_list(Type)) || {atom, Type} <- segment_types(Segments)];
opcode(I) when is_tuple(I) -> [element(1, I)];
opcode(I) when is_atom(I) -> [I].

%% Each segment of bs_create_bin has six operands: the type, the segment number, the unit, the flags, the source, and the size.
segment_types([Type, _, _, _, _, _ | Rest]) -> [Type | segment_types(Rest)];
segment_types([]) -> [].

called(MFA, Called) ->
    case lists:member(MFA, Called) of
        true -> " (called)";
        false -> ""
    end.

mfa({M, F, A}) -> io_lib:format("~p:~tp/~p", [M, F, A]).

words(Items) -> lists:join(" ", [io_lib:format("~ts", [to_text(I)]) || I <- Items]).

to_text(I) when is_atom(I) -> atom_to_list(I);
to_text(I) -> I.

%% The origin of a module: erts for a preloaded module, the application name of an OTP or Elixir ebin directory without the version, consolidated for a consolidated protocol, terra for a Terra module of the example build, and example for the other modules of the example build (Counter, TerraCounter, pbeam_boot).
origins(Title, Modules) ->
    Groups = lists:foldl(fun({M, Path}, Acc) -> maps:update_with(origin(M, Path), fun(Ms) -> [M | Ms] end, [M], Acc) end, #{}, Modules),
    [Title, ":\n", [io_lib:format("- ~ts (~p): ~ts~n", [Origin, length(Ms), words(lists:sort(Ms))]) || {Origin, Ms} <- lists:sort(maps:to_list(Groups))]].

origin(_M, preloaded) -> "erts";
origin(_M, Path) when not is_list(Path) -> "unknown";
origin(M, Path) ->
    Dir = filename:dirname(Path),
    case filename:basename(Dir) of
        "consolidated" -> "consolidated";
        "ebin" -> hd(string:split(filename:basename(filename:dirname(Dir)), "-"));
        _ ->
            case atom_to_list(M) of
                "Elixir.Terra" ++ Rest when Rest =:= ""; hd(Rest) =:= $. -> "terra";
                _ -> "example"
            end
    end.
