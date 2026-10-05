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

## 20261005: Put shared limits in their owning module

**Problem:** The same atom-name limit was defined in both the loader and a BIF module.

**Root Cause:** The limit was added where it was first needed instead of where the term owns it.

**Fix:** Define the limit once in the atom term module and import it from both consumers.

**Guardrail:** Search for an existing domain limit before adding a constant.

**Reference:** `src/term/atom.rs`
