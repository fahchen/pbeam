-module(literal_probe).
-export([main/0, build/1]).
main() -> {[flush], {error, enotsup}, same(?MODULE:build(a))}.
build(X) -> [X].
same([a]) -> same;
same(_) -> different.
