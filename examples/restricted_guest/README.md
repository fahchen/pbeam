# Restricted Guest

This example runs `RestrictedGuest.main/0` of `restricted_guest.ex` with the Elixir policy module `RestrictedGuest.Policy` of `policy.ex`. The root process starts three workers, one at a time, with `spawn/3` of its own module `RestrictedGuest.Worker`. Each worker writes one line with `IO.puts/1` and tells the root process that it is done. The root process then prints the count.

## Run

From the repository root, this command builds the example and runs it with the `pbeam` binary and the policy:

```sh
mise run example restricted_guest --policy Elixir.RestrictedGuest.Policy
```

The task gives `--boot --policy Elixir.RestrictedGuest.Policy` to the `pbeam` binary. The boot of `pbeam_boot` runs with the boot authority, and the guest runs with the capabilities of the policy, so each effect entry checks the capabilities of its caller. Add `--erts` without `--policy` to run the same entry on the pinned ERTS, which has no policy:

```sh
mise run example restricted_guest --erts
```

The two commands give the same standard output, no standard error output, and status 0:

```text
worker 1: hello
worker 2: hello
worker 3: hello
main: 3 workers done
```

## Policy

The boot processes of `pbeam_boot`, for example `init`, `logger`, and the OTP IO servers `user` and `standard_error`, run with the boot authority, not under the policy. Before `pbeam_boot` calls `RestrictedGuest.main/0`, it registers `user` and `standard_error` as IO services with `pbeam_cap:serve/2`, and the root process enters the runtime with `pbeam_cap:enter_runtime/2`. Then `initial/1` gives the root process its capabilities, and each worker gets a copy of them. The policy gives only the capabilities that the guest uses:

- `process`: `spawn` of `RestrictedGuest.Worker`, `send`, and `monitor`. A worker sends `{:done, pid}` to the root process, and `io` monitors the IO server and sends it an IO request.
- `terminal`: `write`. The IO service entry of `user` passes an output request only from a process that holds `terminal` `write`, so the IO server writes the output of the guest with the authority of the guest.

A run without one of these operations, without the `modules` name `RestrictedGuest.Worker`, or with `delegable: false` for both capabilities does not write the output of the guest. The root process gets no other capability. A message from the guest to a boot process that is not an IO request that its capabilities permit raises `error:capability_denied`, for example a call of `logger` or of `supervisor:start_child/2` for `logger_sup`. `initial/1` returns `deny` for another entry, and `grant/3` denies each request of `pbeam_cap:request/2`. An operation that the policy does not permit raises `error:capability_denied`. The test `runs_restricted_guest_with_its_strict_policy_as_pinned_erts_and_denies_other_operations` of `tests/feature/example.rs` runs `restricted_guest_probe:denied/0` of `fixtures/policy` with the capabilities of this policy and shows each denial. See "Decisions of #338" and "Decisions of #463" in the [capability security migration](../../docs/capability-security-migration.md) for the reasons.
