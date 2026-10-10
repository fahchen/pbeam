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

## 20261010: Recheck documented module state after a fixture closure change

**Problem:** The fixture closure started to load `code_server` and `error_handler`, but the documentation still said that they were not loaded. It predicted an unresolved external MFA error, but the call raised `badarg`. The PR also claimed that no runtime result changed.

**Root Cause:** The documented results were not run again after the closure change.

**Fix:** Update the code-loading and undefined-function rows and remove the stale PR claim.

**Guardrail:** When a change adds modules to a fixture closure, run again each documented result that depends on the load state of a module.

**Reference:** <https://github.com/fahchen/pbeam/pull/407#issuecomment-6099847541>

## 20261010: Replace verification placeholders before publishing

**Problem:** A PR description kept a placeholder in the Verification section after an earlier edit did not apply.

**Root Cause:** The final PR description was not checked in the repository context after the edit command ran outside the repository.

**Fix:** Replace every placeholder with the final run link and result before publishing the PR.

**Guardrail:** Read the final PR description from the repository context and check that no placeholder remains before publishing it.

**Reference:** <https://github.com/fahchen/pbeam/pull/356#issuecomment-6099017860>

## 20261010: Record deferred wake limits with triggers

**Problem:** A cross-thread revocation could remain queued while the scheduler waited on an idle host event source.

**Root Cause:** The implementation had no wake handle, and the review did not find a current consumer that required one.

**Fix:** Remove the incorrect synchronization claim, document the observable wake limit, and record the wake mechanism as backlog work with its trigger.

**Guardrail:** When concurrency work is deferred, document its observable limit and the condition that requires the follow-up mechanism.

**Reference:** <https://github.com/fahchen/pbeam/pull/357#issuecomment-6097751596>

## 20261010: Preserve descendant revocation after holder exit

**Problem:** A delegation chain could lose the record needed to revoke a surviving descendant when an intermediate holder exited.

**Root Cause:** The design removed capability copies with the holder but did not define the lifetime of delegation records separately.

**Fix:** Keep enough delegation ancestry to reach surviving descendants after an intermediate holder exits. Add this exit case to the revocation task.

**Guardrail:** Test revocation through an exited intermediate holder before accepting a delegation design.

**Reference:** <https://github.com/fahchen/pbeam/pull/314#issuecomment-6096908500>

## 20261009: Check complete follow-up coverage

**Problem:** The PR description omitted two follow-up tasks for discovered gaps and kept an incomplete owner action.

**Root Cause:** The follow-up list was not compared with the complete gap inventory before the PR was published.

**Fix:** Create a task for each discovered gap, link existing tasks, and update the PR description with the complete set.

**Guardrail:** Before publishing a PR, compare every discovered gap with its task or explicit owner decision and verify that the description lists no stale action.

**Reference:** <https://github.com/fahchen/pbeam/pull/270#issuecomment-6078067360>

## 20261009: Qualify stream-specific behavior in user-facing claims

**Problem:** A PR description said that piping the output hides the progress steps, but steps remain visible when only stdout is redirected and stderr remains a terminal.

**Root Cause:** The statement treated all piping and redirection as one output path without naming the stream that controls the behavior.

**Fix:** State that redirecting or piping stderr suppresses the steps.

**Guardrail:** When documenting terminal output behavior, identify the relevant stream and qualify each redirection case.

**Reference:** <https://github.com/fahchen/pbeam/pull/296#issuecomment-6081107031>

## 20261009: Preserve child exit status when guest signals change

**Problem:** A child process that exited with status 7 was reported as successful after the guest ignored `SIGCHLD`.

**Root Cause:** The host wait path treated `ECHILD` as an unchanged success status after automatic child reaping.

**Fix:** Isolate child reaping from guest signal changes and handle failed waits explicitly.

**Guardrail:** Test child exit statuses after each guest signal mode and never convert a failed wait into status 0.

**Reference:** <https://github.com/fahchen/pbeam/pull/322#issuecomment-6084149434>

## 20261009: Store the normalized program path

**Problem:** Port information returned bytes after a NUL even though the spawned program used only the prefix.

**Root Cause:** The launch path normalized the program name, but the stored port name kept the original binary.

**Fix:** Normalize the program path once and use the same NUL-truncated value for launch and port information.

**Guardrail:** Test program names with a NUL and trailing bytes and compare every reported name with the launched path.

## 20261009: Trace ERTS policy claims to source behavior

**Problem:** Documentation attributed an observed fallback to terminal support, but ERTS applies a file-descriptor policy before it calls the platform polling operation.

**Root Cause:** The explanation inferred platform behavior from the result instead of checking the pinned OTP source and the complete dispatch path.

**Fix:** State the ERTS policy that causes the fallback and cite the pinned source that implements it.

**Guardrail:** Before documenting an ERTS explanation, inspect the pinned OTP source and verify each claimed policy step against the active path.

**Reference:** <https://github.com/fahchen/pbeam/pull/332#issuecomment-6089418666>

## 20261006: Verify OTP hardware assumptions against source behavior

**Problem:** The clock documentation and test rationale assumed that ERTS returns the host TSC frequency when RDTSCP is available.

**Root Cause:** The implementation inferred ERTS behavior from hardware terminology instead of checking the pinned OTP source and running the reference on the target hosts.

**Fix:** Correct the documentation and keep the fixed test value only after confirming the pinned ERTS result on the supported hosts.

**Guardrail:** Before documenting host-dependent behavior, inspect the pinned OTP feature detection and compare the exact reference output on each supported host.

**Reference:** <https://github.com/fahchen/pbeam/pull/176#issuecomment-6005160493>

## 20261006: Separate BIF and BEAM instruction differences

**Problem:** A difference record said that `raise` was not implemented after the `erlang:raise/3` BIF became supported.

**Root Cause:** The record used the same short name for a BEAM instruction and a BIF without naming the operation type.

**Fix:** State the exact unsupported BEAM instructions and identify the implemented BIF as a separate item.

**Guardrail:** Name the operation type when a difference record contains a BIF and a BEAM instruction with the same name.

**Reference:** <https://github.com/fahchen/pbeam/pull/177#discussion_r4188965048>

## 20261006: Verify documented error behavior against the active dispatch path

**Problem:** The PR description named an unsupported BIF error, but the active dispatch path returned an unresolved external MFA error.

**Root Cause:** The documentation used an evidence location that did not handle the operation.

**Fix:** Run a compiled probe through the current runtime and document the observed error.

**Guardrail:** Trace each documented error from the operation's dispatch entry to its fallback before writing the claim.

**Reference:** <https://github.com/fahchen/pbeam/pull/181#issuecomment-6005540768>

## 20261006: Verify PR claims against the final diff

**Problem:** A PR description said that no existing test changed, but the diff changed an existing test helper.

**Root Cause:** The description was not checked against the final diff after the helper was refactored.

**Fix:** Replace the stale statement with a precise statement that identifies the unchanged assertions and the changed helper.

**Guardrail:** Check every change summary and verification claim against the final diff before publishing the PR.

**Reference:** <https://github.com/fahchen/pbeam/pull/182#issuecomment-6005541337>

## 20261007: Verify acceptance references against the current issue

**Problem:** A PR description cited an obsolete acceptance criterion and assigned it a requirement that the current issue does not contain.

**Root Cause:** The description reused an older issue interpretation without checking the current acceptance items.

**Fix:** Check the current issue and cite only the acceptance items that support the recorded decision.

**Guardrail:** Before merging a PR description, compare every acceptance reference and requirement claim with the current issue text.

**Reference:** <https://github.com/fahchen/pbeam/pull/191#issuecomment-6008923646>

## 20261006: Keep semantic branches explicit

**Problem:** A one-line closure compressed proper-list and improper-list behavior into one expression.

**Root Cause:** The implementation optimized for brevity instead of showing the separate semantic cases.

**Fix:** Move each case into a multi-line match or a named helper with explicit branches.

**Guardrail:** When a branch handles distinct runtime cases, inspect each case on its own line before merging.

**Reference:** <https://github.com/fahchen/pbeam/pull/174#discussion_r4188917531>

## 20261006: Label measured verification by revision

**Problem:** A pull request described measurements from an older revision as if they applied to the current head and listed coverage totals without revision labels.

**Root Cause:** The verification section was updated without tying each result to the revision that produced it.

**Fix:** Name the tested revision for every measurement and identify the check that verifies the final head.

**Guardrail:** Before publishing a pull request, match each verification result to its revision and mark the final-head result.

**Reference:** <https://github.com/fahchen/pbeam/pull/187#issuecomment-6006626861>

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

**Problem:** An explicit exception raise site was grouped with BIFs that raise `badarg`, although it raises the class `error` with its first argument as the reason.

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
