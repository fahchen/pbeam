//! The ERTS heap word size of a term, as `size_object` computes it.
//!
//! PBeam keeps a term as a graph of `Arc` containers, not as heap words.
//! [`flat_size`] gives the number of words that the term takes on an ERTS
//! heap of a 64-bit emulator, with each shared subterm counted once for
//! each reference, as `size_object_x` does with no literal option
//! (`erts/emulator/beam/copy.c:88-260`). `erts_debug:flat_size/1` returns
//! this number (`beam_debug.c:70-76`).
//!
//! The walk holds an explicit work stack, as `size_object_x` holds an
//! `ESTACK`, so a long list does not recurse on the Rust stack. It
//! allocates only that stack.

use super::{
    Term,
    external_id::ExternalData,
    map::{Child, Node, Repr},
};

/// The words of a pid or a port of another node: a header, a next pointer,
/// a node pointer, and one data word (`EXTERNAL_PID_HEAP_SIZE`,
/// `EXTERNAL_PORT_HEAP_SIZE`, `erl_term.h:1150-1230`).
const EXTERNAL_WORDS: usize = 4;

/// The words of a float: a header and one data word (`FLOAT_SIZE_OBJECT`,
/// `erl_term.h:526`).
const FLOAT_WORDS: usize = 2;

/// The words of a fun without its free variables (`ERL_FUN_SIZE`,
/// `erl_fun.h:99`): the header and the entry pointer.
const FUN_WORDS: usize = 2;

/// The words of an ordinary or magic reference (`ERTS_REF_THING_SIZE`,
/// `ERTS_MAGIC_REF_THING_SIZE`, `erl_term.h:990-992`).
const REFERENCE_WORDS: usize = 3;

/// The words of a pid reference (`ERTS_PID_REF_THING_SIZE`,
/// `erl_term.h:991`).
const PID_REFERENCE_WORDS: usize = 4;

/// The words of a sub-bitstring and its `BinRef` (`ERL_REFC_BITS_SIZE`,
/// `erl_bits.h:147`).
const REFC_BITS_WORDS: usize = 8;

/// The largest number of bytes that ERTS keeps in a heap binary
/// (`ERL_ONHEAP_BINARY_LIMIT`, `erl_bits.h:167`).
const ONHEAP_BINARY_LIMIT: usize = 64;

/// Return the ERTS heap word size of `term` (`size_object`, `copy.c:88`).
///
/// A binary of at most 64 bytes counts as a heap binary and a larger one as
/// a reference-counted binary, because PBeam does not keep the ERTS form of
/// a binary (`docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(flat_size(&Term::Float(1.0)), 2);
/// ```
pub(crate) fn flat_size(term: &Term) -> usize {
    let mut words = 0;
    let mut pending = vec![term];
    while let Some(term) = pending.pop() {
        words += match term {
            // A match context never reaches a BIF: the compiler converts it
            // to a binary first.
            Term::Atom(_) | Term::Nil | Term::Pid(_) | Term::Port(_) | Term::MatchContext(_) => 0,
            Term::Integer(_) if term.small_integer().is_some() => 0,
            // One 64-bit digit and the header (`copy.c:244-246`).
            Term::Integer(_) => 2,
            Term::BigInt(value) => {
                usize::try_from(value.bits().div_ceil(64)).expect("a digit count fits in usize") + 1
            }
            Term::Float(_) => FLOAT_WORDS,
            Term::Reference(reference) if reference.is_pid_reference() => PID_REFERENCE_WORDS,
            Term::Reference(_) | Term::Resource(_) => REFERENCE_WORDS,
            // The header, the next pointer, the node, and the data words:
            // one for a pid or a port, and the 32-bit count and the numbers
            // of a reference in 64-bit words (`EXTERNAL_THING_HEAD_SIZE`,
            // `erl_term.h:1150-1230`, `external.c:4859-4866`).
            Term::External(id) => match &id.data {
                ExternalData::Pid { .. } | ExternalData::Port(_) => EXTERNAL_WORDS,
                ExternalData::Reference(numbers) => EXTERNAL_WORDS + numbers.len() / 2,
            },
            Term::Binary(binary) => binary_words(binary.bit_size()),
            Term::Cons(cell) => {
                pending.push(&cell.head);
                pending.push(&cell.tail);
                2
            }
            // The empty tuple is a global literal of no words
            // (`copy.c:130-140`).
            Term::Tuple(tuple) if tuple.is_empty() => 0,
            Term::Tuple(tuple) => {
                pending.extend(tuple.iter());
                tuple.len() + 1
            }
            Term::Fun(fun) => {
                pending.extend(fun.free());
                FUN_WORDS + fun.free().len()
            }
            // The instance words and the definition tuple, which ERTS keeps
            // as a literal and counts (`erl_record.h:41-77`).
            Term::Record(record) => {
                pending.push(&record.def().tuple);
                pending.extend(record.values());
                record.values().len() + 2
            }
            Term::Map(map) => match &**map.arc() {
                // The header, the size, the key tuple pointer, the values,
                // and the key tuple (`copy.c:174-191`).
                Repr::Flat(pairs) => {
                    for (key, value) in pairs {
                        pending.push(key);
                        pending.push(value);
                    }
                    let keys = if pairs.is_empty() { 0 } else { pairs.len() + 1 };
                    pairs.len() + 3 + keys
                }
                // A head node has a size word (`copy.c:192-214`).
                Repr::Hash(hamt) => 1 + node_words(&hamt.root, &mut pending),
            },
        };
    }
    words
}

/// Return the words of a bitstring of `bits` bits: a heap binary
/// (`heap_bits_size`, `erl_bits.h:156-161`) or a sub-bitstring with its
/// `BinRef` (`copy.c:219-231`).
fn binary_words(bits: usize) -> usize {
    let bytes = bits.div_ceil(8);
    if bytes <= ONHEAP_BINARY_LIMIT {
        2 + bytes.div_ceil(8)
    } else {
        REFC_BITS_WORDS
    }
}

/// Return the words of the HAMT node `node` and its child nodes, without
/// the size word of a head node, and push the keys and the values of its
/// leaves. A node has a header and one word for each child; a leaf is a
/// cons cell of a key and a value, and a collision node is a tuple of such
/// cells (`copy.c:192-214`, `erl_map.c:879-883`). The depth of a HAMT is
/// bounded, so the recursion is short.
fn node_words<'a>(node: &'a Node, pending: &mut Vec<&'a Term>) -> usize {
    let mut words = 1 + node.children.len();
    for child in &node.children {
        words += match child {
            Child::Leaf(key, value) => {
                pending.push(key);
                pending.push(value);
                2
            }
            Child::Node(node) => node_words(node, pending),
            Child::Collision(pairs) => {
                for (key, value) in pairs {
                    pending.push(key);
                    pending.push(value);
                }
                1 + 3 * pairs.len()
            }
        };
    }
    words
}
