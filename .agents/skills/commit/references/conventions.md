# PBeam Commit and PR Conventions

Shared reference for the `commit` and `draft-pr` skills. Cross-check against `AGENTS.md` and `.github/pull_request_template.md` before drafting.

## Language

Write commit messages, PR titles, PR bodies, and issue text in ASD-STE100 Simplified Technical English. Use short sentences, one instruction per sentence, approved words, and the same term for the same concept. Keep code identifiers, commands, and error strings exact. Write each paragraph or bullet as one unwrapped line; do not add manual line-breaks mid-sentence.

## Commit subject grammar

```text
<type>: <description>
```

- `type` is lowercase and comes from the table below.
- Do not use a scope. The project history uses plain types.
- The description starts lowercase, uses the imperative mood, and has no trailing period. Code identifiers keep their casing.
- Do not use `!`. Record breaking or risky changes in the PR decisions.
- Keep the subject at 72 characters or fewer.
- Do not add issue numbers. The PR links the task issue.

## Types (Conventional Commits v1.0.0)

| Type | Use |
| --- | --- |
| `feat` | New runtime, loader, or command capability |
| `fix` | Defect fix |
| `refactor` | Restructure with no behavior change |
| `perf` | Performance improvement with no behavior change |
| `test` | Tests or fixtures only |
| `docs` | Documentation, Rustdoc, or agent guidance only |
| `chore` | Tooling, configuration, or repository maintenance |
| `build` | Build system or dependency change |
| `ci` | CI workflow change |
| `style` | Formatting only |
| `revert` | Revert of a previous commit |

## Worked commit subjects

Good:

- `feat: run BEAM processes on a cooperative scheduler`
- `refactor: keep the current process in the process table`
- `docs: document process instruction encodings and runtime helpers`
- `test: add compiled Hello World reference and inventory`

Bad:

- `Feat: Scheduler` - title case and no description.
- `feat(runtime): add scheduler` - the project does not use scopes.
- `feat: added the scheduler.` - past tense and trailing period.
- `fix: address review comments` - does not say what changes.
- `feat: scheduler [#49]` - issue number in the subject.

## Trailers

Do not add `Co-Authored-By`, `Claude-Session`, or other attribution
trailers to commits.

## PR title grammar

The PR title is the task issue title:

```text
[mN-eN] <Task title>
```

- Copy the task issue title exactly, for example `[m2-e1] Add a cooperative process runtime for OTP IO`.
- A PR without a task issue is an exception. Ask the user before you open one. If the user confirms, use a commit-style title (`docs: sync Hello World execution status`).

## PR labels

- Do not apply structural labels (`kind:*`) to PRs.
- Apply content labels that describe the actual scope: `enhancement` for new capability, `documentation` for documentation and planning, `bug` for defects. Multiple content labels are allowed.
- Do not use status labels on PRs. GitHub review state tracks review.

## PR body rules

Use `.github/pull_request_template.md` as the skeleton. Keep its sections and order: `# Task`, `## Behavior`, `## Implementation decisions`, `## Verification`, `## Review boundary`.

- `# Task`: `Closes #<task>`, the epic issue, the milestone tracking issue, and the prerequisite PRs with their merge commits.
- `## Behavior`: the observable result and the scope of the slice.
- `## Implementation decisions`: for each decision, the choice, the alternatives and rationale, and the consequences. Explain departures from the issue, the specification, or ERTS. State explicitly when no new decisions were necessary.
- `## Verification`: exact commands and results, acceptance evidence, and known gaps.
- `## Review boundary`: check only the items that are true.
- Do not leave template placeholder text. Remove HTML comments.
- Write each paragraph and each bullet as one unwrapped line. Do not add manual line-breaks mid-sentence; GitHub soft-wraps the text for display.
