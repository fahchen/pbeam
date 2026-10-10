%% Run TerraCounter.main/0 of the terra_counter example with the ERTS call counters on, and write the called functions as Erlang terms.
%% Usage: erl +S 1 -noshell -pa <code path of the example> -pa <directory of terra_calls.beam> -eval "terra_calls:run(\"<BEAM list>\", \"<output file>\"), halt()."
%% scripts/terra_reference.sh compiles this module and runs it in the pseudo-terminal of scripts/terra_reference.escript, with the code path of target/examples/terra_counter/erts.paths.
-module(terra_calls).
-export([run/2]).

%% Load each module of the BEAM list (target/examples/terra_counter/pbeam.beams, one file for each line), turn on the call counters, run TerraCounter.main/0, and write the terms to Out:
%% {result, Result} with the return value of main/0, {module, Module, Path} for each loaded module (code:all_loaded/0), and {call, Module, Function, Arity, Count} for each function with a count above zero.
%%
%% The module of each BEAM file loads by its name from the code path, so a consolidated protocol loads from the consolidated directory, as in the run of the example. The run then loads only the modules that the list does not have, so the counts show the calls of the scenario and not the calls of the code server that loads a module on its first call. A module that the list does not have and that the run calls loads through the code server, and the code server calls show in the counts.
%%
%% Terra sends the result to main/0 before its terminal owner restores the terminal (Terra lib/terra/runtime.ex:282-288, 356-359), so the counters stay on until the owner process stops. Thus the counts have the calls of the restore.
%%
%% The counters count calls of BEAM functions, BIFs, and NIFs in all processes (erlang:trace_pattern/3 with call_count). A BIF that the compiler makes an instruction, for example erlang:element/2 in a guard, has no count.
run(List, Out) ->
    {ok, Bin} = file:read_file(List),
    Modules = [list_to_atom(filename:basename(binary_to_list(Line), ".beam")) || Line <- binary:split(Bin, <<"\n">>, [global, trim_all])],
    ok = code:ensure_modules_loaded(Modules),
    erlang:trace_pattern(on_load, true, [call_count]),
    erlang:trace_pattern({'_', '_', '_'}, true, [call_count]),
    Result = 'Elixir.TerraCounter':main(),
    ok = await_owner(),
    erlang:trace_pattern(on_load, false, [call_count]),
    erlang:trace_pattern({'_', '_', '_'}, pause, [call_count]),
    Loaded = code:all_loaded(),
    Calls = [{call, M, F, A, N} || {M, _} <- Loaded, M =/= ?MODULE, {F, A} <- M:module_info(functions), {call_count, N} <- [erlang:trace_info({M, F, A}, call_count)], is_integer(N), N > 0],
    Terms = [{result, Result} | lists:sort([{module, M, Path} || {M, Path} <- Loaded, M =/= ?MODULE]) ++ lists:sort(Calls)],
    ok = file:write_file(Out, [io_lib:format("~0tp.~n", [Term]) || Term <- Terms]).

%% Wait until the registered terminal owner of Terra stops. A wait of more than 10 seconds stops the run with an error, and the harness reports a failed check.
await_owner() ->
    case whereis('Elixir.Terra.Terminal.Owner') of
        undefined -> ok;
        Owner ->
            Ref = monitor(process, Owner),
            receive
                {'DOWN', Ref, process, Owner, _} -> ok
            after 10000 -> error({terminal_owner_running, Owner})
            end
    end.
