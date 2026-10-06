# Improvements

Lessons from wrong turns. Only **titles are loaded into context** — the agent reads full entries on demand when a title seems relevant to the current task.

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

## 20261006: Verify OTP hardware assumptions against source behavior

**Problem:** The clock documentation and test rationale assumed that ERTS returns the host TSC frequency when RDTSCP is available.

**Root Cause:** The implementation inferred ERTS behavior from hardware terminology instead of checking the pinned OTP source and running the reference on the target hosts.

**Fix:** Correct the documentation and keep the fixed test value only after confirming the pinned ERTS result on the supported hosts.

**Guardrail:** Before documenting host-dependent behavior, inspect the pinned OTP feature detection and compare the exact reference output on each supported host.

**Reference:** <https://github.com/fahchen/pbeam/pull/176#issuecomment-6005160493>

## 20261006: Verify task references in difference records

**Problem:** A difference row named a prerequisite task as the owner of a follow-up that the task does not implement.

**Root Cause:** The row reused a nearby task reference without checking its actual scope and the status of the follow-up work.

**Fix:** Remove the stale task reference and name the task that records the difference, or leave the owner unset until a task exists.

**Guardrail:** Check each issue reference in a difference record against the task scope before merging the documentation.

**Reference:** `docs/erts-differences.md`

## 20261006: Preserve failure artifacts until assertions finish

**Problem:** A test removed its temporary output directory before it asserted child-process success.

**Root Cause:** Cleanup ran before the assertion that could explain a failed child process.

**Fix:** Assert child-process success first and include the directory path in the failure message.

**Guardrail:** Keep diagnostic artifacts until all failure-producing assertions that use them have completed.

**Reference:** <https://github.com/fahchen/pbeam/pull/131#discussion_r4188922478>

## 20261005: Keep explicit exception raise sites separate

**Problem:** An explicit exception raise site was grouped with BIFs that raise
`badarg`, although it raises the class `error` with its first argument as the reason.

**Root Cause:** The documentation grouped operations by a similar failure shape instead of recording each operation's exact exception class and reason.

**Fix:** List explicit raise sites separately from `badarg` BIFs and state the class and reason that each site produces.

**Guardrail:** Compare every documented exception row with a pinned ERTS result before grouping it with another raise site.

**Reference:** `docs/erts-differences.md`

## 20261005: Put shared limits in their owning module

**Problem:** The same atom-name limit was defined in both the loader and a BIF module.

**Root Cause:** The limit was added where it was first needed instead of where the term owns it.

**Fix:** Define the limit once in the atom term module and import it from both consumers.

**Guardrail:** Search for an existing domain limit before adding a constant.

**Reference:** `src/term/atom.rs`
