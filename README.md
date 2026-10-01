# PBeam

PBeam is a planned Rust runtime for executing existing Erlang/Elixir `.beam`
artifacts without ERTS and packaging applications as single native executables.

**Status: specification and delivery planning.** The interpreter, runtime, and
build tooling are not implemented yet. `mix pbeam.build` is a proposed command,
not an available installation or build instruction.

## Design

- Interpret existing BEAM artifacts produced by the normal Erlang/Elixir toolchain.
- Start with immutable, reference-counted terms and separate binary handling.
- Run multiple scheduler threads with reductions and work stealing.
- Link Rust-native functions by module/function/arity into the executable.
- Isolate platform services behind System/Host boundaries.
- Embed one application bundle in a Rust launcher.

Terra is the first intended compatibility target. Its exact project, revision,
and acceptance scenario must be pinned before implementation. Full OTP
compatibility, hot code upgrades, JIT/AOT, and traditional NIF ABI compatibility
are outside the initial scope. Each executable targets a particular platform;
system libraries may still be required.

See the [runtime specification](docs/portable-beam-spec.md) for architecture,
acceptance criteria, and unresolved decisions.

## Delivery

Work is managed in [GitHub Issues](https://github.com/fahchen/pbeam/issues)
and [milestones](https://github.com/fahchen/pbeam/milestones):

**Milestone tracking issue → epic issue → task issue.**

Each milestone has a dedicated tracking issue. Epics and tasks are assigned to
the GitHub milestone; native sub-issue links express the hierarchy and native
blocked-by links express dependencies. Milestone trackers and epics list their
issues as linked bullets. Start with
[the first milestone](https://github.com/fahchen/pbeam/issues/32).

Milestone titles use `[m1] Title`. Epic and task titles use `[m1-e1] Title`;
GitHub issue numbers identify individual tasks.

Implementation proceeds through small PRs linked to task issues with
`Closes #<issue>`. Each PR records implementation decisions, alternatives,
rationale, tradeoffs, and verification. Prerequisite PRs must be reviewed and
merged before dependent work begins. Issues and milestone checkpoints remain
open until their acceptance evidence is reviewed.

There are no file-based task plans or duplicate progress checklists. Follow
[AGENTS.md](AGENTS.md) when contributing and use the
[PR template](.github/pull_request_template.md) for review.
