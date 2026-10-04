-module(exception_probe).
-compile(nowarn_deprecated_catch).
-export([main/0, id/1, catch_stub/0]).

main() -> {try_value(?MODULE:id(ok)), catch_value(), catch_error(), try_stub(), catch_exit(), try_exit(), stack_is_list()}.

id(X) -> X.

catch_stub() -> catch stub().

try_value(X) -> try ?MODULE:id(X) of V -> {value, V} catch _:_ -> caught end.
catch_value() -> catch ?MODULE:id(ok).
catch_error() -> case catch stub() of {'EXIT', {R, _}} -> {exit, R} end.
try_stub() -> try stub() catch error:R -> {error, R} end.
catch_exit() -> catch exit(bye).
try_exit() -> try exit(bye) catch exit:R -> {exit, R} end.
stack_is_list() -> case catch stub() of {'EXIT', {_, S}} when is_list(S) -> true end.
stub() -> erlang:nif_error(undef).
