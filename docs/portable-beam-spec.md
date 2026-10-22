# Portable BEAM runtime specification

Status: architecture baseline reconstructed from the design conversation; constant-return and bounded-call slices are implemented. PR #47 merged as `265f173` and loads the pinned Elixir IO module. PR #52 merged as `5f94923` and executes its compiled IO path through `:io.put_chars/2`. Task #42 runs both Hello World entries with the `pbeam` binary and the boot module `pbeam_boot` of `examples/hello_world`, through the OTP IO servers to the host write. Their stdout, stderr, and exit status match ERTS in a locale whose codeset is UTF-8.

Source: [Branch · Portable BEAM实现原理](chatgpt-conversation://6abe6693-526c-83ee-8922-0af41462fb49).

The source conversation and supplied handoff are design input. This specification records the decisions accepted for this repository and the current issue scopes. Its requirements are not evidence of implementation unless an acceptance check says so. Open questions remain explicit.

## 1. Goal

Run existing Erlang/Elixir `.beam` artifacts on a Rust runtime without ERTS, and package an application as a single native executable.

The normal Erlang/Elixir compiler remains part of the build toolchain. PBeam consumes its output; it does not introduce a language frontend or compile Erlang/Elixir source directly to native code.

The [Hello World contract](hello-world-target.md) is the first target: PBeam executes the Elixir entry, prints exactly `Hello, world!\n`, and exits successfully. Compilation, artifact inspection, and ERTS execution are preparation and do not satisfy this target. The pinned [Terra Counter contract](terra-target.md) follows as a broader application compatibility target: a real PTY interaction on macOS arm64, with a headless probe as an earlier check. PBeam execution and the reproducible reference harness remain pending. Compatibility grows from the instructions, BIFs, OTP modules, and host operations each target actually needs. This is not an initial promise of complete BEAM or OTP compatibility.

## 2. Architecture decisions

| Area | Baseline |
| --- | --- |
| Execution | Interpreter first; decode BEAM instructions once into an internal instruction representation. |
| Memory | Immediate terms where appropriate; `Arc<Value>` for shared, immutable compound values. |
| Binary data | Separate binary/bitstring representation and operations from general compound values. |
| Scheduling | One OS process, multiple scheduler threads, one scheduler per OS thread, work stealing. |
| ERTS alignment | Follow the ERTS implementation model and observable behavior for processes, scheduling, messages, and BIFs. Record each intentional departure and its reason in the task, the PR, and [ERTS differences](erts-differences.md). |
| Native extensions | Rust functions registered by module/function/arity (MFA), linked into the executable. |
| OTP compatibility | Keep required OTP BEAM modules and replace their native/ERTS boundaries only where the selected application needs them. |
| Platform access | Capability Host boundary for authorization, trusted provider selection, platform services, and asynchronous completion. See the proposed [security design](capability-security-design.md). |
| Security authority | Host-issued, process-scoped grants with distinct use and delegation authority. Protected requests and resource handles bind authority to runtime-authenticated identity. |
| Native trust | All in-process native code and dependencies are part of the trusted computing base. Static linking and Rust type safety do not establish extension isolation. |
| Security profiles | Trusted application and secure guest profiles use the same authorization path. Secure execution remains unavailable until its coverage and bypass acceptance gates pass. |
| Code loading | The bundle defines the loadable module set. Interactive mode loads a module on demand; embedded mode loads all modules at startup. Loaded code does not change (no code replacement). See [code loading](code-loading.md). |
| Packaging | One application bundle appended to the `pbeam` binary, with a 16-byte trailer that names the bundle. |
| Validation order | Hello World, then Terra Counter. |

Firefly is an architectural reference for separation of responsibilities, scheduler organization, and native integration. Its compiler pipeline is not part of this design, and the design does not require a Firefly dependency.

The security additions are design requirements for implementation after M2 acceptance. The [capability security design](capability-security-design.md) defines their guarantees and limits. The [migration guide](capability-security-migration.md) defines the terminal-first implementation gates. They do not establish current security support or require unused provider domains and crates.

## 3. Scope and exclusions

The initial runtime must support the selected application slice end to end: loading its BEAM modules, interpreting required instructions, executing processes, calling required native functions, accessing required host services, and running from an embedded bundle.

The initial scope excludes:

- A compiler frontend, Core/Kernel/SSA pipeline, LLVM/MLIR, AOT, and JIT.
- Arena allocation or a replacement process heap/GC implementation.
- Hot code upgrades, code replacement, code purge, multiple versions of a module, and loading a module from outside the loadable module set.
- Full `erl_nif`/Rustler ABI compatibility. One limited exception exists: the NIF ABI layer of `docs/adr/0001-unsafe-in-the-nif-abi-layer.md` loads a NIF shared library of NIF version 2.18 and gives it only the `enif_*` functions that a supported package calls.
- Complete OTP compatibility as a prerequisite for the first application.
- Rewriting BEAM instructions to replace native dependencies, or replacing an entire OTP application by default.

Rust procedural macros for native bindings, broad automatic type conversion, a standalone `pbeam run app.pbeam` command, and alternate memory backends are future extensions. Do not build their scaffolding before a consumer needs them.

## 4. Responsibility boundaries

The discussion proposes the following logical boundaries. They do not require creating every crate before its first consumer exists.

| Boundary | Responsibility |
| --- | --- |
| `pbeam-system` | Runtime-independent platform primitives. |
| `pbeam-term` | Terms, values, atoms, and term semantics. |
| `pbeam-binary` | Binary/sub-binary/bitstring storage, construction, and matching. |
| `pbeam-beam` | BEAM file parsing and validation. |
| `pbeam-bytecode` | Decoded instruction representation. |
| `pbeam-runtime` | Processes, interpreter integration, mailboxes, exceptions, and BIFs. |
| `pbeam-scheduler` | Runnable processes, reductions, scheduler threads, and work stealing. |
| `pbeam-native` | Native registry, call context, arguments, results, and extension registration. |
| `pbeam-native-otp` | Rust implementations for OTP NIFs required by the supported application slice. |
| Authorization | Runtime-authenticated subjects, scopes, grants, delegation, revocation, budgets, and protected request creation. Independent of concrete OS providers. |
| `pbeam-host` | Capability routing and providers for terminal, files, network, timers, clocks, and randomness used by the application. Providers execute only authorized requests. |
| Bootstrap/CRT | Embedded bundle, trusted policy/profile selection, initial grants, native registration, and entry process startup. |

Keep platform-specific calls behind System/Host rather than spreading them through terms, instructions, and BIFs. Keep bootstrap distinct from execution without introducing unused alternative runtime implementations.

Protected VM operations use the same authority model without going through an OS provider. Keep root policy issuance out of application native contexts. Add authorization modules within the existing crate when the first consumer needs them. A separate security crate is not a prerequisite.

## 5. Loading and interpretation

The loader reads the bundle, validates its metadata and modules, and decodes supported BEAM instructions into executable internal instructions. The bundle defines the loadable module set. In interactive mode, the code server loads a module of that set on demand. In embedded mode, the loader loads all modules of that set at startup. Loaded code does not change: there is no code replacement.

An external-call instruction such as `call_ext` identifies an imported MFA. It does not mean that the call leaves the BEAM VM. Resolve the target first. If the target has BEAM code, execute that code in the interpreter. Implement only the VM instructions, scheduler, process operations, BIFs, and native boundaries required by the executed path. Do not replace a BEAM function with a Rust shortcut.

Schedulers share code and modules through `Arc`. No scheduler mutates module code during execution.

A process holds its registers, stack, mailbox, execution position, and reductions state. The interpreter executes the supported bytecode and preserves the required BEAM behavior for calls, returns, exceptions, and message handling.

Unsupported bytecode, unresolved functions, and unsupported runtime operations must produce actionable failures rather than silently changing program behavior. The exact supported OTP/BEAM version and instruction/BIF inventory must be recorded before compatibility can be claimed.

Secure execution also requires bounded loader input and a complete inventory of reachable BIF, opcode, native, and protected service paths. Equivalent operations must apply equivalent authorization. Unknown effectful operations fail closed. Import scanning assists review but does not prove coverage of dynamic calls or message protocols.

## 6. Terms and binary ownership

The memory model uses shared immutable values backed by `Arc<Value>`, with immediate representations where appropriate. A copy of a compound term increments a reference count, and every term is `Send` and `Sync`. Processes may share immutable term storage; mutable execution state remains owned by the executing process. Thus a message, an ETS insert and lookup, and `persistent_term` share the stored term and do not copy it. A binary match context holds its shared binary and one atomic word that holds its bit position and its flag of a known position, and only the process that owns it changes the position.

Binary handling has its own representation for binaries, sub-binaries, and bitstrings. Construction and matching must retain the distinctions between byte length, bit length, and slice offsets. A binary cannot be reduced to an undifferentiated general value wrapper.

The design leaves room to change storage later, but v0 does not implement an arena, allocator framework, or configurable heap backend. Exact term layout and binary backing storage remain implementation decisions.

Shared immutable storage does not grant resource authority. Protected resource handles are opaque, holder-bound values with rights, generation, and revocation checks. Term copying does not delegate them. Define explicit transfer semantics and retained-memory charging before claiming secure resource or memory limits.

## 7. Scheduling and asynchronous work

Each scheduler runs on one OS thread. Processes initially run on their creating scheduler. Idle schedulers can steal runnable processes from other schedulers.

Reductions bound a process's interpreter turn so other runnable processes can progress. A process must never execute concurrently on two schedulers. Waiting processes become runnable when a message, timeout, or host completion satisfies their wait.

Process ownership follows ERTS. The process table holds every live process, including the running process. Each scheduler records its running process in a `current_process` field, as the ERTS scheduler data does. The run queue holds only processes that wait for a turn. A scheduler removes a process from the run queue when the process starts to run. The scheduler adds the process to the queue again only when the process is preempted or a message wakes it. A per-process state, such as the ERTS `ACTIVE` and `IN_RUNQ` flags, prevents a second queue entry for one process. The single-thread scheduler gives a process a budget of `CONTEXT_REDS` (4000) reductions at each schedule-in. The process uses the budget at the cost points of the ERTS JIT, and it goes to the end of the run queue when it switches out at an ERTS switch point. Row "Scheduling" of `docs/erts-differences.md` lists the cost points that PBeam implements.

Host work that can block must not block a scheduler thread. The intended asynchronous flow is:

1. A process requests a host or native operation.
2. The runtime records its continuation and suspends that process.
3. The host executes the operation asynchronously.
4. Completion enters a scheduler inbox/completion queue.
5. The scheduler makes the process runnable and resumes the call with its result.

Tokio was an example executor in the discussion, not a finalized dependency requirement. Completion ownership, cancellation, process exit, and shutdown behavior must be defined when the first asynchronous operation is implemented. Late completion must not resume a dead process or resume the same call twice.

The first asynchronous operation is `prim_tty:write_nif/2`, which ERTS runs on a dirty IO scheduler. Task #19 runs it on 10 dirty IO threads of the standard library, as the 10 ERTS dirty IO schedulers, with no other executor. The scheduler records the call on the process before a thread gets it, and the completion identifies the process and the call. Row "Dirty NIFs" of `docs/erts-differences.md` records the behavior. Task #20 defines the rest of the lifecycle. While the call runs, the scheduler handles the signals of the caller, as `erts_dirty_process_signal_handler` does. A signal that ends the caller drops the claim of the caller on the call: a call in the queue then does not run, and the scheduler drops the result of a call that runs, because the host call cannot stop. At the end of the run, the run drops all claims and does not wait for a call that runs. Task #847 generalizes the path to each NIF binding: a binding declares the job `Normal`, `CpuBound`, or `IoBound`, as the flags of an ERTS `ErlNifFunc`. A `CpuBound` call runs on a dirty CPU pool with one thread for each scheduler, and an `IoBound` call runs on the dirty IO pool. The pools belong to the node, so the schedulers of a run share them, and each scheduler keeps the inbox of the completions of its calls.

A process owns its security context, including its usable and delegable capabilities. The subject is the process. A realm, a process incarnation, and a budget account come with their first consumers (see "Transition 2 decisions" in `docs/capability-security-migration.md`). Migration carries that context. Spawn cannot exceed the parent's delegable authority or duplicate a budget. A supervisor restart is a new process with the capabilities that the supervisor can delegate at that time, so it cannot restore grants that were revoked from the supervisor.

Host intents become authorized requests only through runtime checks. Fix the subject, operation, parameters, resource identity, grant generation, incarnation, and budget reservation before submission. Recheck queued authority at a synchronized dispatch decision. Revocation before that decision prevents execution; later revocation cannot promise rollback. Completion must settle cost and release resources even when the original subject no longer exists.

Privileged OTP services require an explicit authenticated request contract. Checking only the service process's grants is insufficient. Generic messages and caller PID fields do not carry trusted caller authority. The migration guide defines the first protected IO service contract while preserving the compiled OTP path.

## 8. Rust-native extensions

### OTP and system dependencies

NIF compatibility means behavioral compatibility at each required native boundary. It does not mean copying OTP's native source or implementing every NIF in OTP. Static Rust bindings are the default. The limited NIF ABI layer of `docs/adr/0001-unsafe-in-the-nif-abi-layer.md` loads a NIF shared library only for a supported package that has no static binding. Support the NIF MFAs reached by the selected application and pinned OTP/host targets. Expand that set only when a supported application requires more behavior. Record the OTP release, target, observed arguments and results, errors, and side effects for each supported binding.

For required OTP applications, first run their original `.beam` modules. Resolve ordinary calls through the code server; dispatch calls that reach an ERTS primitive or registered native leaf to the corresponding PBeam implementation. Do not patch bytecode or replace an OTP application merely because it uses a native dependency. This applies to `:httpc`, `:ssl`, and `:ssh` as a strategy, not as a claim that any of them runs on PBeam today.

For example, an HTTPS call may traverse `:httpc` and OTP `:ssl` BEAM code before reaching crypto and socket operations. Preserve those BEAM layers where feasible; implement the required crypto/NIF and socket/ERTS boundaries in Rust. OTP `:ssh` likewise does not imply a dependency on a target machine's OpenSSH executable. A separately invoked external program is a distinct host dependency and needs its own explicit handling.

If an OTP subsystem proves impractical to support at its native boundary, replacing the subsystem requires a separate, evidence-backed compatibility decision. No such replacement is part of the baseline.

### Elixir declaration

The interface uses ordinary Elixir function calls:

```elixir
defmodule MyApp.NativeHash do
  use PBeam.Native

  defnative sha256(data)
end
```

The library is the Mix project at the repository root, and its single module is `PBeam.Native`. `use PBeam.Native` registers the module attribute `pbeam_native` with accumulation and persistence. `defnative name(arguments)` adds one `{name, arity}` pair to that attribute and defines a stub of the same name and arity, where the arguments are those of the head. Each argument must be a plain variable. A pattern or a guard is not a supported form, because PBeam dispatches at the entry of the stub.

The compiler writes the pairs into the `Attr` chunk of the BEAM file. PBeam reads that chunk at the load of a module and reports the pairs as the native declarations of the module. A bundler reads the same declarations to discover the required native implementations.

The declaration also adds the pair to the `nifs` attribute of the module, so the compiler writes the `nif_start` instruction at the entry of the stub. That instruction is where PBeam looks up a registered native function.

Executing the stub on ordinary ERTS raises `:pbeam_native_only` through `:erlang.nif_error/1`. PBeam does the same when no registered native function matches the declaration, so an unknown declaration cannot run unnoticed.

The declaration alone does not dispatch to a Rust implementation. Task #12 delivers the lookup at `nif_start`, task #13 delivers the declaration and the declarations that PBeam reads, and task #14 delivers the first registered native function.

### Rust registration and dispatch

A Rust extension declares a `Library` for one module and registers it with `register_library` before the run. The library lists the NIF functions of the module, and each of its `Binding` entries maps an exact MFA such as `Elixir.MyApp.NativeHash:sha256/1` to a Rust implementation with its job. The registry owns this mapping and rejects a second library of a module, a binding outside the functions of its library, and a second binding of a function. A binding accepts BEAM terms and returns a BEAM term or a defined runtime exception. Host operations are available through a call context; platform details stay in `pbeam-host`.

Registration is controlled by trusted build/startup code. Guest code cannot register or shadow protected runtime/security MFAs. Native entries declare operation authority, determinism, observability, execution class, and cost separately. The native context exposes checked operations and cannot issue root grants. An allowed native call does not authorize every effect inside the binding.

All in-process native adapters remain trusted, including adapters that use only the checked context by convention. Effectful dependencies need reviewed transport/provider paths or a separate isolation boundary. Build scripts and procedural macros have a separate build-time trust boundary. Runtime grants do not restrict their build-time OS access.

Resolve an imported MFA through the code server. Execute its BEAM implementation when one is available. Dispatch to a registered native implementation only when the target is explicitly declared as native or is a verified VM/NIF boundary. Native integration does not require a separate `PBeam.Native.call/3` instruction or BIF.

The bundler may use BEAM imports, `-nifs` declarations, `erlang:load_nif/2` calls, OTP source, and reference-runtime traces to find candidate native MFAs. These are evidence for an inventory, not proof that every imported or declared MFA is native or reachable. The build must compare required native MFAs with the linked registry. Missing required bindings must fail with the MFA and the supported OTP target. Dynamic call paths remain a documented coverage limit.

For OTP compatibility, register the native leaf or ERTS operation being replaced. A BEAM wrapper such as `:crypto.hash/2` should continue to execute as BEAM code when its implementation calls a lower native function. Dispatch must also account for native entry paths that do not use `call_ext`; the required paths are determined from the selected OTP version and application slice.

The initial native API needs a term argument interface, checked conversion for the types actually used, a context for constructing return terms, and a result/error contract. Incorrect arguments must become defined runtime errors rather than unchecked Rust indexing or conversion failures.

Keep the crates and source paths grouped by responsibility. The target layout is:

```text
crates/
  pbeam-native/
    src/                 # MFA registry and native call contract
  pbeam-native-otp/
    src/                 # OTP bindings, grouped by OTP module
      prim_tty.rs
      ...
  pbeam-host/
    src/                 # OS services, grouped by capability
      terminal.rs
      ...
```

An OTP binding keeps the exact OTP MFA and its conversion/error mapping in `pbeam-native-otp`. It calls a capability in `pbeam-host` for OS effects. Do not create a crate or module for every MFA. Add each crate and directory when its first consumer needs it; this layout does not require scaffolding all OTP bindings in advance. Keep BIFs in `pbeam-runtime` and BEAM implementations in their original modules.

All supported ways to invoke a declared native function must preserve the same dispatch behavior. The implementation must account for local calls, dynamic application, and function references if those paths occur in the supported application slice; the source's `call_ext` sketch alone does not settle them.

### Build integration

The proposed Mix configuration declares native crate paths:

```elixir
def project do
  [
    app: :my_app,
    version: "0.1.0",
    pbeam: [native: ["native/hash"]]
  ]
end
```

A native build step links the declared implementations into the application executable and registers them; the exact mechanism follows the native work. Startup does not locate or load extension `.so`, `.dylib`, or `.dll` files, except a NIF shared library that the NIF ABI layer of `docs/adr/0001-unsafe-in-the-nif-abi-layer.md` loads for a supported package.

Missing required implementations and conflicting native registrations must be diagnosed before application execution. The exact validation mechanism remains open.

Build-time inspection should inventory required OTP modules, imports, native declarations, and other observed native dependencies, then check them against bundled BEAM code and registered Rust implementations. An import alone does not prove that a function is native or that every dynamic call is reachable. Record such limits, and report a required third-party NIF with no replacement as unsupported rather than assuming its original native library can load. The one exception is a supported package whose NIF shared library the ADR 0001 layer loads; the build check verifies its library and functions against that layer.

Automatic `FromTerm`/`IntoTerm` conversions and attribute macros may follow when manual bindings become repetitive. Async native functions use the suspension/completion model above; they must not call blocking `block_on` on scheduler threads.

## 9. Bundle and executable build

The proposed command is:

```sh
mix pbeam.build
```

It performs these steps:

1. Compile the Elixir application with the existing toolchain.
2. Collect the Mix release and the modules of the start set.
3. Read native declaration metadata and configured native crate paths.
4. Write one bundle with the `pbeam_bundle` tool.
5. Build the `pbeam` binary for the selected target, append the bundle and the trailer, and write the application executable, illustrated as `dist/my_app` in the discussion.

The bundle contains identifying/version metadata, an entry MFA, compiler/OTP compatibility metadata, a module table, a resource table, and the module/resource payload. Compression was proposed, but its codec and necessity are not settled. The example `otp: 28` is illustrative, not a compatibility commitment.

Capability manifest entries request authority; the trusted launcher grants only what host policy permits. Startup identifies the selected profile, policy, and executable build. Add manifest fields with their build consumer. A manifest signature does not grant authority. The current toolchain pin is OTP 29.1.1 in `mise.toml`; neither an example field nor a pin establishes general compatibility.

Use one bundle that carries the whole release and the start set. Bundle parsing is independent of the trailer, so an external bundle can start the same way.

At startup a run without an argument finds the trailer, parses the bundle, extracts it into a new temporary directory, and replaces the process with the boot of the extracted release, so a signal reaches the boot as in a normal run. The boot initializes the atom, code, and native tables from the start set, starts the scheduler, and spawns the entry MFA. The boot removes the extracted directory at the exit. Required initialization must finish before application code runs.

The delivered executable must not require installed `erl`, `elixir`, external BEAM files, or separately distributed native extension libraries. Build-time Erlang/Elixir and Rust toolchains are still required.

## 10. Portability

Single executable means one application artifact per target platform. It does not mean one binary runs on every OS/architecture, or that every target avoids system libraries.

The discussion proposes macOS arm64/x86_64, Linux musl arm64/x86_64, and Windows MSVC arm64/x86_64 as a future target matrix. The first validation target is macOS arm64; runtime support remains unverified.

macOS system frameworks/libraries remain normal dependencies. Linux musl builds aim to avoid a target-machine glibc dependency where all linked dependencies permit it. Native crates that require external C libraries must be evaluated against the single-artifact requirement. Rust-native libraries such as rustls and RustCrypto are preferred where they satisfy application needs; specific network/crypto libraries are not mandatory in advance.

## 11. Acceptance checks

These checks operationalize the design; expand them only as supported behavior grows.

- [Implemented] Load checked-in BEAM fixtures; read `AtU8`, `Code`, `ExpT`, and required `ImpT` data; execute exported constants and a bounded local/external call chain with a small integer; reject malformed containers and unsupported instructions with diagnostics. This is a narrow fixture check, not general BEAM compatibility.
- [Implemented by #42] Run both compiled Hello World entries without ERTS through compiled Elixir IO and the OTP group-leader request/reply path. PR #52 verifies the compiled Elixir path through `:io.put_chars/2`. Tasks #55, #50, and #79 execute the OTP path through `user`, `user_drv`, and `prim_tty:write_nif/2` to the host stdout, and `pbeam_boot` starts the OTP IO servers. `tests/feature/hello.rs` runs both entries with the `pbeam` binary and matches the ERTS stdout, stderr, and exit status. This covers only the binary standard-output path in a locale whose codeset is UTF-8. The failures and their ERTS differences are in the "Failures" row of [Hello World target](hello-world-target.md). Do not intercept `Elixir.IO:puts/1` or `:io.put_chars/2` with direct host output.
- Demonstrate process scheduling, reduction-based yielding, message wakeup, and work stealing with multiple schedulers. Verify exclusive process execution.
- Exercise binary construction, sub-binary offsets, and non-byte-aligned matching wherever the selected application requires them.
- [Implemented] Call the declared SHA-256 native example from Elixir and compare its result with a known digest. Verify bad arguments and missing registrations fail predictably. The example is `tests/integration/native_sha256.rs`; `BeamProgram::validate_native_declarations` checks each declaration against the registry before a run.
- For the first OTP module requiring a native dependency, verify that its original BEAM code runs through the runtime's native dispatch without bytecode rewriting; document the native operation and any dependency-scan blind spots.
- For the first async operation, show that another process progresses while its caller waits, and that completion after caller exit is harmless.
- [Implemented] Build an executable, move it away from the build output, and run it without an installed Erlang/Elixir runtime or accompanying application files. Verify required embedded resources remain accessible. `tests/feature/serve.rs` runs the `examples/plug_hello` executable in a clean directory, and `tests/feature/example.rs` runs the `examples/terra_counter` executable with no `erl` or `elixir` on `PATH`; both compare the result with the pinned ERTS. [Bundle format](bundle-format.md) gives the layout.
- [Implemented by #3, #21, and #23] Run the pinned Terra scenario and compare its observable behavior with the same scenario on ERTS. Record required modules, instructions, BIFs, host services, and known gaps. [Terra reference](terra-reference.md) gives the reference run and the inventory, and [the Terra Counter example](../examples/terra_counter/README.md) gives the scope of the one executable.

Passing small fixtures does not establish Terra compatibility or complete OTP compatibility.

The post-M2 security gate uses the delivered terminal operation as its first consumer. Granted trusted-profile execution preserves the accepted OTP behavior. Denied guests cannot use native entry paths, direct service messages, forged handles, resource reuse, or delegated workers to bypass authority. Both the native terminal provider and a memory provider enforce the same policy. Secure guest execution remains disabled until all reachable operation paths and resource limits pass their specified checks. See the [migration acceptance gates](capability-security-migration.md).

Security implementation is milestone M6. The owner decisions of 2026-10-15 set the milestone order: M5, M11, M7, M8, M9, M6, M10. The owner decision of 2026-10-19 postponed M6 again after M9 Pinned Terra scenario closed, so M10 Multiple schedulers is next. M6 resumes only after a new owner decision. The landed capability checks of epic #305 stay. This documentation does not close milestones, change issue dependencies, or supply a second task tracker. Create and link focused implementation issues after design review. The asynchronous dispatch transition follows the M5 async tasks. Later native, bundle, Terra, and scheduler tasks (M7–M10) must preserve the accepted security contracts.

## 12. Unresolved details before implementation claims

- BEAM opcode/BIF subset and runtime compatibility with the pinned mise toolchains; selecting versions is not a support claim.
- Application startup semantics, dependency initialization, entry arguments, exit status, and shutdown policy.
- Required process semantics, including selective receive, timers, links, monitors, and exception behavior for the chosen slice.
- Native registry conflict policy, validation timing, call-path coverage, errors, resource ownership, and async cancellation.
- Bundle encoding/versioning, validation limits, and resource lookup semantics: resolved by task #15 and `docs/bundle-format.md`. Compression remains open and stays out until a measured need justifies it.
- Scheduler count policy for the initial macOS arm64 target.

Resolve each detail when its first implementation slice requires it. Add only the foundations that slice consumes; do not interpret the logical architecture or future platform matrix as a request to scaffold unused components.
