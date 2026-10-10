# Restricted Guest

This example runs `RestrictedGuest.main/0` of `restricted_guest.ex` with the Elixir policy module `RestrictedGuest.Policy` of `policy.ex`. The root process starts three workers, one at a time, with `spawn/3` of its own module `RestrictedGuest.Worker`. Each worker writes one line with `IO.puts/1` and tells the root process that it is done. The root process then prints the count.

## Run

From the repository root, this command builds the example and runs it with the `pbeam` binary and the policy:

```sh
mise run example restricted_guest --policy Elixir.RestrictedGuest.Policy
```

The task gives `--policy Elixir.RestrictedGuest.Policy` to the `pbeam` binary, so each effect entry checks the capabilities of its caller. Add `--erts` without `--policy` to run the same entry on the pinned ERTS, which has no policy:

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

`initial/1` gives the root process of `pbeam_boot:start/2` its capabilities, and each child gets a copy of them. The guest runs in this root process, and the OTP IO servers that write its output are its descendants. Thus the policy gives the capabilities that the boot and the guest use, and no other:

- `process`: `spawn` of `RestrictedGuest.Worker` and of the start wrappers `pbeam_boot`, `proc_lib`, and `erlang`, and `link`, `monitor`, `send`, and `inspect`.
- `terminal`: `write`, `read`, `raw_mode`, `window_size`, `setup`, and `query`, for `user_drv` and `prim_tty`.
- `signal`: `handle` of `sigcont` and `sigwinch`.
- `runtime`: `system_flag` and `load_nif`.
- `persistent_term`: `read` and `write`.

The root process gets no `env` and no `port` capability, and no `exit_other`, `group_leader`, or `halt` operation. `initial/1` returns `deny` for a root process that is not `pbeam_boot:start/2`, and `grant/3` denies each request of `pbeam_cap:request/2`. An operation that the policy does not permit raises `error:capability_denied`. The test `runs_restricted_guest_with_its_policy_as_pinned_erts_and_denies_other_operations` of `tests/feature/example.rs` runs `restricted_guest_probe:denied/0` of `fixtures/policy` with this policy and shows each denial. See "Decisions of #338" in the [capability security migration](../../docs/capability-security-migration.md) for the reasons.
