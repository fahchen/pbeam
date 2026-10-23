//! The ERTS term hashes, as in ERTS OTP 29.1.1 (`erl_term_hashing.c`): the
//! internal hash (`erts_internal_hash` and `make_internal_hash`), the old
//! term hash `make_hash` of `erlang:phash/2`, and the term hash
//! `make_hash2` of `erlang:phash2/1,2`. The atom table hash `atom_hash`
//! (`atom.c:83-105`) here serves the two phash hashes and the stored hash of
//! an ETS key.
//!
//! An immediate term hashes as `ihash_mix64` of its ERTS word:
//!
//! - an atom: `(index << 6) | 0xB`, so `true` (index 1) is `0x4B`;
//! - a small integer: `(value << 4) | 0xF`, so `1` is `0x1F`;
//! - `[]`: `0x3B`;
//! - a pid: `(pixdata(number) << 4) | 0x3`, with the PBeam process number
//!   as the ERTS pid data, spread over the table index bits.
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

use num_bigint::Sign;

use super::{
    Atom, AtomTable, Binary, Fun, Map, PortId, ProcessId, Reference, Target, Term,
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
/// `IHASH_TYPE_FLOAT`, after the three external types 18 to 20
/// (`erl_term_hashing.c:1653-1657`).
const TYPE_FLOAT: u64 = 21;

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
    Binary(&'a Binary),
    /// An integer outside the small-integer range: its sign and its
    /// magnitude in 64-bit digits, least significant first.
    Bignum(bool, Vec<u64>),
    /// The bits of a float.
    Float(u64),
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
        Term::Pid(ProcessId(number)) => {
            Shape::Immediate((pixdata(*number) << IMMED1_TAG_BITS) | PID_TAG)
        }
        Term::Port(PortId(number)) => {
            Shape::Immediate((pixdata(*number) << IMMED1_TAG_BITS) | PORT_TAG)
        }
        Term::Nil => Shape::Immediate(NIL),
        Term::Cons(_) => Shape::List(Cells {
            first: None,
            rest: term,
        }),
        Term::Integer(value) => match term.small_integer() {
            Some(small) => {
                Shape::Immediate(((small as u64) << IMMED1_TAG_BITS) | SMALL_INTEGER_TAG)
            }
            None => Shape::Bignum(*value < 0, vec![value.unsigned_abs()]),
        },
        Term::BigInt(value) => Shape::Bignum(
            value.sign() == Sign::Minus,
            value.magnitude().to_u64_digits(),
        ),
        Term::Float(value) => Shape::Float(value.to_bits()),
        Term::Tuple(elements) => Shape::Tuple(elements),
        Term::Map(map) => Shape::Map(map),
        Term::Binary(bytes) => Shape::Binary(bytes),
        Term::Reference(reference) => Shape::Reference(*reference),
        Term::Fun(fun) => Shape::Fun(fun),
        Term::MatchContext(_) => unreachable!("{MATCH_CONTEXT_ESCAPE}"),
    }
}

/// The index bits of the ERTS process and port tables: `log2` of the
/// default `process_limit` (`ERTS_DEFAULT_MAX_PROCESSES`,
/// `erl_process.h:86`) and of the `port_limit` that `erl +Q 1048576` gives.
const PTAB_BITS: u32 = 20;

/// Spread a pid or port number over the table index bits, as ERTS
/// `erts_ptab_data2pixdata` does (`erl_ptab.h:301-317`) with 8 slots per
/// cache line (`erl_ptab.c:405-415`). The word of a local pid or port holds
/// this value (`erts_ptab_make_id`, `erl_ptab.h:320-326`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pixdata(1), 8);
/// assert_eq!(pixdata(1 << 17), 1);
/// assert_eq!(pixdata(1 << 20), 1 << 20);
/// ```
fn pixdata(number: u64) -> u64 {
    let cli_shift = PTAB_BITS - 3;
    (number & !((1 << PTAB_BITS) - 1))
        | ((number & ((1 << cli_shift) - 1)) << 3)
        | ((number >> cli_shift) & 7)
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
                // ERTS mixes the size in bits, the whole bytes, and then the
                // tail bits shifted down, so the bits after the size do not
                // change the hash (`erl_term_hashing.c:2029-2091`).
                let size = bytes.bit_size();
                let whole = size / 8;
                let tail = size % 8;
                self.mix_alpha(TYPE_BINARY);
                self.mix_beta(size as u64);
                let bits = bytes.bit_bytes();
                self.mix_bytes(&bits[..whole]);
                if tail != 0 {
                    self.mix_alpha(u64::from(bits[whole] >> (8 - tail)));
                }
            }
            Shape::Bignum(negative, digits) => {
                // ERTS mixes the sign and the number of 64-bit digits, then
                // the digits two at a time, and a last odd digit
                // (`erl_term_hashing.c:2101-2127`).
                let sign = if negative {
                    TYPE_NEG_BIGNUM
                } else {
                    TYPE_POS_BIGNUM
                };
                self.mix_alpha(pair(sign, digits.len() as u64));
                let (pairs, rest) = digits.as_chunks::<2>();
                for [low, high] in pairs {
                    self.mix_alpha(*low);
                    self.mix_beta(*high);
                }
                if let [last] = rest {
                    self.mix_beta(*last);
                }
            }
            Shape::Float(bits) => {
                // `IHASH_MIX_BETA_2F32(ff.fw[0], ff.fw[1])` joins the two
                // 32-bit words of the double, low word first on a
                // little-endian host, which gives the bits of the float
                // (`erl_term_hashing.c:2193-2202`).
                self.mix_alpha(TYPE_FLOAT);
                self.mix_beta(bits);
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

/// Return the atom table hash of `atom`, as `atom_hash` (`atom.c:83-105`)
/// gives it: the `hashpjw` hash of the UTF-8 bytes of the name, which this
/// function reads from `atoms`. A two-byte UTF-8 sequence for a Latin-1
/// character hashes as the low byte of that character. The atom table
/// stores the value as the `hvalue` of the atom, and the two phash hashes
/// and `MAKE_HASH` of an ETS key read it from there.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(atom_hash(&atoms, &a), 0x61);
/// assert_eq!(atom_hash(&atoms, &ab), 0x672);
/// ```
pub(crate) fn atom_hash(atoms: &AtomTable, atom: &Atom) -> u64 {
    let mut bytes = atoms.name_of(atom).bytes().peekable();
    let mut hash: u64 = 0;
    while let Some(mut byte) = bytes.next() {
        if byte & 0xFE == 0xC2
            && let Some(next) = bytes.next_if(|next| next & 0xC0 == 0x80)
        {
            byte = (byte << 6) | (next & 0x3F);
        }
        hash = (hash << 4) + u64::from(byte);
        let high = hash & 0xF000_0000;
        if high != 0 {
            hash ^= high >> 24;
            hash ^= high;
        }
    }
    hash
}

/// The prime numbers just above 2^28 (`FUNNY_NUMBER1` to `FUNNY_NUMBER14`,
/// `erl_term_hashing.c:100-115`). `FUNNY_NUMBER7` and `FUNNY_NUMBER15`
/// belong to the record paths, which PBeam has no term for.
const FUNNY_1: u32 = 268_440_163;
const FUNNY_2: u32 = 268_439_161;
const FUNNY_3: u32 = 268_435_459;
const FUNNY_4: u32 = 268_436_141;
const FUNNY_5: u32 = 268_438_633;
const FUNNY_6: u32 = 268_437_017;
const FUNNY_8: u32 = 268_437_511;
const FUNNY_9: u32 = 268_439_627;
const FUNNY_10: u32 = 268_440_479;
const FUNNY_11: u32 = 268_440_577;
const FUNNY_12: u32 = 268_440_581;
const FUNNY_13: u32 = 268_440_593;
const FUNNY_14: u32 = 268_440_611;

/// The golden-ratio mix value of `make_hash2` (`HCONST`,
/// `erl_term_hashing.c:437`).
const HCONST: u32 = 0x9e37_79b9;
/// `HCONST * n mod 2^32` for the mixes of `make_hash2`
/// (`erl_term_hashing.c:866-888`). The unused multiples belong to the record
/// and external-term paths.
const HCONST_2: u32 = 0x3c6e_f372;
const HCONST_3: u32 = 0xdaa6_6d2b;
const HCONST_4: u32 = 0x78dd_e6e4;
const HCONST_5: u32 = 0x1715_609d;
const HCONST_6: u32 = 0xb54c_da56;
const HCONST_7: u32 = 0x5384_540f;
const HCONST_9: u32 = 0x8ff3_4781;
const HCONST_10: u32 = 0x2e2a_c13a;
const HCONST_11: u32 = 0xcc62_3af3;
const HCONST_12: u32 = 0x6a99_b4ac;
const HCONST_13: u32 = 0x08d1_2e65;
const HCONST_14: u32 = 0xa708_a81e;
const HCONST_15: u32 = 0x4540_21d7;
const HCONST_16: u32 = 0xe377_9b90;
const HCONST_19: u32 = 0xbe1e_08bb;

/// The `NIL_DEF` tag value of `[]` in a `make_hash2` mix
/// (`erl_term.h:1442-1443`).
const NIL_DEF: u32 = 2;

/// The mask of the pid number in the ERTS pid data word, below the serial
/// bits (`_PID_NUM_SIZE`, `erl_term.h:645-656`).
const PID_NUMBER_MASK: u64 = (1 << 28) - 1;

/// Return true for a cons cell, as the ERTS C check `is_list` does
/// (`erl_term.h:1260-1261`): `[]` is not a cons. The Erlang guard
/// `is_list/1` of [`Term::is_list`] also accepts `[]`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(!is_cons(&Term::nil()));
/// assert!(is_cons(&Term::list_from([Term::Integer(1)])));
/// ```
fn is_cons(term: &Term) -> bool {
    matches!(term, Term::Cons(_))
}

/// The block size of the binary hash of `make_hash2`
/// (`BLOCK_HASH_BYTES_PER_ITER`, `erl_term_hashing.c:440`).
const BLOCK_BYTES: usize = 12;

/// The value that `make_hash2` gives `[]` when it is the whole term
/// (`erl_term_hashing.c:1546-1548`).
const NIL_HASH: u32 = 3_468_870_702;

/// The steps of the walk of [`make_hash`], as the term work stack (`WSTACK`)
/// of ERTS holds its operations (`erl_term_hashing.c:169-180`).
enum HashStep<'a> {
    /// Hash this term.
    Term(&'a Term),
    /// Hash the elements of a tuple from `index`, then mix the arity
    /// (`MAKE_HASH_TUPLE_OP`).
    Tuple {
        /// The elements of the tuple.
        elements: &'a [Term],
        /// The element to hash next.
        index: usize,
    },
    /// Hash the free variables of a local fun from `index`
    /// (`MAKE_HASH_TERM_ARRAY_OP`).
    Array {
        /// The free variables of the fun.
        elements: &'a [Term],
        /// The variable to hash next.
        index: usize,
    },
    /// Hash the cells of a list (`LIST_DEF`).
    Cells(&'a Term),
    /// Continue at the tail of a list (`MAKE_HASH_CDR_PRE_OP`).
    CdrPre(&'a Term),
    /// Mix the mark of the tail of a list (`MAKE_HASH_CDR_POST_OP`).
    CdrPost,
}

/// Return the value of the old term hash of `term`, as `make_hash`
/// (`erl_term_hashing.c:165-427`) gives it to `erlang:phash/2`. `atoms` owns
/// the names of the atoms of `term`.
///
/// A term that is not immediate goes through a work stack, so a deep term
/// does not recurse.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(make_hash(&atoms, &Term::static_atom("abc")), 26499);
/// assert_eq!(make_hash(&atoms, &Term::Integer(1)), 1_688_579_892);
/// ```
pub(crate) fn make_hash(atoms: &AtomTable, term: &Term) -> u32 {
    let mut hash: u32 = 0;
    let mut stack: Vec<HashStep> = Vec::new();
    let mut next = Some(HashStep::Term(term));
    while let Some(step) = next.take().or_else(|| stack.pop()) {
        match step {
            HashStep::Term(term) => match term {
                Term::Nil => hash = hash.wrapping_mul(FUNNY_3).wrapping_add(1),
                Term::Atom(atom) => {
                    let value = atom_hash(atoms, atom) as u32;
                    hash = hash.wrapping_mul(FUNNY_1).wrapping_add(value);
                }
                Term::Integer(value) => match term.small_integer() {
                    Some(small) => hash = small_hash(hash, small),
                    None => hash = bignum_hash(hash, *value < 0, &[value.unsigned_abs()]),
                },
                Term::BigInt(value) => {
                    let digits = value.magnitude().to_u64_digits();
                    hash = bignum_hash(hash, value.sign() == Sign::Minus, &digits);
                }
                Term::Float(value) => {
                    // `GET_DOUBLE` reads the two 32-bit words of the double,
                    // and `FLOAT_DEF` mixes their exclusive or
                    // (`erl_term_hashing.c:270-282`). ERTS first makes the
                    // negative zero positive, so both zeros mix the bits of
                    // the positive zero.
                    let value = if *value == 0.0 { 0.0 } else { *value };
                    let bits = value.to_bits();
                    hash = hash
                        .wrapping_mul(FUNNY_6)
                        .wrapping_add((bits as u32) ^ ((bits >> 32) as u32));
                }
                Term::Binary(bytes) => hash = binary_hash(hash, bytes),
                Term::Pid(pid) => {
                    // ERTS hashes the pid number only, not its serial
                    // (`internal_pid_number`, `erl_term.h:645-656`).
                    let number = pid.0 & PID_NUMBER_MASK;
                    hash = uint32_step(hash, number, FUNNY_5).wrapping_mul(FUNNY_6);
                }
                Term::Port(port) => {
                    let (low, high) = (port.0 as u32, (port.0 >> 32) as u32);
                    if high != 0 {
                        hash = uint32_step(hash, u64::from(high), FUNNY_11);
                    }
                    hash = uint32_step(hash, u64::from(low), FUNNY_9).wrapping_mul(FUNNY_10);
                }
                Term::Reference(reference) => {
                    let number = u64::from(reference.numbers()[0]);
                    hash = uint32_step(hash, number, FUNNY_9).wrapping_mul(FUNNY_10);
                }
                Term::Tuple(elements) => stack.push(HashStep::Tuple { elements, index: 0 }),
                Term::Map(_) => {
                    hash = hash
                        .wrapping_mul(FUNNY_13)
                        .wrapping_add(FUNNY_14)
                        .wrapping_add(make_hash2(atoms, term).0);
                }
                Term::Cons(_) => next = Some(HashStep::Cells(term)),
                Term::Fun(fun) => next = fun_hash(atoms, fun, &mut hash, &mut stack),
                Term::MatchContext(_) => unreachable!("{MATCH_CONTEXT_ESCAPE}"),
            },
            HashStep::Tuple { elements, index } => match elements.get(index) {
                Some(element) => {
                    next = Some(HashStep::Term(element));
                    stack.push(HashStep::Tuple {
                        elements,
                        index: index + 1,
                    });
                }
                None => {
                    hash = hash
                        .wrapping_mul(FUNNY_9)
                        .wrapping_add(elements.len() as u32);
                }
            },
            HashStep::Array { elements, index } => {
                if let Some(element) = elements.get(index) {
                    next = Some(HashStep::Term(element));
                    stack.push(HashStep::Array {
                        elements,
                        index: index + 1,
                    });
                }
            }
            HashStep::Cells(list) => next = cells_hash(&mut hash, &mut stack, list),
            HashStep::CdrPre(tail) => {
                if is_cons(tail) {
                    next = Some(HashStep::Cells(tail));
                } else {
                    stack.push(HashStep::CdrPost);
                    next = Some(HashStep::Term(tail));
                }
            }
            HashStep::CdrPost => hash = hash.wrapping_mul(FUNNY_8),
        }
    }
    hash
}

/// Mix a small integer as `SMALL_DEF` (`erl_term_hashing.c:227-238`) does:
/// the four bytes of the value and, for a value above 2^32, the four bytes
/// above them, then the sign mark.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(small_hash(0, -1), 1_680_185_270);
/// ```
fn small_hash(hash: u32, small: i64) -> u32 {
    let magnitude = small.unsigned_abs();
    let mut value = uint32_step(hash, magnitude, FUNNY_2);
    if magnitude >> 32 != 0 {
        value = uint32_step(value, magnitude >> 32, FUNNY_2);
    }
    value.wrapping_mul(if small < 0 { FUNNY_4 } else { FUNNY_3 })
}

/// Mix the digits of a bignum as `BIG_DEF` (`erl_term_hashing.c:340-380`)
/// does: every byte of every digit below the top digit, then the bytes of
/// the top digit, of which ERTS skips the four high zero bytes, then the
/// sign mark. The digits are 64-bit words, least significant first.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(bignum_hash(0, false, &[1]), 1_688_579_892 >> 0);
/// ```
fn bignum_hash(hash: u32, negative: bool, digits: &[u64]) -> u32 {
    let (top, below) = digits.split_last().expect("a bignum has a digit");
    let mut hash = hash;
    for digit in below {
        for byte in digit.to_le_bytes() {
            hash = hash.wrapping_mul(FUNNY_2).wrapping_add(u32::from(byte));
        }
    }
    let bytes = if top >> 32 == 0 { 4 } else { 8 };
    let mut digit = *top;
    for _ in 0..bytes {
        hash = hash
            .wrapping_mul(FUNNY_2)
            .wrapping_add((digit & 0xFF) as u32);
        digit >>= 8;
    }
    hash.wrapping_mul(if negative { FUNNY_4 } else { FUNNY_3 })
}

/// Mix the bytes of a binary as the list path of `BITSTRING_DEF`
/// (`hash_binary_bytes`, `erl_term_hashing.c:117-160`) does: each byte below
/// the tail bits, then the byte with the tail bits and their count, then the
/// number of whole bytes.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(binary_hash(0, &Binary::from(b"abc".to_vec())), 0x28fd5cae);
/// ```
fn binary_hash(hash: u32, bytes: &Binary) -> u32 {
    let bit_size = bytes.bit_size();
    let whole = bit_size / 8;
    let tail = bit_size % 8;
    let mut hash = hash;
    let bits = bytes.bit_bytes();
    for byte in bits[..whole].iter() {
        hash = hash.wrapping_mul(FUNNY_1).wrapping_add(u32::from(*byte));
    }
    if tail > 0 {
        // The bits of the tail are the high bits of the last byte of the
        // packed form of the binary (`Binary::bit_bytes`).
        let last = bits[whole] >> (8 - tail);
        hash = hash
            .wrapping_mul(FUNNY_1)
            .wrapping_add(u32::from(last))
            .wrapping_mul(FUNNY_12)
            .wrapping_add(tail as u32);
    }
    hash.wrapping_mul(FUNNY_4).wrapping_add(whole as u32)
}

/// Mix the module, the index, the old unique value, and the free variables
/// of a fun as `FUN_DEF` (`erl_term_hashing.c:240-268`) does, or the arity
/// and the name of an external fun. Return the step that continues the walk
/// for the first free variable, if the fun has one.
///
/// # Examples
///
/// ```rust,ignore
/// let mut stack = Vec::new();
/// let fun = Term::Fun(Fun::external(module, function, 1));
/// assert!(fun_hash(&atoms, &fun, 0, &mut stack).is_none());
/// ```
fn fun_hash<'a>(
    atoms: &AtomTable,
    fun: &'a Fun,
    hash: &mut u32,
    stack: &mut Vec<HashStep<'a>>,
) -> Option<HashStep<'a>> {
    match fun.target() {
        Target::Local(lambda) => {
            let free = fun.free();
            let module = atom_hash(atoms, lambda.module()) as u32;
            *hash = hash
                .wrapping_mul(FUNNY_10)
                .wrapping_add(free.len() as u32)
                .wrapping_mul(FUNNY_1)
                .wrapping_add(module)
                .wrapping_mul(FUNNY_2)
                .wrapping_add(lambda.index as u32)
                .wrapping_mul(FUNNY_2)
                .wrapping_add(lambda.old_unique as u32);
            let (first, rest) = free.split_first()?;
            if !rest.is_empty() {
                stack.push(HashStep::Array {
                    elements: rest,
                    index: 0,
                });
            }
            Some(HashStep::Term(first))
        }
        Target::External {
            module,
            function,
            arity,
        } => {
            let module = atom_hash(atoms, module) as u32;
            let function = atom_hash(atoms, function) as u32;
            *hash = hash
                .wrapping_mul(FUNNY_11)
                .wrapping_add(*arity)
                .wrapping_mul(FUNNY_1)
                .wrapping_add(module)
                .wrapping_mul(FUNNY_1)
                .wrapping_add(function);
            None
        }
    }
}

/// Hash the cells of a list from `list` as `LIST_DEF`
/// (`erl_term_hashing.c:289-337`) does: a byte element mixes
/// `FUNNY_NUMBER2` and continues, and the first element that is not a byte
/// continues as a term while the tail waits on the work stack. Return the
/// step that continues the walk.
///
/// # Examples
///
/// ```rust,ignore
/// let list = Term::list_from([Term::Integer(1), Term::Integer(2)]);
/// let mut stack = Vec::new();
/// assert!(cells_hash(&mut 0, &mut stack, &list).is_none());
/// ```
fn cells_hash<'a>(
    hash: &mut u32,
    stack: &mut Vec<HashStep<'a>>,
    list: &'a Term,
) -> Option<HashStep<'a>> {
    let mut current = list;
    loop {
        let (head, rest) = current.cons_cell()?;
        if let Some(byte) = byte(head) {
            *hash = hash.wrapping_mul(FUNNY_2).wrapping_add(byte as u32);
            if !is_cons(rest) {
                stack.push(HashStep::CdrPost);
                return Some(HashStep::Term(rest));
            }
            current = rest;
            continue;
        }
        stack.push(HashStep::CdrPre(rest));
        return Some(HashStep::Term(head));
    }
}

/// Mix a 32-bit value into a hash in four steps, as `UINT32_HASH_STEP`
/// (`erl_term_hashing.c:183-193`) does.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(uint32_step(0, 0x01020304, FUNNY_2), 0x04030201 * 0);
/// ```
fn uint32_step(hash: u32, value: u64, prime: u32) -> u32 {
    let mut hash = hash;
    let mut value = value;
    for _ in 0..4 {
        hash = hash.wrapping_mul(prime).wrapping_add((value & 0xFF) as u32);
        value >>= 8;
    }
    hash
}

/// One item of the work stack of [`make_hash2`], as the `ESTACK` of ERTS
/// holds it (`erl_term_hashing.c:908-910`).
enum Hash2Item<'a> {
    /// Hash this term.
    Term(&'a Term),
    /// Hash the children of this HAMT node.
    Node(&'a Node),
    /// The end of the key and the value of one pair of a map
    /// (`HASH_MAP_PAIR`).
    Pair,
    /// The end of the pairs of a map (`HASH_MAP_TAIL`), with the hash and
    /// the exclusive or of the pairs around the pairs of the map.
    Tail {
        /// The hash before the pairs.
        hash: u32,
        /// The exclusive or before the pairs.
        xor: u32,
    },
}

/// The three 32-bit words of one `MIX` state (`ErtsBlockHashHelperCtx`,
/// `erl_term_hashing.c:418-424`).
struct Mix {
    /// The first word (`a`).
    a: u32,
    /// The second word (`b`).
    b: u32,
    /// The third word (`c`).
    c: u32,
}

impl Mix {
    /// Mix the three words, as the `MIX` macro
    /// (`erl_term_hashing.c:427-439`) does. Every difference wraps and every
    /// shift is logical, as the `Uint32` arithmetic of ERTS does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut mix = Mix { a: 1, b: 2, c: 3 };
    /// mix.mix();
    /// ```
    fn mix(&mut self) {
        let (mut a, mut b, mut c) = (self.a, self.b, self.c);
        a = a.wrapping_sub(b);
        a = a.wrapping_sub(c);
        a ^= c >> 13;
        b = b.wrapping_sub(c);
        b = b.wrapping_sub(a);
        b ^= a << 8;
        c = c.wrapping_sub(a);
        c = c.wrapping_sub(b);
        c ^= b >> 13;
        a = a.wrapping_sub(b);
        a = a.wrapping_sub(c);
        a ^= c >> 12;
        b = b.wrapping_sub(c);
        b = b.wrapping_sub(a);
        b ^= a << 16;
        c = c.wrapping_sub(a);
        c = c.wrapping_sub(b);
        c ^= b >> 5;
        a = a.wrapping_sub(b);
        a = a.wrapping_sub(c);
        a ^= c >> 3;
        b = b.wrapping_sub(c);
        b = b.wrapping_sub(a);
        b ^= a << 10;
        c = c.wrapping_sub(a);
        c = c.wrapping_sub(b);
        c ^= b >> 15;
        self.a = a;
        self.b = b;
        self.c = c;
    }
}

/// Mix two values into a `make_hash2` value as `UINT32_HASH_2` does
/// (`erl_term_hashing.c:911-917`): the constant plus the first value is `a`,
/// the constant plus the second is `b`, and `MIX` mixes them into the value,
/// which becomes `c`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(hash2_with(0, 1, 0, HCONST), hash_with(0, 1, HCONST));
/// ```
fn hash2_with(hash: u32, first: u32, second: u32, constant: u32) -> u32 {
    let mut mix = Mix {
        a: constant.wrapping_add(first),
        b: constant.wrapping_add(second),
        c: hash,
    };
    mix.mix();
    mix.c
}

/// Mix one value into a `make_hash2` value as `UINT32_HASH` does: the second
/// value is 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(hash_with(0, 1, HCONST), hash2_with(0, 1, 0, HCONST));
/// ```
fn hash_with(hash: u32, value: u32, constant: u32) -> u32 {
    hash2_with(hash, value, 0, constant)
}

/// Return the block hash of `bytes` with the initial value `initval`, as
/// `block_hash` (`erl_term_hashing.c:500-521`) does: the whole 12-byte blocks
/// (`block_hash_buffer`, `457-477`), then the last bytes with the whole
/// length added to the third word (`block_hash_final_bytes`, `479-498`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(block_hash(b"", HCONST_13), 0x3f1a8f42);
/// ```
fn block_hash(bytes: &[u8], initval: u32) -> u32 {
    let (blocks, last) = bytes.as_chunks::<BLOCK_BYTES>();
    let mut mix = Mix {
        a: HCONST,
        b: HCONST,
        c: initval,
    };
    for block in blocks {
        // Each block is three 32-bit words, least significant byte first.
        for (word, bytes) in block.as_chunks::<4>().0.iter().enumerate() {
            let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            match word {
                0 => mix.a = mix.a.wrapping_add(value),
                1 => mix.b = mix.b.wrapping_add(value),
                _ => mix.c = mix.c.wrapping_add(value),
            }
        }
        mix.mix();
    }
    let (mut a, mut b) = (mix.a, mix.b);
    let mut c = mix.c.wrapping_add(bytes.len() as u32);
    for (index, byte) in last.iter().enumerate() {
        let value = u32::from(*byte);
        // The low byte of the third word holds the length
        // (`block_hash_final_bytes`, `erl_term_hashing.c:486-498`).
        match index {
            0..=3 => a = a.wrapping_add(value << (8 * index)),
            4..=7 => b = b.wrapping_add(value << (8 * (index - 4))),
            _ => c = c.wrapping_add(value << (8 * (index - 8) + 8)),
        }
    }
    let mut mix = Mix { a, b, c };
    mix.mix();
    mix.c
}

/// Mix a small integer into a `make_hash2` value as the `SMALL` case does
/// (`erl_term_hashing.c:1553-1563`): a value of the s28 range mixes as the
/// two's complement of the value, which the negative range mixes twice, and
/// a larger value mixes its magnitude as two words (`NOT_SSMALL28_HASH`,
/// `952-968`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(small_hash2(0, 0), 3_175_731_469);
/// assert_eq!(small_hash2(0, -1), 1_117_813_597);
/// ```
fn small_hash2(hash: u32, small: i64) -> u32 {
    if !(-(1 << 27)..(1 << 27)).contains(&small) {
        let (constant, magnitude) = if small < 0 {
            (HCONST_10, small.unsigned_abs())
        } else {
            (HCONST_11, small as u64)
        };
        return hash2_with(hash, magnitude as u32, (magnitude >> 32) as u32, constant);
    }
    let hash = match small < 0 {
        true => hash_with(hash, small.unsigned_abs() as u32, HCONST),
        false => hash,
    };
    hash_with(hash, small as i32 as u32, HCONST)
}

/// Mix the digits of a bignum into a `make_hash2` value as the
/// `POS_BIG_SUBTAG` and `NEG_BIG_SUBTAG` cases do
/// (`erl_term_hashing.c:1400-1432`): one mix for each 64-bit digit, least
/// significant first, with the low word first. Return the number of digits,
/// which is the number of iterations of the ERTS walk.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(bignum_hash2(0, false, &[1]), (hash2_with(0, 1, 0, HCONST_11), 1));
/// ```
fn bignum_hash2(hash: u32, negative: bool, digits: &[u64]) -> (u32, u64) {
    let constant = if negative { HCONST_10 } else { HCONST_11 };
    let mut hash = hash;
    for digit in digits {
        hash = hash2_with(hash, *digit as u32, (*digit >> 32) as u32, constant);
    }
    (hash, digits.len() as u64)
}

/// Push the pairs of a map for [`make_hash2`], with one iteration for each
/// pair, and return the number of children. The value comes first and then
/// the key, as the pushed `[Key | Value]` cells of ERTS do
/// (`erl_term_hashing.c:1194-1206`, `1224-1250`).
///
/// # Examples
///
/// ```rust,ignore
/// let mut stack = Vec::new();
/// assert_eq!(push_children(&[], &mut stack, &[]), 0);
/// ```
fn push_children<'a>(
    stack: &mut Vec<Hash2Item<'a>>,
    iterations: &mut u64,
    children: &'a [Child],
) -> usize {
    let push_pair = |stack: &mut Vec<Hash2Item<'a>>, key: &'a Term, value: &'a Term| {
        stack.push(Hash2Item::Pair);
        stack.push(Hash2Item::Term(value));
        stack.push(Hash2Item::Term(key));
    };
    for child in children {
        match child {
            Child::Leaf(key, value) => push_pair(stack, key, value),
            Child::Node(node) => stack.push(Hash2Item::Node(node)),
            Child::Collision(pairs) => {
                for (key, value) in pairs {
                    push_pair(stack, key, value);
                }
            }
        }
    }
    *iterations += children.len() as u64;
    children.len()
}

/// Return the value of the term hash of `term` and the number of its
/// iterations, as `make_hash2_helper` (`erl_term_hashing.c:858-1596`) gives
/// them to `erlang:phash2/1,2`. `atoms` owns the names of the atoms of
/// `term`. An atom and a small integer return from the fast path with no
/// iteration, and the walk of a binary counts one iteration for each 12
/// bytes.
///
/// ERTS charges one reduction for each 64 iterations, so a caller charges
/// `iterations / 64`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(make_hash2(&atoms, &Term::static_atom("abc")), (26499, 0));
/// assert_eq!(make_hash2(&atoms, &Term::Integer(1)).0, 539_485_162);
/// ```
pub(crate) fn make_hash2(atoms: &AtomTable, term: &Term) -> (u32, u64) {
    // The fast paths before the walk (`erl_term_hashing.c:1005-1030`).
    if let Term::Atom(atom) = term {
        return (atom_hash(atoms, atom) as u32, 0);
    }
    if let Term::Integer(_) = term
        && let Some(small) = term.small_integer()
    {
        return (small_hash2(0, small), 0);
    }
    let mut hash: u32 = 0;
    let mut xor: u32 = 0;
    let mut iterations: u64 = 0;
    let mut stack: Vec<Hash2Item> = Vec::new();
    let mut next: Option<&Term> = Some(term);
    loop {
        let term = match next.take() {
            Some(term) => term,
            None => match stack.pop() {
                None => return (hash, iterations),
                Some(Hash2Item::Term(term)) => term,
                Some(Hash2Item::Pair) => {
                    xor ^= hash;
                    hash = 0;
                    iterations += 1;
                    continue;
                }
                Some(Hash2Item::Tail {
                    hash: saved,
                    xor: saved_xor,
                }) => {
                    hash = hash_with(saved, xor, HCONST_19);
                    xor = saved_xor;
                    // The tail of a map ends the walk here, and the walk of
                    // ERTS returns from `hash2_common` before its trailing
                    // charge point (`erl_term_hashing.c:1565-1570`).
                    if !stack.is_empty() {
                        iterations += 1;
                    }
                    continue;
                }
                Some(Hash2Item::Node(node)) => {
                    // An inner node has no size word and no mix of its own:
                    // it reads its children into the pair context of the map
                    // around it (`erl_map.h:203`, `erl_term_hashing.c:1192-1210`).
                    push_children(&mut stack, &mut iterations, node.children());
                    // The walk of ERTS charges the iteration of the
                    // `hash2_common_3` that follows the node
                    // (`erl_term_hashing.c:1560-1572`).
                    iterations += 1;
                    continue;
                }
            },
        };
        iterations += 1;
        match term {
            Term::Nil => {
                hash = if hash == 0 {
                    NIL_HASH
                } else {
                    hash_with(hash, NIL_DEF, HCONST_2)
                };
            }
            Term::Atom(atom) => {
                let value = atom_hash(atoms, atom) as u32;
                hash = if hash == 0 {
                    value
                } else {
                    hash_with(hash, value, HCONST_3)
                };
            }
            Term::Integer(value) => match term.small_integer() {
                Some(small) => hash = small_hash2(hash, small),
                None => {
                    let (value, count) = bignum_hash2(hash, *value < 0, &[value.unsigned_abs()]);
                    hash = value;
                    iterations += count;
                }
            },
            Term::BigInt(value) => {
                let digits = value.magnitude().to_u64_digits();
                let (value, count) = bignum_hash2(hash, value.sign() == Sign::Minus, &digits);
                hash = value;
                iterations += count;
            }
            Term::Float(value) => {
                // The two 32-bit words of the double, the high word first
                // (`erl_term_hashing.c:1592-1601`).
                let value = if *value == 0.0 { 0.0 } else { *value };
                let bits = value.to_bits();
                hash = hash2_with(hash, (bits >> 32) as u32, bits as u32, HCONST_12);
            }
            Term::Binary(bytes) => {
                let bit_size = bytes.bit_size();
                let whole = bit_size / 8;
                // ERTS truncates the byte size to 32 bits, so the bytes
                // above 2^32 do not change the value (`erl_term_hashing.c:1274-1278`).
                let size = (whole & 0xFFFF_FFFF) as u32;
                let tail = bit_size % 8;
                let start = HCONST_13.wrapping_add(hash);
                if size == 0 && tail == 0 {
                    hash = start;
                } else {
                    iterations += u64::from((size / BLOCK_BYTES as u32).max(1));
                    let bits = bytes.bit_bytes();
                    hash = block_hash(&bits[..size as usize], start);
                    if tail > 0 {
                        let last = bits[size as usize] >> (8 - tail);
                        hash = hash2_with(hash, tail as u32, u32::from(last), HCONST_15);
                    }
                }
            }
            // ERTS hashes the pid number only, not its serial
            // (`internal_pid_number`, `erl_term.h:645-656`).
            Term::Pid(pid) => hash = hash_with(hash, (pid.0 & PID_NUMBER_MASK) as u32, HCONST_5),
            Term::Port(port) => {
                hash = hash2_with(hash, port.0 as u32, (port.0 >> 32) as u32, HCONST_6);
            }
            Term::Reference(reference) => {
                hash = hash_with(hash, reference.numbers()[0], HCONST_7);
            }
            Term::Tuple(elements) => {
                let arity = elements.len() as u32;
                hash = hash_with(hash, arity, HCONST_9);
                if let Some((first, rest)) = elements.split_first() {
                    for element in rest.iter().rev() {
                        stack.push(Hash2Item::Term(element));
                        iterations += 1;
                    }
                    next = Some(first);
                }
            }
            Term::Map(map) => match map.form() {
                Form::Flat(pairs) => {
                    hash = hash_with(hash, pairs.len() as u32, HCONST_16);
                    if !pairs.is_empty() {
                        stack.push(Hash2Item::Tail { hash, xor });
                        hash = 0;
                        xor = 0;
                        for (key, value) in pairs.iter().rev() {
                            stack.push(Hash2Item::Pair);
                            stack.push(Hash2Item::Term(value));
                            stack.push(Hash2Item::Term(key));
                            iterations += 1;
                        }
                    }
                }
                Form::Hash { size, root } => {
                    hash = hash_with(hash, size as u32, HCONST_16);
                    if size != 0 {
                        stack.push(Hash2Item::Tail { hash, xor });
                        hash = 0;
                        xor = 0;
                        push_children(&mut stack, &mut iterations, root.children());
                    }
                }
            },
            Term::Cons(_) => {
                let mut chunk: u32 = 0;
                let mut count: u32 = 0;
                let mut tail = term;
                while let Some((head, rest)) = tail.cons_cell()
                    && let Some(byte) = byte(head)
                {
                    chunk = (chunk << 8) | byte as u32;
                    if count == 3 {
                        hash = hash_with(hash, chunk, HCONST_4);
                        count = 0;
                        chunk = 0;
                    } else {
                        count += 1;
                    }
                    tail = rest;
                    if !is_cons(tail) {
                        break;
                    }
                    iterations += 1;
                }
                if count > 0 {
                    hash = hash_with(hash, chunk, HCONST_4);
                }
                match tail.cons_cell() {
                    Some((head, rest)) => {
                        stack.push(Hash2Item::Term(rest));
                        next = Some(head);
                    }
                    // The walk of ERTS processes the tail as a term, so an
                    // improper list hashes its tail (`erl_term_hashing.c:1046-1068`).
                    None => next = Some(tail),
                }
            }
            Term::Fun(fun) => match fun.target() {
                Target::Local(lambda) => {
                    let free = fun.free();
                    hash = hash2_with(
                        hash,
                        free.len() as u32,
                        atom_hash(atoms, lambda.module()) as u32,
                        HCONST,
                    );
                    hash = hash2_with(hash, lambda.index as u32, lambda.old_unique as u32, HCONST);
                    if let Some((first, rest)) = free.split_first() {
                        for element in rest.iter().rev() {
                            stack.push(Hash2Item::Term(element));
                            iterations += 1;
                        }
                        next = Some(first);
                    }
                }
                Target::External {
                    module,
                    function,
                    arity,
                } => {
                    hash = hash2_with(hash, *arity, atom_hash(atoms, module) as u32, HCONST);
                    hash = hash_with(hash, atom_hash(atoms, function) as u32, HCONST_14);
                }
            },
            Term::MatchContext(_) => unreachable!("{MATCH_CONTEXT_ESCAPE}"),
        }
        // `hash2_common` of ERTS returns before its trailing charge point
        // when no item waits, so the last item of a walk costs no iteration
        // (`erl_term_hashing.c:1565-1570`).
        if next.is_none() && stack.is_empty() {
            iterations = iterations.saturating_sub(1);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/term/hash.rs"]
mod tests;
