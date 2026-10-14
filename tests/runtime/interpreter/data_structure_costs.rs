//! The measurement of the data structures and of the derivations that
//! `docs/data-structure-audit.md` reports.
//!
//! The audit asks one question: which value that a hot path derives could a
//! structure store instead? This module measures the current derivation of
//! each candidate, the size of each structure, and the counterfactual of
//! each candidate that the document recommends. It asserts no time, because
//! the numbers move with the host and its load.
//!
//! Every printed line is the minimum of [`RUNS`] timed runs of one
//! operation, in nanosecond for each operation, and every result passes
//! through [`black_box`]. The `STRUCT` lines are byte counts, not times.
//!
//! Run the release measurement with
//! `cargo test --locked --release --lib -- data_structure_costs -- --nocapture --test-threads=1`
//! and read the `STRUCT` and `COST` lines.

use std::{collections::HashMap, hint::black_box, mem::size_of, time::Instant};

use super::{code_table, function_at_label, write_operand};
use crate::{
    beam::{
        BeamModule,
        code::{Instruction, Operand},
    },
    runtime::{
        ets::{Access, EtsTables, Table},
        mailbox::Mailbox,
        persistent_term::PersistentTerms,
        process::{Frame, ProcessState, StackSlot},
    },
    term::{Atom, AtomTable, Binary, Fun, Map, ProcessId, Term, internal_hash},
};

/// The number of timed runs of one operation. The report gives the minimum
/// of the runs, as the term measurement does.
const RUNS: usize = 9;

/// The number of operations of one timed run of a cheap operation.
const OPERATIONS: usize = 100_000;

/// The number of operations of one timed run of an ETS lookup whose current
/// form walks the object vector of the table, and of the other operation
/// that costs more than a field read.
const ETS_OPERATIONS: usize = 200;

/// Print one measured line: the name of the operation and the nanosecond for
/// each operation.
fn report(name: &str, ns: f64) {
    println!("COST {name} {ns:.3} ns");
}

/// Print one measured line: the name of the structure and its size in bytes.
fn report_size<T>(name: &str) {
    println!("STRUCT {name} {} bytes", size_of::<T>());
}

/// Print one measured line for a value that is not a sized type in this
/// module, such as the body of a `Term` variant.
fn report_size_of<T>(name: &str, value: &T) {
    println!("STRUCT {name} {} bytes", std::mem::size_of_val(value));
}

/// Print the alignment of a type in bytes. The stride of an enum rounds the
/// largest arm up to this value.
fn report_align<T>(name: &str) {
    println!("STRUCT {name} {} bytes", std::mem::align_of::<T>());
}

/// Print one byte count that the code computes, such as the heap bytes of the
/// key index of an ETS table.
fn report_bytes(name: &str, bytes: usize) {
    println!("STRUCT {name} {bytes} bytes");
}

/// Return the nanoseconds for each operation of `body`, as the minimum of
/// [`RUNS`] timed runs. The caller performs exactly `operations` operations.
fn measure(operations: usize, mut body: impl FnMut() -> u64) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..RUNS {
        let start = Instant::now();
        let result = body();
        let elapsed = start.elapsed().as_nanos() as f64;
        black_box(result);
        best = best.min(elapsed / operations as f64);
    }
    black_box(best)
}

/// Return the nanoseconds for each operation of `body`, as the minimum of
/// [`RUNS`] timed runs. [`measure`] reads a state that does not change;
/// this form builds the state of each run with `setup` outside the timer, so
/// a measurement that changes its input starts each run from the same state.
fn measure_with<State>(
    operations: usize,
    mut setup: impl FnMut() -> State,
    mut body: impl FnMut(&mut State) -> u64,
) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..RUNS {
        let mut state = setup();
        let start = Instant::now();
        let result = body(&mut state);
        let elapsed = start.elapsed().as_nanos() as f64;
        black_box(result);
        black_box(&state);
        best = best.min(elapsed / operations as f64);
    }
    black_box(best)
}

/// Return the elements of a tuple of `len` integers.
fn tuple_of(len: usize) -> Term {
    Term::tuple((0..len).map(|value| Term::Integer(value as i64)).collect())
}

/// Return a module of `functions` functions, each with its own label. The
/// shape gives the label tables of a real module, so the `jump_targets`
/// and `function_at_label` rows measure the real lookup and not a copy of
/// it. The runtime no longer runs the `jump_targets` lookup, and this row
/// keeps the baseline of the audit that the loader change removed.
fn module_of(functions: u32) -> BeamModule {
    let mut instructions = Vec::new();
    for index in 1..=functions {
        instructions.push(Instruction::FunctionInfo {
            module: "audit".into(),
            name: format!("f{index}"),
            arity: 0,
        });
        instructions.push(Instruction::Label(index));
    }
    instructions.push(Instruction::End);
    BeamModule::new("audit".into(), HashMap::new(), instructions).expect("the fixture module loads")
}

/// Return an ETS set table of `objects` objects, keyed by an integer.
fn table_of(atoms: &AtomTable, objects: usize) -> Table {
    let mut table = Table::new(ProcessId(0), Access::Public, 1);
    for index in 0..objects {
        table.insert(
            atoms,
            vec![Term::Integer(index as i64), Term::Integer(index as i64)],
        );
    }
    table
}

/// A hashed index of the keys of a `set` table, in the shape that a hashed
/// ETS store could use. The ERTS internal hash of a key selects a slot, and
/// the slot holds the object index of every key with that slot.
///
/// The counterfactual must check the stored hash and then the stored key.
/// Pinned OTP rejects an entry whose stored hash value is not the hash of the
/// key and only then compares the stored key with `EQ` (`has_live_key` and
/// `has_key`, `erl_db_hash.c:766-790`), so the model stores the full hash of
/// each key and compares that hash before it compares the key. An index that
/// compares hashes alone cannot separate two keys in one slot, and an index
/// that compares keys alone pays an equality that the stored hash of OTP
/// rejects.
struct KeyIndex {
    /// The object indexes of each slot.
    slots: Vec<Vec<usize>>,
    /// The full ERTS internal hash of each object index, as the `hvalue` of
    /// an ERTS hash-table entry (`erl_db_hash.h`).
    hashes: Vec<u64>,
    /// The stored key of each object index.
    keys: Vec<Term>,
}

impl KeyIndex {
    /// Return an empty index with `slots` slots, which a power of two makes
    /// the mask of a slot one subtraction.
    fn with_slots(slots: usize) -> Self {
        KeyIndex {
            slots: vec![Vec::new(); slots],
            hashes: Vec::new(),
            keys: Vec::new(),
        }
    }

    /// Return the index of `keys` with `slots` slots.
    fn of(keys: Vec<Term>, slots: usize) -> Self {
        let mut index = KeyIndex::with_slots(slots);
        for key in keys {
            index.insert(key);
        }
        index
    }

    /// Return the slot of a key hash: the hash masked to the slot count.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let index = KeyIndex::of(vec![Term::Integer(1)], 1024);
    /// assert!(index.slot(index.hashes[0]) < 1024);
    /// ```
    fn slot(&self, hash: u64) -> usize {
        hash as usize & (self.slots.len() - 1)
    }

    /// Store `key`, as `db_put_hash` does: an equal stored key keeps its
    /// object, and a new key joins the slot of its hash.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut index = KeyIndex::with_slots(1024);
    /// index.insert(Term::Integer(1));
    /// index.insert(Term::Integer(1));
    /// assert_eq!(index.keys.len(), 1);
    /// assert_eq!(index.hashes.len(), 1);
    /// ```
    fn insert(&mut self, key: Term) {
        let hash = internal_hash(&key);
        if let Some(index) = self.find(hash, &key) {
            self.hashes[index] = hash;
            self.keys[index] = key;
            return;
        }
        let slot = self.slot(hash);
        self.hashes.push(hash);
        self.keys.push(key);
        self.slots[slot].push(self.keys.len() - 1);
    }

    /// Return the object index of the key with the hash `hash`, or `None`
    /// when the slot holds no such key. The walk compares the stored hash of
    /// each entry first, as `has_key` does, and compares the stored key only
    /// when the two hashes are equal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut index = KeyIndex::with_slots(1024);
    /// index.insert(Term::Integer(1));
    /// assert_eq!(index.find(index.hashes[0], &Term::Integer(1)), Some(0));
    /// ```
    fn find(&self, hash: u64, key: &Term) -> Option<usize> {
        self.slots[self.slot(hash)]
            .iter()
            .copied()
            .find(|index| self.hashes[*index] == hash && self.keys[*index] == *key)
    }

    /// Return the object index of an equal stored key, or `None` when no
    /// stored key is equal. The walk compares the stored hash of each entry
    /// of the slot first, so a slot with more than one key pays the equality
    /// of a key only when a stored hash is equal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut index = KeyIndex::with_slots(1024);
    /// index.insert(Term::Integer(1));
    /// assert_eq!(index.get(&Term::Integer(1)), Some(0));
    /// assert_eq!(index.get(&Term::Integer(2)), None);
    /// ```
    fn get(&self, key: &Term) -> Option<usize> {
        let hash = internal_hash(key);
        self.find(hash, key)
    }

    /// Return the key at `position` of the first slot that holds `count` or
    /// more keys, so a lookup of it walks one entry with a different stored
    /// hash first. The caller makes sure that such a slot exists.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let index = KeyIndex::of((0..64).map(Term::Integer).collect(), 8);
    /// assert!(index.get(&index.colliding_key(2)).is_some());
    /// ```
    fn colliding_key(&self, count: usize) -> Term {
        let slot = self
            .slots
            .iter()
            .position(|slot| slot.len() >= count)
            .expect("the index holds a slot with enough keys");
        self.keys[self.slots[slot][count - 1]].clone()
    }
}

/// A model of the record stride of [`Instruction`]. The real type is 136
/// bytes and its largest arm, `BinaryGetBinary`, takes 132 bytes of fields,
/// so the discriminant takes 4 bytes. The model holds `ARM` bytes of arm
/// fields, a 4-byte discriminant, and the alignment that the `align`
/// attribute states. Use it to measure what a second `u32` field of the
/// largest arm does to the stride of every instruction.
#[repr(C, align(8))]
struct StrideModel<const ARM: usize> {
    /// The fields of the largest arm.
    arm: [u8; ARM],
    /// The discriminant of `Instruction`.
    discriminant: u32,
}

/// Print the size of every structure of the runtime inventory.
#[test]
fn prints_the_size_of_every_structure() {
    let atoms = AtomTable::new();
    // The term representations.
    report_size::<Term>("term");
    report_size::<Option<Term>>("option_term");
    report_size::<Vec<Term>>("term_vector_header");
    report_size::<Map>("map");
    report_size::<Binary>("binary");
    report_size::<Fun>("fun");
    report_size::<Atom>("atom");
    report_size::<ProcessId>("process_id");
    report_size::<crate::term::Reference>("reference");
    report_size::<crate::term::PortId>("port_id");
    let cons = Term::list_from([Term::Integer(1)]);
    if let Term::Cons(cell) = &cons {
        report_size_of("cons", &**cell);
    }
    let tuple = tuple_of(1);
    if let Term::Tuple(elements) = &tuple {
        report_size_of("tuple", elements);
    }

    // The loader and code tables.
    report_size::<Instruction>("instruction");
    report_align::<Instruction>("instruction_alignment");
    report_size::<Operand>("operand");
    report_align::<Operand>("operand_alignment");
    report_size::<StrideModel<132>>("instruction_stride_model_132_arm");
    report_size::<StrideModel<136>>("instruction_stride_model_136_arm");
    report_size::<BeamModule>("beam_module");
    report_size::<super::bif::BifId>("bif_id");
    report_size::<super::bif::BifArguments>("bif_arguments");
    let module = module_of(1000);
    report_size_of("module_tables", &module.tables);
    report_size_of("jump_targets_header", &module.tables.jump_targets);
    report_size_of("function_starts_header", &module.tables.function_starts);
    report_size_of("exports_header", &module.exports);
    report_size_of("instructions_header", &module.instructions);
    report_size_of("functions_header", &module.tables.functions);
    report_size::<code_table::CodeTable>("code_table");

    // The process and scheduler state.
    report_size::<ProcessState>("process_state");
    report_size::<Frame>("frame");
    report_size::<StackSlot>("stack_slot");
    report_size::<Mailbox>("mailbox");

    // The ETS tables and the persistent terms.
    report_size::<EtsTables>("ets_tables");
    report_size::<Table>("ets_table");
    report_size::<PersistentTerms>("persistent_terms");

    // The key index of a `set` table: one stored hash and one position for
    // each slot of the open-addressing table. The rows report the heap bytes
    // of the index of a table of 1, 10, 100, and 1000 objects.
    report_size::<(u64, usize)>("ets_key_index_entry");
    for objects in [1usize, 10, 100, 1000] {
        report_bytes(
            &format!("ets_key_index_{objects}"),
            table_of(&atoms, objects).index_bytes(),
        );
    }
}

/// Print the cost of every derivation that the audit names.
#[test]
fn prints_the_cost_of_every_derivation() {
    let atoms = AtomTable::new();
    let tuple_1000 = tuple_of(1000);
    let tuple_32 = tuple_of(32);
    let module = module_of(1000);
    let table_1000 = table_of(&atoms, 1000);
    let table_100 = table_of(&atoms, 100);
    let table_10 = table_of(&atoms, 10);
    let table_1 = table_of(&atoms, 1);

    // The owner's first candidate: the element count of a tuple. The count
    // is a field of the element vector, so the read is a pointer chase and
    // one field read, and no walk.
    let tuple_len = |term: &Term| match term {
        Term::Tuple(elements) => elements.len(),
        _ => 0,
    };
    report(
        "tuple_len_1000",
        measure(OPERATIONS, || {
            let mut sum = 0u64;
            for _ in 0..OPERATIONS {
                sum += tuple_len(black_box(&tuple_1000)) as u64;
            }
            sum
        }),
    );
    report(
        "tuple_len_32",
        measure(OPERATIONS, || {
            let mut sum = 0u64;
            for _ in 0..OPERATIONS {
                sum += tuple_len(black_box(&tuple_32)) as u64;
            }
            sum
        }),
    );

    // The counterfactual of a stored value: one field read of a plain
    // structure, which is the shape that a resolved code position takes.
    struct Position {
        index: usize,
    }
    let position = Position { index: 12 };
    report(
        "stored_position_read",
        measure(OPERATIONS, || {
            let mut sum = 0u64;
            for _ in 0..OPERATIONS {
                // The reference goes through `black_box` first, so the load
                // of the field happens after the opaque call. A bare
                // `black_box(position.index)` passes the known constant 12
                // and folds the load away.
                sum += black_box(&position).index as u64;
            }
            sum
        }),
    );

    // The label lookup of `jump_targets` and of `function_at_label`. Both
    // read the side tables of the module of the running code. The runtime
    // no longer runs the `jump_targets` lookup, and this row keeps the
    // baseline of the audit that the loader change removed.
    report(
        "label_index_1000_labels",
        measure(OPERATIONS, || {
            let mut sum = 0u64;
            for _ in 0..OPERATIONS {
                sum += black_box(
                    *module
                        .tables
                        .jump_targets
                        .get(black_box(&500))
                        .expect("label 500 exists"),
                ) as u64;
            }
            sum
        }),
    );
    report(
        "function_at_label_1000_labels",
        measure(OPERATIONS, || {
            let mut sum = 0u64;
            for _ in 0..OPERATIONS {
                sum += black_box(function_at_label(black_box(&module), black_box(500)).unwrap())
                    .start as u64;
            }
            sum
        }),
    );
    report(
        "frame_creation_1000_functions",
        measure(OPERATIONS, || {
            let mut sum = 0u64;
            for _ in 0..OPERATIONS {
                sum += black_box(code_table::frame(black_box(&module), black_box(500), 0).unwrap())
                    .stack
                    .len() as u64;
            }
            sum
        }),
    );

    // The ETS set lookup. The table walks its object vector and compares
    // each key with the exact equality of `Term`, so the cost grows with the
    // object count. The key is the last object, which is the worst case of a
    // hit.
    for (name, table, key) in [
        ("ets_object_1", &table_1, Term::Integer(0)),
        ("ets_object_10", &table_10, Term::Integer(9)),
        ("ets_object_100", &table_100, Term::Integer(99)),
        ("ets_object_1000", &table_1000, Term::Integer(999)),
        ("ets_object_1000_miss", &table_1000, Term::Integer(-1)),
    ] {
        report(
            name,
            measure(ETS_OPERATIONS, || {
                let mut hits = 0u64;
                for _ in 0..ETS_OPERATIONS {
                    if black_box(table.object(&atoms, black_box(&key))).is_some() {
                        hits += 1;
                    }
                }
                hits
            }),
        );
    }

    // The counterfactual of the ETS lookup: a hashed index of the 1000 keys.
    // One lookup pays the ERTS internal hash of the key, the slot probe, and
    // then the stored hash of each entry of the slot before the stored key,
    // as `has_live_key` and `has_key` check the stored hash value and then
    // the stored key with `EQ` (`erl_db_hash.c:766-790`). The hash is inside
    // the lookup, so no row adds it again.
    let keys_1000: Vec<Term> = (0..1000).map(Term::Integer).collect();
    let index_1000 = KeyIndex::of(keys_1000, 1024);
    let hit_key = Term::Integer(999);
    let miss_key = Term::Integer(-1);
    let collision_key = index_1000.colliding_key(2);
    report(
        "internal_hash_integer",
        measure(OPERATIONS, || {
            let mut sum = 0u64;
            for value in 0..OPERATIONS {
                sum ^= internal_hash(black_box(&Term::Integer(value as i64)));
            }
            sum
        }),
    );
    report(
        "hash_index_lookup_1000",
        measure(ETS_OPERATIONS, || {
            let mut hits = 0u64;
            for _ in 0..ETS_OPERATIONS {
                if index_1000.get(black_box(&hit_key)).is_some() {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "hash_index_lookup_1000_miss",
        measure(ETS_OPERATIONS, || {
            let mut hits = 0u64;
            for _ in 0..ETS_OPERATIONS {
                if index_1000.get(black_box(&miss_key)).is_some() {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "hash_index_lookup_1000_collision",
        measure(ETS_OPERATIONS, || {
            let mut hits = 0u64;
            for _ in 0..ETS_OPERATIONS {
                if index_1000.get(black_box(&collision_key)).is_some() {
                    hits += 1;
                }
            }
            hits
        }),
    );

    // The insert path of the same table: the walk to find the key, and the
    // lookup and the insert of the counterfactual. Each timed run starts with
    // the table of 1000 objects and grows it to at most 1200, so the walk of
    // the current form is the cost of 1000 to 1200 key comparisons.
    report(
        "ets_insert_miss_1000",
        measure_with(
            ETS_OPERATIONS,
            || table_of(&atoms, 1000),
            |table| {
                for round in 0..ETS_OPERATIONS {
                    table.insert(&atoms, vec![Term::Integer(-1 - round as i64), Term::Nil]);
                }
                ETS_OPERATIONS as u64
            },
        ),
    );

    // The insert and the delete of the real table at each object count, so a
    // change to the key index shows what it adds to each write path. Each
    // timed run inserts 200 new keys into a table of `objects` objects, or
    // removes 200 keys from a table of `objects` + 200 objects. The setup of
    // each run is outside the timer.
    for objects in [1usize, 10, 100, 1000] {
        report(
            &format!("ets_insert_{objects}"),
            measure_with(
                ETS_OPERATIONS,
                || table_of(&atoms, objects),
                |table| {
                    for round in 0..ETS_OPERATIONS {
                        table.insert(&atoms, vec![Term::Integer(-1 - round as i64), Term::Nil]);
                    }
                    ETS_OPERATIONS as u64
                },
            ),
        );
        report(
            &format!("ets_delete_{objects}"),
            measure_with(
                ETS_OPERATIONS,
                || table_of(&atoms, objects + ETS_OPERATIONS),
                |table| {
                    for round in 0..ETS_OPERATIONS {
                        table.delete(&atoms, &Term::Integer((objects + round) as i64));
                    }
                    ETS_OPERATIONS as u64
                },
            ),
        );
    }
    // The index maintenance alone: one key and its hash. The caller of a
    // real insert also builds the object that the table stores, and the row
    // above pays that storage while this row does not. The document must not
    // compare the two rows as equal work.
    report(
        "hash_index_insert_key_1000",
        measure_with(
            ETS_OPERATIONS,
            || KeyIndex::of((0..1000).map(Term::Integer).collect(), 1024),
            |index| {
                for round in 0..ETS_OPERATIONS {
                    index.insert(Term::Integer(-1 - round as i64));
                }
                ETS_OPERATIONS as u64
            },
        ),
    );

    // The `map_get` path of a 1000-key HAMT and of a 32-key flatmap. The
    // HAMT splits on the hash bits that its nodes already hold, and it does
    // not store the full hash of a key or the number of levels that a key
    // walks.
    let pairs_1000: Vec<(Term, Term)> = (0..1000)
        .map(|key| (Term::Integer(key), Term::Integer(key)))
        .collect();
    let map_1000 = Map::from_pairs(pairs_1000, &crate::term::test_atom_table()).unwrap();
    let flat_32 = Map::from_pairs(
        (0..32)
            .map(|key| (Term::Integer(key), Term::Integer(key)))
            .collect(),
        &crate::term::test_atom_table(),
    )
    .unwrap();
    let flat_4 = Map::from_pairs(
        (0..4)
            .map(|key| (Term::Integer(key), Term::Integer(key)))
            .collect(),
        &crate::term::test_atom_table(),
    )
    .unwrap();
    let flat_2 = Map::from_pairs(
        (0..2)
            .map(|key| (Term::Integer(key), Term::Integer(key)))
            .collect(),
        &crate::term::test_atom_table(),
    )
    .unwrap();
    for (name, map, key) in [
        ("map_get_2_flatmap", &flat_2, 1),
        ("map_get_4_flatmap", &flat_4, 3),
        ("map_get_32_flatmap", &flat_32, 31),
    ] {
        report(
            name,
            measure(ETS_OPERATIONS, || {
                let mut sum = 0u64;
                for _ in 0..ETS_OPERATIONS {
                    sum += map.get(black_box(&Term::Integer(key))).is_some() as u64;
                }
                sum
            }),
        );
    }
    report(
        "map_get_1000_hashmap",
        measure(ETS_OPERATIONS, || {
            let mut sum = 0u64;
            for _ in 0..ETS_OPERATIONS {
                sum += map_1000.get(black_box(&Term::Integer(500))).is_some() as u64;
            }
            sum
        }),
    );

    // The linear choice scan of `select_val`. The interpreter compares the
    // value with each choice in order, with the exact equality of `Term`.
    for choices in [4usize, 16, 64] {
        let list: Vec<(Term, u32)> = (0..choices)
            .map(|index| (Term::Integer(index as i64), index as u32))
            .collect();
        let value = Term::Integer((choices - 1) as i64);
        report(
            &format!("select_val_scan_{choices}"),
            measure(OPERATIONS, || {
                let mut sum = 0u64;
                for _ in 0..OPERATIONS {
                    sum += black_box(list.iter().find(|(choice, _)| choice == black_box(&value)))
                        .map_or(0, |(_, label)| *label) as u64;
                }
                sum
            }),
        );
    }

    // The exact equality of `Term`: an immediate compares its payload, and a
    // compound term walks its elements on a pair stack that the comparison
    // allocates. The plain integer comparison of the same values bounds the
    // payload check on its own.
    let other_1000 = tuple_of(1000);
    report(
        "compare_integer_direct",
        measure(OPERATIONS, || {
            let left = 1i64;
            let right = 1i64;
            let mut hits = 0u64;
            for _ in 0..OPERATIONS {
                if black_box(left) == black_box(right) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_integer",
        measure(OPERATIONS, || {
            let mut hits = 0u64;
            for _ in 0..OPERATIONS {
                if black_box(Term::Integer(1)) == black_box(Term::Integer(1)) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_atom",
        measure(OPERATIONS, || {
            let left = Term::static_atom("ok");
            let right = Term::static_atom("ok");
            let mut hits = 0u64;
            for _ in 0..OPERATIONS {
                if black_box(&left) == black_box(&right) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_tuple_1000_shared",
        measure(ETS_OPERATIONS, || {
            let mut hits = 0u64;
            for _ in 0..ETS_OPERATIONS {
                if black_box(&tuple_1000) == black_box(&tuple_1000) {
                    hits += 1;
                }
            }
            hits
        }),
    );
    report(
        "term_eq_tuple_1000_equal",
        measure(100, || {
            let mut hits = 0u64;
            for _ in 0..100 {
                if black_box(&tuple_1000) == black_box(&other_1000) {
                    hits += 1;
                }
            }
            hits
        }),
    );

    // The ERTS internal hash of a compound key walks the whole term, as the
    // hash of a map key and of a persistent term key does.
    report(
        "internal_hash_atom",
        measure(OPERATIONS, || {
            let atom = Term::static_atom("ok");
            let mut sum = 0u64;
            for _ in 0..OPERATIONS {
                sum ^= internal_hash(black_box(&atom));
            }
            sum
        }),
    );
    report(
        "internal_hash_tuple_1000",
        measure(ETS_OPERATIONS, || {
            let mut sum = 0u64;
            for _ in 0..ETS_OPERATIONS {
                sum ^= internal_hash(black_box(&tuple_1000));
            }
            sum
        }),
    );

    // The X register write of `write_operand`. Every write calls `resize`,
    // and the counterfactual is a write into a register file that the
    // function preallocated.
    let term = Term::Integer(1);
    let mut grown: Vec<Option<Term>> = Vec::new();
    let mut preallocated: Vec<Option<Term>> = vec![None; 1024];
    report(
        "register_write_grow",
        measure(OPERATIONS, || {
            let mut writes = 0u64;
            for _ in 0..OPERATIONS {
                write_operand(
                    black_box(&Operand::X(1023)),
                    black_box(term.clone()),
                    &mut grown,
                    &mut [],
                )
                .unwrap();
                writes += grown.len() as u64;
            }
            writes
        }),
    );
    report(
        "register_write_preallocated",
        measure(OPERATIONS, || {
            let mut writes = 0u64;
            for _ in 0..OPERATIONS {
                preallocated[black_box(1023)] = Some(black_box(term.clone()));
                writes += preallocated[1023].is_some() as u64;
            }
            writes
        }),
    );

    // The persistent term lookup: the hash of the key and the probe of the
    // open-addressing table.
    let mut terms = PersistentTerms::default();
    for key in 0..100 {
        terms.put(&Term::Integer(key), &Term::Integer(key));
    }
    report(
        "persistent_term_get_100",
        measure(OPERATIONS, || {
            let mut sum = 0u64;
            for value in 0..OPERATIONS {
                sum += terms
                    .get(black_box(&Term::Integer((value % 100) as i64)))
                    .is_some() as u64;
            }
            sum
        }),
    );

    // The shapes stay alive to the end of the test, so the compiler keeps
    // the storage that the measurement reads.
    black_box((
        &tuple_1000,
        &tuple_32,
        &module,
        &table_1000,
        &table_100,
        &table_10,
        &table_1,
        &other_1000,
        &terms,
        &position,
        &index_1000,
        &hit_key,
        &miss_key,
        &collision_key,
        &map_1000,
        &flat_32,
        &flat_4,
        &flat_2,
        &grown,
        &preallocated,
    ));
}
