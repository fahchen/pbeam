-module(group_probe).
-export([registry/0, guards/0, kind/1, driver/1]).

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

driver(Caller) ->
    receive
        {Group, {put_chars_sync, Enc, Chars, {Caller, Ref} = From}} when is_pid(Group), is_reference(Ref) ->
            Caller ! {request, {Enc, Chars}},
            Group ! {reply, From, ok}
    end.
