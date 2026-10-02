# Knowledge

Read this file before implementation. These rules come from code review.

## Parse BEAM into a module

**Rule:** Use `parse` for binary BEAM input and return `BeamModule` directly. Use `tokenize` only for source text that produces tokens.
**Why:** The parser output is the module. A tokenizer or wrapper type would misstate the result without adding a current responsibility.

## Name format values and modules by responsibility

**Rule:** Use BEAM terms and semantic names such as `declared_function_count`. Keep code parsing, terms, and errors in modules that match those responsibilities.
**Why:** Reviewers must be able to connect a name and file to the format value or responsibility it represents.

## Keep foundations minimal

**Rule:** Add a function, field, wrapper, or registration step only when the current task has a consumer. Record an architecture decision for a design that needs unused structure.
**Why:** Unused structure expands the design before its behavior can be reviewed.

## Make the supported BEAM format clear

**Rule:** Keep inline BNF beside the parser. Document each parsed part and supported instruction with its operands, limits, and a byte-level example.
**Why:** The grammar, code, and tests must describe the same supported subset.

## Name and validate format values

**Rule:** Use named constants for numeric format values in implementation code. Keep wire values literal in BNF and byte examples. Validate every declared field that affects the supported format.
**Why:** Names explain the implementation while literal grammar and examples show the file representation.

## Use project language and evidence

**Rule:** Use `BEAM`, not `S`, in names and documentation. Write project documents in ASD-STE100 Simplified Technical English.
**Why:** Consistent terms and short sentences make decisions and limits clear.
