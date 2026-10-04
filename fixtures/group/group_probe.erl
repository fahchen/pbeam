-module(group_probe).
-export([main/0, registry/0, guards/0, kind/1, start_group/2, driver/1]).

main() ->
    Old = group_leader(),
    Driver = spawn(?MODULE, driver, [self()]),
    Group = spawn(?MODULE, start_group, [self(), Driver]),
    group_leader(Group, self()),
    self() ! noise,
    Result = io:put_chars(standard_io, [<<"Hello, world!">>, 10]),
    group_leader(Old, self()),
    Request = receive {request, R} -> R end,
    Kept = receive noise -> noise_ok end,
    {Result, Request, Kept}.

registry() ->
    Undefined = try register(undefined, self()) catch error:R1 -> R1 end,
    First = register(pbeam_probe_reg, self()),
    Again = try register(pbeam_probe_reg, self()) catch error:R2 -> R2 end,
    Other = spawn(?MODULE, driver, [self()]),
    Taken = try register(pbeam_probe_reg, Other) catch error:R3 -> R3 end,
    Name = process_info(self(), registered_name),
    {Undefined, First, Again, Taken, Name}.

guards() -> [?MODULE:kind({put_chars, a}), ?MODULE:kind(atom), ?MODULE:kind({x})].

kind(X) when tuple_size(X) >= 2, element(1, X) =:= put_chars -> output;
kind(_) -> other.

start_group(Parent, Driver) ->
    true = register(pbeam_probe_user, self()),
    put('$ancestors', [self(), Parent]),
    process_flag(trap_exit, true),
    State = {state, list, Driver, false, none, true, noshell, disabled, undefined, false, undefined, undefined, {[], []}, [], undefined},
    gen_statem:enter_loop(group, [], server, State).

driver(Caller) ->
    receive
        {Group, {put_chars_sync, Enc, Chars, {Caller, Ref} = From}} when is_pid(Group), is_reference(Ref) ->
            Caller ! {request, {Enc, Chars}},
            Group ! {reply, From, ok}
    end.
