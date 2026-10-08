# Capability security and host effect design

Status: design proposal for post-M2 implementation. No security mechanism described here is an implementation claim. Documentation task: [#210](https://github.com/fahchen/pbeam/issues/210).

This document consolidates the capability discussions of 2026-10-05 and the terminal-first migration review. The [runtime specification](portable-beam-spec.md) defines the runtime architecture. The [migration guide](capability-security-migration.md) defines implementation contracts and acceptance gates. GitHub Issues and the project board remain the sources of task status.

## Goal and guarantees

Keep the interpreter, immutable shared terms, process scheduler, MFA native dispatch, request/completion model, and single executable architecture. Add authorization at every boundary that uses host resources, changes protected VM state, or uses another subject's authority. Do not rewrite guest bytecode for security.

Under a correct runtime and trusted computing base (TCB), admitted guest code must not perform an unauthorized host operation, forge authority, or use another subject's protected resource. The first implementation has one realm. A realm identifier does not establish tenant isolation.

The TCB includes the loader, interpreter, scheduler identity management, authorization engine, resource table, protected adapters, providers, and all in-process native code and dependencies. Safe Rust, static linking, and capability manifests do not prevent native code from calling the OS. Capability-bound native code remains in the TCB. Untrusted native code requires a separate process, WASM, or another independently enforced boundary.

Do not claim protection from all VM defects, side channels, malicious in-process extensions, or unbounded native CPU work. Grants to both read data and send data permit data disclosure within those scopes. Runtime authorization cannot infer application business rules. Host effect records alone do not provide deterministic replay or exactly-once external effects.

## Authority and provider selection

A grant defines an operation, scope, holder, lifetime, and revocation relationship. Trusted host configuration selects a provider. These decisions meet at the same capability boundary, but provider identity is not a substitute for authority.

```text
Guest operation
  +-- protected VM operation --> authorization --> runtime state
  +-- host intent -----------> authorization --> authorized request --> provider
```

The launcher selects the policy and profile and issues initial grants. A bundle manifest requests capabilities; it cannot grant them. A signature proves origin or integrity, not permission. Guest configuration cannot select a provider that increases authority.

```text
startup grants <= host policy
child grants <= parent delegable authority
effective authority = valid grant AND request scope AND realm policy AND resource rights
```

Use authority and delegation authority are separate. A planner can have permission to create a narrowly authorized worker without permission to perform the worker's host operation itself. Delegation must reduce or retain scope and lifetime. It must obey the receiver's policy and must not copy an independent budget.

Policy configuration can use names or path patterns. Runtime authorization uses structured scopes and opaque records. Strings, integers, PIDs, references, and ordinary guest terms are not authorization credentials.

## Identity and ownership

Each process owns a security context with a subject, usable grants, delegable grants, and budget account reference. A subject contains a realm, process identity, and process incarnation. The runtime authenticates these values. Guest arguments cannot select an issuer or subject.

Scheduler migration transfers this context with process ownership. Neither OS thread-local state nor a worker's current PID supplies asynchronous request identity. A restart has a new incarnation and uses a validated child security template. It cannot recover revoked authority by rereading guest configuration.

Lease expiry uses a trusted monotonic host clock. A virtual guest clock cannot extend a grant. Root policy and grant issuance are not available through an application native context.

## Protected VM operations

Authorization covers send, spawn, signals, links, monitors, names, inspection, tracing, ETS, persistent terms, and other protected shared state. OS IO is not required for an operation to need authorization. Normal same-realm operations can be allowed by a profile, but their visibility and quotas must remain explicit.

Create an inventory of BIF, opcode, native, and service entry paths. Equivalent operations must receive equivalent checks. Metadata describes required authority, determinism, observability, execution class, and cost separately. A pure declaration is an audited statement, not a proof. Unknown effectful operations cannot enter secure execution.

The first secure profile must classify every reachable operation already supported by M2, including clocks, environment-derived data, OS information, terminal setup, signals, shared tables, and native library calls. Disabled domains still need an explicit rejection path. Resource limits cover loader input, atoms, retained terms and binaries, mailboxes, processes, native work, resources, and in-flight requests. Reductions do not preempt arbitrary native functions.

## Protected service requests

Checking only a privileged service's process grants creates a confused deputy. A low-authority caller must not gain terminal or file access by sending a message to a high-authority service.

A protected request entry authenticates the caller and binds a request grant to the target service, operation, validated parameters, budget reservation, and correlation identity. Execution uses that grant within the service ceiling. It does not automatically use the service's ambient authority.

Keep the trusted request record outside guest message fields. Each reviewed forwarding step must preserve the operation and its limits through an explicit adapter contract. Ordinary message copying does not propagate authority. A supplied caller PID or reference does not authenticate the requester. Do not use a global dynamic caller context for generic messages or GenServer calls.

The first consumer is the actual M2 IO chain through `io`, `group`, `user_drv`, and `prim_tty`. Keep the compiled OTP code and the exact native leaf dispatch. Define reviewed protected entries and forwarding adapters for this chain. If a required adapter cannot preserve the contract, secure access stays disabled until an evidence-backed compatibility decision is reviewed. Do not add a direct Rust output shortcut to bypass the chain.

Direct legacy service messages that can trigger privileged effects must be rejected or receive equivalent authentication. Unreviewed privileged services are unavailable to secure guests. Explicit host-issued service authority can support a business operation, but caller admission and operation limits remain required.

## Resources, revocation, and asynchronous requests

A protected resource record has a kind, realm, generation, provider binding, and revocation relationship. Its opaque handle binds rights and a holder to a grant. Internal slot identifiers do not grant access. Every use checks kind, holder, realm, rights, grant validity, and generation. Deserialization cannot restore live authority.

Arc cloning and message copying do not delegate a resource. Explicit delegation produces a receiver-bound handle with no greater rights or lifetime. The proposed secure profile rejects unauthorized handle transfer at send time. The implementation must cover handles nested inside compound terms and shared state, not only top-level message terms.

Process exit, close, and cancellation release owned resources and reservations. Cleanup can close or restore runtime-owned state without a live guest grant. It must not perform a new application business effect. Socket use rights remain distinct from permission to establish a connection.

An authorized host request fixes the subject, operation, normalized parameters, resource identity, grant generation, process incarnation, and budget reservation. Only the runtime authorization path creates it. Providers cannot accept a guest-supplied authorization context.

Reserve a shared budget atomically before submission. Queued work rechecks revocation and expiry at a defined dispatch decision. Revocation before that decision prevents execution. Revocation after permission to start prevents later operations and attempts cancellation; it cannot roll back an external write. Do not present a check followed by an unprotected dispatch as a linearization point.

Completion routes by request identity and incarnation. A dead or replaced process receives no guest delivery. Settle actual cost and clean up returned resources even when delivery is discarded. Release each reservation once. Approval, if added later, binds the same parameters, policy generation, budget, and expiry and requires revalidation before dispatch.

## Provider contract and operation domains

A provider executes authorized requests and must preserve the documented operation semantics. Replacement must preserve policy scope and resource isolation. Unsupported semantics fail closed. An object store provider does not acquire full filesystem semantics by implementing an interface with the same name.

| Domain | Planned operations and limits |
| --- | --- |
| Terminal | `terminal.text_write`, `terminal.input`, `terminal.tui_control`, and `terminal.raw`; ordinary text excludes privileged control sequences. |
| Filesystem | Read, write, create, delete, and metadata; directory/resource scope and explicit symlink and mount policy. |
| Network | Connect, listen, and resolve; destination, address, port, and protocol scope; raw socket and HTTP contracts remain distinct. |
| Environment and secrets | Explicit variable names and secret identities; no complete host environment inheritance. |
| Subprocess | Fixed executable identity, argument rules, cwd, environment, descriptors, and independently enforced OS restrictions. |
| Native and code | Exact MFA/implementation scope and trusted registration; guest code loading and native loading are separate decisions. |
| Runtime and shared state | Subject, service, namespace, and resource-scoped operations with quotas. |
| Clock and random | Source access and determinism contract; virtual sources do not control grant expiry. |
| Application tools | Named service and business operation limits; raw network permission does not establish a semantic tool contract. |

This is a domain inventory, not a requirement to implement all domains together. Terminal is the first provider consumer, even though earlier drafts listed six other v0 domains. Add filesystem, network, environment, process, native, and runtime operations when an accepted consumer needs them. Do not create unused provider traits or configuration parsers.

Filesystem authorization must constrain the operation itself through an authorized directory handle and relative access. Canonicalize-then-open is insufficient for race protection. Rename, links, deletion, devices, and platform-specific path behavior need separate acceptance evidence. Network providers must constrain actual connections as well as names; DNS, redirects, proxies, and pool reuse cannot bypass policy. Subprocess permissions alone do not constrain the resulting program's syscalls. Reject an isolation request when the platform cannot enforce its contract.

## Profiles and native integration

Trusted application and secure guest profiles use one authorization implementation. Trusted application grants are explicit and can be broad. Secure guest grants deny external access by default and admit only reviewed service paths and classified operations. Neither profile bypasses the gate for compatibility or speed. Do not enable secure guest execution before the full inventory, resource bounds, and bypass tests pass.

Terminal text output must filter or reject prohibited controls according to its documented contract. TUI and raw access need separate grants. The trusted profile preserves accepted M2 terminal behavior through explicit authority. Cleanup restores runtime-owned terminal state after cancellation or failure.

Native registration is a trusted build/startup operation. Reserve runtime and security MFAs. Application bindings cannot silently replace them. Record implementation identity, version, metadata, and execution class. A `native.call` permission must not become a blanket grant for effects inside a binding. Each protected runtime or host operation still needs its own check.

Review effectful dependencies for indirect IO, DNS, redirects, proxy environment, certificate files, and background connections. Place them behind an authorized provider, inject reviewed transports, or use an isolated worker. Build scripts, procedural macros, and dependency updates have a separate build-time trust boundary. Runtime permissions do not protect the build.

Use OTP-compatible denial results where the supported operation defines them. Define pBEAM-specific errors separately. Do not expose secrets, complete policy details, or another realm's resources in errors. Authorized compatibility uses pinned ERTS evidence. Permission denial uses explicit security tests because ERTS does not define this policy.

## Audit, replay, and later isolation

The common boundary can record run, realm, subject, operation, request identity, policy version, grant provenance, decision, result, cost, and causal identity. Audit omits sensitive bodies by default. Full recording is opt-in and needs access control, encryption, and retention policy.

Replay substitutes recorded responses and rejects missing or mismatched requests without live effects. Full deterministic replay also needs message order, scheduling, timers, random sources, external inputs, and code/native identity. Calling a model again or continuing with live providers creates a new run. Durable retries need idempotency or reconciliation; do not claim exactly-once effects.

## Design decisions and lessons

| Decision | Alternative and rationale | Consequence |
| --- | --- | --- |
| Authorize VM operations and host effects. | A BIF-only gate misses native leaves, opcode paths, and protected shared state. | Maintain a path inventory and bypass tests with each new operation. |
| Keep grants distinct from provider selection. | Treating a provider name as authority confuses routing with permission. | Provider changes must retain scope and semantic limits. |
| Use process, request, and resource identity. | Thread identity and a broker's current PID lose the original requester. | Explicit protected forwarding and incarnation checks are required. |
| Keep in-process extensions in the TCB. | Rust types and manifests cannot enforce a same-process OS boundary. | Isolate genuinely untrusted extensions separately. |
| Start with the delivered terminal path. | Building six unused domains postpones proof of the difficult service boundary. | Terminal protection precedes broad provider expansion. |
| Keep one authorization engine for both profiles. | A trusted bypass path would hide missing checks and split semantics. | Trusted compatibility tests also exercise the gate. |
| Add modules and types with current consumers. | Logical responsibility does not require an immediate crate split. | Extraction follows actual dependencies and must not expose root policy. |

## Open implementation decisions

Resolve the protected IO adapter placement, forwarding ownership, normalization rules, and denial mapping before enabling its secure entry. Resolve scope subset algorithms and configuration syntax before the first scoped configuration consumer. Resolve dispatch/revocation synchronization with the first queued effect. Resolve shared allocation charging before claiming retained-memory limits. Resolve provider-specific platform constraints before supporting that operation on a platform. Complete record/replay formats and isolated worker selection remain later designs.
