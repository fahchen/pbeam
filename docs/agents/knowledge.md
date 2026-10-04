# Review Conventions

Read this file before implementation. Record only problem classes that recur
in review. Keep individual comments and one-time decisions in their task or PR.

## Domain concepts and code names drift apart

Use the same domain concepts across specifications, grammar, modules, and
identifiers. Choose names that describe the data and responsibility they own.

## Code boundaries follow operations instead of responsibilities

Group code by stable responsibility. Keep concepts with different owners in
separate modules, even when one operation uses them together.

## Format contracts remain implicit

Keep the supported binary grammar and layout near the code that reads it. Name
non-obvious format values in implementation code. Show supported forms with
their limits and representative byte examples.

## Function behavior lacks examples

Document every function with Rustdoc comments and a `# Examples` section. Show
representative input and output in Rust code fences. For parser functions, use
concrete byte forms. State important observable errors and limits.

## Runtime models depart from ERTS

Before you model a BIF, instruction, or term operation, run it on the pinned ERTS and record the observable result: return value, exit reason shape, messages sent, and limits. Follow that result strictly: do not copy a similar BIF, and do not add a limit or shortcut that ERTS does not have; change the PBeam design instead. Record only scope that is not implemented yet as an ERTS difference, never a workaround.

## Tests pass without the behavior they claim

A test must fail when its claimed behavior breaks. Drive behavior through compiled BEAM fixtures, and make each test double perform the step under test. Assert exact values from a pinned ERTS run, not only types.
