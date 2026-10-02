# Patterns

Implementation references. Source code and the runtime specification are the
authoritative records.

## Parse the supported BEAM subset

Parse the container, atoms, exports, and Code chunk into a `BeamModule`.
**Reference:** `src/beam/code.rs`

## Execute the parsed module

Resolve an exported function and execute the supported entry instructions.
**Reference:** `src/beam/module.rs`

## Keep runtime types separate

Terms and runtime errors have their own modules.
**Reference:** `src/beam/term.rs`, `src/beam/error.rs`

## Describe binary layouts beside parsing code

Keep inline BNF next to the parser and align grammar names with code names.
**Reference:** `src/beam/code.rs`, `docs/portable-beam-spec.md`
