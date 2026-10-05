# Improvements

Lessons from wrong turns. Only **titles are loaded into agent context** — the agent reads full entries on demand when a title seems relevant to the current task.

If a lesson becomes a standing rule, **graduate it to `knowledge.md`** and remove it from here.

Detailed incident writeups with full context live in `postmortems/`.

<!-- Entry format:

## YYYYMMDD: [Title that describes the lesson, not just the incident]

**Problem:** What went wrong

**Root Cause:** Why it happened

**Fix:** How it was resolved

**Guardrail:** What quick check prevents this next time

**Reference:** `path/to/file` or `postmortems/YYYY-MM-DD-slug.md`
-->

## 20261005: Keep explicit exception raise sites separate

**Problem:** An explicit exception raise site was grouped with BIFs that raise `badarg`, although it raises the class `error` with its first argument as the reason.

**Root Cause:** The documentation grouped operations by a similar failure shape instead of recording each operation's exact exception class and reason.

**Fix:** List explicit raise sites separately from `badarg` BIFs and state the class and reason that each site produces.

**Guardrail:** Compare every documented exception row with a pinned ERTS result before grouping it with another raise site.

**Reference:** `docs/erts-differences.md`
