# Feature test classification

The inventory in `scripts/test_classes.exs` records nextest identities, purpose, and observed dependencies separately. A case can have more than one purpose or dependency. For example, the terminal-mode `stty` test is an ERTS comparison and a host integration test. It uses a PTY. The other `stty` test uses pipes and `/dev/null`, not a PTY. The SSH tunnel test uses ERTS as a peer; it is not an ERTS comparison.

The dependency boundary covers the test harness, its helpers, and the observable host interactions of the programs and guest fixtures it launches. It covers processes, pipes, PTYs, files, network and epmd, host signals, non-toolchain host commands, build tools, native extensions, and semantic wall-time waits. OTP compilers such as `erlc` are build tools, not `host_commands`. It does not attempt to enumerate implicit kernel services or every dynamically loaded system library. `none` means none of the tracked external dependencies, not an absence of all host dependencies.

The inventory records all 312 feature cases in the macOS arm64 list used for this review. The Linux list has not yet been checked. Empty purpose or an `unknown` dependency means that a case needs further review. Do not treat a missing label as evidence that a case does not have that dependency. Each dependency label records an observed use on at least one applicable host. `erts_reference` records a case that calls both reference and PBeam probes; it does not by itself assert that their results are directly compared. The reference call can be conditional on the host.

Generate a full workspace test list on the platform to inspect:

```sh
mise exec -- cargo metadata --locked --no-deps --format-version 1 > /tmp/pbeam-metadata.json
mise exec -- cargo nextest list --locked --workspace --all-features --profile ci --message-format json > /tmp/pbeam-tests.json
mise exec -- elixir scripts/test_classes.exs /tmp/pbeam-metadata.json /tmp/pbeam-tests.json
```

The report checks all Cargo test targets under `tests/feature/` against the nextest list. It rejects a list that omits a feature target and reports newly observed identities as `unrecorded`. A recorded identity absent on the current host is reported as `not listed`, because platform conditions can remove cases. Review those entries before treating one as stale. The inventory was first recorded from a macOS arm64 list; a Linux list must be checked before it can validate Linux identities.

Use `--cases` to print each case. Use `--purpose LABEL` or `--dependency LABEL` to filter that output. The filters show only known labels, so an incomplete inventory cannot prove that an unlabelled case lacks the selected facet.

```sh
mise exec -- elixir scripts/test_classes.exs --cases --purpose erts_comparison --dependency pty /tmp/pbeam-metadata.json /tmp/pbeam-tests.json
```

The selected count comes from the nextest filter status in the supplied list. It describes that invocation; it does not prove that a test ran. A partition changes the selected count. `--strict` fails if a listed case has an unknown purpose or dependency, an unrecorded case is found, or a recorded case is absent. It is an audit mode. CI uses the normal report to reject newly unrecorded feature cases; it reports platform-absent cases for review.

The first CI test partition checks the inventory after its test run, using the coverage build to list the cases. It keeps list-only profiles separate from test coverage. CI still runs the required test partitions and checks their combined application line coverage. A future frequency decision requires evidence for the affected behavior and its coverage. Do not move a test out of the PR gate only because it has a label.
