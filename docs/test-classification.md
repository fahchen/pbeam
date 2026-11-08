# Test classification

The report classifies each nextest case by purpose and observed dependencies. The default scope is `feature`. Pass `--scope unit` for the library test binary or `--scope integration` for compiled fixture tests. A case can have more than one purpose or dependency.

The reviewed macOS arm64 list has 312 feature, 1,208 unit, and 1,051 integration cases. All have reviewed facets. The inventories also record 17 unit cases and three integration cases excluded on macOS by source conditions. A Linux CI duration report listed 1,204 selected unit and 1,029 selected integration cases, all present in the inventories. That report does not include unselected cases, so it does not establish the complete Linux inventory.

The unit report uses `measurement` for performance probes. The scheduler release-cost probe also reads `/proc/loadavg` or runs `sysctl` to report host load. `real_time` marks a wait used by the test behavior, not an ordinary failure deadline. `test_process` identifies a child test executable. These facets do not change the selected tests.

The inventories are in `scripts/test_classes.exs`, `scripts/test_unit_classes.txt`, and `scripts/test_integration_classes.txt`. Purpose facets include pure logic, runtime integration, ERTS comparison or reference, host integration, interoperability, native extension, application end-to-end, CLI, measurement, and test helpers. For example, the terminal-mode `stty` test is an ERTS comparison and a host integration test. It uses a PTY. The other `stty` test uses pipes and `/dev/null`, not a PTY. The SSH tunnel test uses ERTS as a peer; it is not an ERTS comparison.

The dependency boundary covers the test harness, its helpers, and the observable host interactions of the programs and guest fixtures it launches. It covers processes, pipes, PTYs, files, network and epmd, host signals, non-toolchain host commands, build tools, native extensions, and semantic wall-time waits. OTP compilers such as `erlc` are build tools, not `host_commands`. It does not attempt to enumerate implicit kernel services or every dynamically loaded system library. `none` means none of the tracked external dependencies, not an absence of all host dependencies.

The `network` facet includes loopback sockets and local Unix socket pairs. It does not by itself mean that a test needs outbound network access. The `local_socket` facet identifies an observed local Unix socket; the scheduler tests use both facets for socket pairs. `epmd` identifies tests that start or use an actual EPMD service. `native_library` records direct native-library use, including the OpenSSL digest in BEAM loading. It does not by itself mean that the target host must provide a shared library.

Linux CI lists 311 feature cases; the terminal restore case is compiled on macOS only. Empty purpose or an `unknown` dependency means that a case needs further review. Do not treat a missing label as evidence that a case does not have that dependency. Each dependency label records an observed use on at least one applicable host. `erts_reference` records a case that calls both reference and PBeam probes; it does not by itself assert that their results are directly compared. The reference call can be conditional on the host. `erts_only` records a case that runs an ERTS probe without running PBeam.

Generate a full workspace test list on the platform to inspect:

```sh
mise exec -- cargo metadata --locked --no-deps --format-version 1 > /tmp/pbeam-metadata.json
mise exec -- cargo nextest list --locked --workspace --all-features --profile ci --message-format json > /tmp/pbeam-tests.json
mise exec -- elixir scripts/test_classes.exs /tmp/pbeam-metadata.json /tmp/pbeam-tests.json
```

Use the same files with `--scope unit` or `--scope integration` for those tests. The list must include the targets of the selected scope, so do not pass `--test` to `cargo nextest list`.

The report rejects a list that omits a target in the selected scope and reports newly observed identities as `unrecorded`. Known platform-specific cases are reported as `platform excluded`. Other absent identities are reported as `not listed`; review them before removing an entry.

Use `--cases` to print each case. Use `--purpose LABEL` or `--dependency LABEL` to filter that output. The filters show only known labels, so an incomplete inventory cannot prove that an unlabelled case lacks the selected facet.

```sh
mise exec -- elixir scripts/test_classes.exs --cases --purpose erts_comparison --dependency pty /tmp/pbeam-metadata.json /tmp/pbeam-tests.json
```

The selected count comes from the nextest filter status in the supplied list. It describes that invocation; it does not prove that a test ran. A partition changes the selected count. `--strict` fails if a listed case has an unknown purpose or dependency, an unrecorded case is found, or a recorded case is absent without a known platform condition. CI runs this strict check. The report uses the current platform by default; use `--platform linux` or `--platform macos` when the list came from another host.

The first CI test partition checks all three inventories after its test run, using the coverage build to list the cases. It keeps list-only profiles separate from test coverage. CI checks the combined application line coverage of all partitions.

For a pull request that changes only Rust files under `src/`, CI omits the individually reviewed ERTS-only checks in `scripts/ci_erts_only.txt`. They run ERTS reference probes and do not execute PBeam. Changes to tests, fixtures, examples, tool pins, or CI configuration run the full existing CI test set. A push to main and a daily scheduled run also use the full set. The existing measurement exclusions still apply to both sets.

CI checks the full and reduced nextest lists. It requires the difference to match the reviewed list exactly and checks each listed case against the ERTS-only inventory. New cases do not move out of the pull request gate automatically. The 100% application line coverage gate applies to both sets. Running the reference checks after merge or daily can delay detection of ERTS-only failures for a source-only pull request. In main run 42421930202, the 21 checks took 30.24 seconds of summed test-process time; this does not establish a wall-time saving.
