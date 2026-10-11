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

## 20261011: Qualify behavior claims by execution mode

**Problem:** Documentation stated that a denied `initial/1` call means that the root process does not start, but boot mode starts the root process before that call.

**Root Cause:** The statement did not name the execution mode that controls the observed behavior.

**Fix:** Limit the no-start statement to runs without boot mode. State that boot-mode denial stops the existing run.

**Guardrail:** When behavior depends on an execution mode, document each relevant mode and its observable result separately.

**Reference:** <https://github.com/fahchen/pbeam/pull/474#issuecomment-6104568265>

## 20261011: Reject guest callbacks before a privileged call

**Problem:** Formatting and map-order options could execute guest functions with boot authority, because the allowlist checked the MFA shape but did not reject nested callback values.

**Root Cause:** The check validated the called function but did not validate the full argument terms that the privileged API executes.

**Fix:** Validate the full option term before the privileged call. Reject a fun in a formatting comparator or a map-order option and return a denial before any callback can run.

**Guardrail:** Before a privileged API runs, validate all arguments recursively and reject guest-controlled functions, comparators, and callbacks, including ones nested inside options or containers.

**Reference:** <https://github.com/fahchen/pbeam/pull/466#issuecomment-6104226874>

## 20261011: Define authority at each runtime transition

**Problem:** The proposed sender check rejected the normal application-master relay, and boot-child inheritance granted boot authority across the application boundary.

**Root Cause:** The design inferred authority from the immediate sender and from parent-child inheritance instead of from the runtime transition that creates or forwards the request.

**Fix:** Record the original caller when a trusted relay forwards a request and authenticate that caller at the receiving boundary. Create the application root with the authority that `initial/1` defines.

**Guardrail:** Define each authority change at the runtime boundary that creates or forwards a request. Do not infer authority from parent-child inheritance when the runtime creates a new application boundary.

## 20261011: Stop a timed step at its deadline before reading queued input

**Problem:** A step of the Terra reference harness read queued port data after its deadline and could still pass, and continuous output could keep a step running past its bound.

**Root Cause:** The wait loop checked the deadline only when no input was available.

**Fix:** Return the timeout result when the deadline has passed, before the loop reads more input.

**Guardrail:** In each wait loop with a deadline, check the deadline before accepting queued input or starting more work.

**Reference:** <https://github.com/fahchen/pbeam/pull/432>

## 20261010: Check runtime paths and test names in documentation

**Problem:** Documentation described a capability requirement that the boot path did not use and named a test that did not exist.

**Root Cause:** The final documentation was not checked against the active boot path and current test names.

**Fix:** Trace each claim through the runtime path and replace each test reference with the exact current test name.

**Guardrail:** Before publishing documentation, verify behavior claims against the active path and resolve every referenced test name.

**Reference:** <https://github.com/fahchen/pbeam/pull/422#issuecomment-6100634869>

## 20261010: Check tail-call stack frames for indirect fun calls

**Problem:** An indirect tail fun call to `erlang:error/2` reported the tail-called function in the first stack frame instead of the surviving caller.

**Root Cause:** The implementation selected the active function without modelling the ERTS step that removes the tail-call frame before dispatch.

**Fix:** Pass the call context to `error_2` and select the surviving caller for an indirect tail call. Add compiled coverage for tail fun calls, tail applies, and a tail fun call at process start.

**Guardrail:** Compare stack-frame results for direct calls, tail applies, and indirect tail fun calls with pinned ERTS output before accepting error-frame logic.

**Reference:** <https://github.com/fahchen/pbeam/pull/415#issuecomment-6100194687>

## 20261010: Include hidden work in complexity claims

**Problem:** A PR description and effect inventory stated an `O(r)` policy revocation cost but omitted the sort of the matching copies.

**Root Cause:** The analysis counted selection work but did not count the ordering work needed for deterministic signal delivery.

**Fix:** State the cost as `O(r log r)` and define the variables in the affected documentation.

**Guardrail:** Expand every documented complexity claim into its complete operation sequence, including sorting and deterministic-order steps.

**Reference:** <https://github.com/fahchen/pbeam/pull/413#issuecomment-6100116408>

## 20261010: Check that edited sentences keep their referent

**Problem:** A documentation edit left "So does" pointing to the wrong sentence. The text then said that a request after the policy process exits returns `ok`, but the request raises `error:capability_denied`.

**Root Cause:** The edit changed the nearby sentences but did not check the transition that referred to them.

**Fix:** Replace the transition with an explicit statement of the denial behavior.

**Guardrail:** After you edit documentation, read each pronoun and transition again and verify that it still refers to the intended behavior. Prefer an explicit statement to a reference such as "So does".

**Reference:** <https://github.com/fahchen/pbeam/pull/408#issuecomment-6099846363>

## 20261010: Recheck documented module state after a fixture closure change

**Problem:** The fixture closure started to load `code_server` and `error_handler`, but the documentation still said that they were not loaded. It predicted an unresolved external MFA error, but the call raised `badarg`. The PR also claimed that no runtime result changed.

**Root Cause:** The documented results were not run again after the closure change.

**Fix:** Update the code-loading and undefined-function rows and remove the stale PR claim.

**Guardrail:** When a change adds modules to a fixture closure, run again each documented result that depends on the load state of a module.

**Reference:** <https://github.com/fahchen/pbeam/pull/407#issuecomment-6099847541>

## 20261010: Preserve bit offsets in UTF-8 bitstring reads

**Problem:** A UTF-8 bitstring read used the nearest lower byte boundary for an unaligned position, so it returned the wrong codepoint and could read past the remaining bits.

**Root Cause:** The implementation did not match the ERTS alignment and remaining-bit checks for UTF-8 extraction.

**Fix:** Align the read to the required whole-byte boundary and check that enough bits remain before consuming the UTF-8 sequence.

**Guardrail:** Add a compiled regression fixture for a UTF-8 read after a non-byte-aligned match and compare its exact result with pinned ERTS.

**Reference:** <https://github.com/fahchen/pbeam/pull/403#issuecomment-6099778845>

## 20261010: Validate arguments before capability checks

**Problem:** A denied process received `capability_denied` for invalid `system_flag(max_heap_size, Value)` values instead of the pinned ERTS `badarg` result.

**Root Cause:** The capability check ran before complete validation of the flag value and its range.

**Fix:** Validate the value and its minimum before authorization. Add denied-policy probes that compare invalid values with the pinned ERTS.

**Guardrail:** For every capability-gated operation, test invalid input from both allowed and denied callers and preserve the ERTS error precedence.

**Reference:** <https://github.com/fahchen/pbeam/pull/375#issuecomment-6098512127>

## 20261010: Separate assumed decisions from owner approval

**Problem:** A PR description said that the owner chose an option when the task record said that the option was only an assumption because the owner was away.

**Root Cause:** The description changed an unconfirmed assumption into an approved decision.

**Fix:** State that the option was selected under an assumption and that the owner has not confirmed it.

**Guardrail:** Before publishing a decision, check whether the cited owner or task record confirms it.

**Reference:** <https://github.com/fahchen/pbeam/pull/430#issuecomment-6101621215>

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
