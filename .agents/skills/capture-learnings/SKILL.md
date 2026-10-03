---
name: capture-learnings
description: This skill should be used when the user asks to "capture learnings", "update knowledge", "save this rule", "/capture-learnings", or otherwise wants to persist a generalisable review convention discovered during a session into docs/agents/knowledge.md. Enforces abstraction, recurrence, good/bad examples, overlap detection, a capacity ceiling, and explicit user confirmation before any write.
version: 0.1.0
user-invocable: true
allowed-tools:
  - Read
  - Edit
  - Write
  - Bash(wc:*)
  - Bash(grep:*)
  - Bash(rg:*)
---

# Capture Learnings

Persist a generalisable rule into [docs/agents/knowledge.md](../../../docs/agents/knowledge.md). That file records only problem classes that recur in review. Context budget is tight: only durable, abstract, recurring rules earn a slot.

Trigger phrases: "capture learnings", "save this rule", "update knowledge", "/capture-learnings".

## Hard rules

1. **No file:line pointers.** The knowledge file stores rules, not pointers into the codebase.
2. **No one-off bug notes.** A fix for one bug belongs in `git` and in its task issue or PR.
3. **Good and Bad examples are mandatory in the proposal.** They prove that the rule is concrete. A rule without a `Bad` example is not abstract enough. Reject it and tell the user. The written entry follows the existing style of the file (see `references/rule-format.md`).
4. **Always confirm before writing.** Print the proposed entry, target position, and overlap analysis. Wait for an explicit `yes`.
5. **Capacity ceiling is 40 rules.** At or above the ceiling, prune before adding. The user picks what to drop.
6. **ASD-STE100.** Write the entry in Simplified Technical English: short sentences, one instruction per sentence, consistent terms.
7. **No manual line-breaks.** Write the entry's rule and every proposal line as one unwrapped sentence or bullet per line.

## Workflow (execute in order)

### Step 1: Identify candidate rules

Scan the recent conversation for:

- A user correction ("do not do X", "always do Y").
- A pattern the user explicitly endorsed.
- A review comment class that already occurred more than once.
- A root cause that generalises (for example, a runtime shortcut that departs from ERTS behavior).

### Step 2: Abstraction check

Can the rule be stated without a specific file, function, PR, issue, or commit?

- Pass: "Document every struct field with its owner and meaning."
- Fail: "In `Scheduler::deliver`, check the target first."

On failure, tell the user:

> "Candidate rule is too specific. It describes only [file/function]. Skipping. Restate it if a general rule exists."

### Step 3: Recurrence check

Will the situation recur in different code?

- Pass: "Compare new runtime behavior with ERTS before you choose a model."
- Fail: "Pin OTP to 29.1.1." (one-time event; it belongs in `mise.toml`)

On failure: "Candidate rule is one-off. Skipping."

### Step 4: Examples check

Both `Good` and `Bad` must exist. Derive `Bad` from the mistake that caused the correction. If no credible `Bad` exists, reject:

> "Cannot write a Bad example. The rule is not abstract enough. Skipping."

### Step 5: Overlap check

Read `docs/agents/knowledge.md` end to end. For each existing rule:

- Same prescription, different words: **merge**.
- Opposing prescription: **replace**, and explain why.
- Adjacent but distinct: **keep separate**.
- Unclear: **ask the user**.

Print:

```text
Candidate rule: <one-line statement>

Existing rules with potential overlap:
  1. "<existing heading>" - <merge | replace | distinct | unsure>
     reason: <one sentence>

Proposed action: <merge into #1 | replace #1 | new entry | skip>
```

### Step 6: Capacity check

```bash
grep -c '^## ' docs/agents/knowledge.md
```

If the count is 40 or more, list 5 prune candidates (see [references/rule-format.md](references/rule-format.md)), wait for the user decision, and prune before you continue.

### Step 7: Confirm

Print the proposal verbatim in the proposal template from [references/rule-format.md](references/rule-format.md), with the target position and the overlap decision. Wait for an explicit `yes`. If the user asks for edits, revise and ask again.

### Step 8: Write

`Edit` `docs/agents/knowledge.md`. Write only the entry (heading and rule), not the `Why`, `Good`, or `Bad` parts:

- Append the new entry at the end, unless it belongs next to a related rule.
- When merging, replace the existing entry body in place.
- When replacing, delete the old entry and insert the new one in the same position.
- Do not reorder or change unrelated entries.

Run `rumdl check docs/agents/knowledge.md`. Report the action (added / merged / replaced) and the new rule count. Do not commit; the `commit` skill owns commits.

## What not to capture

- Code facts with file paths, function names, or line references.
- One-off bug fixes.
- Project facts such as tool versions or commands. They belong in `mise.toml`, `README.md`, or `AGENTS.md`.
- Workflow rules for GitHub issues, labels, or PRs. They belong in `AGENTS.md`.
- Vague preferences without a concrete `Bad` example.
- Anything the user describes as "for this PR" or "for now".

## See also

- [references/rule-format.md](references/rule-format.md): entry template and judgement aids.
