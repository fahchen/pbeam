# Copilot review instructions

PBeam runs compiled Erlang and Elixir `.beam` files on a Rust runtime without
ERTS. Read `AGENTS.md`, `docs/agents/knowledge.md`, and
`docs/portable-beam-spec.md` before you review a change.

Write review comments in ASD-STE100 Simplified Technical English. Use short
sentences and one instruction per sentence. Report only problems that change
correctness, behavior, maintainability, or the rules below. Do not report
formatting that `cargo fmt` or `rumdl` controls.

## ERTS alignment

- Compare runtime behavior with ERTS. Processes, scheduling, messages,
  receive, monitors, links, and BIFs must follow the ERTS implementation
  model and observable behavior.
- Flag a departure from ERTS that the PR does not record with its reason.
- Flag a Rust shortcut that replaces a BEAM function. The runtime must
  execute BEAM code and replace only native or ERTS boundaries.
- Flag silent behavior changes. Unsupported instructions, BIFs, and
  operations must return an actionable error.

## Scope

- Each PR implements one task issue. Flag unrelated changes.
- Flag functions, fields, parameters, or configuration that have no current
  consumer, unless the PR records the decision.
- Flag new abstractions or dependencies where existing code or the standard
  library is sufficient.

## Rust code

- Every function needs a Rustdoc comment with a `# Examples` section.
  Parser functions show concrete byte forms.
- Every struct field needs a doc comment.
- Each instruction variant documents its encoding with a byte example and
  the Erlang source that produces it.
- Names describe the data and responsibility. Use the same terms as the
  specification and ERTS.
- Keep concepts with different owners in separate modules.
- Clippy warnings and formatting are merge blockers. Do not accept `allow`
  attributes that hide warnings.
- Application line coverage must stay at 100%. Flag excluded production code
  and tests that do not assert behavior.

## Tests

- Runtime behavior needs a compiled fixture whose result matches ERTS.
- Tests must not change runtime configuration that other tests read.

## Pull requests

- The PR uses the template, links its task, epic, and milestone tracker,
  and records decisions, alternatives, consequences, and exact verification.
- Commit messages have no attribution trailers.
