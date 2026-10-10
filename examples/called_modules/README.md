# Called Modules

This example runs `CalledModules.main/0` of `called_modules.ex`. It prints the number of characters of `héllo` with `String.length/1` of Elixir and with `string:length/1` of OTP. Each function calls `unicode_util:gc/1` of OTP. The example does not call `unicode_util` directly.

The `pbeam` binary loads only the BEAM files that it gets. Thus the task gives it each OTP and Elixir module that the example calls, directly or through other modules (see the [Hello World example](../hello_world/README.md)). With only the modules that the example calls directly, the `pbeam` run stops with `unresolved external MFA unicode_util:gc/1` and status 1.

## Run

From the repository root, this command builds the example and runs it with the `pbeam` binary:

```sh
mise run example called_modules
```

Add `--erts` to run the same entry on the pinned ERTS with `erl +S 1 -noshell`:

```sh
mise run example called_modules --erts
```

Each run prints these lines and exits with status 0:

```text
String.length: 5
:string.length: 5
```
