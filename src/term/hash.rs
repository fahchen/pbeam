//! The ERTS internal term hash, as in ERTS OTP 29.1.1 (`erts_internal_hash`
//! and `make_internal_hash` in `erl_term_hashing.c`).
//!
//! An immediate term hashes as `ihash_mix64` of its ERTS word:
//!
//! - an atom: `(index << 6) | 0xB`, so `true` (index 1) is `0x4B`;
//! - a small integer: `(value << 4) | 0xF`, so `1` is `0x1F`;
//! - `[]`: `0x3B`;
//! - a pid: `(number << 4) | 0x3`, with the PBeam process number as the
//!   ERTS pid data.
//!
//! Any other term goes through `make_internal_hash`, a MurmurHash3 variant
//! with two 64-bit states. It reads the term with a work stack, as ERTS does,
//! so a deep term does not recurse. A reference hashes as an ERTS local
//! reference that scheduler 1 made with the PBeam reference counter as its
//! value (`erts_set_ref_numbers` in `erl_bif_unique.h`).
//!
//! A local fun mixes the index and the old unique value of its fun entry,
//! the atom word of its module, and then its free variables. An external fun
//! mixes the address of its ERTS export entry, which changes from run to run,
//! so PBeam mixes the word `(module index << 40) | (function index << 20) |
//! arity` in its place. Equal external funs give equal words.

use super::{
    Fun, Map, ProcessId, Reference, Target, Term,
    map::{Child, Form, Node},
};

/// The width of the tag of an ERTS small integer or pid word
/// (`_TAG_IMMED1_SIZE`).
const IMMED1_TAG_BITS: u32 = 4;
/// The tag bits of an ERTS small integer word (`_TAG_IMMED1_SMALL`).
const SMALL_INTEGER_TAG: u64 = 0xF;
/// The tag bits of an ERTS local pid word (`_TAG_IMMED1_PID`).
const PID_TAG: u64 = 0x3;
/// The width of the tag of an ERTS atom word (`_TAG_IMMED2_SIZE`).
const ATOM_TAG_BITS: u32 = 6;
/// The tag bits of an ERTS atom word (`_TAG_IMMED2_ATOM`).
const ATOM_TAG: u64 = 0xB;
/// The ERTS word of `[]` (`NIL`, `_TAG_IMMED2_NIL`).
const NIL: u64 = 0x3B;

/// The type codes that `make_internal_hash` mixes in (`IHASH_TYPE_*`).
const TYPE_IMMEDIATE: u64 = 1;
const TYPE_ARRAY_ELEMENT: u64 = 2;
const TYPE_CAR: u64 = 3;
const TYPE_CDR: u64 = 4;
const TYPE_STRING: u64 = 5;
const TYPE_TUPLE: u64 = 6;
const TYPE_FLATMAP: u64 = 8;
const TYPE_HASHMAP_HEAD_ARRAY: u64 = 9;
const TYPE_HASHMAP_HEAD_BITMAP: u64 = 10;
const TYPE_HASHMAP_NODE: u64 = 11;
const TYPE_BINARY: u64 = 12;
const TYPE_LOCAL_FUN: u64 = 13;
const TYPE_EXTERNAL_FUN: u64 = 14;
const TYPE_NEG_BIGNUM: u64 = 15;
const TYPE_POS_BIGNUM: u64 = 16;
const TYPE_LOCAL_REF: u64 = 17;

/// The MurmurHash3 multipliers (`IHASH_C1`, `IHASH_C2`).
const C1: u64 = 0x87C3_7B91_1142_53D5;
const C2: u64 = 0x4CF5_AD43_2745_937F;

/// The string chunk size: `make_internal_hash` mixes list elements in
/// 0..=255 four at a time.
const STRING_CHUNK: u64 = 4;
/// The binary block size: two 64-bit words, one for each state.
const BINARY_BLOCK: usize = 16;

/// The mask of the first reference number (`REF_MASK`, 18 bits).
const REF_NUMBER_0_MASK: u64 = 0x3_FFFF;
/// The mask of the counter bits in the second reference number
/// (`ERTS_REF1_NUM_MASK__`).
const REF_NUMBER_1_MASK: u64 = 0xFFFC_0000;
/// The scheduler thread number of a PBeam reference. ERTS gives the first
/// normal scheduler the number 1 (`init_scheduler_data` in
/// `erl_process.c`).
const REF_THREAD: u64 = 1;

/// The shift of the module atom index in the export word of an external
/// fun. An atom index is below `ATOM_LIMIT`, 2^20.
const EXPORT_MODULE_SHIFT: u32 = 40;
/// The shift of the function atom index in the export word of an external
/// fun. The arity is at most 255 (`MAX_ARG`), below the function index.
const EXPORT_FUNCTION_SHIFT: u32 = 20;

/// Return the ERTS internal hash of a term (`erts_internal_hash`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(internal_hash(&Term::Integer(1)), 7938660269479274665);
/// assert_eq!(internal_hash(&Term::Tuple(vec![])), 4510301282025278020);
/// ```
pub(crate) fn internal_hash(term: &Term) -> u64 {
    match shape(term) {
        Shape::Immediate(word) => mix64(word),
        _ => make_internal_hash(term),
    }
}

/// The ERTS layout of a term: an immediate word, or the content of a term
/// that ERTS keeps on the heap.
enum Shape<'a> {
    /// The word of an atom, a small integer, `[]`, or a pid.
    Immediate(u64),
    /// The cells of a list that is not empty.
    List(&'a [Term]),
    Tuple(&'a [Term]),
    Map(&'a Map),
    Binary(&'a [u8]),
    /// An integer outside the small-integer range.
    Bignum(i64),
    /// The counter value of a reference.
    Reference(u64),
    Fun(&'a Fun),
}

/// Return the ERTS layout of a term.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(shape(&Term::static_atom("true")), Shape::Immediate(0x4B)));
/// assert!(matches!(shape(&Term::List(vec![])), Shape::Immediate(0x3B)));
/// assert!(matches!(shape(&Term::Integer(1 << 59)), Shape::Bignum(_)));
/// ```
fn shape(term: &Term) -> Shape<'_> {
    match term {
        Term::Atom(atom) => Shape::Immediate(atom_word(atom.index())),
        Term::Pid(ProcessId(number)) => Shape::Immediate((number << IMMED1_TAG_BITS) | PID_TAG),
        Term::List(cells) if cells.is_empty() => Shape::Immediate(NIL),
        Term::List(cells) => Shape::List(cells),
        Term::Integer(value) => match term.small_integer() {
            Some(small) => {
                Shape::Immediate(((small as u64) << IMMED1_TAG_BITS) | SMALL_INTEGER_TAG)
            }
            None => Shape::Bignum(*value),
        },
        Term::Tuple(elements) => Shape::Tuple(elements),
        Term::Map(map) => Shape::Map(map),
        Term::Binary(bytes) => Shape::Binary(bytes),
        Term::Reference(Reference(value)) => Shape::Reference(*value),
        Term::Fun(fun) => Shape::Fun(fun),
    }
}

/// Return the ERTS word of the atom with an atom-table index.
///
/// # Examples
///
/// ```rust,ignore
/// // true has index 1.
/// assert_eq!(atom_word(1), 0x4B);
/// ```
fn atom_word(index: u32) -> u64 {
    (u64::from(index) << ATOM_TAG_BITS) | ATOM_TAG
}

/// Return the byte value of a list element, as the ERTS check `is_byte`
/// does: a small integer in 0..=255.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(byte(&Term::Integer(255)), Some(255));
/// assert_eq!(byte(&Term::Integer(256)), None);
/// ```
fn byte(term: &Term) -> Option<u64> {
    match term {
        Term::Integer(value @ 0..=255) => Some(*value as u64),
        _ => None,
    }
}

/// Mix a 64-bit word, as ERTS `ihash_mix64` does (the MurmurHash3 fmix64
/// finalizer). The function is a bijection on `u64`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(mix64(0x1F), 7938660269479274665);
/// ```
fn mix64(mut hash: u64) -> u64 {
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    hash ^ (hash >> 33)
}

/// Join two 32-bit fields into one word, as `IHASH_MIX_*_2F32` does.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pair(1, 2), 0x2_0000_0001);
/// ```
fn pair(low: u64, high: u64) -> u64 {
    low | (high << 32)
}

/// One step of the `make_internal_hash` work stack.
enum Item<'a> {
    /// A term to hash.
    Term(&'a Term),
    /// The cells of a list from one element to the end. No cells is the
    /// tail `[]`.
    Cells(&'a [Term]),
    /// An inner hashmap node.
    Node(&'a Node),
    /// The mark before a list tail that is not a list (`IHASH_CDR_MARKER`).
    CdrMarker,
}

/// The two MurmurHash3 states and the number of mixed words.
#[derive(Default)]
struct State {
    alpha: u64,
    beta: u64,
    ticks: u64,
}

impl State {
    /// Mix a word into the alpha state (`IHASH_MIX_ALPHA`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut state = State::default();
    /// state.mix_alpha(1);
    /// assert_eq!(state.ticks, 1);
    /// ```
    fn mix_alpha(&mut self, word: u64) {
        self.alpha ^= word.wrapping_mul(C1).rotate_left(31).wrapping_mul(C2);
        self.alpha = self.alpha.rotate_left(27).wrapping_add(self.beta);
        self.alpha = self.alpha.wrapping_mul(5).wrapping_add(0x52DC_E729);
        self.ticks += 1;
    }

    /// Mix a word into the beta state (`IHASH_MIX_BETA`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut state = State::default();
    /// state.mix_beta(1);
    /// assert_eq!(state.ticks, 1);
    /// ```
    fn mix_beta(&mut self, word: u64) {
        self.beta ^= word.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
        self.beta = self.beta.rotate_left(31).wrapping_add(self.alpha);
        self.beta = self.beta.wrapping_mul(5).wrapping_add(0x3849_5AB5);
        self.ticks += 1;
    }

    /// Mix an immediate word (`IHASH_MIX_IMMEDIATE` on a 64-bit system).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut state = State::default();
    /// state.mix_immediate(NIL);
    /// assert_eq!(state.ticks, 2);
    /// ```
    fn mix_immediate(&mut self, word: u64) {
        self.mix_alpha(TYPE_IMMEDIATE);
        self.mix_beta(word);
    }

    /// Mix an immediate element at once, or mark a boxed element and push
    /// it onto the work stack (`IHASH_PUSH_TERM`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (mut state, mut stack) = (State::default(), Vec::new());
    /// state.push_term(&mut stack, &Term::Integer(1));
    /// assert!(stack.is_empty());
    /// state.push_term(&mut stack, &Term::Tuple(vec![]));
    /// assert_eq!(stack.len(), 1);
    /// ```
    fn push_term<'a>(&mut self, stack: &mut Vec<Item<'a>>, term: &'a Term) {
        match shape(term) {
            Shape::Immediate(word) => self.mix_immediate(word),
            _ => {
                self.mix_alpha(TYPE_ARRAY_ELEMENT);
                stack.push(Item::Term(term));
            }
        }
    }

    /// Mix the leaves of a hashmap node and push its inner nodes, in slot
    /// order. A leaf pushes its value and then its key, as the `[Key | Value]`
    /// cell of an ERTS leaf does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let pairs = (1..=33).map(|key| (Term::Integer(key), Term::Integer(key))).collect();
    /// let map = Map::from_pairs(pairs)?;
    /// let Form::Hash { root, .. } = map.form() else { unreachable!() };
    /// let (mut state, mut stack) = (State::default(), Vec::new());
    /// state.push_children(&mut stack, root);
    /// // Integer keys and values are immediate, so only inner nodes wait.
    /// assert!(stack.iter().all(|item| matches!(item, Item::Node(_))));
    /// ```
    fn push_children<'a>(&mut self, stack: &mut Vec<Item<'a>>, node: &'a Node) {
        for child in node.children() {
            match child {
                Child::Leaf(key, value) => {
                    self.push_term(stack, value);
                    self.push_term(stack, key);
                }
                Child::Node(inner) => stack.push(Item::Node(inner)),
            }
        }
    }

    /// Mix the bytes of a binary: each 16-byte block as two words, and the
    /// tail without a state rotation, as `make_internal_hash` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut state = State::default();
    /// state.mix_bytes(&[0; 17]);
    /// assert_eq!(state.ticks, 2);
    /// ```
    fn mix_bytes(&mut self, bytes: &[u8]) {
        let little_endian = |bytes: &[u8]| {
            bytes
                .iter()
                .rev()
                .fold(0, |word, byte| (word << 8) | u64::from(*byte))
        };
        let (blocks, tail) = bytes.as_chunks::<BINARY_BLOCK>();
        for block in blocks {
            let (low, high) = block.split_at(BINARY_BLOCK / 2);
            self.mix_alpha(little_endian(low));
            self.mix_beta(little_endian(high));
        }
        if tail.len() > BINARY_BLOCK / 2 {
            let word = little_endian(&tail[BINARY_BLOCK / 2..]);
            self.beta ^= word.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
        }
        if !tail.is_empty() {
            let word = little_endian(&tail[..tail.len().min(BINARY_BLOCK / 2)]);
            self.alpha ^= word.wrapping_mul(C1).rotate_left(31).wrapping_mul(C2);
        }
    }

    /// Return the final hash of the two states.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let hash = State::default().finish();
    /// ```
    fn finish(mut self) -> u64 {
        self.alpha ^= self.ticks;
        self.beta ^= self.ticks;
        self.alpha = self.alpha.wrapping_add(self.beta);
        self.beta = self.beta.wrapping_add(self.alpha);
        self.alpha = mix64(self.alpha);
        self.beta = mix64(self.beta);
        self.alpha = self.alpha.wrapping_add(self.beta);
        self.beta = self.beta.wrapping_add(self.alpha);
        self.alpha ^ self.beta
    }
}

/// Hash a term that is not immediate, as ERTS `make_internal_hash` with the
/// salt 0 does.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(make_internal_hash(&Term::Binary(vec![])), 10873612744977862784);
/// ```
fn make_internal_hash(term: &Term) -> u64 {
    let mut state = State::default();
    let mut stack = Vec::new();
    let mut next = Some(Item::Term(term));
    loop {
        let Some(item) = next.take().or_else(|| stack.pop()) else {
            return state.finish();
        };
        match item {
            Item::Term(term) => next = state.mix_term(&mut stack, term),
            Item::Cells(cells) => next = state.mix_cells(&mut stack, cells),
            Item::Node(node) => {
                state.mix_alpha(TYPE_HASHMAP_NODE);
                state.push_children(&mut stack, node);
            }
            Item::CdrMarker => state.mix_beta(TYPE_CDR),
        }
    }
}

impl State {
    /// Mix one term. The last element of a tuple and the last key of a
    /// flatmap continue at once, so the function returns them. Other boxed
    /// elements wait on the work stack.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (mut state, mut stack) = (State::default(), Vec::new());
    /// let term = Term::Tuple(vec![Term::Tuple(vec![]), Term::Integer(1)]);
    /// assert!(matches!(state.mix_term(&mut stack, &term), Some(Item::Term(Term::Integer(1)))));
    /// assert_eq!(stack.len(), 1);
    /// ```
    fn mix_term<'a>(&mut self, stack: &mut Vec<Item<'a>>, term: &'a Term) -> Option<Item<'a>> {
        match shape(term) {
            Shape::Immediate(word) => self.mix_immediate(word),
            Shape::List(cells) => return self.mix_cells(stack, cells),
            Shape::Tuple(elements) => {
                self.mix_alpha(TYPE_TUPLE);
                self.mix_beta(elements.len() as u64);
                let (last, elements) = elements.split_last()?;
                for element in elements {
                    self.push_term(stack, element);
                }
                return Some(Item::Term(last));
            }
            Shape::Map(map) => match map.form() {
                Form::Flat(pairs) => {
                    self.mix_alpha(TYPE_FLATMAP);
                    self.mix_beta(pairs.len() as u64);
                    let ((last_key, last_value), pairs) = pairs.split_last()?;
                    for (key, value) in pairs {
                        self.push_term(stack, value);
                        self.push_term(stack, key);
                    }
                    self.push_term(stack, last_value);
                    return Some(Item::Term(last_key));
                }
                Form::Hash { size, root } => {
                    self.mix_alpha(if root.is_full() {
                        TYPE_HASHMAP_HEAD_ARRAY
                    } else {
                        TYPE_HASHMAP_HEAD_BITMAP
                    });
                    self.mix_beta(size as u64);
                    self.push_children(stack, root);
                }
            },
            Shape::Binary(bytes) => {
                // ERTS mixes the size in bits.
                self.mix_alpha(TYPE_BINARY);
                self.mix_beta(bytes.len() as u64 * 8);
                self.mix_bytes(bytes);
            }
            Shape::Bignum(value) => {
                // The magnitude of a 64-bit value is one 64-bit bignum digit.
                let sign = if value < 0 {
                    TYPE_NEG_BIGNUM
                } else {
                    TYPE_POS_BIGNUM
                };
                self.mix_alpha(pair(sign, 1));
                self.mix_beta(value.unsigned_abs());
            }
            Shape::Reference(value) => {
                self.mix_alpha(pair(TYPE_LOCAL_REF, value & REF_NUMBER_0_MASK));
                self.mix_beta(pair((value & REF_NUMBER_1_MASK) | REF_THREAD, value >> 32));
            }
            Shape::Fun(fun) => match fun.target() {
                Target::Local(lambda) => {
                    let free = fun.free();
                    self.mix_alpha(pair(TYPE_LOCAL_FUN, free.len() as u64));
                    // ERTS widens the C `int` fields with their sign.
                    self.mix_beta(pair(
                        i64::from(lambda.index) as u64,
                        i64::from(lambda.old_unique) as u64,
                    ));
                    self.mix_immediate(atom_word(lambda.module().index()));
                    let (last, free) = free.split_last()?;
                    for value in free {
                        self.push_term(stack, value);
                    }
                    return Some(Item::Term(last));
                }
                Target::External {
                    module,
                    function,
                    arity,
                } => {
                    self.mix_alpha(TYPE_EXTERNAL_FUN);
                    self.mix_beta(
                        (u64::from(module.index()) << EXPORT_MODULE_SHIFT)
                            | (u64::from(function.index()) << EXPORT_FUNCTION_SHIFT)
                            | u64::from(*arity),
                    );
                }
            },
        }
        None
    }

    /// Mix the cells of a list from one element: first the bytes in 4-byte
    /// string chunks, then one element that is not a byte. Return the item
    /// that continues at once.
    ///
    /// An immediate head is mixed at once, and the tail continues. A boxed
    /// head continues, and the tail waits on the work stack, after a CDR
    /// mark when the tail is `[]`. No cells is the tail `[]`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (mut state, mut stack) = (State::default(), Vec::new());
    /// let cells = [Term::Integer(104), Term::Tuple(vec![])];
    /// assert!(matches!(state.mix_cells(&mut stack, &cells), Some(Item::Term(_))));
    /// assert!(matches!(stack[..], [Item::Cells([]), Item::CdrMarker]));
    /// ```
    fn mix_cells<'a>(
        &mut self,
        stack: &mut Vec<Item<'a>>,
        mut cells: &'a [Term],
    ) -> Option<Item<'a>> {
        let (mut chunk, mut count) = (0, 0);
        while let Some(value) = cells.first().and_then(byte) {
            chunk = (chunk << 8) | value;
            count += 1;
            if count == STRING_CHUNK {
                self.mix_alpha(pair(TYPE_STRING | (count << 8), chunk));
                (chunk, count) = (0, 0);
            }
            cells = &cells[1..];
        }
        if count > 0 {
            self.mix_alpha(pair(TYPE_STRING | (count << 8), chunk));
        }
        let Some((head, tail)) = cells.split_first() else {
            self.mix_immediate(NIL);
            return None;
        };
        match shape(head) {
            Shape::Immediate(word) => {
                self.mix_alpha(pair(TYPE_IMMEDIATE, TYPE_CAR));
                self.mix_beta(word);
                if tail.is_empty() {
                    self.mix_alpha(TYPE_CDR);
                }
                Some(Item::Cells(tail))
            }
            _ => {
                stack.push(Item::Cells(tail));
                if tail.is_empty() {
                    stack.push(Item::CdrMarker);
                }
                self.mix_alpha(TYPE_CAR);
                Some(Item::Term(head))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{internal_hash, mix64};
    use crate::term::{Atom, Fun, Lambda, Map, ProcessId, Reference, Term};

    fn atom(name: &str) -> Term {
        Term::static_atom(name)
    }

    fn integers(values: &[i64]) -> Term {
        Term::List(values.iter().map(|value| Term::Integer(*value)).collect())
    }

    fn map(pairs: impl IntoIterator<Item = (Term, Term)>) -> Term {
        Term::Map(Map::from_pairs(pairs.into_iter().collect()).unwrap())
    }

    #[test]
    fn mixes_a_word_as_erts_ihash_mix64() {
        assert_eq!(mix64(0x1F), 7938660269479274665);
        // true has index 1: its ERTS word is (1 << 6) | 0xB = 0x4B.
        assert_eq!(mix64(0x4B), 14558535449716669319);
    }

    #[test]
    fn hashes_each_term_kind_as_pinned_erts() {
        // Each value is erts_debug:get_internal_state({internal_hash, Term}) on ERTS 29.1.1.
        let tuple = |elements: Vec<Term>| Term::Tuple(elements);
        let cases = [
            (atom("ok"), 4036026909208831136),
            (Term::Integer(1), 7938660269479274665),
            (Term::Integer(-1), 7256831767414464289),
            (Term::List(vec![]), 16046243290777390298),
            (tuple(vec![]), 4510301282025278020),
            (
                tuple(vec![atom("ok"), Term::Integer(1)]),
                13883435329789904289,
            ),
            (
                tuple(vec![Term::Integer(1), tuple(vec![Term::Integer(2)])]),
                15167167310543595139,
            ),
            (Term::List(vec![atom("ok")]), 466713979249896199),
            (
                integers(b"hello".map(i64::from).as_slice()),
                5671500630603279798,
            ),
            (integers(&[300]), 15858254366034436735),
            (
                Term::List(vec![tuple(vec![atom("ok")])]),
                15994800526372216946,
            ),
            (Term::List(vec![Term::List(vec![])]), 7939938580689537527),
            (
                Term::List(vec![Term::Binary(vec![]), Term::Integer(2)]),
                2437268029134553953,
            ),
            (
                Term::List(vec![Term::Integer(1), tuple(vec![]), Term::Integer(2)]),
                11634306265473751121,
            ),
            (Term::Binary(vec![]), 10873612744977862784),
            (Term::Binary(vec![1, 2, 3]), 3963608694498431467),
            (
                Term::Binary(b"0123456789abcde".to_vec()),
                13603490536549166064,
            ),
            (
                Term::Binary(b"0123456789abcdef".to_vec()),
                8513549014846254092,
            ),
            (
                Term::Binary(b"0123456789abcdefg".to_vec()),
                11292494362027136432,
            ),
            (Term::Integer(1 << 59), 10054563828903259082),
            (Term::Integer(-(1 << 59) - 1), 12317164193119007090),
            (Term::Integer(i64::MAX), 1866222494570913558),
            (Term::Integer(i64::MIN), 11874823041652698399),
            (map([]), 9376759162891905861),
            (
                map([
                    (atom("ok"), Term::List(vec![])),
                    (Term::Integer(1), Term::Binary(vec![])),
                ]),
                13992029790355293803,
            ),
            // maps:from_list([{I, I} || I <- lists:seq(1, 33)]): a HEAD_BITMAP root.
            (
                map((1..=33).map(|key| (Term::Integer(key), Term::Integer(key)))),
                178448403113005431,
            ),
            // maps:from_list([{I, [I]} || I <- lists:seq(1, 200)]): a HEAD_ARRAY root
            // with inner nodes.
            (
                map((1..=200).map(|key| (Term::Integer(key), integers(&[key])))),
                5428453677902343107,
            ),
            // <0.10.0> has the pid word 0x503, so its ERTS pid data is 0x50.
            (Term::Pid(ProcessId(0x50)), 8954564308403532307),
            // #Ref<0.4116665985.1537736705.33174> has the numbers 0x8196, 0x5BA80001, and
            // 0xF55F5681: the counter value 0xF55F56815BA88196 of scheduler 1.
            (
                Term::Reference(Reference(0xF55F_5681_5BA8_8196)),
                11474811932501652611,
            ),
        ];
        for (term, hash) in cases {
            assert_eq!(internal_hash(&term), hash, "{term:?}");
        }
    }

    /// Return a local fun of `lists` with a fun entry and free variables.
    fn local(index: i32, old_unique: i32, arity: u32, free: Vec<Term>) -> Term {
        let module = Atom::from_static("lists").unwrap();
        Term::Fun(Fun::new(
            Lambda::new(module, index, old_unique, 53, arity),
            free,
        ))
    }

    fn external(function: &str, arity: u32) -> Term {
        let atom = |name| Atom::from_static(name).unwrap();
        Term::Fun(Fun::external(atom("lists"), atom(function), arity))
    }

    #[test]
    fn hashes_local_funs_as_pinned_erts() {
        // Each value is erts_debug:get_internal_state({internal_hash, Term}) on ERTS 29.1.1,
        // for a fun that binary_to_term/1 makes from NEW_FUN_EXT with the module lists, the
        // index, the old index, the old unique value, and the free variables.
        let zero = || local(0, 7, 1, vec![]);
        let free = local(0, 7, 1, vec![Term::Integer(1), Term::Tuple(vec![])]);
        let negative = || local(3, -5, 2, vec![Term::Binary(b"ab".to_vec()), atom("ok")]);
        let cases = [
            (zero(), 16900316212850717680),
            (free.clone(), 15970829942560899780),
            (negative(), 12860429061993126401),
            (
                Term::Tuple(vec![zero(), Term::Integer(1)]),
                9006137623669004246,
            ),
            (
                map([(atom("ok"), free), (Term::Integer(1), zero())]),
                14001711515840955162,
            ),
            (Term::List(vec![negative()]), 1176312308706464788),
        ];
        for (term, hash) in cases {
            assert_eq!(internal_hash(&term), hash, "{term:?}");
        }
        // The entry label only selects code, so it is not part of the hash.
        let module = Atom::from_static("lists").unwrap();
        let relabeled = Term::Fun(Fun::new(Lambda::new(module, 0, 7, 61, 1), vec![]));
        assert_eq!(internal_hash(&relabeled), internal_hash(&zero()));
    }

    #[test]
    fn hashes_external_funs_by_their_mfa() {
        // ERTS mixes the export entry address, which changes from run to run:
        // fun lists:reverse/1 gave 638061310806519221 and 18158401298541009015.
        // PBeam mixes a word of the module, function, and arity in its place.
        let reverse = external("reverse", 1);
        assert_eq!(internal_hash(&reverse), internal_hash(&reverse.clone()));
        let funs = [
            reverse,
            external("reverse", 2),
            external("member", 1),
            Term::Fun(Fun::external(
                Atom::from_static("erlang").unwrap(),
                Atom::from_static("reverse").unwrap(),
                1,
            )),
            local(0, 7, 1, vec![]),
        ];
        for (index, term) in funs.iter().enumerate() {
            for other in &funs[index + 1..] {
                assert_ne!(internal_hash(term), internal_hash(other), "{term:?}");
            }
        }
    }

    #[test]
    fn hashes_a_deep_term_without_recursion() {
        let deep = |levels: usize| {
            (0..levels).fold(Term::List(vec![]), |inner, level| match level % 4 {
                0 => Term::Tuple(vec![inner, Term::Binary(vec![])]),
                1 => Term::List(vec![inner, Term::Integer(1)]),
                2 => map([(atom("ok"), inner)]),
                _ => local(0, 7, 1, vec![inner, Term::Integer(1)]),
            })
        };
        let hash = internal_hash(&deep(1_000_000));
        assert_eq!(hash, internal_hash(&deep(1_000_000)));
        assert_ne!(hash, internal_hash(&deep(999_999)));
    }
}
