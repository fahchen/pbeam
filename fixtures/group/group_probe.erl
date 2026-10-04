-module(group_probe).
-export([guards/0, kind/1]).

guards() -> [?MODULE:kind({put_chars, a}), ?MODULE:kind(atom), ?MODULE:kind({x})].

kind(X) when tuple_size(X) >= 2, element(1, X) =:= put_chars -> output;
kind(_) -> other.
