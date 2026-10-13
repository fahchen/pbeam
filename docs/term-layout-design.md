# Term layout and the two mechanisms of row 6

Status: measurement and design record for task [#572](https://github.com/fahchen/pbeam/issues/572), a sub-issue of [#553](https://github.com/fahchen/pbeam/issues/553), under epic [#450](https://github.com/fahchen/pbeam/issues/450) and milestone tracker [#38](https://github.com/fahchen/pbeam/issues/38). Row 6 of the [term proposal](term-performance-design-codex-v2.md) is the design. This record changes no runtime code. The immediate atom root is [#553](https://github.com/fahchen/pbeam/issues/553), and the compact term is [#571](https://github.com/fahchen/pbeam/issues/571).

## 1. The measured layout at `3b3a21d`

The command is `RUSTC_BOOTSTRAP=1 mise exec -- cargo rustc --locked --lib -- -Zprint-type-sizes`. The toolchain is `rustc 1.99.0 (b940084d7 2026-09-28)` and `cargo 1.99.0 (5f94df478 2026-08-27)`, which are the pins of `mise.toml`. The revision is `3b3a21d2cea70f027c0274d4141893bacadc8cc0`, the head of `origin/main` at the measurement. The host is Darwin 27.0.0 arm64, Apple M1 Max, 10 CPUs.

| Type | Size, bytes |
| --- | --- |
| `Term` | 40 |
| `Option<Term>` | 40 |
| `Atom` | 24 |
| `Binary` | 32 |
| `Operand` | 40 |
| `Instruction` | 136 |
| `Fun`, `Map`, `Tuple`, `ConsRef` | 8 |
| `Target` | 56 |
| `Lambda` | 40 |
| `MatchState` | 40 |
| `map::Hamt`, `map::Repr` | 40 |
| `map::Node` | 32 |
| `FunThing` | 80 |
| `list::Cons` | 80 |
| `map::Child` | 80 |
| `ProcessState` | 848 |
| `Scheduler` | 1176 |

`Term` holds a discriminant of 8 bytes. The payload of each arm is:

| Arm | Payload, bytes |
| --- | --- |
| `Binary` | 32 |
| `Atom` | 24 |
| `Integer` | 8 |
| `Cons` | 8 |
| `Tuple` | 8 |
| `Map` | 8 |
| `Pid` | 8 |
| `Port` | 8 |
| `Reference` | 8 |
| `Fun` | 8 |
| `MatchContext` | 8 |
| `Nil` | 0 |

`Binary` is the largest arm, so `Term` is an 8-byte discriminant plus the 32-byte `Binary` arm, which is 40 bytes. `Option<Term>` is also 40 bytes, so the register slot does not grow: the compiler uses an unused bit pattern of `Term` for `None`, and the option tag costs no extra byte.

`Atom` is 24 bytes: the 16-byte `Arc<str>` fat pointer of the name, the 4-byte `index`, and 4 bytes of alignment padding (`src/term/atom.rs:48-55`). The name of an atom therefore lives in the `Arc<str>` inside every atom term. The `AtomTable` holds the same name again as a `String` key of its index map (`src/term/atom.rs:104-107`).

A scratch layout model projects the two mechanisms. The model mirrors the arm sizes that the compiler reports: `Binary` as 32 bytes, `Atom` as 24 bytes, and each other compound arm as 8 bytes. The model is not committed. Its only purpose is the projection.

| Enum | Size, bytes | `Option`, bytes |
| --- | --- | --- |
| Current (`Binary` 32, `Atom` 24) | 40 | 40 |
| Immediate atom only (`Atom` 4, `Binary` 32) | 40 | 40 |
| Boxed binary only (`Atom` 24, `Binary` 8) | 32 | 32 |
| Both mechanisms (`Atom` 4, `Binary` 8) | 16 | 16 |
| The proposal sketch (`Small`, `Float`, `Atom(u32)`, `Nil`, `Pid`, `Port`, `Boxed`) | 16 | 16 |

The immediate atom alone does not change `Term`, because `Binary` holds the 40-byte size. The atom mechanism and the compact term together reach 16 bytes.

## 2. The consumers of an atom name

The survey reads each non-comment line of `src/` and `tests/` that calls `.name()`. The count in `src/` is 173. One of them is `ExceptionClass::name` at `src/runtime/interpreter.rs:2773`, which reads no atom, so 172 sites in `src/` read an atom name. The count in `tests/` is 45. The groups below list every site as `path: line`.

A line can belong to two kinds, for example a `match` on a name whose arms are literals reads the name and compares it. The groups use one kind for each line, and the rule is a regular expression over the line text. The counts are therefore exact for the rule, and a reader who needs a different boundary can read the same list with another rule.

### 2.1 A literal comparison (93 sites)

Most of these sites compare the name with a tag such as `"true"`, `"flush"`, `"nopush"`, or a spawn option. A static atom has the same index in every table, so a comparison with a static name is an index comparison and needs no table. The sites that compare a non-static name, such as a module or a function name, need the table.

- `src/beam/code/instruction.rs`: 1621, 1668, 2268, 2296, 2381, 2395, 2400, 2408, 2415, 2435
- `src/runtime/capability.rs`: 1122, 1158, 1187
- `src/runtime/interpreter.rs`: 142, 952
- `src/runtime/time.rs`: 84
- `src/runtime/scheduler.rs`: 1805
- `src/runtime/interpreter/bif.rs`: 862, 906, 932
- `src/runtime/scheduler/policy.rs`: 555
- `src/runtime/interpreter/native/prim_tty.rs`: 138
- `src/runtime/interpreter/bif/binary.rs`: 610, 611, 612, 620
- `src/runtime/interpreter/bif/erts_internal.rs`: 355, 368, 441, 444
- `src/runtime/interpreter/bif/pbeam_cap.rs`: 129, 216
- `src/runtime/interpreter/bif/unicode.rs`: 108, 109, 169, 280, 281
- `src/runtime/interpreter/bif/re.rs`: 77, 84
- `src/runtime/interpreter/bif/table.rs`: 99, 152, 153, 263, 384
- `src/runtime/interpreter/bif/ets.rs`: 82, 98, 106, 107, 111, 583
- `src/runtime/interpreter/bif/erlang.rs`: 113, 122, 135, 161, 178, 279, 286, 381, 424, 780, 781, 859, 900, 902, 903, 910, 1025, 1026, 1027, 1030, 1031, 1035, 1631, 1723, 2171, 2172, 2497, 2975, 3158, 3163, 3233, 3235, 3237, 3257
- `src/runtime/interpreter/bif/io_service.rs`: 81, 95, 103, 104, 105, 109
- `src/runtime/interpreter/bif/os.rs`: 92, 93, 97

### 2.2 A print or a text conversion (10 sites)

`print_term` writes the ERTS text of a term (`src/term/print.rs:67`, `:144`, `:146`). `erlang:atom_to_binary/2` and `erlang:atom_to_list/1` write the bytes of the name (`src/runtime/interpreter/bif/erlang.rs:1602`, `:1632`, `src/runtime/interpreter/bif/erts_internal.rs:479`). The `module_loaded` and `function_exported` paths read the name as bytes for a comparison (`src/runtime/interpreter/bif/erlang.rs:2859`, `:2967`, `src/runtime/interpreter/bif/os.rs:107`). The `Fun` print path names the module of a fun (`src/term/print.rs:132`).

- `src/term/print.rs`: 67, 132, 144, 146
- `src/runtime/interpreter/bif/erts_internal.rs`: 479
- `src/runtime/interpreter/bif/erlang.rs`: 1602, 1632, 2859, 2967
- `src/runtime/interpreter/bif/os.rs`: 107

### 2.3 An order rule (6 sites, with 12 transitive call sites in six files)

ERTS compares two atoms by the bytes of the name (`erts_cmp_atoms`, `erts/emulator/beam/erl_utils.h:193-215`), not by the index. `Term::compare` uses the name (`src/term/order.rs:112`), and `Fun` and `Target` compare the module and the function names (`src/term/fun.rs:116`, `:117`, `:215`, `:216`, `:217`).

- `src/term/order.rs`: 112
- `src/term/fun.rs`: 116, 117, 215, 216, 217

The transitive call sites of `Term::compare` in `src/` are 12, in six files: `src/runtime/interpreter.rs:1084` and `:1096`, `src/term/map.rs:962`, `:1075`, and `:1130`, `src/term/map/build.rs:382`, `src/term/order.rs:164`, `src/runtime/interpreter/bif/table.rs:87`, `:93`, `:243`, and `:248`, and `src/runtime/interpreter/bif/subtract.rs:528`. The flatmap key order already compares two atoms by the index (`flatmap_order`, `src/term/map.rs:1120-1131`), and only its fallback to `Term::compare` reads a name.

### 2.4 A hash rule (1 site)

The internal hash of a map key uses the ERTS atom word `(index << 6) | 0xB` and reads no name (`src/term/hash.rs:125`, `:156`). The ETS hash reads the name (`atom_hash`, `src/runtime/ets.rs:473`, `:489-506`).

- `src/runtime/ets.rs`: 473

### 2.5 A name-keyed lookup or an owned name (61 sites)

These sites read the name to key a registry (`src/runtime/ets.rs:200`, `src/runtime/scheduler.rs:611`), to look up a capability or an operation (`src/runtime/capability.rs:1060`, `src/runtime/interpreter/bif/pbeam_cap.rs:45`), to build a diagnostic or a frame (`src/runtime/interpreter/code_table.rs:128`), or to return an owned name to a caller (`src/runtime/interpreter/bif/erlang.rs:109`).

- `src/beam/code/instruction.rs`: 1497, 1500
- `src/beam/code.rs`: 357, 498, 533, 536
- `src/runtime/capability.rs`: 1060, 1106, 1115, 1162, 1192
- `src/runtime/interpreter.rs`: 556, 768, 1995, 2014, 2111, 2118, 2126, 2422, 3725
- `src/runtime/ets.rs`: 200
- `src/runtime/scheduler.rs`: 611, 1393, 1660, 1807, 1810, 1834, 1974
- `src/runtime/process.rs`: 549
- `src/runtime/interpreter/code_table.rs`: 128
- `src/runtime/scheduler/policy.rs`: 178
- `src/runtime/interpreter/bif/erts_internal.rs`: 383, 447, 737, 749
- `src/runtime/interpreter/bif/pbeam_cap.rs`: 45, 90
- `src/runtime/interpreter/bif/re.rs`: 85
- `src/runtime/interpreter/bif/table.rs`: 340, 488
- `src/runtime/interpreter/bif/ets.rs`: 83, 91, 118, 140
- `src/runtime/interpreter/bif/erlang.rs`: 109, 159, 175, 282, 305, 371, 854, 1634, 2284, 2288, 2397, 2495, 2729, 2864, 2972, 3046
- `src/runtime/interpreter/bif/os.rs`: 103

### 2.6 `Debug for Term` (1 site)

`Debug for Term` writes `Atom({:?})`, with the name of the atom (`src/term.rs:653`).

- `src/term.rs`: 653

### 2.7 The tests (45 sites)

- `tests/beam/code/instruction.rs`: 28, 1098, 1152, 1926, 1986
- `tests/beam/code.rs`: 61, 98
- `tests/integration/signal_queue.rs`: 117
- `tests/runtime/interpreter.rs`: 2218
- `tests/integration/policy.rs`: 30
- `tests/integration/code.rs`: 182
- `tests/term/atom.rs`: 16
- `tests/integration/input_reductions.rs`: 493
- `tests/integration/map.rs`: 323
- `tests/integration/revoke.rs`: 64
- `tests/integration/port.rs`: 24, 25, 27
- `tests/integration/binary_find.rs`: 67
- `tests/runtime/interpreter/bif/file.rs`: 16
- `tests/integration/from_keys.rs`: 31
- `tests/integration/list_to_integer.rs`: 32
- `tests/integration/prim_file_read.rs`: 72
- `tests/integration/iolist_split.rs`: 42
- `tests/integration/module_info.rs`: 28
- `tests/integration/bif_costs.rs`: 163
- `tests/runtime/interpreter/native/prim_file.rs`: 78
- `tests/runtime/interpreter/native/prim_tty.rs`: 15
- `tests/integration/dispatch_baseline.rs`: 245
- `tests/integration/exit_costs.rs`: 82
- `tests/integration/preempt.rs`: 264
- `tests/integration/bs_create_bin.rs`: 582
- `tests/integration/inspect_bif.rs`: 90
- `tests/integration/find_executable.rs`: 79
- `tests/integration/error_handler.rs`: 262
- `tests/integration/subtract.rs`: 40
- `tests/integration/halt.rs`: 70
- `tests/integration/user_drv.rs`: 183
- `tests/integration/bif_audit.rs`: 61, 62
- `tests/integration/exit_owned.rs`: 119
- `tests/integration/list_reductions.rs`: 266
- `tests/integration/effect_capability.rs`: 917
- `tests/integration/io_start.rs`: 391, 392

### 2.8 The `Debug` problem

`Debug for Term` is a trait implementation. Its signature is `fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result`, so the caller cannot pass an atom table and the implementation cannot read one. An id-only atom therefore has no name to print in `Debug`.

This matters because production messages print a term with `{:?}`, and a changed `Debug` would change their text. The sites include `unsupported map key {key:?}` (`src/runtime/interpreter.rs:1000`, `src/runtime/interpreter/bif/maps.rs:161`, `:255`, `:327`, `:558`), `unsupported ets:new/2 heir {heir:?}` (`src/runtime/interpreter/bif/ets.rs:134`), `unsupported erlang:node/1 argument {term:?}: not a pid` (`src/runtime/interpreter/bif/erlang.rs:32`), `unsupported term order of two references {left:?} and {right:?}` (`src/term/order.rs:119`), and the list-source and match-context invariant errors of `src/runtime/interpreter.rs`.

## 3. The design of the immediate atom root

The atom term holds the ERTS atom-table index and nothing else, as the proposal's representation section records (`docs/term-performance-design-codex-v2.md`, the `Atom(u32)` row). The `AtomTable` owns each immutable name one time. A copy of an atom then copies 4 bytes and touches no reference count.

The owner of the name is outside the term. `Atom` keeps no name alive, so every read of a name needs the table that owns it. The proposal states the same rule: "A returned host value retains its runtime owner outside the term, so a name cannot outlive its table", and "Host terms from different runs need explicit import into the destination atom table" (`docs/term-performance-design-codex-v2.md`). No mechanism of this design uses `unsafe`, a raw pointer, a `transmute`, a `union`, or an unchecked access.

### 3.1 The context of each group

| Group | Context the group needs | Change |
| --- | --- | --- |
| A literal comparison with a static name | None | Compare `atom.index()` with the index of the static atom. `Atom::from_static` already gives that index with no table. |
| A literal comparison with a dynamic name | The owning `AtomTable` | Read the name from the table, or compare the index with the index that the table gives for the name. |
| A print or a text conversion | The owning `AtomTable` | `print_term(term, &atoms)`, and `&atoms` reaches `atom_to_binary/2` and `atom_to_list/1`. |
| An order rule | The owning `AtomTable` | `Term::compare(&self, other, &atoms)`, `Target::compare(&self, other, &atoms)`, and `flatmap_order(left, right, &atoms)`. |
| The ETS hash | The owning `AtomTable` | Read the name, or read a hash that the table precomputes for each name. |
| A name-keyed lookup or an owned name | The owning `AtomTable` | Read the name from the table. Most sites already hold a `Scheduler`, a `BifContext`, or the loader's table. |
| `Debug for Term` | None | Print the index. |

### 3.2 The order and hash rules with an id

The term order of two atoms stays the order of the name bytes. `Term::compare` and the transitive callers above therefore take `&AtomTable`. The map key order of two atoms stays the index order that `flatmap_order` already uses (`erts_cmp_flatmap_keys`, `erts/emulator/beam/erl_utils.h:267-272`). The internal map hash stays the atom word of the index and needs no table. The ETS hash needs the name, because ERTS hashes an atom by its name; the table can hold the hash with the name so that the ETS path reads a slot.

### 3.3 How `Debug for Term` prints a name

`Debug for Term` prints `Atom(<index>)`. The sites of section 2.8 that must keep the atom name in their text take a table-aware formatter, for example `debug_term(&key, &atoms)`, or they use `print_term(term, &atoms)` where the ERTS text is the right text. The record names the exact sites so that the atom task can keep each message.

### 3.4 A host result and an import between runs

A run does not share the program table today. `BeamProgram` holds `atoms: AtomTable` (`src/runtime/interpreter.rs:55-62`), `execute_issued` builds `Scheduler::new(self.atoms.clone())` (`:375-378`), and `erlang:list_to_atom/1` puts a new name into that clone (`src/runtime/interpreter/bif/table.rs:223`, and the `atoms.put(&name)` of `src/runtime/interpreter/bif/erlang.rs:1696`). The scheduler owns the clone, and `execute_issued` drops it when the run ends, so a name that a run creates does not survive into a later run today. The design keeps it with an owned host result.

The host result is `HostTerm { term: Term, atoms: Arc<AtomTable> }`. The public entry points `BeamProgram::execute`, `BeamProgram::execute_with_policy`, and `BeamProgram::execute_with_policy_module` return `Result<HostTerm, BeamError>` in the place of `Result<Term, BeamError>`. This is the public API change that the proposal records.

The retention step. `Scheduler::into_root_result` consumes the scheduler after the run (`src/runtime/interpreter.rs:383`). The design replaces it with `Scheduler::into_run_result(self) -> (Option<Result<Term, Term>>, AtomTable)`, which returns the root result and the table of the run together. `execute_issued` then builds `HostTerm { term, atoms: Arc::new(atoms) }`. The table of the run moves into the result, and it holds every name that `AtomTable::put` added during the run, which includes each name of `erlang:list_to_atom/1`. The retention needs no second table lookup and no lock. The alternative is one table that the program and each run share through `Arc<RwLock<AtomTable>>`: that one table already holds the program names and the run names, so it needs no merge step. The design rejects it for two reasons. Every read of a name on the hot path would take the lock. The table would also live as long as the program, because the program holds it, so a name that a run creates stays for the life of the program even when no host result holds it; the owned result lets the names of one run drop with that result.

A nested term. The one `AtomTable` of a `HostTerm` owns every atom of the whole term, at any depth, because each `Term::Atom` holds an index into exactly that table. A nested atom therefore needs no second owner and no second lookup. A compound root that two host results share keeps the table of the result that built it, so a compound that holds an atom crosses a table boundary only through the import below.

The import between two runs. `HostTerm::import_into(&self, destination: &mut AtomTable) -> Result<Term, BeamError>` calls `AtomTable::import_term(&mut self, source: &AtomTable, term: &Term) -> Result<Term, BeamError>`, which walks the term with an explicit work stack, as `Term::drop` (`src/term.rs:394`, `take_nested`) and `Term::eq` (`EqStack`) do. The walk remaps each `Term::Atom`, and it also remaps the atoms of an external fun: the `module` and `function` fields of a `Target::External` (`src/term/fun.rs:163-166`). The drop walk is not enough for the fun metadata, because it visits only the free variables of a fun, and an external fun holds no free variable at all. The import therefore rebuilds `Target` with `AtomTable::put` and rebuilds `Term::Fun` as an external fun. A fun whose module or function is not remapped would keep the index of the source table and would resolve to a different name in the destination table. The import rejects a local fun, and the next paragraph gives the rule and its errors.

The code of a local fun. A local fun also holds the entry label of its function, and `call_fun` builds the frame from that label in the module that the destination table names (`src/runtime/interpreter.rs:2124-2126`). Two programs can hold different code under one module name, and a run can load different code under one module name on demand, so a local fun can call the wrong function even when every name resolves. The import therefore rejects every local fun. `AtomTable` carries the identity of its program in a `program: ProgramId` field, which a clone keeps, and `AtomTable::import_term` compares the identity of the source table with the identity of the destination table at the `Term::Fun` arm, before it remaps the atoms. A `Target::Local` whose source program differs from the destination program gives the error `local fun of module {module} belongs to another program`, and a local fun of the same program gives `local fun of module {module} has no verified code in this program`, because the `HostTerm` carries the atom table and not the code identity of a module, and the destination run can load other code under that name after the import. A `Target::External` is the supported case: it holds no label and resolves through an export at call time, and a term without a local fun imports across programs. The design rejects a check that the destination module holds identical code, because that check needs the code tables of both programs at the import and the `HostTerm` carries the atom table only.

For each atom the walk reads the name from `self.atoms` with `AtomTable::name(index)` and calls `destination.put(name)`, which returns the index in the destination table and creates the name when the destination does not hold it. The import is the second table lookup that the rule asks for: one read of the source name and one `AtomTable::put` of the destination for each atom, in the term and in the fun metadata. A name that the first run created resolves through `AtomTable::put` when the second run uses the imported term. A static atom resolves to the same index in the destination, so one rule covers every atom and no special case is needed.

The rule when a name is not in the source table. A term atom or a fun metadata atom whose index is at or above the number of names of the source table is not a value of that table. `AtomTable::name(index) -> Option<&str>` then returns `None`, and the import returns the error `BeamError` with the text `atom index {index} is outside the atom table of the host term`. The text names the index and no name, because the index is all that the immediate atom holds. The same rule covers a hand-built `Term::Atom`.

The second error of the import is the atom limit of the destination table. `AtomTable::put` returns the error `no more index entries in atom_tab (max=1048576)` when the destination table already holds `ATOM_LIMIT` atoms (`src/term/atom.rs:149-153`). The destination table gives this error and the source table never gives it, because the import only reads the source. `HostTerm::import_into` propagates the error. The names that the import already mapped stay in the destination table, and the source term is unchanged, because the import builds a new term and never changes the source.

The tests. The atom task adds two tests to `tests/term/atom.rs`, the unit tests of `src/term/atom.rs`. `imports_a_fun_between_two_atom_tables` builds two tables whose indices differ for the module and the function of an external fun, imports the external fun from the first table into the second, and asserts that the module name and the function name of the imported fun resolve in the second table; it also asserts that an index outside the source table gives the first error, and that the import rejects the local fun of the first table with the error `local fun of module {module} has no verified code in this program`. `fails_to_import_when_the_destination_table_is_full` fills the destination table to `ATOM_LIMIT` and imports a term that holds one atom whose name the destination does not hold, and asserts the text `no more index entries in atom_tab (max=1048576)` of the second error. A third test, `rejects_a_local_fun_of_another_program`, goes in a new `tests/integration/host_term.rs` with its `[[test]]` target in `Cargo.toml`, because it builds two programs from compiled fixtures. The two programs load two BEAM files that share one module name and hold different code; an exported function of the first program returns a local fun, and the test passes that host term as an argument to a run of the second program and asserts the error text `local fun of module {module} belongs to another program`. The test also passes an external fun from the first program to the second and asserts that the import accepts it.

The import also serves the arguments of a run. All three public entry points take `arguments: &[HostTerm]` in the place of `arguments: &[Term]`: `BeamProgram::execute` (`src/runtime/interpreter.rs:251-255`), `BeamProgram::execute_with_policy` (`:278-282`), and `BeamProgram::execute_with_policy_module` (`:346-350`). `execute_issued` also takes `arguments: &[HostTerm]` (`:367-372`) in the place of its `arguments: &[Term]`, because a `&[Term]` input cannot reach the tables that own its atoms. After `execute_issued` builds the scheduler (`:375-378`), it imports each argument with `HostTerm::import_into(&mut scheduler.atoms)` into a local `Vec<Term>`, and it passes that vector as `&[Term]` to `run_root` (`:424-430`). `run_root` keeps `arguments: &[Term]`, because it runs after the import. A host term that the program built shares the table of the program: `BeamProgram::atoms` becomes `Arc<AtomTable>`, and `BeamProgram::host_term(&self, term: Term) -> HostTerm` pairs a term that the caller built with `BeamProgram::atom` with that table. A term from another table is a `HostTerm` of its own table, so the import accepts it and maps each of its names. The entry points take `&[HostTerm]`, so a caller cannot pass an argument without a table.

The precondition of `BeamProgram::host_term`. The function reads no atom of the term, because a bare `Term::Atom` carries no table identity, and it cannot prove that the term belongs to the program. The design keeps that construction and states the precondition: every atom of the term must come from this program, through `BeamProgram::atom`, or be a static atom, whose index is the same in every table. A term of another table enters through `HostTerm::import_into` first. A foreign index is a caller error. When the program table gives that index to a different name, the host term reads that other name, and no run-time check detects the mismatch, because the index is all that the term holds.

The design takes that second option: it keeps the bare `host_term`, states the precondition in the API documentation of `BeamProgram::host_term`, and does not claim that the API prevents a mismatched table. The alternative is an owner-aware construction that only accepts a term the program built. A form that cannot be defeated needs a table identity beside every index, which is 4 bytes beside the index or a 16-byte owner handle, and then a registry from the id to the table, or a check on each read of an atom before a name resolves. The proposal fixes the atom at a bare index and puts the owner outside the term, and `host_term` is the boundary where the owner is the caller's to state. The typed `HostTerm` still prevents a term without any table at the entry points, and the import is the supported path for a term of another table.

The size cost. A `HostTerm` is a `Term` of 40 bytes and an `Arc<AtomTable>` of 8 bytes. The `Arc` makes the result cheap to clone and to pass to another host call. The table itself holds the index map and the name list of `AtomTable`, so a host result holds the names that its own term can name and no name of another run.

### 3.5 The size that the atom mechanism reaches

The atom arm goes from 24 bytes to 4 bytes. `Term` stays 40 bytes and `Option<Term>` stays 40 bytes, because the 32-byte `Binary` arm holds the size. The mechanism alone does not reach the 16-byte target of row 6. It removes the last arm above 8 bytes other than `Binary`, so the compact term then reaches 16 bytes.

### 3.6 The alternatives that this design rejects

| Alternative | Reason to reject it |
| --- | --- |
| Keep `Atom { index: u32, name: Arc<str> }` (the current 24 bytes) | The root is not immediate. This is the state that #460 left open. |
| Put a shared `Arc<AtomTable>` handle in each atom | The atom is 16 bytes and still holds a pointer. Every atom keeps the whole table alive, and a copy still touches a count. |
| Store a leaked `&'static str` name and make `Atom` `Copy` | R12 of the proposal rejects a leaked dynamic name: a host can create and drop many runs, and a per-run limit does not bound the names. A `Copy` type that holds an `Arc` also duplicates the handle without a count and releases it twice. |
| Keep one process-global name table that never frees an entry | The same R12 reason. The atom limit would become a global limit across runs. |
| Tag a pointer or NaN-box the id | `AGENTS.md` forbids new `unsafe` code, and R7 and R8 of the proposal reject the first tagged store. |
| Read a register or an index without a check | `AGENTS.md` forbids unchecked access, and the dispatch plan keeps checked access (`docs/interpreter-dispatch-plan.md`, section 3). |

## 4. The design of the compact term

The mechanism boxes the 32-byte binary descriptor behind one pointer, as the representation section of the proposal asks: "Box the binary descriptor in the layout comparison" (`docs/term-performance-design-codex-v2.md`). `Term::Binary` then holds one `Arc` to the view, and the view keeps the bytes, the bit offset, the size, and the packed bytes that `src/term/binary.rs:44-60` holds today.

The model of section 1 gives the sizes. After the binary move, `Binary` is 8 bytes, so every compound arm is at most 8 bytes and `Nil` is 0. With the immediate atom of section 3, the largest arm is 8 bytes and the discriminant is 8 bytes, which is 16 bytes. `Option<Term>` is also 16 bytes.

| Arm | Payload after the move, bytes |
| --- | --- |
| `Binary` | 8 |
| `Atom` | 4 |
| `Integer`, `Pid`, `Port`, `Reference` | 8 |
| `Cons`, `Tuple`, `Map`, `Fun`, `MatchContext` | 8 |
| `Nil` | 0 |

The design departs from the proposal sketch in these places. PBeam keeps the separate `Fun`, `Map`, `Tuple`, `Cons`, and `MatchContext` arms, each one an 8-byte handle, and the sketch merges them into `Boxed(Arc<Object>)`. Both reach 16 bytes. The sketch adds `Float(f64)`, and PBeam has no float term: an unsupported numeric path keeps its current error, as the acceptance of [#553](https://github.com/fahchen/pbeam/issues/553) requires. The `Small(i64)` arm of the sketch is the `Integer(i64)` arm of PBeam.

The model measures the arm sizes and the enum size only. It does not measure the cost of the move. A binary root then pays one allocation for the view and one dependent load for each read of the descriptor. Row 6 states the requirement: "Measure the added binary-view allocation and indirection."

## 5. The measurement plan that decides row 6

The gate of row 6 is: a term of at most 16 bytes on the verified host, with at least a 10 percent end-to-end Terra gain, or at least a 20 percent reduction of peak live bytes with no time regression above 10 percent. A narrow clone and drop result alone does not pass the gate (`docs/term-performance-design-codex-v2.md`). The plan requires the end-to-end Terra timing limb as the gate of the row, because a time has a control, a unit, and a repeatable sample. The peak-live-byte limb needs the live allocation count of the table below. A sample of the resident set size cannot establish a 20 percent reduction of the live bytes, for the reason in section 7.

Each run records the revision, the host, the toolchain, the mode, and the load, as the merged records of the dispatch work do (`docs/interpreter-dispatch-measurements.md`). A measurement runs in a quiet window: no other heavy `cargo` or `mise` process, and a one-minute load at or below 12. The value of a one-minute load above 12 makes the run weak, and the record states it.

The control of a session is the same probe module and the same harness on the revision before the change, in the same session, with the ERTS and PBeam order alternated. A timing number is the median of at least 30 samples, with the minimum and the p95; a floor-loop number stays the minimum of nine probe runs, as `mise run bench-dispatch` reports it.

| Measurement | Command | What it covers |
| --- | --- | --- |
| Term size and slot size | `RUSTC_BOOTSTRAP=1 mise exec -- cargo rustc --locked --lib -- -Zprint-type-sizes` | `Term`, `Option<Term>`, and each arm at the pinned rustc. |
| The floor loop | `mise run bench-dispatch` | `call_only`, the position shape, and each added instruction, each against pinned ERTS in the same run. |
| End-to-end Terra time | `bash scripts/terra_latency.sh release <out>` | The first frame and the four keys of the Terra Counter example through a pseudo-terminal. |
| Peak live bytes | `mise run bench-live-bytes`, the harness of `tests/integration/term_live_bytes.rs` with its `[[test]]` target in `Cargo.toml`; the resident set size sample of section 7 is supporting evidence only | The harness holds one Terra scene and reports the peak live bytes of the guest containers, which is the maximum of the sampled live bytes. The bytes added and the bytes released are the changes after the baseline, the first sample, and the harness reports the baseline, the totals of the added and released bytes, and the counts of the sampling steps that added and released bytes, with the revision of the loaded example. The harness counts the bytes of the runtime containers and of the persistent terms, the ETS objects, and the active timer messages through `Scheduler::live_bytes`, one count for each allocation, and it installs no allocator and adds no `unsafe`. It does not count a guest term that only the loaded code holds, for example a literal of a decoded instruction: the code table is not part of the walk. It reports the peak resident set size of the same run as a second, independent number. The existing counting allocators of `tests/integration/bif_allocations.rs`, `tests/integration/drop_allocations.rs`, and `tests/term/data_structure_costs.rs` count allocation events only: their `alloc` adds one to a count and their `dealloc` records nothing, so they cannot give a live-byte number. |
| Clone and drop counts | `mise run bench-term`, `mise run bench-drop` | The clone and drop cost of an atom, a tuple, a shared map, and an immediate. |

The gate is measured on the full Terra path, not on an isolated loop. The floor loop, the added instructions, and the clone and drop counts support the timing limb and do not replace it. The record keeps the timing limb as the gate of the row, because the live-byte harness of `tests/integration/term_live_bytes.rs` holds one scene of the Terra target and not the full Terra Counter application. A claim of at least a 20 percent reduction of peak live bytes waits for a run of that harness on the changed revision against the first record of [#587](https://github.com/fahchen/pbeam/issues/587), and this record does not claim that count.

## 6. The split of row 6

Row 6 holds two independent mechanisms, so it splits, as the rule of the proposal asks. [#553](https://github.com/fahchen/pbeam/issues/553) owns the immediate atom root and records the term and slot sizes. [#571](https://github.com/fahchen/pbeam/issues/571) owns the compact term and the 16-byte target, and its start size is the size that [#553](https://github.com/fahchen/pbeam/issues/553) records. [#572](https://github.com/fahchen/pbeam/issues/572), this record, blocks both, because each mechanism uses the consumer list and the measurement plan of this document. Native sub-issue links express the order.

## 7. The baseline numbers of this record

These numbers belong to `3b3a21d`, the revision of this record. The load of each run is stated.

The floor loop, from `mise run bench-dispatch`, at load `{ 8.87 12.28 11.05 }` at the start of the run and `{ 12.62 12.91 11.31 }` at the end. Each number is the minimum of nine probe runs. The script records the load before its release build, and the build then raised the load, so the load of the timed section is not below the recorded value. Each floor-loop number is therefore a floor under a higher load, and the same limit belongs to the earlier records (`docs/interpreter-dispatch-measurements.md`, section "The cost of one instruction, by part").

| Shape, 200000 iterations | PBeam, ns for each iteration | Pinned ERTS, ns for each iteration |
| --- | --- | --- |
| `call_only` floor loop of `dispatch_probe` | 146.8 | 2.60 |
| `call_only` floor loop after 20 guest functions | 146.7 | 2.65 |
| `call_ext_only` floor loop of `dispatch_probe` | 267.3 | 2.66 |
| `call_only` plus `gc_bif '+'` | 221.1 | 2.67 |
| `call_only` plus the `bif` of `tuple_size/1` | 219.0 | 3.37 |
| `call_only` plus the `bif` of `map_get/2` on a 1000-key map | 274.5 | 7.38 |

| Added instruction | Added PBeam, ns for each iteration |
| --- | --- |
| `gc_bif '+'` | 74.3 |
| the `bif` of `tuple_size/1` | 72.3 |
| the `bif` of `element/2` | 77.8 |
| the `bif` of `hd/1` | 75.6 |
| the `bif` of `map_get/2` on a 1000-key map | 127.7 |

The Terra Counter example, release build, from `bash scripts/terra_latency.sh release`. The record uses a run as a control only when the one-minute load is below 12. The first run has a one-minute load of 12.15, which is above the bound, so the record discards it. The second run has a one-minute load of 6.94, and it also samples the resident set size of the `pbeam` process every 50 ms.

| Run | Load | First frame | Key `j` | Key `j` | Key `k` | Key `q` | Peak resident set size |
| --- | --- | --- | --- | --- | --- | --- | --- |
| discarded run, `3b3a21d` | `{ 12.15 12.65 11.32 }` | 0.56 s | 0.016 s | 0.018 s | 0.020 s | 0.024 s | not measured |
| control run, `3b3a21d` | `{ 6.94 18.96 18.36 }` | 0.29 s | 0.021 s | 0.030 s | 0.015 s | 0.027 s | 62080 KB |

The resident set size of the release process is 62080 KB, which is 60.6 MiB, for a full scene of the control run. The method is a wrapper that samples `ps -o rss` of the `pbeam` process every 50 ms and keeps the maximum. The number is not the live guest bytes and it is not an upper bound of them. It can be above the live bytes, because the allocator keeps freed memory, and because the loaded BEAM code, the binary, and the allocator arenas are part of the resident set. It can be below a short peak, because a sample every 50 ms does not see a peak between two samples. The sample is supporting evidence only, as section 5 states, and it cannot establish a 20 percent reduction of the live bytes. The control run is a single run, and its four keys carry the variance of a single sample. The record keeps the discarded run only to show why the bound of the measurement plan matters.
