# Implementation Conventions

Read this document before implementation. It records review feedback from the
BEAM parser work. Apply these rules to each implementation change.

## Names and responsibilities

- Use BEAM terms for parser stages. Use `parse` for bytes that produce a module.
  Use `tokenize` only when the input is source text and the output is tokens.
- The parsed result is `BeamModule`. Do not add a wrapper type without a current
  consumer and an architecture decision.
- Use `BEAM` in names and documentation. Do not use `S` as a substitute.
- Name parsed values after their meaning in the format. For example, use
  `declared_function_count` instead of `count`.
- Split files by responsibility. Keep terms, errors, and BEAM code parsing in
  their own modules.
- Do not add a wrapper, function, field, or registration step without a current
  consumer. Record an architecture decision when the design needs one.

## BEAM format and instruction docs

- Put the supported binary grammar beside the parser as inline BNF. Use the
  same names in the grammar and in the code.
- Document each supported chunk and each parsed part, including atoms, values,
  exports, and the Code header.
- Document each supported instruction. Give one byte-level example for each
  instruction. State its operands and limits.
- Give format numbers names in implementation code. Keep literal numbers in the
  grammar and byte examples when they show the format itself.
- Validate every declared field that affects parsing. Check counts, bounds,
  labels, and supported header values. Do not describe a field as checked when
  the parser ignores it.

## Review and verification

- Keep each commit focused and easy to review. Include only changes for the
  current task.
- Read every review comment. Fix it or record a clear reason to defer it. Keep
  issue, PR, and documentation records in sync when a decision or solution
  changes.
- Run the required checks. Report exact commands, results, and any check that
  could not run.
- Write project documentation and GitHub text in ASD-STE100 Simplified
  Technical English. Use short sentences and consistent terms.
