# Inspect Tuple

This example runs `InspectTuple.main/0` of `inspect_tuple.ex`. It prints the tuple `{:ok, 1}` with `IO.inspect/1`, as the last call of `TerraCounter.main/0` prints the result of the Terra run. Like the Terra Counter example, it first sets `standard_io` to binary mode.

`IO.inspect/1` calls `erlang:tuple_to_list/1` for the tuple, `erlang:integer_to_binary/2` for the integer, and `String.Tokenizer.tokenize/1` for the atom. The tokenizer gets its module from `elixir_config:identifier_tokenizer/0`.

## Run

From the repository root, this command builds the example and runs it with the `pbeam` binary:

```sh
mise run example inspect_tuple
```

Add `--erts` to run the same entry on the pinned ERTS with `erl +S 1 -noshell`:

```sh
mise run example inspect_tuple --erts
```

Each run prints this line and exits with status 0:

```text
{:ok, 1}
```
