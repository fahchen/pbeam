-module(ets_cause_probe).
-export([main/0, uncaught/0]).

%% Each case gives {Reason, ErrorInfo, Text}: the reason, the error_info
%% map of the first frame, and the text of erl_stdlib_errors:format_error/2.
%% The text of a select call needs ets:is_compiled_ms/1, which PBeam does
%% not have, so a select call gives no text.
uncaught() ->
    exit(main()).

main() ->
    Self = self(),
    Owner = spawn(fun() ->
                          Self ! {self(), ets:new(x, [protected]), ets:new(y, [private]),
                                  ets:new(z, [public])},
                          receive stop -> ok end
                  end),
    {Protected, Private, Public} =
        receive {Owner, A, B, C} -> {A, B, C} end,
    Set = ets:new(set, [set]),
    true = ets:insert(Set, {k, 1}),
    Bag = ets:new(bag, [bag]),
    Deleted = ets:new(deleted, []),
    true = ets:delete(Deleted),
    Named = ets:new(ets_cause_named, [named_table]),
    Cases =
        [fun() -> ets:insert(nosuch, {a}) end,
         fun() -> ets:insert(1, {a}) end,
         fun() -> ets:lookup(make_ref(), a) end,
         fun() -> ets:lookup(Deleted, a) end,
         fun() -> ets:insert(Deleted, [{a}, {b}]) end,
         fun() -> ets:insert(Protected, {a}) end,
         fun() -> ets:insert(Protected, [{a}, {b}]) end,
         fun() -> ets:insert(Protected, [a]) end,
         fun() -> ets:insert_new(Protected, {a}) end,
         fun() -> ets:lookup(Private, a) end,
         fun() -> ets:member(Private, a) end,
         fun() -> ets:insert(Set, a) end,
         fun() -> ets:new(Named, [named_table]) end,
         fun() -> ets:lookup_element(Set, missing, 2) end,
         fun() -> ets:update_element(Set, k, {1, x}) end,
         fun() -> ets:update_element(Set, k, [{2, x}, {3, x}]) end,
         fun() -> ets:update_element(Set, k, [{0, x}]) end,
         fun() -> ets:update_element(Set, k, {1 bsl 59, x}) end,
         fun() -> ets:update_element(Set, k, {1 bsl 60, x}) end,
         fun() -> ets:update_element(Set, k, [{2, x} | bad]) end,
         fun() -> ets:update_element(Set, k, [{1, x} | bad]) end,
         fun() -> ets:update_element(Set, k, [bad, {1, x}]) end,
         fun() -> ets:update_element(Set, k, {2, x}, a) end,
         fun() -> ets:update_element(Set, k, {2, x}, {}) end,
         fun() -> ets:update_element(Bag, k, {2, x}) end,
         fun() -> ets:update_element(Deleted, k, {2, x}) end,
         fun() -> ets:update_element(Private, k, {2, x}, a) end,
         fun() -> ets:info(Deleted) end,
         fun() -> ets:info(Deleted, size) end,
         fun() -> ets:info(make_ref()) end,
         fun() -> ets:info(make_ref(), size) end,
         fun() -> ets:info(Private, size) end,
         fun() -> ets:select(nosuch, []) end,
         fun() -> ets:select(Deleted, [], 1) end,
         fun() -> ets:match_object(Deleted, '_') end,
         fun() -> ets:select_delete(Deleted, []) end,
         fun() -> ets:delete_all_objects(Deleted) end,
         fun() -> ets:select_delete(Protected, []) end,
         fun() -> ets:first(Deleted) end,
         fun() -> ets:delete(Deleted) end,
         fun() -> ets:insert(Public, {a}) end,
         fun() -> T = ets:new(order, []), R = make_ref(), {T < R, T > R} end],
    Results = [run(Case) || Case <- Cases],
    Owner ! stop,
    Results.

run(Case) ->
    try Case() of
        Value -> {value, Value}
    catch
        error:Reason:Stack ->
            [{M, F, Args, Info} | _] = Stack,
            ErrorInfo = proplists:get_value(error_info, Info),
            Text = case ErrorInfo of
                       _ when F =:= select; F =:= internal_select_delete -> skipped;
                       #{module := erl_stdlib_errors} ->
                           try erl_stdlib_errors:format_error(Reason, Stack)
                           catch _:_ -> format_crash
                           end;
                       _ -> none
                   end,
            {Reason, {M, F, length(Args)}, ErrorInfo, Text}
    end.
