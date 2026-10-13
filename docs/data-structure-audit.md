# Data structure audit

This document is the audit of issue [#539](https://github.com/fahchen/pbeam/issues/539). It answers one question: which value that a hot path derives could a structure store instead? No runtime change lands in this task.

The owner asked for the audit while reading the cost table of `docs/interpreter-dispatch-measurements.md`: store the tuple size directly, review every data structure and its operations, and trade some space for time. Section 3.1 gives the answer for the tuple size, and section 4 gives the other candidates.

Each candidate that the owner accepts becomes its own task under the epic that owns it. The terms belong to [#450](https://github.com/fahchen/pbeam/issues/450) and the interpreter and code tables belong to [#506](https://github.com/fahchen/pbeam/issues/506).

## 1. The environment of the numbers

Baseline revision: `6d3884d`, the head of `origin/main` at the start of the task. Measurement revision: `ae97835`, the commit that loads the stored field through a reference and labels the index insert row. The document commit adds no Rust code.

| Item | Value |
| --- | --- |
| Host | Darwin 27.0.0 arm64, Apple M1 Max, 10 CPUs |
| Rust | `rustc 1.99.0 (b940084d7 2026-09-28)`, `cargo 1.99.0 (5f94df478 2026-08-27)` |
| Load | run 1 `{ 6.46 7.82 9.78 }`, run 2 `{ 6.81 7.84 9.77 }`, run 3 `{ 6.58 7.78 9.73 }` |
| Profile | release, `--test-threads=1` |
| Number of runs | three runs of the test, and nine timed runs of each operation inside one run |
| Raw record | `target/data-structure-costs.txt` |

Every number in section 3 is the minimum of the three runs of the revision `ae97835`, unless the text names another source. The spread between the three runs is up to 22 percent on the rows that walk, allocate, or load a field, and at most 2.4 percent on the rows that only mix a word in a register. The text gives the spread where it decides the result. The load is 6.46 to 9.78 during the runs, and other work runs on the host, so the numbers are a floor and not a best case.

An estimated number is a forecast or an arithmetic result, and the text marks it. A computed number comes from measured sizes, and the text marks it. Do not read an estimate as a result.

### 1.1. The raw measurement path

The measurement is the library unit test `prints_the_size_of_every_structure` and `prints_the_cost_of_every_derivation` in `tests/runtime/interpreter/data_structure_costs.rs`, which `src/runtime/interpreter.rs:4048-4053` loads. Run it with:

```sh
cargo test --locked --release --lib -- data_structure_costs -- --nocapture --test-threads=1
```

The module sits beside the interpreter, because it reads the private `label_index`, `function_at_label`, `code_table::frame`, and `write_operand` functions. A copy of those functions would measure the copy and not the runtime.

The test prints one `STRUCT` line for each structure and one `COST` line for each operation, in nanosecond for each operation. Every result passes through `std::hint::black_box`. The test asserts no time, because the load of the host changes the number. The check gate runs the same test under the test profile, which proves that every measured shape runs; those numbers are not the report numbers.

`target/data-structure-costs.txt` holds the revision, the host, the toolchain, the load of each run, and the lines of the three runs. The file is not committed.

## 2. The structure inventory

Each entry names a type, and its number is the measured `size_of` at the revision `ae97835`.

### 2.1. The term representations

**`Term`** (`src/term.rs:42-92`), 40 bytes. The variants are `Atom(Atom)`, `Binary(Binary)`, `Integer(i64)`, `Nil`, `Cons(ConsRef)`, `Tuple(Tuple)`, `Map(Map)`, `Pid(ProcessId)`, `Port(PortId)`, `Reference(Reference)`, `Fun(Fun)`, and `MatchContext(Arc<MatchState>)`. An immediate term (`Integer`, `Nil`, `Pid`, `Port`, `Reference`) holds its value in the 40 bytes. A compound term holds one 8-byte shared handle, and the payload goes to the heap.

**`Tuple`** (`src/term/tuple.rs:24`), 8 bytes. The field is `Tuple(Option<Arc<Vec<Term>>>)`. The `Option` exists only because `Term` has a `Drop`, and it holds no placeholder allocation. The payload of a tuple is an `Arc` of 16 bytes for its two counters, a `Vec` header of 24 bytes (measured `term_vector_header`), and one `Term` of 40 bytes for each element. Thus a tuple of `n` elements holds 40 + 40n bytes on the heap, computed from the measured sizes. `Tuple::len` (`src/term/tuple.rs:35-37`) returns `self.arc().len()`, which is the length field of the `Vec` header.

**The cons cell and the list.** `Cons { head: Term, tail: Term }` (`src/term/list.rs:20-28`), 80 bytes. `ConsRef(Option<Arc<Cons>>)` (`src/term/list.rs:34`), 8 bytes. `Term::Nil` is an immediate. The list stores no length; a length walk is inherent to `length/1` and to the list API, and the audit finds no place that could store it without a write at each `put_list` and each `get_tl`.

**`Map`** (`src/term/map.rs:114-120`), 8 bytes. The field is `repr: Option<Arc<Repr>>`. `Repr` (`src/term/map.rs:124-128`) is `Flat(Vec<(Term, Term)>)` for at most 32 pairs, or `Hash(Hamt)`. `Hamt { root: Node, size: usize }` (`src/term/map.rs:133-138`) stores the key count in `size`. `Node { bitmap: u16, children: Vec<Child> }` (`src/term/map.rs:153-158`) is 32 bytes, computed from the two fields and the padding. `Child` (`src/term/map.rs:162-175`) is 80 bytes, the size of its largest arm `Leaf(Term, Term)`. `Map::len` (`src/term/map.rs:266-271`) reads `pairs.len()` for a flatmap and `hamt.size` for a hashmap, so both forms give the count in O(1).

**`Binary`** (`src/term/binary.rs:46-59`), 32 bytes. The fields are `bytes: Arc<Vec<u8>>` (8 bytes), `offset: usize` (8 bytes), `size: usize` (8 bytes), and `packed: Option<Arc<Vec<u8>>>` (8 bytes). `size` is the bit length of the view, so the byte length is stored and needs no walk.

**`Fun`** (`src/term/fun.rs:248`), 8 bytes. The field is `Fun(Option<Arc<FunThing>>)`. `FunThing { target: Target, free: Vec<Term> }` (`src/term/fun.rs:228-233`) holds the lambda of a local fun or the module, function, and arity of an external fun, and the free variables. `Lambda` (`src/term/fun.rs:35-47`) holds the module atom, the `FunT` index, the old unique value, and the arity.

**`Atom`** (`src/term/atom.rs:48-55`), 24 bytes. The fields are `index: u32` and `name: Arc<str>`. A comparison reads the index, so two atoms compare without a string compare.

**The reference and pid terms.** `ProcessId(u64)` (`src/term.rs:520`), 8 bytes. `PortId(u64)` (`src/term.rs:528`), 8 bytes. `Reference(u64)` (`src/term.rs:535`), 8 bytes. `MatchState { bytes: Binary, position: AtomicU64 }` (`src/term/match_context.rs:58-72`) is 40 bytes, computed from the measured `Binary` of 32 bytes and the atomic word of 8 bytes.

### 2.2. The loader and the code tables

**`BeamModule`** (`src/beam/module.rs:141-152`), 264 bytes. The fields are `name: String` (24 bytes), `id: ModuleId` (4 bytes), `exports: HashMap<(NameId, u32), u32>` (48 bytes), `instructions: Vec<Instruction>` (24 bytes), and `tables: ModuleTables` (160 bytes). The name table interns each function name once, and a frame, an export key, and a diagnostic hold the `NameId`.

**The decoded instruction vector.** `instructions: Vec<Instruction>`, with a 24-byte header. `Instruction` (`src/beam/code/instruction.rs:176`) is 136 bytes and `Operand` (`src/beam/code/instruction.rs:1428`) is 40 bytes at the revision `ae97835`, which agrees with the measurement at `ea4dd13` (`docs/interpreter-dispatch-measurements.md:119`). Each arm has the stride of the largest arm, `BinaryGetBinary`, whose fields take 132 bytes. The variants that hold a `Vec` or a `String` are the call, literal, and BIF variants; a `Bif` instruction holds no name since step 4.

**`ModuleTables`** (`src/beam/module.rs:161-176`), 160 bytes. The fields are `jump_targets: LabelMap<usize>` (32 bytes), `function_starts: LabelMap<Option<FunctionStart>>` (32 bytes), `names: NameTable` (72 bytes), and `functions: Vec<(NameId, u32)>` (24 bytes). `LabelMap` (`src/beam/module.rs:27`) is a `HashMap` with the FNV-1a `LabelHasher` (`src/beam/module.rs:31`). The loader fills both maps one time in `ModuleTables::build` (`src/beam/module.rs:225-259`).

**`CodeTable`** (`src/runtime/interpreter/code_table.rs:20-34`), 144 bytes. The fields are `modules: HashMap<String, Rc<BeamModule>>`, `by_id: HashMap<ModuleId, Rc<BeamModule>>`, and `prepared: HashMap<Reference, BeamModule>`. No operation removes a module or changes its code (`src/runtime/interpreter/code_table.rs:13-18`).

**The export, label, and function tables.** The export table is `BeamModule::exports`, keyed by `(NameId, arity)`, with the entry label as its value. The label table is `ModuleTables::jump_targets`, keyed by the label number, with the index of the next instruction as its value. The function table is `ModuleTables::function_starts`, keyed by the label number, with a `FunctionStart { function: usize, start: usize }` as its value. Each read of a label or of a function is one hash lookup in `label_index` (`src/runtime/interpreter.rs:2859-2867`), `function_at_label` (`src/runtime/interpreter.rs:3119-3134`), and `code_table::frame` (`src/runtime/interpreter/code_table.rs:400-419`).

**`BifId`** (`src/runtime/interpreter/bif/table.rs`, generated) is 4 bytes. `Instruction::Bif` holds `fail_label: u32`, `bif: BifId`, `arguments: Vec<Operand>`, and `destination: Operand` (`src/beam/code/instruction.rs:352-364`), and `Instruction::CallExternal` holds `bif: Option<BifId>`. `BifArguments` (`src/runtime/interpreter/bif.rs:314`) is 128 bytes and holds the borrowed arguments of one `bif` instruction on the stack.

### 2.3. The process and the scheduler state

**`ProcessState`** (`src/runtime/process.rs:207-349`), 848 bytes. The fields that a hot path reads are:

- `frames: Vec<Frame>` (`src/runtime/process.rs:211`), the call stack, with the active frame last.
- `registers: Vec<Option<Term>>` (`src/runtime/process.rs:217`), the X registers, shared by all frames of the process. The slot is `Option<Term>` at 40 bytes measured.
- `mailbox: Mailbox` (`src/runtime/process.rs:220`).
- `fcalls: i32` (`src/runtime/process.rs:289`), the reductions of the turn, and `reductions: u64` (`:293`).
- `length_trap: Option<Vec<Term>>` (`:311`) and `pending_bif: Option<PendingBif>` (`:298`), the trap states.
- The state fields that no hot path reads: `dictionary`, `links`, `monitors`, `monitored_by`, `bif_timers`, `security`, `registered_name`, `error_handler`, and the flags.

**`Frame`** (`src/runtime/process.rs:49-60`), 40 bytes. The fields are `module_id: ModuleId`, `function: NameId`, `next_instruction: usize`, and `stack: Vec<StackSlot>`. A frame creation pushes a frame and a deallocation pops it (`src/runtime/interpreter.rs:2454-2470` for the `Cursor`, and `src/runtime/interpreter/code_table.rs:413-418` for the frame).

**`StackSlot`** (`src/runtime/process.rs:83`), 40 bytes. The variants are `Empty`, `Term(Term)`, and `Catch(usize)`.

**`Mailbox`** (`src/runtime/mailbox.rs:131-156`), 184 bytes. The fields are the node arena `nodes: Vec<Node>`, the free list `free: Vec<NodeId>`, the four `Link` boundaries `first`, `last`, `fetched`, `received`, the outer counts, the receive cursor `save: Link` (`src/runtime/mailbox.rs:150`), and the marker deque. A receive reads from `save` and walks the inner part in arrival order, which is the ERTS signal-queue design.

**`Scheduler`** (`src/runtime/scheduler.rs:153`). The fields that a hot path reads are `processes: HashMap<ProcessId, ProcessState>`, `runnable: VecDeque<ProcessId>`, `current_process: Option<ProcessId>`, and `code: CodeTable`. The node-wide stores are `persistent_terms: PersistentTerms`, `ets: EtsTables`, `atoms: AtomTable`, `names: HashMap<String, ProcessId>`, `resources`, `timers`, and `nif_libraries`.

### 2.4. The ETS tables

**`EtsTables`** (`src/runtime/ets.rs:30-41`), 144 bytes. The fields are `tables: HashMap<Reference, Table>`, `names: HashMap<String, Reference>`, and `freeing: HashMap<ProcessId, Vec<i32>>`.

**`Table`** (`src/runtime/ets.rs:45-71`), 48 bytes. The fields are `owner: ProcessId`, `access: Access`, `keypos: i32`, `objects: Vec<Vec<Term>>`, and `buckets: usize`. PBeam supports only `set` tables. `buckets` models the ERTS segment count for the exit reductions (`src/runtime/ets.rs:418`), and the store does not use it for the lookup. The lookup, the insert, and the delete walk `objects` linearly with the exact equality of `Term` (`src/runtime/ets.rs:308`, `:330`, `:353`).

### 2.5. `persistent_term`

**`PersistentTerms`** (`src/runtime/persistent_term.rs:22-27`), 32 bytes. The fields are `buckets: Vec<Option<(Term, Term)>>` and `num_entries: usize`. A new table has 8 slots. A lookup computes the ERTS internal hash of the key and probes the open-addressing table (`src/runtime/persistent_term.rs:57-61`); a put doubles the table at a fill of 50 percent.

## 3. The operations that a hot path calls

The table gives the current derivation of each operation, its file and line, and its measured cost for one operation at the revision `ae97835`. The last column says whether the operation walks, allocates, hashes, or reads a stored value.

| Operation | File and line | Cost for one operation | Derivation |
| --- | --- | --- | --- |
| The element count of a 1000-element tuple | `src/term/tuple.rs:35-37` | 0.484 ns | reads the stored length of the element vector |
| The element count of a 32-element tuple | `src/term/tuple.rs:35-37` | 0.483 ns | reads the stored length |
| A plain field read through a reference, as the counterfactual of a stored code position | `tests/runtime/interpreter/data_structure_costs.rs` | 0.320 ns | loads a field of a local |
| `label_index` of a 1000-label module | `src/runtime/interpreter.rs:2859-2867` | 2.458 ns | hashes the label and probes the FNV-1a map |
| `function_at_label` of a 1000-function module | `src/runtime/interpreter.rs:3119-3134` | 4.885 ns | hashes the label and probes the map, then reads the function vector |
| `code_table::frame`, a frame creation | `src/runtime/interpreter/code_table.rs:400-419` | 7.425 ns | the lookup above, an arity compare, and an empty `Vec` |
| An ETS set lookup, 1 object | `src/runtime/ets.rs:330-334` | 21.875 ns | walks the object vector and compares each key |
| An ETS set lookup, 10 objects | `src/runtime/ets.rs:330-334` | 233.540 ns | the same walk |
| An ETS set lookup, 100 objects | `src/runtime/ets.rs:330-334` | 2153.545 ns | the same walk |
| An ETS set lookup, 1000 objects | `src/runtime/ets.rs:330-334` | 20961.040 ns | the same walk |
| An ETS set lookup that misses, 1000 objects | `src/runtime/ets.rs:330-334` | 20885.625 ns | the same walk, with no hit |
| An ETS set insert of a new object, 1000 to 1200 objects | `src/runtime/ets.rs:308-317` | 23058.540 ns | builds the object, walks for the key, then pushes |
| The ERTS internal hash of a small integer | `src/term/hash.rs:91` | 4.725 ns | reads the word and mixes it |
| The ERTS internal hash of an atom | `src/term/hash.rs:91` | 2.795 ns | reads the atom word and mixes it |
| The ERTS internal hash of a 1000-element tuple | `src/term/hash.rs:91` | 4965.835 ns | walks every element |
| A hashed index lookup of 1000 keys that hits | the counterfactual of the ETS index | 24.170 ns | hashes the key, probes the slot, compares the stored hash, and compares the stored key |
| A hashed index lookup of 1000 keys that misses | the counterfactual of the ETS index | 3.955 ns | hashes the key, probes the slot, and compares the stored hashes only |
| A hashed index lookup in a slot of two keys | the counterfactual of the ETS index | 24.375 ns | rejects the first entry on its stored hash, then compares the equal key |
| The index maintenance of one insert into 1000 keys | the counterfactual of the ETS index | 19.585 ns | hashes the key, walks the slot on the stored hashes, and adds it, without the object storage of the caller |
| `Map::get` of a 2-key flatmap | `src/term/map.rs:315-320` | 44.375 ns | scans the pairs and compares each key |
| `Map::get` of a 4-key flatmap | `src/term/map.rs:315-320` | 109.585 ns | the same scan |
| `Map::get` of a 32-key flatmap | `src/term/map.rs:315-320` | 686.460 ns | the same scan |
| `Map::get` of a 1000-key hashmap | `src/term/map.rs:322-341` | 50.830 ns | hashes the key and walks the HAMT |
| The `select_val` scan of 4 choices | `src/runtime/interpreter.rs:1316-1322` | 83.063 ns | scans the choices and compares each one |
| The `select_val` scan of 16 choices | `src/runtime/interpreter.rs:1316-1322` | 335.459 ns | the same scan |
| The `select_val` scan of 64 choices | `src/runtime/interpreter.rs:1316-1322` | 1374.619 ns | the same scan |
| A plain integer comparison | the counterfactual of `Term::eq` | 0.310 ns | compares two words |
| `Term::eq` of two integers | `src/term.rs:353-410` | 24.197 ns | allocates a pair stack, then compares the word |
| `Term::eq` of two atoms | `src/term.rs:353-410` | 25.291 ns | allocates a pair stack, then compares the index |
| `Term::eq` of two tuples that share one array | `src/term.rs:353-410` | 21.665 ns | allocates a pair stack and reads the two pointers |
| `Term::eq` of two equal 1000-element tuples | `src/term.rs:353-410` | 2537.910 ns | walks 1000 pairs |
| An X register write through `write_operand` | `src/runtime/interpreter.rs:4013` | 6.507 ns | calls `Vec::resize`, then stores the term |
| A write into a preallocated register file | the counterfactual of the register write | 4.827 ns | stores the term |
| `persistent_term:get/1` of a 100-key table | `src/runtime/persistent_term.rs:57-61` | 37.291 ns | hashes the key and probes the table |

The `label_index` and `function_at_label` numbers are measured on a module of 1000 labels, and the hash cost of the FNV-1a mixer does not change with the count. The `select_val` scan measures the scan only; the instruction then calls `label_index` and pays the 2.458 ns of the same table.

The hashed index rows are a model in the measurement module and not the proposed runtime index. A lookup of the model does the work that pinned OTP does: it compares the stored hash value and then the stored key with `EQ` (`has_live_key` and `has_key`, `erl_db_hash.c:766-790`). The hash of the key is inside the lookup, so the text never adds the 4.815 ns of `internal_hash_integer` to a lookup row.

### 3.1. The first candidate: the tuple size

The owner named the tuple size first. The audit finds no place that computes a tuple size by iterating. Every site reads a stored count:

- `test_arity` reads `elements.len()` (`src/runtime/interpreter.rs:682`).
- `select_tuple_arity` reads `elements.len()` (`src/runtime/interpreter.rs:710`).
- The `bif` of `tuple_size/1` reads `elements.len()` (`src/runtime/interpreter/bif/table.rs:330`).
- `Term::eq` reads `left.len()` and `right.len()` (`src/term.rs:363`).
- The term order reads `left.len()` and `right.len()` (`src/term/order.rs:122-126`).
- The loader reads the arity from the header byte of an ETF small tuple (`src/beam/code.rs:144`, `:782-792`), and it does not walk the elements to find it.

The count is stored in the `Vec` header of `Tuple` (`src/term/tuple.rs:24`), and the read is the measured 0.484 ns for a 1000-element tuple. The read costs the same for a tuple of 32 elements (0.483 ns), so the count does not change the cost.

The measurement record gives the `bif` of `tuple_size/1` an added delta of 80.0 ns and the `bif` of `+` an added delta of 75.6 ns at `ea4dd13` (`docs/interpreter-dispatch-measurements.md:200-201`). The difference is 4.4 ns, which is inside the run-to-run noise of about 10 percent that the same document records. The size read is therefore not separable from the rest of the BIF path, and the audit claims no gain for it.

The bytes that storing the count adds are zero, because the count is stored. The write path pays nothing extra: `Vec::push` writes the new length, which it already does.

Recommendation: **keep**. The owner's first candidate is already in the code.

## 4. The candidates

Each heading gives the candidate, its measurement, the bytes that storing the value adds, and what a write path pays to keep it current. The recommendation of each candidate is in section 5.

### 4.1. The ETS set key lookup stores no index

**The current computation.** `Table::object` walks `objects` and compares each key with the exact equality of `Term` (`src/runtime/ets.rs:330-334`). `Table::insert` (`src/runtime/ets.rs:308-317`) and `Table::delete` (`src/runtime/ets.rs:353-366`) do the same walk. The comment at `src/runtime/ets.rs:55-58` records the reason: `Term` has no stored hash, so the store uses a linear search. The `buckets` field models the ERTS hash table for the exit reductions only.

**The measurement.** One lookup costs 21.875 ns at 1 object, 233.540 ns at 10 objects, 2153.545 ns at 100 objects, and 20961.040 ns at 1000 objects. The spread of the 1000-object row over the three runs is 3657.085 ns, which is 17.4 percent. One insert of a new object into a table of 1000 to 1200 objects costs 23058.540 ns.

The counterfactual is a hashed index that does the work of one ERTS lookup: it hashes the key, probes the slot, compares the stored hash of each entry, and compares the stored key only when the two hashes are equal (`has_live_key` and `has_key`, `erl_db_hash.c:766-790`). The measured lookup is 24.170 ns for a hit and 3.955 ns for a miss, because a miss rejects every entry on its stored hash and compares no key. A slot that holds two keys costs 24.375 ns, because the walk rejects the first entry on its stored hash and then compares the equal key. The hash of the key is 4.725 ns and is inside those numbers.

The break-even is at about one object: at one object the 21.875 ns walk is below the 24.170 ns of the hashed lookup, and at 10 objects the 233.540 ns walk is above it. A table of one object loses about 2 ns for each lookup.

The 24.170 ns hit is almost all one key comparison: `Term::eq` of two integers is 24.197 ns in the same run. Task #541 removes that pair stack, and both paths of this section must be measured again after it lands. The linear walk and the hashed lookup pay the same key comparison, and the hashed path still pays the hash and the probe, so this audit claims no break-even after that change.

**The bytes that storing adds.** The index stores one 64-bit hash for each object and one slot for each object. That is an estimated 12 to 16 bytes for each object, before the load factor of the slot table. The `Table` header grows by an estimated 24 to 48 bytes for each table. The measured `Table` header is 48 bytes today. The estimate stores the hash of every key, so a compound key also pays the hash walk one time at the insert.

**The write path.** Each insert hashes the key, walks the slot on the stored hashes, and adds or replaces the entry; each delete removes an entry. The index maintenance of one insert costs 19.585 ns for each key. The current form stores a full object, and one insert of a new object into a table of 1000 to 1200 objects costs 23058.540 ns. The two rows are not equal work: the current row builds a two-term object, and the index row stores one key and its hash. The audit therefore compares no equal-work insert, and it does not measure an insert into a small table. Section 6 records both gaps.

**The uncertainty.** The audit measures one operation and not a workload. It does not measure the object count of a real ETS table, nor the number of `ets:lookup/2` calls in a run. The hashed index of the counterfactual is a model: its slot count, its load factor, and its memory layout are the audit's choices, and not a design. The trigger for the change is an ETS table above a few objects on a hot path.

### 4.2. The exact equality of a term allocates a pair stack

**The current computation.** `Term::eq` allocates `vec![(self, other)]` at `src/term.rs:354` and pushes each nested pair (`src/term.rs:357-410`). The drop of the `Vec` follows each comparison. The comparison reads no structure, so the allocation is pure overhead for a shallow or a shared term.

**The measurement.** `Term::eq` of two integers costs 24.197 ns, of two atoms 25.291 ns, and of two tuples that share one array 21.665 ns, against 0.310 ns for a plain integer comparison. Thus about 21.4 to 25.0 ns of each comparison is the allocation and the walk setup, and it is paid even when the two terms are equal and no element is compared. A comparison that walks 1000 pairs costs 2537.910 ns, which is 2.54 ns for each pair.

The measurement record already counts this allocation as one of the two allocations that the call path of step 3 keeps (`docs/interpreter-dispatch-measurements.md:79`, `:294`), and no step of the interpreter plan owns it.

**Where the hot paths pay it.** Every key comparison of an ETS set (`src/runtime/ets.rs:308`, `:330`, `:353`), of a map (`src/term/map.rs:317`, `:322-341`), and of the `select_val` scan (`src/runtime/interpreter.rs:1319`). Every comparison BIF. Every match of a message in a receive.

**The bytes that storing adds.** None to a structure. The candidate stores the pending pairs in a fixed inline array in the frame of the comparison: 16 bytes for each pair, because a pair of references is two 8-byte pointers. Four pairs take 64 bytes of the stack of the comparison, and a deeper term keeps the heap path. The measured `term_vector_header` of 24 bytes leaves the heap vector in place for that case.

**The write path.** None. The comparison builds the inline array with a few stores instead of one allocation. The comparison order and the values do not change.

**The uncertainty.** The audit does not measure the number of comparisons in a real workload, nor the distribution of their depth. The absolute gain carries the run noise of these rows, which is up to 22 percent, but the conclusion does not: the payload comparison is 0.310 ns and the allocation is the rest. The gain is about 21 to 25 ns for each comparison that the inline array covers and 0 ns for a deeper one. Measure the depth distribution before the choice of the array length.

### 4.3. The code position resolves its label at each visit

**The current computation.** `label_index` reads `module.tables.jump_targets` and hashes the label (`src/runtime/interpreter.rs:2859-2867`). The interpreter calls it at 25 sites, once for each jump and each taken fail or catch label (`src/runtime/interpreter.rs:685` to `:1524`). `function_at_label` reads `module.tables.function_starts` and then the function vector (`src/runtime/interpreter.rs:3119-3134`); `code_table::frame` calls it at each frame creation, from three local-call sites (`src/runtime/interpreter.rs:652`, `:658`, `:671`) and from the export path (`src/runtime/interpreter/code_table.rs:224`, `:274`).

**The measurement.** `label_index` costs 2.458 ns against 0.320 ns for a stored field read through a reference, so the resolution of a taken branch costs about 2.14 ns. `function_at_label` costs 4.885 ns, so the resolution of a frame creation costs about 4.57 ns, and the whole frame creation costs 7.425 ns.

**The bytes that storing adds.** Zero for the form that replaces the label with the resolved instruction index. The label is a `u32` and the index is a `u32`, so no variant changes its size, and the measured `Instruction` of 136 bytes with its 8-byte alignment holds for every arm.

The form that keeps the label beside the index is not free, and the largest arm is the reason. `BinaryGetBinary` already holds a `fail_label: u32` (`src/beam/code/instruction.rs:1001-1007`) and its fields take 132 bytes of the 136-byte record, so it is the largest arm (`docs/interpreter-dispatch-measurements.md:119`). A second `u32` field of that arm makes 136 bytes of fields. The audit measures a model of the record stride: with the measured 8-byte alignment of `Instruction`, a 132-byte arm and a 4-byte discriminant take 136 bytes, and a 136-byte arm takes 144 bytes. The model is `StrideModel` in the measurement module, and the audit did not measure the proposed type. The add-beside form therefore grows the stride of every instruction in every module from 136 bytes to 144 bytes, unless the proposal moves a field of the largest arm to another place. A frame-start form adds 8 bytes for each local call.

**The write path.** None at run time. The loader resolves each label once, in `ModuleTables::build` (`src/beam/module.rs:225-259`). No operation changes the code of a loaded module (`src/runtime/interpreter/code_table.rs:13-18`), so a stored index cannot become stale. A module that `erlang:finish_loading/1` adds gets its own module identity and its own loader pass.

**The uncertainty.** The gain is for one taken branch and for one frame creation, and the audit does not measure how many of each a workload takes. The dispatch benchmark of `docs/interpreter-dispatch-measurements.md` already removed the scan that the lookup replaced, so the remaining 2.14 ns is the residue of the fix of step 1.

### 4.4. The X register write grows the register file at each write

**The current computation.** `write_operand` calls `registers.resize(registers.len().max(index + 1), None)` at `src/runtime/interpreter.rs:4025` for every write to an X register. The register file is `Vec<Option<Term>>` (`src/runtime/process.rs:217`), and one slot is 40 bytes measured.

**The measurement.** The current write costs 6.507 ns and a write into a preallocated file costs 4.827 ns, so the difference is 1.680 ns for each X write. The two rows do not differ by the `resize` alone: the current row matches the `Operand` and builds the error path of `write_operand`, so 1.680 ns is an upper bound on the growth cost.

**The bytes that storing adds.** The stored value is the highest X index of each function, the `max_x` table of step 8 of `docs/interpreter-dispatch-plan.md:50`. One `u32` for each function is an estimated 4 bytes for each function; the measured `ModuleTables` header is 160 bytes and its `functions` vector header is 24 bytes.

The register reservation raises the peak register space of a process. Today the file grows only to the highest index that an executed write reaches. A reservation at the entry of the function allocates every slot up to `max_x`, including the slots of a branch that the process never takes. One slot is 40 bytes (measured `option_term`), so a function with `max_x` = 1023 reserves 1024 slots, which is 1024 x 40 = 40960 bytes (computed) for each process that enters it, against the 160 bytes of a path that writes x0 to x3 only. The plan quotes 1027 slots at 41080 bytes for each process (`docs/interpreter-dispatch-plan.md:19`, `:27`). A path that never reaches the highest register therefore pays the slots that it does not use. The audit does not measure the X index distribution of a real module, so it cannot bound that peak.

**The write path.** None at run time. The loader writes `max_x` one time, and the frame entry extends the file one time. The plan already assigns the table to step 8, and step 8 is skipped (`docs/interpreter-dispatch-plan.md:196`, `docs/interpreter-dispatch-measurements.md:125`), so no sibling step adds the table today.

**The uncertainty.** The audit does not measure the share of X writes in the instruction mix of a workload, and it does not measure the peak register space of a path that never reaches the highest X index. The gain is 1.680 ns for each X write and the peak cost is up to 40 bytes for each unused slot, so the trade needs its own measurement. The table also has no consumer before a step that reserves the file, and a step that adds the table alone fails the rule of `AGENTS.md` against a field with no consumer.

### 4.5. The `select_val` instruction scans its choices

**The current computation.** `Instruction::SelectValue` scans `choices` with the exact equality of `Term` and takes the first match (`src/runtime/interpreter.rs:1316-1322`). It then calls `label_index` for the target label. The loader builds the choice vector from the stream (`src/beam/code/instruction.rs:1541-1559`).

**The measurement.** The scan costs 83.063 ns at 4 choices, 335.459 ns at 16 choices, and 1374.619 ns at 64 choices. The cost is about 21 ns for each choice, which is the `Term::eq` cost of section 4.2. The `label_index` call adds 2.458 ns of section 4.3.

**The bytes that storing adds.** Zero for a sorted vector of the same entries. An estimated 8 to 16 bytes for each choice for a hash index. The term order is total, so the loader can sort the vector that it already builds.

**The write path.** The loader sorts or indexes the choices one time. The interpreter compares fewer choices for each visit.

**The uncertainty.** The audit does not measure the choice count of a real `select_val`, nor the number of its visits. The floor loop of the dispatch benchmark has no `select_val`. The scan also loses about 21 of its 21.5 ns for each choice when section 4.2 lands, so the sort has value only above a few choices. Measure the choice counts of a real workload first.

### 4.6. `persistent_term` hashes the key at each lookup

**The current computation.** `PersistentTerms::get` calls `lookup`, which computes the ERTS internal hash of the key and probes the open-addressing table (`src/runtime/persistent_term.rs:57-61`).

**The measurement.** `persistent_term:get/1` of a 100-key table costs 37.291 ns. The hash of the key is 4.725 ns for a small integer and 2.795 ns for an atom. The hash of a 1000-element tuple is 4965.835 ns, which is the walk of the key.

**The bytes that storing adds.** A stored hash adds 8 bytes for each term that carries it. The measured `Term` is 40 bytes, so the term grows by 20 percent, and every term of the runtime pays it, including the immediate terms that no key uses.

**The write path.** A term that a map, a tuple, or a fun builds would write the hash once, at the build.

**The uncertainty.** The gain is 2.8 to 4.8 ns for each `persistent_term:get/1` with a small key, which is a small part of the 37.291 ns. The space is paid by every term. The audit recommends no measurement of a larger change, because the key shapes of a persistent term are small.

### 4.7. The flatmap scan and the HAMT

**The current computation.** `Map::get` scans the pairs of a flatmap linearly and walks the HAMT of a hashmap (`src/term/map.rs:315-341`). The flatmap order is the ERTS order, and guest code reads it through `maps:keys/1`, so a second order is not free to add.

**The measurement.** `Map::get` costs 44.375 ns at 2 keys, 109.585 ns at 4 keys, 686.460 ns at 32 keys, and 50.830 ns at 1000 keys. The flatmap scan is about 25 ns for each pair that it compares, which is the `Term::eq` cost of section 4.2. The HAMT walk of a 1000-key map is about 34 ns of the `map_get/2` added delta (`docs/interpreter-dispatch-measurements.md:169`), and the map stores the key count in `Hamt::size` (`src/term/map.rs:137`).

**The bytes that storing adds.** None for the audit's recommendation.

**The write path.** None.

**The uncertainty.** None for the recommendation. The audit does not recommend a tree or an index for the flatmap, because the measured pair compare is the `Term::eq` cost and section 4.2 removes it.

### 4.8. The decoded instruction form

`Instruction` is 136 bytes and `Operand` is 40 bytes at `ae97835`, which agrees with the measurement at `ea4dd13` (`docs/interpreter-dispatch-measurements.md:119`). The alignment of `Instruction` is 8 bytes, and the stride model of section 4.3 gives 136 bytes for a 132-byte arm and 144 bytes for a 136-byte arm. Step 6 of the dispatch plan measured the memory-system proxy and decided to keep the decoded form (`docs/interpreter-dispatch-measurements.md:125`). The audit re-measures both sizes and agrees with the decision. Recommendation: **keep**.

### 4.9. The mailbox

The receive reads from the save pointer and walks the inner part of the signal queue in arrival order (`src/runtime/mailbox.rs:150`, `:131-156`). That order is the ERTS signal-queue order that the code follows, and a per-message index would change it. The audit finds no derivable value in the mailbox. Recommendation: **keep**.

## 5. The ranked list

The rank orders the candidates by the measured gain of one operation against the space and the maintenance cost. The gain of a candidate is for one operation; the audit does not measure the number of operations of a workload, and each row says so where the frequency decides the result.

| Rank | Candidate | Measured gain for one operation | Space | Maintenance | Recommendation |
| --- | --- | --- | --- | --- | --- |
| 1 | The ETS set key lookup stores no index (`src/runtime/ets.rs:330`) | 20.96 us at 1000 objects, 2154 ns at 100, 234 ns at 10, 21.88 ns at 1, against 24.17 ns for a hashed lookup that checks the stored hash and the stored key, 3.96 ns for a miss, and 24.38 ns in a slot of two keys; the index maintenance of one insert is 19.59 ns for each key | estimated 12 to 16 bytes for each object, 24 to 48 bytes for each table | one hash, one key walk, and one slot at each insert; the key comparison is a `Term::eq` of the stored key, and the caller builds the object for both forms | **store** for a table above a few objects on a hot path, **measure more** for the object counts of a real workload; the gain grows when rank 2 removes the pair stack |
| 2 | The exact equality of a term allocates a pair stack (`src/term.rs:354`) | 21.67 to 25.29 ns for each comparison, against 0.310 ns for the payload check; 21.4 to 25.0 ns is the allocation | 16 bytes for each pair of the inline array; 64 bytes for four pairs of the comparison frame | none; the order and the values do not change | **store**; it is also the key comparison of rank 1 and of rank 5 |
| 3 | The code position resolves its label at each visit (`src/runtime/interpreter.rs:2859`, `:3119`) | 2.14 ns for each taken branch, 4.57 ns for each frame creation, against a 0.320 ns stored field read | 0 bytes for the stride if the index replaces the label, because both are `u32`; the add-beside form takes the stride from 136 bytes to 144 bytes, because the largest arm is at 132 bytes of fields | the loader resolves each label one time; no run-time update | **store** the replace form only |
| 4 | The X register write grows the register file (`src/runtime/interpreter.rs:4025`) | 1.680 ns for each X write, and it is an upper bound, because the counterfactual also skips the operand match; the spread of the counterfactual row is 0.114 ns | 4 bytes for each function for `max_x`, and up to 40 bytes for each reserved X slot of a path that never reaches the highest index | the loader writes `max_x` one time | **measure more**; the peak register space of a real module can grow, and the field has no consumer today |
| 5 | The `select_val` scan of the choices (`src/runtime/interpreter.rs:1316`) | about 21 ns for each choice; 1375 ns at 64 choices | 0 bytes for a sorted vector; an estimated 8 to 16 bytes for each choice for a hash index | the loader sorts or indexes the choices one time | **measure more**; the choice counts of a real workload decide, and rank 2 removes most of the per-choice cost |
| 6 | The tuple element count (`src/term/tuple.rs:35`) | 0.484 ns, and the count is stored; the BIF delta of 80.0 ns against 75.6 ns for `+` is inside the noise | 0 bytes, because the value is stored | none | **keep** |
| 7 | The `persistent_term` key hash (`src/runtime/persistent_term.rs:57`) | 2.8 to 4.7 ns of the 37.291 ns of one lookup | 8 bytes for each term of the runtime, a 20 percent growth of `Term` | one hash write at the build of each term | **keep** |
| 8 | The flatmap scan and the HAMT (`src/term/map.rs:315`) | about 34 ns for the HAMT walk of a 1000-key map; the flatmap scan is the cost of rank 2 | none | none | **keep** the forms, and let rank 2 remove the pair compare |
| 9 | The decoded instruction stride (`src/beam/code/instruction.rs:176`) | 136 bytes for each instruction | not applicable | not applicable | **keep** the decision of step 6 |
| 10 | The mailbox (`src/runtime/mailbox.rs:131`) | no derivable value | not applicable | not applicable | **keep** |

The largest measured gain for one operation is rank 1: 20.96 us for one ETS set lookup in a table of 1000 objects, against 24.17 ns for a hashed lookup that checks the stored hash and the stored key. Its uncertainty is the call frequency of `ets:lookup/2` in a real workload, which this audit does not measure, the space estimate, which is an estimate and not a measurement, and the object count of a real table. The largest gain on a measured interpreter path is rank 2: about 21 to 25 ns for each comparison, with the uncertainty of the comparison count and the depth distribution. Rank 2 also sets the floor of rank 1 and rank 5, because the key comparison of both is the same `Term::eq`.

Rank 1 and rank 2 are both a trade of space for time. Rank 3 is a trade of no space for time, because the loader already resolves the value. Ranks 4 and 5 need a measurement of the frequency before a task.

## 6. What the audit could not cover

- **The frequency of each operation in a workload.** The audit measures the cost of one operation, and not the number of operations of a run. The ranking therefore does not give the total gain of a change. The dispatch benchmark of `docs/interpreter-dispatch-measurements.md` covers the floor loop only, and it has no ETS, `select_val`, or flatmap row.
- **The object count and the call count of ETS.** The audit measures 1, 10, 100, and 1000 objects, and it does not measure the size or the read count of a real table.
- **An equal-work ETS insert.** The audit measures the current insert of a full two-term object and the index maintenance alone, which stores one key and its hash. It does not measure an insert that pays the same object storage in both forms, and it does not measure an insert into a small table, so section 4.1 compares no equal-work insert.
- **The choice count of `select_val`.** The audit measures 4, 16, and 64 choices, and it does not measure the distribution of a real module.
- **The comparison count and the depth of `Term::eq`.** The audit measures the comparison of two integers, two atoms, two tuples that share an array, and two equal 1000-element tuples. It does not measure the depth distribution, so it cannot fix the length of the inline array of rank 2.
- **The share of X writes in the instruction mix.** The audit measures one write and not the mix.
- **The peak register space of the reservation.** The audit does not measure the X index distribution of a real module, so it cannot bound the slot count that a reservation adds to a path that never reaches the highest X index.
- **The hashed index of the counterfactual.** It is a model in the measurement module, and not the proposed runtime index. The audit chose 1024 slots for 1000 keys, one `Vec<usize>` for each slot, and the ERTS internal hash masked to the slot count. A real index has other choices and other costs.
- **The instruction stride of the add-beside form.** The 144 bytes come from a model of the record layout and from the measured 8-byte alignment of `Instruction`. The audit did not measure the proposed type.
- **The cache behavior.** The host is macOS, which has no `perf`, so no hardware counter is available. The audit measures no cache miss, and the cache evidence of step 6 remains a memory-system proxy (`docs/interpreter-dispatch-measurements.md:295`).
- **The heap size of a stored value.** The audit measures `size_of` for the types that the test module can name. The payload sizes of a tuple, of a HAMT node, and of a `Child` are computed from the measured field sizes and are not measurements of an allocation.
- **The scheduler run queue, the signal handling, the timers, the ports, the dirty IO, the capability state, and the resources.** The audit read them and found no derivable value with a measured cost, and it measured none of them.
- **The mailbox.** The receive order is the ERTS order, and the audit found no value to store. The audit did not measure the receive walk.
- **The instruction stride of the current form.** Step 6 owns that decision, and the audit only re-measures the two sizes and the alignment.
- **The module load and the code table.** The loader resolves each label and each export one time, so the audit finds no run-time candidate there. The audit did not measure the load path.
- **The term hashing.** The audit measures the hash of an atom, of a small integer, and of a 1000-element tuple. It does not measure the hash of a binary, of a map, or of a list.
