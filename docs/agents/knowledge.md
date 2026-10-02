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
