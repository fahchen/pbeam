# Feature test classification

The default report classifies feature tests. Pass `--scope unit` to report the library test binary using the separate `scripts/test_unit_classes.txt` inventory. This unit inventory is incomplete: unreviewed cases have an empty purpose and the `unknown` dependency. `--strict --scope unit` fails until every listed case has reviewed facets. The existing CI gate still checks only the feature inventory.

The reviewed macOS arm64 list has 1,208 unit cases. The term, BEAM, bundle, embedded, error, scheduler, in-memory runtime container, ETS, zlib, external-term, and Erlang BIF groups account for 773 reviewed cases. The remaining 435 macOS cases are marked unknown. The inventory also records 17 Linux-specific cases. A Linux CI duration report listed 1,204 selected unit cases; all were present in the inventory. That report does not include unselected cases, so it does not establish the complete Linux inventory. Other missing identities stay unresolved.

The unit report uses `measurement` for performance probes. The scheduler release-cost probe also reads `/proc/loadavg` or runs `sysctl` to report host load. `real_time` marks a wait used by the test behavior, not an ordinary failure deadline. `test_process` identifies a child test executable. These facets do not change the selected tests.

Pass `--scope integration` to report the compiled fixture integration tests using `scripts/test_integration_classes.txt`. The inventory has 1,054 identities: 1,051 cases from the macOS arm64 list and three source-identified cases excluded on Apple hosts. A Linux CI duration report lists 1,029 selected cases, all recorded here. The report excludes unselected cases, so the Linux inventory may have more cases. The arithmetic, bignum, atom-to-binary, binary conversion, binary matching, ETS, and binary construction groups contain 149 reviewed cases. All other cases remain unknown. Strict integration reporting fails until those cases are reviewed.

The inventory in `scripts/test_classes.exs` records nextest identities, purpose, and observed dependencies separately. A case can have more than one purpose or dependency. For example, the terminal-mode `stty` test is an ERTS comparison and a host integration test. It uses a PTY. The other `stty` test uses pipes and `/dev/null`, not a PTY. The SSH tunnel test uses ERTS as a peer; it is not an ERTS comparison.

The dependency boundary covers the test harness, its helpers, and the observable host interactions of the programs and guest fixtures it launches. It covers processes, pipes, PTYs, files, network and epmd, host signals, non-toolchain host commands, build tools, native extensions, and semantic wall-time waits. OTP compilers such as `erlc` are build tools, not `host_commands`. It does not attempt to enumerate implicit kernel services or every dynamically loaded system library. `none` means none of the tracked external dependencies, not an absence of all host dependencies.

The `network` facet includes loopback sockets and local Unix socket pairs. It does not by itself mean that a test needs outbound network access. The `local_socket` facet identifies an observed local Unix socket; the scheduler tests use both facets for socket pairs. `epmd` identifies tests that start or use an actual EPMD service. `erts_process_may_be_conditional` records an ERTS process that runs only on some hosts or paths.

The inventory records all 312 feature cases in the macOS arm64 list used for this review. Linux CI lists 311 cases; the terminal restore case is compiled on macOS only. Empty purpose or an `unknown` dependency means that a case needs further review. Do not treat a missing label as evidence that a case does not have that dependency. Each dependency label records an observed use on at least one applicable host. `erts_reference` records a case that calls both reference and PBeam probes; it does not by itself assert that their results are directly compared. The reference call can be conditional on the host. `erts_only` records a case that runs an ERTS probe without running PBeam.

Generate a full workspace test list on the platform to inspect:

```sh
mise exec -- cargo metadata --locked --no-deps --format-version 1 > /tmp/pbeam-metadata.json
mise exec -- cargo nextest list --locked --workspace --all-features --profile ci --message-format json > /tmp/pbeam-tests.json
mise exec -- elixir scripts/test_classes.exs /tmp/pbeam-metadata.json /tmp/pbeam-tests.json
```

Use the same files with `--scope unit` to report the library tests. The list must include the library binary, so do not pass `--test` to `cargo nextest list`.

The report checks all Cargo test targets under `tests/feature/` against the nextest list. It rejects a list that omits a feature target and reports newly observed identities as `unrecorded`. The known macOS-only terminal restore case is reported as `platform excluded` on Linux. Other absent identities are reported as `not listed`; review them before removing an entry. The inventory was checked against macOS arm64 and Linux CI lists.

Use `--cases` to print each case. Use `--purpose LABEL` or `--dependency LABEL` to filter that output. The filters show only known labels, so an incomplete inventory cannot prove that an unlabelled case lacks the selected facet.

```sh
mise exec -- elixir scripts/test_classes.exs --cases --purpose erts_comparison --dependency pty /tmp/pbeam-metadata.json /tmp/pbeam-tests.json
```

The selected count comes from the nextest filter status in the supplied list. It describes that invocation; it does not prove that a test ran. A partition changes the selected count. `--strict` fails if a listed case has an unknown purpose or dependency, an unrecorded case is found, or a recorded case is absent without a known platform condition. CI runs this strict check. The report uses the current platform by default; use `--platform linux` or `--platform macos` when the list came from another host.

The first CI test partition checks the inventory after its test run, using the coverage build to list the cases. It keeps list-only profiles separate from test coverage. CI still runs the required test partitions and checks their combined application line coverage. A future frequency decision requires evidence for the affected behavior and its coverage. Do not move a test out of the PR gate only because it has a label.
