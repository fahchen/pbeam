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
//! so a deep term does not recurse. An improper list mixes its tail after its
//! cells, as the ERTS cons cells give it. A reference hashes as an ERTS local
//! reference that scheduler 1 made with the PBeam reference counter as its
//! value (`erts_set_ref_numbers` in `erl_bif_unique.h`).
//!
//! A local fun mixes the index and the old unique value of its fun entry,
//! the atom word of its module, and then its free variables. An external fun
//! mixes the address of its ERTS export entry, which changes from run to run,
//! so PBeam mixes the word `(module index << 40) | (function index << 20) |
//! arity` in its place. Equal external funs give equal words.

use super::{
    Fun, Map, PortId, ProcessId, Reference, Target, Term,
    map::{Child, Form, Node},
    match_context::MATCH_CONTEXT_ESCAPE,
};

/// The width of the tag of an ERTS small integer or pid word
/// (`_TAG_IMMED1_SIZE`).
const IMMED1_TAG_BITS: u32 = 4;
/// The tag bits of an ERTS small integer word (`_TAG_IMMED1_SMALL`).
const SMALL_INTEGER_TAG: u64 = 0xF;
/// The tag bits of an ERTS local pid word (`_TAG_IMMED1_PID`).
const PID_TAG: u64 = 0x3;
/// The tag bits of an ERTS local port word (`_TAG_IMMED1_PORT`,
/// `erl_term.h:82`).
const PORT_TAG: u64 = 0x7;
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
/// assert_eq!(internal_hash(&Term::tuple(vec![])), 4510301282025278020);
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
    List(Cells<'a>),
    Tuple(&'a [Term]),
    Map(&'a Map),
    Binary(&'a [u8]),
    /// An integer outside the small-integer range.
    Bignum(i64),
    Reference(Reference),
    Fun(&'a Fun),
}

/// Return the ERTS layout of a term.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(shape(&Term::static_atom("true")), Shape::Immediate(0x4B)));
/// assert!(matches!(shape(&Term::nil()), Shape::Immediate(0x3B)));
/// assert!(matches!(shape(&Term::Integer(1 << 59)), Shape::Bignum(_)));
/// ```
fn shape(term: &Term) -> Shape<'_> {
    match term {
        Term::Atom(atom) => Shape::Immediate(atom_word(atom.index())),
        Term::Pid(ProcessId(number)) => Shape::Immediate((number << IMMED1_TAG_BITS) | PID_TAG),
        Term::Port(PortId(number)) => Shape::Immediate((number << IMMED1_TAG_BITS) | PORT_TAG),
        Term::Nil => Shape::Immediate(NIL),
        Term::Cons(_) => Shape::List(Cells {
            first: None,
            rest: term,
        }),
        Term::Integer(value) => match term.small_integer() {
            Some(small) => {
                Shape::Immediate(((small as u64) << IMMED1_TAG_BITS) | SMALL_INTEGER_TAG)
            }
            None => Shape::Bignum(*value),
        },
        Term::Tuple(elements) => Shape::Tuple(elements),
        Term::Map(map) => Shape::Map(map),
        Term::Binary(bytes) => Shape::Binary(bytes),
        Term::Reference(reference) => Shape::Reference(*reference),
        Term::Fun(fun) => Shape::Fun(fun),
        Term::MatchContext(_) => unreachable!("{MATCH_CONTEXT_ESCAPE}"),
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

/// The ERTS cons cells of a list from one cell on: the head `first`, when
/// it is `Some`, and then the cons cells of `rest`. The term after the last
/// cell is the tail: `[]` for a proper list. `first` is `Some` only for the
/// `[Key | Value]` cell of a collision node, where `rest` is the value.
#[derive(Clone, Copy)]
struct Cells<'a> {
    first: Option<&'a Term>,
    rest: &'a Term,
}

impl<'a> Cells<'a> {
    /// Return the head of the next cell and the cells after it, or the tail
    /// when no cell is left.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = Term::list_from([Term::Integer(120)]);
    /// let cells = Cells { first: Some(&Term::Integer(1)), rest: &value };
    /// let Ok((head, cells)) = cells.split() else { unreachable!() };
    /// assert_eq!(head, &Term::Integer(1));
    /// assert!(matches!(cells.split(), Ok((Term::Integer(120), _))));
    /// assert!(matches!(Cells { first: None, rest: &Term::nil() }.split(), Err(Term::Nil)));
    /// ```
    fn split(self) -> Result<(&'a Term, Cells<'a>), &'a Term> {
        let (head, rest) = match self.first {
            Some(head) => (head, self.rest),
            None => self.rest.cons_cell().ok_or(self.rest)?,
        };
        Ok((head, Cells { first: None, rest }))
    }

    /// Return true when no cell is left.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Cells { first: None, rest: &Term::Integer(1) }.is_empty());
    /// ```
    fn is_empty(self) -> bool {
        self.split().is_err()
    }
}

/// One step of the `make_internal_hash` work stack.
enum Item<'a> {
    /// A term to hash.
    Term(&'a Term),
    /// The cells of a list from one cell on, and the tail after them.
    Cells(Cells<'a>),
    /// A hashmap collision node, which ERTS hashes as the tuple of its
    /// `[Key | Value]` cells.
    Collision(&'a [(Term, Term)]),
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
    /// state.push_term(&mut stack, &Term::tuple(vec![]));
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
    /// let map = Map::from_pairs(pairs, &AtomTable::new())?;
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
                Child::Collision(pairs) => stack.push(Item::Collision(pairs)),
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
/// assert_eq!(make_internal_hash(&Term::binary(vec![])), 10873612744977862784);
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
            Item::Collision(pairs) => next = state.mix_collision(&mut stack, pairs),
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
    /// let term = Term::tuple(vec![Term::tuple(vec![]), Term::Integer(1)]);
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
            Shape::Reference(reference) => {
                let [number_0, number_1, number_2] = reference.numbers().map(u64::from);
                self.mix_alpha(pair(TYPE_LOCAL_REF, number_0));
                self.mix_beta(pair(number_1, number_2));
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

    /// Mix the cells of a list from one cell on, with the tail after them,
    /// as the list path of `make_internal_hash` does
    /// (`erl_term_hashing.c:1783-1841`): first the bytes in 4-byte string
    /// chunks, then one element that is not a byte. Return the item that
    /// continues at once.
    ///
    /// An immediate head is mixed at once, and the tail continues. A boxed
    /// head continues, and the tail waits on the work stack, after a CDR
    /// mark when the tail is not a list. With no cells, the tail continues
    /// as a term, so `[]` mixes as an immediate
    /// (`erl_term_hashing.c:2211-2213`, 2247-2251).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (mut state, mut stack) = (State::default(), Vec::new());
    /// let list = Term::list_from([Term::Integer(104), Term::tuple(vec![])]);
    /// let cells = Cells { first: None, rest: &list };
    /// assert!(matches!(state.mix_cells(&mut stack, cells), Some(Item::Term(_))));
    /// assert!(matches!(stack[..], [Item::Cells(_), Item::CdrMarker]));
    /// // The `[Key | Value]` cell of a collision node with the value {}.
    /// let (key, value) = (Term::Integer(1), Term::tuple(vec![]));
    /// let next = state.mix_cells(&mut stack, Cells { first: Some(&key), rest: &value });
    /// assert!(matches!(next, Some(Item::Cells(_))));
    /// ```
    fn mix_cells<'a>(
        &mut self,
        stack: &mut Vec<Item<'a>>,
        mut cells: Cells<'a>,
    ) -> Option<Item<'a>> {
        let (mut chunk, mut count) = (0, 0);
        while let Ok((head, after)) = cells.split()
            && let Some(value) = byte(head)
        {
            chunk = (chunk << 8) | value;
            count += 1;
            if count == STRING_CHUNK {
                self.mix_alpha(pair(TYPE_STRING | (count << 8), chunk));
                (chunk, count) = (0, 0);
            }
            cells = after;
        }
        if count > 0 {
            self.mix_alpha(pair(TYPE_STRING | (count << 8), chunk));
        }
        let (head, cells) = match cells.split() {
            Ok(split) => split,
            Err(tail) => return Some(Item::Term(tail)),
        };
        match shape(head) {
            Shape::Immediate(word) => {
                self.mix_alpha(pair(TYPE_IMMEDIATE, TYPE_CAR));
                self.mix_beta(word);
                if cells.is_empty() {
                    self.mix_alpha(TYPE_CDR);
                }
                Some(Item::Cells(cells))
            }
            _ => {
                stack.push(Item::Cells(cells));
                if cells.is_empty() {
                    stack.push(Item::CdrMarker);
                }
                self.mix_alpha(TYPE_CAR);
                Some(Item::Term(head))
            }
        }
    }

    /// Mix a hashmap collision node as the tuple it is in ERTS: a tuple of
    /// the `[Key | Value]` cells of its pairs, in their map-key order
    /// (`erl_map.h:197`, 205-210; tuple path `erl_term_hashing.c:1878-1893`).
    /// The node itself mixes no array element mark, as a pushed inner node
    /// (`erl_term_hashing.c:1976-1983`). Each cell except the last waits on
    /// the work stack, and the last cell continues at once.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (mut state, mut stack) = (State::default(), Vec::new());
    /// let pairs = [(a, Term::Integer(100)), (b, Term::Integer(200))];
    /// let next = state.mix_collision(&mut stack, &pairs);
    /// // The tuple mixes its type and arity, then one array element.
    /// assert_eq!(state.ticks, 3);
    /// assert!(matches!(stack[..], [Item::Cells(Cells { rest: Term::Integer(100), .. })]));
    /// assert!(matches!(next, Some(Item::Cells(Cells { rest: Term::Integer(200), .. }))));
    /// ```
    fn mix_collision<'a>(
        &mut self,
        stack: &mut Vec<Item<'a>>,
        pairs: &'a [(Term, Term)],
    ) -> Option<Item<'a>> {
        self.mix_alpha(TYPE_TUPLE);
        self.mix_beta(pairs.len() as u64);
        let cell = |(key, value): &'a (Term, Term)| {
            Item::Cells(Cells {
                first: Some(key),
                rest: value,
            })
        };
        let (last, pairs) = pairs.split_last()?;
        for pair in pairs {
            self.mix_alpha(TYPE_ARRAY_ELEMENT);
            stack.push(cell(pair));
        }
        Some(cell(last))
    }
}

#[cfg(test)]
#[path = "../../tests/term/hash.rs"]
mod tests;
