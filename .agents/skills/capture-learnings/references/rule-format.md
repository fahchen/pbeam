# Rule Format and Judgement Aids

Reference for `capture-learnings`. Use these aids to decide whether a
candidate rule earns a slot in `docs/agents/knowledge.md`.

## Entry template

Match the existing entries of `docs/agents/knowledge.md`. The heading names
the problem class that review found. The body states the rule.

```markdown
## <Problem class, for example "Runtime models depart from ERTS">

<Rule: 1 to 3 sentences in ASD-STE100. State what to do and the most
important limit.>
```

Example of an existing entry:

```markdown
## Function behavior lacks examples

Document every function with Rustdoc comments and a `# Examples` section.
Show representative input and output in Rust code fences.
```

## Proposal template

Show this to the user in Step 7. Only the entry goes into the file. The
`Why`, `Good`, and `Bad` parts prove that the rule is concrete; they stay in
the conversation.

````markdown
Entry:

## <Problem class>

<Rule>

Why: <one sentence that names the concrete failure>

Good:

```rust
<2 to 6 lines that apply the rule>
```

Bad:

```rust
<2 to 6 lines that violate the rule>
```
````

Use `text` or `markdown` fences when the rule is not about Rust code.

## Concision budget

- Heading: 60 characters or fewer. Name the problem, not the fix.
- Rule: 3 sentences or fewer.
- Why: 1 sentence.
- Good / Bad: 2 to 6 lines each, no commentary inside the block.

If a rule needs more room, it does two things. Split it or drop the
secondary point.

## Abstraction test

All must hold:

- [ ] No file paths in the rule.
- [ ] No function names other than standard library, ERTS, or BEAM symbols.
- [ ] No PR numbers, issue numbers, or commit SHAs.
- [ ] The rule stays meaningful in a different module.

## Recurrence test

Any one must hold:

- It applies each time a developer writes a kind of code (parser,
  instruction, BIF, process operation, test).
- It applies each time a tool runs (`mise run check`, `erlc -S`, `gh`).
- The mistake comes from a misunderstanding that can repeat (for example,
  a Rust shortcut in place of ERTS behavior).

## Why the `Bad` example is mandatory

A rule without a credible `Bad` example usually:

- restates default behavior that nothing violates,
- is too vague to enforce, or
- states a preference without a failure mode.

Reject it and ask the user for the violation that caused the rule.

## Overlap categories

| Relation | Action |
| --- | --- |
| Same prescription, different words | Merge. Keep the clearer words and combine examples. |
| Same area, candidate is stricter | Replace. The stricter rule wins. |
| Same area, candidate is broader | Skip. The stricter rule covers the case. |
| Opposing prescriptions | Replace, and tell the user the old rule was wrong. |
| Adjacent but distinct | Keep separate. Add a new entry. |
| Unclear | Ask the user. Print both rules side by side. |

## Pruning candidates

Score each existing rule. The lowest score is the strongest prune candidate:

- A broader rule already covers a narrow rule.
- The rule refers to code or tools that the project no longer uses.
- The rule does not prevent a concrete failure.
- The rule covers a rare code path.

Print 5 candidates and let the user choose. Never prune automatically.

## Where other content goes

| Content | Location |
| --- | --- |
| Tool versions and commands | `mise.toml`, `README.md` |
| GitHub issue, label, and PR workflow | `AGENTS.md` |
| Architecture decisions | `docs/portable-beam-spec.md` |
| One-off bug fix notes | `git log`, task issue, PR |
| Personal communication preferences | user memory, not project files |
