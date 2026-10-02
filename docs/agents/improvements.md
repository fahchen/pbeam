# Improvements

## 20261002: Make BEAM parsing easy to review

**Problem:** Review asked for meaningful format constants and names, responsibility-based files, parser terminology, a direct module result, and instruction layout examples.
**Root Cause:** Some code names and file boundaries did not show the format or responsibility. Some layout rules were not next to the parser.
**Fix:** Name format values, use semantic names, return `BeamModule`, separate code/term/error responsibilities, and keep BNF and byte-level examples beside the parser.
**Guardrail:** Read `knowledge.md` and `patterns.md` before implementation. Check every new field and helper for a current consumer.
**Reference:** `src/beam/code.rs`, `src/beam/module.rs`, `src/beam/term.rs`, `src/beam/error.rs`
