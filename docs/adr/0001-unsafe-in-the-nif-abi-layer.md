# ADR 0001: Allow unsafe only in the NIF ABI layer

- Status: Accepted
- Date: 2026-10-21
- Deciders: owner
- Related: #856, #865, #866

## Context

PBeam must load a NIF shared library through the ERTS NIF ABI. Calling a function pointer from `nif_init`, reading `ErlNifEntry` and `ErlNifFunc`, and calling `enif_*` symbols require `unsafe`. The project rule in `AGENTS.md` forbids new `unsafe` in runtime code. The owner approved an exception for the NIF binding layer on 2026-10-21.

## Decision

- `unsafe` is allowed only in the NIF ABI layer (`src/runtime/nif/abi/` while the runtime is one crate, or the `pbeam-nif-abi` crate after a split).
- The scheduler, interpreter, term, process, and loader callers stay in safe Rust.
- The layer exposes a safe Rust interface. No raw pointer and no C struct leaves it.
- Each `unsafe` block states its invariant and the C API contract that it relies on.
- The exception covers the ABI layer only. All other new code keeps the no-`unsafe` rule.

## Consequences

- PBeam can load a NIF shared library and call its functions.
- The native code shares the process with the runtime. A crash terminates the process. This is the OTP model. #865 records it in `docs/erts-differences.md`.
- The ABI layer needs tests for struct layout, symbol resolution, version checks, and error paths.
- The `AGENTS.md` rule points to this ADR.

## Alternatives

- No `unsafe`: no C ABI loading. Only static Rust bindings are possible. The owner rejected this for the first `.so` goal.
- A separate process for native code: crash isolation, but no in-process NIF semantics and a large serialization protocol. Rejected.
- Binary translation of the `.so` into Rust: a decompiler has no reliable semantics for optimized C. Rejected.
