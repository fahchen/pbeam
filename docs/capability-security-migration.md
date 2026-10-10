# Capability security migration and implementation

Status: proposed migration contract. No capability security mechanism is implemented by this document. Read the [security design](capability-security-design.md) and [runtime specification](portable-beam-spec.md) with this guide.

GitHub Issues and their native dependencies define task ownership, order, and status. This guide defines technical transition contracts and acceptance evidence. It is not a second task tracker.

## Baseline and sequence

Start implementation only after M2 acceptance. The owner decision of 2026-10-09 sets the milestone order: M4 Tooling and examples, M5 Interactive host I/O, M6 Capability security, M7 Linked native calls, M8 Embedded application, M9 Pinned Terra scenario, and M10 Multiple schedulers. This security work is M6. It runs in parallel with M5. Issues #7–#23 keep their numbers, and their titles use the new milestone tags. Record the order with native issue dependencies when the implementation tasks are approved.

The current toolchain pins OTP 29.1.1 in `mise.toml`. The discussion's OTP 28 example is not the implementation baseline. Recheck the pinned version before implementation. Capture M2's accepted command, output, errors, native entry paths, module set, and platform evidence as the migration baseline.

The current code is one `pbeam-runtime` crate. The specification's crate names describe logical responsibilities. Keep these responsibilities in modules until a current consumer requires a crate boundary. Do not create six provider interfaces or a security workspace in advance.

Preserve original OTP BEAM execution through `io`, `group`, `user_drv`, and `prim_tty`. Native dispatch belongs at the verified native entry, including `nif_start`. Do not replace `Elixir.IO`, `io:put_chars/2`, or a BEAM wrapper with direct Rust output. The current static `load_nif` model opens no native library file. It must remain distinct from a future dynamic native loading permission.

## Current code and required changes

| Current owner | Current role | Migration contract |
| --- | --- | --- |
| `src/runtime/process.rs` | Process execution state and group leader | Add the subject and security context to process ownership. |
| `src/runtime/scheduler.rs` | Processes, messaging, shared state, timers, and node resources | Authorize protected VM operations. Preserve identity during ownership changes. |
| `src/runtime/interpreter/native.rs` | Static NIF registration and dispatch | Add reviewed operation metadata and restrict the context available to adapters. |
| `src/runtime/interpreter/native/prim_tty.rs` | OTP argument conversion and terminal native bindings | Preserve OTP semantics for authorized calls. Send effects through the authorized boundary. |
| `src/runtime/host/` | Terminal operations, output, signals, clocks, and system information | Separate operation intent from authorized execution and provider selection. |
| `src/runtime/resources.rs` | Node data keyed by Rust `TypeId` | Retain this typed state store. It is not a capability resource table. |
| `src/runtime/tty.rs` | TTY records located through references | Add checked rights, holder, generation, and revocation without treating a reference as sufficient authority. |
| `src/runtime/regex_code.rs` and `src/runtime/pcre2.rs` | Compiled regular expressions and the C library interface | Review ownership, input cost, cleanup, and the native TCB. |

A reference can locate a runtime record without proving permission to use that record. The planned opaque resource handle must not be inferred from an integer, PID, or ordinary reference. Keep lookup identity, ownership, and authority separate.

## Transition 1: preserve behavior and expose the boundaries

First refactor the existing native and host path without changing its accepted behavior. Trace all callers before moving an operation. Keep OTP argument validation, result conversion, exception behavior, and side effects at their current verified boundaries.

List each executed BIF, native MFA, instruction fast path, VM operation, and host operation. For each entry, record authority requirements, determinism, execution class, observable effects, and cost. A pure declaration is reviewed metadata, not a proof about Rust or C code. The [effect inventory](effect-inventory.md) holds this list.

Include existing operations beyond terminal writes:

- Spawn, send, exit, links, monitors, aliases, group-leader changes, names, inspection, and global runtime information.
- ETS, persistent terms, shared resources, atoms, mailboxes, and retained binary storage.
- System and monotonic clocks, timer ordering, timezone and locale reads, and `uname` information.
- Terminal descriptors, mode changes, window size, termcap setup, output, and host signal dispositions.
- Regex compilation and execution, PCRE2 C code, linked terminal libraries, `libc`, and their dependencies.

A host signal disposition or locale change affects the OS process. It cannot be treated as an isolated process-local operation. Classify startup initialization separately from guest requests. Classify scheduler clock reads separately from guest-visible clock access. Moving an operation into a module called Host does not authorize it.

Acceptance: the M2 checks retain their exact results. The inventory names every existing effect entry and its execution owner. Unknown entries remain explicit and prevent secure-profile acceptance.

## Transition 2: process identity and authority

Use a subject with realm identity, process identity, and process incarnation. The first deployment can use one application realm. This does not establish tenant isolation.

The security context refers to usable grants, delegable authority, and a budget account. Only the trusted launcher issues initial authority within host policy. A manifest requests authority; it does not issue authority. Guest terms cannot construct grants or choose an issuer.

Separate use from delegation. A parent can create a restricted worker only within its delegation ceiling. A use grant alone does not grant delegation. Child authority, lease, and budget cannot exceed the validated delegation template.

Attach the context to the process. Scheduler migration moves it with process ownership. Neither thread-local state nor a worker's current PID is an authority source. A restart uses a validated template and a new incarnation. It cannot recover revoked authority.

Reserve child budgets from a shared parent account or reference that account. Never copy an independent full budget on spawn. Define resource ownership and quota failure behavior before enforcing each quota.

Acceptance: forged grants fail. Spawn cannot increase authority or duplicate a budget. Migration preserves the subject. Restart does not restore revoked grants. Trusted execution retains the accepted process behavior.

### Transition 2 decisions

Status: proposed in [#310](https://github.com/fahchen/pbeam/issues/310). The owner must confirm each row before implementation starts. These rows answer the "First grant" and "First budget gate" choices for transition 2. A later consumer that needs a different answer changes the row in its own design task.

The first consumer is the `process.spawn` check. `erlang:spawn/3`, `erlang:spawn_link/3`, and `erlang:spawn_opt/4` make each process through `Scheduler::spawn` (`src/runtime/scheduler.rs`), and that function has one caller (`src/runtime/interpreter.rs:1795`). An OTP supervisor restart is also a spawn by the supervisor process. The supervisor applies the start MFA of the child (`supervisor.erl:1034-1036`), and `proc_lib:start_link/5` calls `erlang:spawn_opt/4` with `link` and `monitor` (`proc_lib.erl:266-269`, `proc_lib.erl:511-514`). Thus one check covers spawn, `spawn_link`, and restart. The terminal leaf is not the first consumer: transition 3 owns it, and a check at the leaf alone permits the service bypass that transition 4 closes.

| Choice | Decision | Alternatives | Reason |
| --- | --- | --- | --- |
| First checked operation | Check `process.spawn` in `Scheduler::spawn`, before it takes a pid. The `link` and `monitor` options are parts of the spawn and get no separate check in transition 2. The check uses no reductions. | Check `prim_tty:write_nif/2` first. Also check `process.link` and `process.monitor` for the spawn options. | Delegation and budget division occur at spawn. A check that uses no reductions keeps the ERTS reduction counts of granted runs. |
| Subject and incarnation | The subject is the `ProcessId`. The realm is the run: one `Scheduler` is one realm. The `ProcessId` is also the incarnation, because `Scheduler::next_pid` never gives a pid again (row "Process and reference counters" of [ERTS differences](erts-differences.md)). Transition 2 adds no subject type. | Add `Subject { realm, process, incarnation }` now. | No transition 2 consumer reads a realm or a separate incarnation. Add the fields when a second realm exists, when PBeam uses pids again as ERTS does, or when completion routing of transition 6 needs them. |
| Security context location | Add one `ProcessState` field that holds the usable grant identifiers, the delegable grant identifiers, and the budget account identifier. | A scheduler table keyed by pid. Thread-local state is not permitted. | The process table owns `ProcessState`, so the context moves with process ownership. A separate table can keep a stale entry. One scheduler has no migration; the multiple-scheduler work verifies that migration keeps the context. |
| Scope syntax | A scope is an operation label of the [effect inventory](effect-inventory.md), as a Rust enum. Transition 2 has one value, `process.spawn`, with no parameter. No text syntax: the launcher gives Rust values. | Parse string labels from a configuration file. Add parameters now, for example the modules that a process can spawn. | No configuration consumer exists. The launcher and manifest work adds a text syntax with its consumer. |
| Subset rules | A grant set is within another set when each grant of the first set is in the second set. A spawn gives the child the delegable grants of the parent as its usable grants and as its delegable grants. A grant that the parent can only use does not go to the child. | The child also gets the usable grants of the parent. The child gets no grant until an explicit delegation call. | A use grant alone does not permit delegation. OTP code that spawns processes keeps working when the launcher issues delegable grants. An explicit call that gives a narrower child set waits for a consumer. |
| Issuer model | Only the trusted launcher issues grants: it gives a Rust policy value at run start. `BeamProgram::execute` keeps the trusted profile: the root process has `process.spawn` as a usable and delegable grant, and its account has no limit. A second entry takes a policy for security tests and later launchers. A grant has no term form. No BIF or native function gives or accepts a grant. A grant record has no issuer field. | Store an issuer identifier in each grant. Add a command-line flag or a policy file now. | The run has one issuer. A guest cannot name or construct a grant, so a forged term cannot add authority. |
| Lease clock | A grant can have an end time on the run clock, `Timers::now` (`src/runtime/timer.rs`): the host monotonic `Instant` from the run start. Guest time BIFs cannot change it. | The host system clock. No lease in transition 2. | The system clock can go back. A lease end is the only revocation that a run can measure without a host control channel, so transition 2 tests revocation with it. |
| Revocation representation | A run-wide grant table maps a `GrantId` to a record. A `GrantId` is a `u64` counter value that the run never gives again. The record holds the operation and the optional end time. The context holds identifiers, not copies. Each check reads the record. A grant that ended is not valid for each holder, also for each process that a later restart makes. An explicit revoke state comes with its first trigger. | Copy the grant into each process. Use one revocation epoch for the realm. | A copy can survive revocation, and a restart can copy it again. One epoch revokes every grant at the same time. |
| Quota unit | One live process of a budget account. | Reductions. Retained bytes. Messages in queues. | The process table gives an exact count. ERTS has a process limit with the same unit (`+P`). Bytes need shared allocation accounting. |
| Reservation ownership | The launcher makes the accounts. A child refers to the account of its parent. A spawn reserves one unit before it takes a pid. The unit returns when the process leaves the process table, at the `ERTS_CONTINUE_EXIT_FREE` step of its exit, where ERTS deletes the process table entry (`erl_process.c:14456`). | The parent reserves a part of its account for a child subtree. | A child never gets a copy of a budget. No consumer gives a worker a smaller budget yet. A sub-account comes with that consumer. |
| Excess behavior | A spawn that has no free unit raises `error:system_limit` and sends the "Too many processes" error report to the logger, as ERTS does at its process limit (`erl_process.c:12537-12540`). The trusted profile has no limit, so trusted runs do not change. | A PBeam-specific error. A runtime error that stops the run. Wait until a unit is free. | Spawn already defines this result, and OTP code handles it. |
| Shared allocation accounting | Transition 2 charges no bytes. Terms that processes share are not charged to an account. | The process that makes a term pays. Each holder pays the full size. Holders pay equal parts. | No transition 2 consumer limits memory. The first retained-memory quota decides this. Secure guest execution stays unavailable until then. |
| Denial result | A spawn with no valid usable `process.spawn` grant raises `error:capability_denied` before it takes a pid. The reason names no grant or policy. | `error:badarg`. `error:system_limit`. A runtime error that stops the run. | ERTS has no authorization policy, so the result is PBeam-specific. A distinct reason does not look like an argument error or a limit. Guest code can catch it. |

The implementation tasks of [m6-e1] are in this order:

1. Gate process creation with a launcher-issued spawn grant. Add the grant table, the context field, the policy entry, the spawn rule, and the check. Acceptance: each existing check passes with no change. A root with no `process.spawn` grant gets `error:capability_denied` from `erlang:spawn/3`, `erlang:spawn_link/3`, and `erlang:spawn_opt/4`, and no process is made. A root with a usable grant that it cannot delegate makes a child, and the spawn of that child is denied. A root with a delegable grant makes a child that makes a grandchild. No guest term, option, or argument adds a grant. The effect inventory rows of process creation and of the spawn BIFs record the check.
2. End spawn grants at their lease end. Blocked by task 1. Acceptance: a parent restarts its linked child before the lease end. After the lease end, each restart is denied, also for a supervisor restart through `proc_lib:start_link/5`.
3. Charge process creation to a shared budget account. Blocked by task 1, and parallel with task 2. Acceptance: a parent and its children use one account. With a limit of N live processes, the next spawn gets `error:system_limit` and the ERTS error report. An exit returns its unit, and a restart after the exit succeeds. A trusted run does not change.

## Transition 3: authorized terminal effects and providers

The first provider contract covers the accepted terminal path. Define text output separately from input, TUI control, and raw mode. Grant only the operations required by the accepted scenario. Ordinary untrusted text must not gain terminal control through escape sequences.

An operation intent contains the requested operation and parameters. The authorization engine checks the actual resource, subject, grant, policy, and budget before it creates an authorized request. The provider execution entry accepts only that runtime-created request. Guest deserialization must not create it.

Trusted host configuration selects the provider. A grant defines authority; it does not let the guest replace the provider. Provider replacement must preserve the supported operation and security contract. An unsupported provider fails closed.

Use a native terminal provider and a memory provider to verify replacement through the same authorization path. A memory provider is a test backend, not a reason to bypass policy. Trusted application and secure guest profiles share the gate. A trusted profile can have explicit broad grants, but does not establish a sandbox guarantee.

Terminal setup and restoration can require limited runtime service authority. Document that authority separately from guest text authority. Cleanup must restore terminal state on exit, error, and cancellation. It must not require authority from an exited guest or perform extra business operations.

Acceptance: a denied request invokes neither provider. A granted operation gives the defined provider result. Native output retains the M2 contract. Text access does not permit raw mode or control output. Failure and cancellation restore terminal state.

## Transition 4: protect the OTP service path

A low-authority process can send a request to a higher-authority IO service. Checking only the service process at the terminal leaf would permit an authorization bypass.

Authenticate the requesting subject at the reviewed protected entry. Create a request grant bound to the target service, operation, validated parameters, budget, and lifetime. The broker uses this grant within its service ceiling. It does not automatically use its full authority for guest work.

Keep trusted per-request records outside ordinary guest message terms. Trace the real pinned OTP `io` → `group` → `user_drv` → `prim_tty` path. Each reviewed forwarding step must preserve the authenticated request relation and limit any derived operation. Define how encoding, batching, and delayed writes relate to the original request before implementation.

Do not infer the caller from a PID supplied in an IO request. Do not use one global or thread-local caller context for a service that can process several requests. Do not make every ordinary send carry ambient authority. Generic `GenServer.call` does not establish business authorization.

Preserve the pinned OTP BEAM modules. Select a runtime service adapter or a reviewed explicit broker adapter only after the forwarding trace identifies the required boundaries. If the supported protocol cannot retain authenticated authority, reject that secure entry until an adapter contract exists.

Direct protocol messages must fail authorization or pass an equivalent authenticated check. Unadapted privileged services remain unavailable to secure guests. Same-realm membership alone is insufficient authorization.

Acceptance: forged caller PIDs, request terms, correlation references, and direct legacy messages cannot trigger privileged output. Concurrent requests keep separate identities. A low-authority caller cannot borrow a broker's wider authority.

## Transition 5: resource lifecycle and revocation

Retain internal resource records. Add opaque handles whose authority binds the resource key, holder, realm, rights, and grant. A slot plus generation, or an equivalent identity scheme, prevents stale lookup after resource reuse. Decide the concrete representation before implementing the first handle.

Check kind, generation, holder, realm, rights, lease, and revocation on every use. Open-time authorization is insufficient. An `Arc` clone or mailbox copy must not delegate authority.

Provide explicit delegation that binds the receiver and can only reduce rights and lifetime. Fix the secure-profile rule for forbidden transfer before implementation. Rejecting the send is the proposed rule. Resolve compatibility and the exact error contract with the pinned ERTS tests and security-specific tests.

Define close, process exit, cancellation, pending operations, and resource ownership together. Cleanup releases or restores resources. It cannot silently issue new privileged application effects. Persisted or cross-VM handle data cannot restore live authority.

Acceptance: forged integers, copied references, stale generations, wrong holders, excess rights, and revoked grants fail. Exit and cancellation release owned resources. Reuse cannot make an old handle valid again.

## Transition 6: asynchronous dispatch

Implement this contract when the first queued operation needs it. Do not create an unused executor framework.

Before enqueue, fix the subject, incarnation, operation, resource, and parameters. Validate authority and atomically reserve the shared budget. The authorized request records its request identity and grant or policy generation.

Define one dispatch decision that orders authorization against revocation. Revocation completed before this decision prevents execution. Recheck queued leases and grants at this point. Revocation after approved dispatch prevents later use and attempts cancellation; it cannot undo a write or a remote operation.

Completion routing checks request identity and process incarnation. A dead or replaced process receives no stale result. Settle actual cost and release the reservation even when guest delivery is discarded. Define ownership of a resource returned after cancellation so it cannot leak.

Acceptance: concurrent reserves cannot overspend. Queued revoked requests never reach the provider. Late completion cannot resume a new incarnation or resume a call twice. Dispatch and cancellation races leave no reservation or resource leak.

## Secure-profile acceptance

Keep secure guest execution unavailable until the supported inventory has complete authorization, bypass tests, and resource-bound evidence. Do not publish a general sandbox claim from one passing terminal test.

Bound hostile bundle decoding, literal expansion, atom creation, retained memory, mailboxes, process counts, in-flight work, and native input or output cost. Define secure-profile admission and quota failures separately from ordinary ERTS behavior. Do not add silent limits or shortcuts to the trusted compatibility path. An unimplemented operation gives a clear runtime failure and an entry in [ERTS differences](erts-differences.md).

In-process Rust and C extensions remain part of the TCB. A controlled `NativeContext`, static linking, safe Rust, and required-capability metadata do not prevent direct OS access. Review transitive dependencies, features, build scripts, and procedural macros. Build-time execution requires its own controlled environment.

Use security-specific negative tests as the authorization oracle. Use the pinned ERTS for granted-operation compatibility. Capture exact source locations and results for any instruction, BIF, term, or OTP protocol decision. Run `mise run lint` and `mise run check` for this documentation change. Implementation PRs retain the shared check and coverage gates.

## Later consumers and constraints

Carry these contracts into the M3, M5, and M7–M10 work. Read the current issues before editing their scope. The links below identify downstream integration points, not completed issue updates.

- M3 [#7–#9](https://github.com/fahchen/pbeam/issues/7) cover fairness, mailbox behavior, and exit. Preserve subjects, delegated spawn authority, shared budgets, and protected IPC.
- M5 [#19–#20](https://github.com/fahchen/pbeam/issues/19) cover suspension, cancellation, and late completion. Transition 6 follows these tasks and adds dispatch ordering, incarnation checks, cancellation cleanup, and budget settlement.
- M7 [#12–#14](https://github.com/fahchen/pbeam/issues/12) cover linked native registration, declarations, and SHA. Classify native entries, reserve protected MFAs, and retain the native TCB boundary.
- M8 [#15–#18](https://github.com/fahchen/pbeam/issues/15) cover bundles, manual launchers, Mix collection, and generated launchers. Treat manifests as requests and record launcher policy, profile, and code or native build identity.
- M9 [#21–#23](https://github.com/fahchen/pbeam/issues/21) cover Terra discovery, compatibility gaps, and release acceptance. Grant the terminal operations that the scenario uses and verify cleanup and protected IO forwarding.
- M10 [#10–#11](https://github.com/fahchen/pbeam/issues/10) cover cross-thread process ownership and work stealing. Move the security context with the process. Preserve exclusive execution and account for retained shared storage without using `Arc` sharing as authorization.

The existing downstream task range is [#7](https://github.com/fahchen/pbeam/issues/7) through [#23](https://github.com/fahchen/pbeam/issues/23). GitHub issue bodies and dependencies remain the source of their exact scope and order.

Add filesystem, network, environment, subprocess, and semantic service capabilities when a current consumer requires them. A permission taxonomy does not require unused provider implementations.

For filesystem access, use authorized directory-relative operations. Define symlink, mount, device, hardlink, and platform behavior. String normalization or canonicalize-then-open alone does not close path races. Test read, create, rename, and delete separately. Return unsupported when the platform cannot meet the claimed constraint.

For network access, distinguish raw connections from HTTP operations. Check resolved addresses and actual connection targets, each redirect, proxy routes, pooled connections, and IPv4 or IPv6 policy. Review transport-internal DNS, certificate files, environment reads, and background connections. A permission check around a client call does not constrain every internal effect.

Subprocess executable and argument restrictions do not contain the program's syscalls, hooks, scripts, or children. Secure subprocess support needs an independently enforced execution boundary. Do not grant shell strings as a constrained execution interface.

Audit, record/replay, controlled simulation, and isolated workers are later consumers. Audit defaults exclude secret bodies. Full record requires explicit retention and access rules. Replay must replace real effects and fail on missing or mismatched events. Complete replay also requires scheduler, timer, random, native, and input ordering. It is not established by a host log alone. No exactly-once external effect guarantee is implied.

## Choices required before each implementation

Resolve each choice in its consuming issue and design change. Do not encode an assumed answer as a placeholder implementation.

| Consumer | Required decision |
| --- | --- |
| First grant | Scope syntax, subset rules, issuer model, lease clock, and revocation representation. Transition 2: see [Transition 2 decisions](#transition-2-decisions). |
| First protected IO request | Entry list, trusted correlation storage, forwarding and batching rules, denial results, and service authority. |
| First resource handle | Opaque representation, generation reuse, transfer rules, ownership, and error mapping. |
| First budget gate | Quota unit, reservation ownership, excess behavior, and shared allocation accounting. Transition 2: see [Transition 2 decisions](#transition-2-decisions). |
| First queued operation | Dispatch linearization, policy changes, cancellation, cost settlement, and completion cleanup. |
| First constrained filesystem or network provider | Supported platform, enforceable semantics, dependency configuration, and real bypass tests. |
| First isolated extension | OS process, WASM, or another boundary and its tested guarantees. |
| First replay consumer | Event format, code identity, scheduler control, sensitive data policy, and mismatch behavior. |
