mod atom;
mod binary;
pub(crate) mod external;
pub(crate) mod float_text;
mod fun;
mod hash;
mod integer;
mod list;
mod live_bytes;
mod map;
mod match_context;
mod order;
mod print;
mod tuple;

pub use atom::Atom;
pub use atom::AtomTable;
pub use atom::HostTerm;
pub(crate) use atom::{MAX_ATOM_CHARACTERS, MAX_ATOM_SZ_LIMIT};
#[cfg(test)]
pub(crate) use atom::{test_atom, test_atom_table};
pub use binary::Binary;
pub use fun::Fun;
pub(crate) use fun::{Lambda, Target, debug_target};
pub(crate) use hash::{atom_hash, internal_hash, make_hash, make_hash2};
pub(crate) use integer::{MAX_BIG_DIGITS, fits_big_arity, low_64_bits};
use list::{Cons, ConsRef};
pub(crate) use live_bytes::LiveBytes;
pub use map::Map;
pub(crate) use map::{Direction, Drain as MapDrain, Insert, flatmap_order, from_list_units};
pub use match_context::MatchState;
pub(crate) use match_context::{segment_equals, segment_integer};
pub(crate) use print::print_term;
use tuple::Tuple;

use num_bigint::BigInt;
use std::{collections::HashSet, fmt, sync::Arc};

/// The smallest ERTS small integer on a 64-bit system: -(2^59).
pub(crate) const SMALL_INTEGER_MIN: i64 = -(1 << 59);
/// The largest ERTS small integer on a 64-bit system: 2^59 - 1.
pub(crate) const SMALL_INTEGER_MAX: i64 = (1 << 59) - 1;
/// The largest number of elements in a tuple: 2^24 - 1
/// (`ERTS_MAX_TUPLE_SIZE` and `MAX_ARITYVAL` in `erl_term.h:322-323`).
pub(crate) const MAX_TUPLE_SIZE: usize = (1 << 24) - 1;

/// A BEAM term. Lists, tuples, maps, funs, and binaries can nest to any
/// depth, as in ERTS. A clone of a compound term shares its storage and
/// takes O(1) time. `Drop`, `PartialEq`, and `Debug` use explicit work
/// stacks and do not recurse on the Rust stack.
pub enum Term {
    /// An atom: its index in the atom table of the program. Read the name
    /// through the table that owns the index.
    Atom(Atom),
    /// A binary: its bytes in a shared array, and the offset and the length
    /// of its view. A copy of a binary increments the reference count of the
    /// array, and a sub-binary keeps the array and the offset of its bytes
    /// (`docs/portable-beam-spec.md:97`).
    Binary(Binary),
    /// An integer in the signed 64-bit range. PBeam keeps each integer in
    /// this range here, also the ERTS bignums from 2^59 to 2^63 - 1 and from
    /// -(2^63) to -(2^59) - 1; [`Term::small_integer`] tells the ERTS small
    /// integers apart.
    Integer(i64),
    /// An integer outside the signed 64-bit range, which ERTS keeps as a
    /// bignum (`POS_BIG_SUBTAG`, `NEG_BIG_SUBTAG`, `erl_term.h`). Copies
    /// share the value. The form is canonical: each runtime operation makes
    /// an integer in the 64-bit range as [`Term::Integer`], so `=:=` and the
    /// hash compare the value. The canonical form is a runtime-produced form,
    /// not a type invariant: the loader and each runtime operation make only
    /// values outside the 64-bit range. A caller that builds a term, such as
    /// an argument of `BeamProgram::execute`, must keep the form: it passes
    /// a value outside the signed 64-bit range here and each other integer
    /// as [`Term::Integer`]. A `Term::BigInt` of `5` is not `=:=` to `5`.
    BigInt(Arc<BigInt>),
    /// A float: an IEEE 754 binary64 value that is finite, as an ERTS float
    /// is (`FLOAT_DEF`, `erl_term.h`). ERTS never makes an infinity or a NaN
    /// term: each operation that would give one raises `badarith` or
    /// `badarg`. The finite value is a runtime-produced form, not a type
    /// invariant: the loader and each runtime operation make only finite
    /// floats. A caller that builds a term, such as an argument of
    /// `BeamProgram::host_term`, must pass a finite value; the term order
    /// and the float text panic on a NaN. `0.0` and `-0.0` are two terms: they are equal in
    /// arithmetic comparison and different in exact comparison.
    Float(f64),
    /// The empty list `[]`.
    Nil,
    /// A cons cell of a list: a head and a tail, for example `[1 | T]`. A
    /// proper list ends in `[]`, and an improper list ends in another term,
    /// for example `[1, 2 | x]`. Copies of a list share its cells. Code
    /// outside the term module uses the list API of `src/term/list.rs`.
    Cons(ConsRef),
    /// A BEAM tuple: one array of elements. Copies of the tuple share the
    /// array, as an ERTS copy of a tuple term is the same boxed pointer. A
    /// change copies a shared array first (`Tuple::arc_mut`). Code outside
    /// the term module builds a tuple with [`Term::tuple`].
    Tuple(Tuple),
    /// A BEAM map: a flatmap up to 32 keys and a hashmap above 32 keys, as
    /// in ERTS. `Map` keeps each map in its canonical form, so two maps with
    /// the same pairs are equal. A key is a binary, an atom, a float, a small integer,
    /// `[]`, a list or a tuple whose elements are atoms, floats, small integers,
    /// `[]`, or such lists or tuples, or an improper list of such elements
    /// whose tail is an atom, a small integer, or such a tuple; a flatmap
    /// also accepts any integer. Top-level atom keys are ordered by their
    /// atom index, and all atoms are hashed by their atom index, as in ERTS.
    Map(Map),
    /// A process identifier owned by this runtime.
    Pid(ProcessId),
    /// A port identifier owned by this runtime.
    Port(PortId),
    /// A reference owned by this runtime.
    Reference(Reference),
    /// A fun: the fun entry of a lambda and its free variables for a local
    /// fun, or the module, function, and arity for an external fun.
    Fun(Fun),
    /// A binary match context that `bs_start_match3` makes in a register.
    /// The term is a reference to the match context, as an ERTS register
    /// holds a boxed pointer to the `ErlSubBits` on the process heap. A
    /// copy of the term, from `move`, `bs_start_match3`, or
    /// `bs_start_match4`, shares the state and its position. The registers
    /// and stack slots of one process own the references, and the last
    /// reference that drops releases the state. Only the binary-match
    /// instructions read it, `move` copies it, and a local call keeps it in
    /// its argument register. Each other instruction that gets a match
    /// context gives a runtime invariant error.
    MatchContext(Arc<MatchState>),
}

/// Check at compile time that a term is `Send` and `Sync`, which the
/// multi-scheduler work of [#10](https://github.com/fahchen/pbeam/issues/10)
/// needs (owner decision R3 of
/// [#450](https://github.com/fahchen/pbeam/issues/450),
/// `docs/copy-on-write-terms.md`). The bound of the trait is the check: a
/// `Term` that is not `Send` or not `Sync` does not satisfy it, so the
/// crate does not compile. The trait has no function, so it adds no
/// runtime code and no coverage line.
#[allow(dead_code)]
trait TermIsSendAndSync: Send + Sync {}
impl TermIsSendAndSync for Term {}

/// Check at compile time that a term stays 16 bytes: a bignum is one
/// pointer to its shared value, as each other boxed term is.
const _: () = assert!(std::mem::size_of::<Term>() == 16);

/// The number of pairs that the work stack of `Term::eq` holds in the frame
/// of the comparison. The stack allocates only when a comparison holds more
/// pending pairs than this count.
const EQ_INLINE_PAIRS: usize = 8;

/// The room that one comparison reserves when its work stack overflows the
/// inline array. The reserve gives the heap vector of a large comparison its
/// first capacity in one allocation, so the vector does not grow one
/// doubling at a time until it holds a whole tuple or map.
const EQ_SPILL_RESERVE: usize = 1024;

/// The term that marks an empty slot of the inline array of [`EqStack`]. The
/// comparison drops a pair of two `Nil` terms, so it never pushes this term
/// as an operand of another pair: `(Nil, Nil)` is equal and gives no nested
/// pair.
static EQ_EMPTY_SLOT: Term = Term::Nil;

/// Two terms whose equality `Term::eq` still has to check.
struct EqPair<'a>(&'a Term, &'a Term);

/// The work stack of `Term::eq`: the pairs whose equality the comparison
/// still has to check, most recent first.
///
/// The stack holds the first [`EQ_INLINE_PAIRS`] pairs in an array at the
/// bottom of its frame and every later pair in a heap vector above the
/// array. A comparison that holds at most that count allocates nothing. The
/// comparison pays one vector allocation when it holds more, and the vector
/// then grows as the old code grew it.
///
/// The comparison calls `push`, `pop`, and `extend` once for each pair, so
/// those three stay in line. The spill stays out of line, because the inline
/// array covers a shallow or a shared comparison and the spill runs once for
/// a large one.
struct EqStack<'a> {
    /// The oldest pairs, in a slot each. A slot holds [`EQ_EMPTY_SLOT`] for
    /// both terms of a pair before the first push writes it, and only the
    /// slots below `len` hold a pending pair. The pair type has no `Drop`, so
    /// the stack copies a pair out of a slot without a move.
    inline: [EqPair<'a>; EQ_INLINE_PAIRS],
    /// The number of pairs in the inline array. Only a pop of an empty
    /// vector lowers it, so it is [`EQ_INLINE_PAIRS`] while the vector holds
    /// a pair.
    len: usize,
    /// The pairs above the inline array, in the order that they arrive. Only
    /// the comparison reads and writes the tail of this vector.
    spilled: Vec<EqPair<'a>>,
}

impl<'a> EqStack<'a> {
    /// Return an empty work stack. The inline array holds no pending pair,
    /// so no slot needs a pair of its own.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stack = EqStack::new();
    /// stack.push(&Term::nil(), &Term::nil());
    /// assert!(stack.pop().is_some());
    /// ```
    fn new() -> Self {
        EqStack {
            inline: [(); EQ_INLINE_PAIRS].map(|()| EqPair(&EQ_EMPTY_SLOT, &EQ_EMPTY_SLOT)),
            len: 0,
            spilled: Vec::new(),
        }
    }

    /// Push the pair `(left, right)`.
    ///
    /// The pair goes to the inline array while the array has room. A full
    /// array stays full until the vector above it is empty, so `len` is the
    /// whole test of the common path. The reserve runs once for a
    /// comparison, before the first pair of the vector.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stack = EqStack::new();
    /// stack.push(&Term::nil(), &Term::nil());
    /// assert!(stack.pop().is_some());
    /// ```
    #[inline]
    fn push(&mut self, left: &'a Term, right: &'a Term) {
        if self.len < EQ_INLINE_PAIRS {
            self.inline[self.len] = EqPair(left, right);
            self.len += 1;
        } else {
            if self.spilled.is_empty() {
                self.reserve_spill(1);
            }
            self.spilled.push(EqPair(left, right));
        }
    }

    /// Reserve room in the heap vector for `rest` further pairs. The
    /// function runs once for a comparison, before the first pair of the
    /// vector, and it stays out of line so the reserve of the common path
    /// costs no code.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stack = EqStack::new();
    /// stack.reserve_spill(1);
    /// assert!(stack.spilled.capacity() >= EQ_SPILL_RESERVE);
    /// ```
    #[cold]
    #[inline(never)]
    fn reserve_spill(&mut self, rest: usize) {
        self.spilled.reserve(EQ_SPILL_RESERVE.max(rest));
    }

    /// Push each pair of `pairs`, in order. A group that overflows the
    /// inline array goes to the heap vector in one batch, so the comparison
    /// of a large tuple or of the free variables of a fun pays one reserve
    /// and one tight write loop for the group.
    ///
    /// A group that fills the array exactly ends with no pair, and it
    /// allocates nothing. The function reads one further pair before it
    /// reserves the batch, because the reserve of an empty group would
    /// allocate the heap vector for a comparison that the array covers.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stack = EqStack::new();
    /// stack.extend([(&Term::nil(), &Term::nil())]);
    /// assert!(stack.pop().is_some());
    /// ```
    #[inline]
    fn extend(&mut self, pairs: impl IntoIterator<Item = (&'a Term, &'a Term)>) {
        let mut pairs = pairs.into_iter();
        while self.len < EQ_INLINE_PAIRS {
            match pairs.next() {
                Some((left, right)) => {
                    self.inline[self.len] = EqPair(left, right);
                    self.len += 1;
                }
                None => return,
            }
        }
        if let Some((left, right)) = pairs.next() {
            self.spill_batch(std::iter::once((left, right)).chain(pairs));
        }
    }

    /// Push each pair of the rest of a large group to the heap vector. The
    /// function keeps the write loop of a whole tuple or fun out of line, so
    /// the code of the common path stays small.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stack = EqStack::new();
    /// stack.spill_batch([(&Term::nil(), &Term::nil())]);
    /// assert!(stack.pop().is_some());
    /// ```
    #[inline(never)]
    fn spill_batch(&mut self, rest: impl Iterator<Item = (&'a Term, &'a Term)>) {
        if self.spilled.is_empty() {
            self.reserve_spill(rest.size_hint().0);
        }
        self.spilled
            .extend(rest.map(|(left, right)| EqPair(left, right)));
    }

    /// Remove the most recent pair and return it, or return `None` when the
    /// stack is empty.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut stack = EqStack::new();
    /// stack.push(&Term::nil(), &Term::nil());
    /// assert!(stack.pop().is_some());
    /// assert!(stack.pop().is_none());
    /// ```
    #[inline]
    fn pop(&mut self) -> Option<EqPair<'a>> {
        if let Some(pair) = self.spilled.pop() {
            return Some(pair);
        }
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        let EqPair(left, right) = self.inline[self.len];
        Some(EqPair(left, right))
    }
}

/// The direct children of one node of a term, as owned work for a release
/// that runs in bounded batches. The element array of a tuple and the free
/// variables of a fun are one group that a caller moves in batches; the pairs
/// of a map are a walk that a caller drives one step at a time.
pub(crate) enum Group {
    /// The element array of a tuple or the free variables of a fun.
    Terms(Vec<Term>),
    /// The pairs of a map.
    Map(MapDrain),
}

impl Term {
    /// Return the term of a static atom, such as `true` or `normal`. A
    /// static atom has the same index in every atom table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::static_atom("ok"), Term::Atom(Atom::from_static("ok").unwrap()));
    /// ```
    ///
    /// # Panics
    ///
    /// Panic when the name is not a static atom. Callers pass a fixed name.
    pub(crate) fn static_atom(name: &str) -> Term {
        Term::Atom(Atom::from_static(name).expect("the name is a static atom"))
    }

    /// Build a tuple of `elements`. The array of the tuple holds the
    /// elements, and copies of the tuple share it. Code outside the term
    /// module builds a tuple with this function, and it does not name
    /// `Term::Tuple`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let tuple = Term::tuple(vec![Term::Integer(1), Term::Integer(2)]);
    /// assert_eq!(tuple, Term::tuple(vec![Term::Integer(1), Term::Integer(2)]));
    /// assert_ne!(tuple, Term::tuple(vec![Term::Integer(1)]));
    /// ```
    pub fn tuple(elements: Vec<Term>) -> Term {
        Term::Tuple(elements.into())
    }

    /// Return the elements of a tuple, or `None` when the term is not a
    /// tuple. The slice borrows the tuple, so the caller copies nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::tuple(vec![Term::Integer(1)]).tuple_elements(), Some([Term::Integer(1)].as_slice()));
    /// ```
    pub fn tuple_elements(&self) -> Option<&[Term]> {
        match self {
            Term::Tuple(tuple) => Some(tuple.arc().as_slice()),
            _ => None,
        }
    }

    /// Return the value of an integer in the ERTS small-integer range,
    /// -(2^59) to 2^59 - 1, as the ERTS checks `is_small` and `signed_val`
    /// do. Return `None` for any other term. ERTS stores a larger integer as
    /// a bignum, which is not a small integer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::Integer(-1).small_integer(), Some(-1));
    /// assert_eq!(Term::Integer(1 << 59).small_integer(), None);
    /// assert_eq!(Term::static_atom("ok").small_integer(), None);
    /// ```
    pub(crate) fn small_integer(&self) -> Option<i64> {
        match self {
            Term::Integer(value) if (SMALL_INTEGER_MIN..=SMALL_INTEGER_MAX).contains(value) => {
                Some(*value)
            }
            _ => None,
        }
    }

    /// Return whether the term is a fun or contains a fun at any depth: in a
    /// list, an improper list and its tail, a tuple, or a key or a value of
    /// a map. The walk uses a work list, not recursion, so a deep term does
    /// not overflow the stack.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Term::list_from(vec![Term::Integer(1)]).contains_fun());
    /// assert!(Term::tuple(vec![Term::list_from(vec![fun])]).contains_fun());
    /// ```
    pub(crate) fn contains_fun(&self) -> bool {
        let mut pending = vec![self];
        while let Some(term) = pending.pop() {
            match term {
                Term::Fun(_) => return true,
                Term::Tuple(elements) => pending.extend(elements.iter()),
                Term::Cons(cell) => {
                    pending.push(&cell.tail);
                    pending.push(&cell.head);
                }
                Term::Map(map) => pending.extend(map.iter().flat_map(|(key, value)| [key, value])),
                _ => {}
            }
        }
        false
    }

    /// Return whether this term is a map and holds the only reference to
    /// its form. Every other term reports `false`, because a change in place
    /// is measured only for a map; a caller that changes another value takes
    /// the copy path.
    ///
    /// A map count of one is a necessary condition for a change in place,
    /// not a sufficient one: the nodes of the trie can still have another
    /// owner, and the change copies them (`Arc::make_mut`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))], &AtomTable::new())?);
    /// assert!(map.unique());
    /// let copy = map.clone();
    /// assert!(!map.unique() || !copy.unique());
    /// assert!(!Term::tuple(vec![Term::Integer(1)]).unique());
    /// assert!(!Term::Integer(1).unique());
    /// ```
    pub(crate) fn unique(&self) -> bool {
        matches!(self, Term::Map(map) if map.unique())
    }

    /// Move the nested terms of a list, tuple, map, or fun onto a work stack
    /// and leave the term empty. A tuple gives up its array
    /// ([`Tuple::take_last`]), and only the last holder of the array moves
    /// its elements onto the stack. A cons term gives up its cell
    /// ([`ConsRef::take_last`]), and only the last holder of the cell moves
    /// its tail and then its head onto the stack. The head comes off the
    /// stack first, so a long list keeps the stack short.
    ///
    /// The method moves one compound node and its direct children, and it
    /// does not walk the graph. The children of a tuple, of a fun, and of a
    /// map can be many, and the whole group moves in this one call: see
    /// [`Term::take_group`] for the bounded release of a tuple or of a fun.
    /// A caller that owns a work stack can release the graph one bounded
    /// step at a time, as `Scheduler::drain_release` does for the exit of a
    /// process that owns a large graph (row 11 of
    /// `docs/term-performance-design-codex-v2.md`, task
    /// [#559](https://github.com/fahchen/pbeam/issues/559)).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut pending = Vec::new();
    /// Term::tuple(vec![Term::Integer(1)]).take_nested(&mut pending);
    /// assert_eq!(pending, vec![Term::Integer(1)]);
    /// ```
    pub(crate) fn take_nested(&mut self, pending: &mut Vec<Term>) {
        match self {
            Term::Tuple(elements) => pending.extend(elements.take_last().unwrap_or_default()),
            Term::Cons(cell) => {
                if let Some(Cons { head, tail }) = cell.take_last() {
                    pending.extend([tail, head]);
                }
            }
            Term::Fun(fun) => pending.extend(fun.take_last().unwrap_or_default()),
            Term::Map(map) => map.drain_into(pending),
            _ => {}
        }
    }

    /// Take the element array of a tuple or the free variables of a fun as
    /// one owned vector, and leave the term empty. Return `None` when the
    /// term holds no such array: an immediate, a binary, a match context, a
    /// cons cell of two terms, an empty container, or a map.
    ///
    /// The vector can hold many terms, and it is the array that the term
    /// already owns, so the call copies nothing. A caller that releases a
    /// graph in bounded steps moves the vector in batches, as
    /// `Scheduler::drain_release` does (row 11 of
    /// `docs/term-performance-design-codex-v2.md`, task
    /// [#559](https://github.com/fahchen/pbeam/issues/559)).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut term = Term::tuple(vec![Term::Integer(1)]);
    /// assert_eq!(term.take_group(), Some(vec![Term::Integer(1)]));
    /// assert_eq!(term.take_group(), None);
    /// ```
    pub(crate) fn take_group(&mut self) -> Option<Group> {
        match self {
            Term::Tuple(elements) => elements
                .take_last()
                .filter(|elements| !elements.is_empty())
                .map(Group::Terms),
            Term::Fun(fun) => fun
                .take_last()
                .filter(|arguments| !arguments.is_empty())
                .map(Group::Terms),
            Term::Map(map) => map::Drain::take(map).map(Group::Map),
            _ => None,
        }
    }
}

#[cfg(test)]
impl Term {
    /// Return whether two terms hold one shared storage: one element array
    /// of a tuple, one cons cell, or one form of a map. A test uses it to
    /// show that a send, an ETS lookup, and a `persistent_term:get/1`
    /// result share the term of the store (owner decision R3 of
    /// [#450](https://github.com/fahchen/pbeam/issues/450),
    /// `docs/copy-on-write-terms.md`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = Term::tuple(vec![Term::Integer(1)]);
    /// assert!(term.shares_storage_with(&term.clone()));
    /// ```
    pub(crate) fn shares_storage_with(&self, other: &Term) -> bool {
        match (self, other) {
            (Term::Cons(left), Term::Cons(right)) => left.same_cell(right),
            (Term::Tuple(left), Term::Tuple(right)) => left.same_array(right),
            (Term::Map(left), Term::Map(right)) => left.same_form(right),
            _ => false,
        }
    }
}

impl Drop for Term {
    /// Move nested terms onto a work stack, so dropping a deep term does not
    /// recurse. A tuple and a fun give the array that they already own to the
    /// walk, so their drop allocates nothing and a term that shares its array
    /// needs no walk. A list and a map give one nested term at a time, and
    /// their walk holds them on a vector.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let deep = (0..1_000_000).fold(Term::tuple(vec![]), |inner, _| Term::tuple(vec![inner]));
    /// drop(deep);
    /// ```
    fn drop(&mut self) {
        match self {
            Term::Tuple(elements) => {
                if let Some(pending) = elements.take_last() {
                    drop_nested(pending);
                }
            }
            Term::Fun(fun) => {
                if let Some(pending) = fun.take_last() {
                    drop_nested(pending);
                }
            }
            Term::Cons(cell) => {
                if let Some(Cons { head, tail }) = cell.take_last() {
                    drop_nested(vec![tail, head]);
                }
            }
            Term::Map(map) => {
                let mut pending = Vec::new();
                map.drain_into(&mut pending);
                drop_nested(pending);
            }
            _ => {}
        }
    }
}

/// Drop each term of `pending`, and then each nested term that it gives. The
/// walk is iterative, so dropping a deep term does not recurse on the Rust
/// stack.
///
/// # Examples
///
/// ```rust,ignore
/// drop_nested(vec![Term::Integer(1)]);
/// ```
fn drop_nested(mut pending: Vec<Term>) {
    while let Some(mut term) = pending.pop() {
        term.take_nested(&mut pending);
    }
}

impl Clone for Term {
    /// Copy a term. The copy of a compound term shares its storage, so it
    /// takes O(1) time and does not walk the nested terms: a tuple copy
    /// shares its element array, a map copy shares the form of the map, a
    /// list copy shares its cons cells, a fun copy shares its free
    /// variables, a binary copy shares its bytes, and a match context copy
    /// shares its state. A send, ETS, and `persistent_term` use this copy,
    /// so they share a term with the process that gives it (owner decision
    /// R3 of [#450](https://github.com/fahchen/pbeam/issues/450),
    /// `docs/copy-on-write-terms.md`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = Term::tuple(vec![Term::list_from([Term::Integer(1)])]);
    /// assert_eq!(term.clone(), term);
    /// ```
    fn clone(&self) -> Self {
        match self {
            Term::Atom(atom) => Term::Atom(atom.clone()),
            Term::Binary(bytes) => Term::Binary(bytes.clone()),
            Term::Float(value) => Term::Float(*value),
            Term::Integer(value) => Term::Integer(*value),
            Term::BigInt(value) => Term::BigInt(Arc::clone(value)),
            Term::Nil => Term::Nil,
            Term::Cons(cell) => Term::Cons(cell.clone()),
            Term::Tuple(elements) => Term::Tuple(elements.clone()),
            Term::Map(map) => Term::Map(map.clone()),
            Term::Pid(pid) => Term::Pid(*pid),
            Term::Port(port) => Term::Port(*port),
            Term::Reference(reference) => Term::Reference(*reference),
            Term::Fun(fun) => Term::Fun(fun.clone()),
            Term::MatchContext(context) => Term::MatchContext(Arc::clone(context)),
        }
    }
}

impl PartialEq for Term {
    /// Compare two terms pair by pair with a work stack. The stack holds the
    /// first [`EQ_INLINE_PAIRS`] pairs in the frame of the comparison, so a
    /// shallow or a shared comparison allocates nothing. Two tuples that
    /// hold one element array are equal with no walk. Two other tuples are
    /// equal when their lengths are equal and each pair of elements is
    /// equal. Two terms that share one cons cell are equal with no walk, and
    /// two other cons cells are equal when their heads and their tails are
    /// equal, as `eq` in `utils.c:1137-1162` compares each head and then the
    /// last tail. Two maps are equal when their pairs in canonical order are
    /// equal, as the structural ERTS comparison of two flatmaps or two
    /// hashmaps is. Two local funs are equal when their fun entries and
    /// their free variables are equal, and two external funs are equal when
    /// their modules, functions, and arities are equal, as `eq` in
    /// `utils.c:1241-1276` gives.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::list_from([Term::Integer(1)]), Term::list_from([Term::Integer(1)]));
    /// assert_ne!(Term::nil(), Term::tuple(vec![]));
    /// assert_ne!(Term::Float(0.0), Term::Float(-0.0));
    /// assert_ne!(Term::Integer(1), Term::Float(1.0));
    /// ```
    fn eq(&self, other: &Self) -> bool {
        self.equal(other, false)
    }
}

impl Eq for Term {}

impl Term {
    /// Return true when `self` and `other` are equal in arithmetic
    /// comparison, as `==` is (`eq` with `exact = 0` in `erts_cmp_compound`,
    /// `utils.c:2355-2470`). An integer and a float are equal when they
    /// have the same value, and `0.0` and `-0.0` are equal. Map keys
    /// compare exactly and map values compare arithmetically, as
    /// `erts_cmp_compound` does (`utils.c:2506-2533`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::Integer(1).arith_eq(&Term::Float(1.0)));
    /// assert!(Term::Float(0.0).arith_eq(&Term::Float(-0.0)));
    /// assert!(!Term::Integer(1).arith_eq(&Term::Float(1.5)));
    /// ```
    pub(crate) fn arith_eq(&self, other: &Term) -> bool {
        self.equal(other, true)
    }

    /// Return the value of a float term, or `None` for each other term.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::Float(1.5).float(), Some(1.5));
    /// assert_eq!(Term::Integer(1).float(), None);
    /// ```
    pub(crate) fn float(&self) -> Option<f64> {
        match self {
            Term::Float(value) => Some(*value),
            _ => None,
        }
    }

    /// Compare two terms for exact equality, or for arithmetic equality when
    /// `arith` is true. See [`PartialEq::eq`] and [`Term::arith_eq`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::Integer(2).equal(&Term::Float(2.0), true));
    /// assert!(!Term::Integer(2).equal(&Term::Float(2.0), false));
    /// ```
    fn equal(&self, other: &Term, arith: bool) -> bool {
        let mut pairs = EqStack::new();
        let mut left = self;
        let mut right = other;
        loop {
            let equal = match (left, right) {
                (Term::Tuple(left), Term::Tuple(right)) => {
                    // Two tuples that hold one array are equal with no walk,
                    // as `eq` compares the two boxed pointers first.
                    if !left.same_array(right) {
                        pairs.extend(left.iter().zip(right.iter()));
                    }
                    left.len() == right.len()
                }
                (Term::Nil, Term::Nil) => true,
                (Term::Cons(left_cell), Term::Cons(right_cell)) => {
                    // The comparison reads the head pair and then the tail
                    // pair, as `eq` in `utils.c:1141-1162` reads each head and
                    // then the last tail. The head pair becomes the current
                    // pair, so a long list pushes one pair for each cell
                    // rather than two and keeps the stack short.
                    if left_cell.same_cell(right_cell) {
                        true
                    } else {
                        pairs.push(&left_cell.tail, &right_cell.tail);
                        left = &left_cell.head;
                        right = &right_cell.head;
                        continue;
                    }
                }
                (Term::Fun(left), Term::Fun(right)) => {
                    // Two funs that hold one thing are equal with no walk,
                    // as `eq` compares the two boxed pointers first.
                    if left.same_thing(right) {
                        true
                    } else {
                        pairs.extend(left.free().iter().zip(right.free()));
                        left.target() == right.target() && left.free().len() == right.free().len()
                    }
                }
                (Term::Map(left), Term::Map(right)) => {
                    let mut keys_equal = true;
                    for ((left_key, left_value), (right_key, right_value)) in
                        left.iter().zip(right.iter())
                    {
                        if arith {
                            // Map keys compare exactly also in arithmetic
                            // comparison, so `#{1 => a} == #{1.0 => a}` is
                            // false.
                            keys_equal &= left_key == right_key;
                        } else {
                            pairs.push(left_key, right_key);
                        }
                        pairs.push(left_value, right_value);
                    }
                    keys_equal && left.len() == right.len()
                }
                (Term::Atom(left), Term::Atom(right)) => left == right,
                (Term::Binary(left), Term::Binary(right)) => left == right,
                (Term::Integer(left), Term::Integer(right)) => left == right,
                (Term::BigInt(left), Term::BigInt(right)) => left == right,
                (Term::BigInt(big), Term::Float(float))
                | (Term::Float(float), Term::BigInt(big))
                    if arith =>
                {
                    order::compare_big_float(big, *float).is_eq()
                }
                // Exact equality compares the bits, so `0.0 =:= -0.0` is
                // false (`eq`, `utils.c:1422-1433`). A float term is never
                // a NaN, so equal bits are the only equal floats.
                (Term::Float(left), Term::Float(right)) if arith => left == right,
                (Term::Float(left), Term::Float(right)) => left.to_bits() == right.to_bits(),
                (Term::Integer(integer), Term::Float(float))
                | (Term::Float(float), Term::Integer(integer))
                    if arith =>
                {
                    order::compare_integer_float(*integer, *float).is_eq()
                }
                (Term::Pid(left), Term::Pid(right)) => left == right,
                (Term::Port(left), Term::Port(right)) => left == right,
                (Term::Reference(left), Term::Reference(right)) => left == right,
                // A match context is a heap object, so its identity is its
                // address. ERTS never compares a match context
                // (`get_term_type`, beam_validator.erl:2903-2908).
                (Term::MatchContext(left), Term::MatchContext(right)) => Arc::ptr_eq(left, right),
                _ => false,
            };
            if !equal {
                return false;
            }
            match pairs.pop() {
                Some(EqPair(next_left, next_right)) => {
                    left = next_left;
                    right = next_right;
                }
                None => return true,
            }
        }
    }
    /// Return true when `self` and `other` hold the same value. The
    /// comparison remembers each pair of shared compounds that it decided,
    /// so a term that holds one subtree more than once takes time in the
    /// size of its storage and not in the size of its expansion. The two
    /// terms must be of one atom table, as two terms of `Term::eq` are.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let shared = Term::tuple(vec![Term::Integer(1)]);
    /// let term = Term::tuple(vec![shared.clone(), shared]);
    /// assert!(term.eq_shared(&term.clone()));
    /// ```
    pub(crate) fn eq_shared(&self, other: &Term) -> bool {
        let mut pending = vec![(self, other)];
        // The pairs of compounds whose equality the comparison decided. A
        // pair of one shared subtree and its rebuilt copy enters once, and
        // each later occurrence of the pair finds the decision.
        let mut decided: HashSet<(usize, usize)> = HashSet::new();
        while let Some((left, right)) = pending.pop() {
            match (left, right) {
                (Term::Tuple(left), Term::Tuple(right)) => {
                    if left.same_array(right) {
                        continue;
                    }
                    if !decided.insert((left.identity(), right.identity())) {
                        continue;
                    }
                    if left.len() != right.len() {
                        return false;
                    }
                    pending.extend(left.iter().zip(right.iter()));
                }
                (Term::Cons(left), Term::Cons(right)) => {
                    if left.same_cell(right) {
                        continue;
                    }
                    if !decided.insert((left.identity(), right.identity())) {
                        continue;
                    }
                    pending.push((&left.tail, &right.tail));
                    pending.push((&left.head, &right.head));
                }
                (Term::Map(left), Term::Map(right)) => {
                    if left.same_form(right) {
                        continue;
                    }
                    if !decided.insert((left.identity(), right.identity())) {
                        continue;
                    }
                    if left.len() != right.len() {
                        return false;
                    }
                    for ((left_key, left_value), (right_key, right_value)) in
                        left.iter().zip(right.iter())
                    {
                        pending.push((left_key, right_key));
                        pending.push((left_value, right_value));
                    }
                }
                (Term::Fun(left), Term::Fun(right)) => {
                    if left.same_thing(right) {
                        continue;
                    }
                    if !decided.insert((left.identity(), right.identity())) {
                        continue;
                    }
                    if left.target() != right.target() || left.free().len() != right.free().len() {
                        return false;
                    }
                    pending.extend(left.free().iter().zip(right.free()));
                }
                _ => {
                    if left != right {
                        return false;
                    }
                }
            }
        }
        true
    }
}

impl fmt::Debug for Term {
    /// Write the text of a derived `Debug`, for example
    /// `Tuple([Atom(725), Integer(1)])` or `Map([(Atom(1150), Integer(1))])`,
    /// with a work stack. An atom writes its table index, because a bare term
    /// carries no table and the caller of `Debug::fmt` cannot pass one;
    /// [`debug_term`] writes the name through a table. A list writes its
    /// elements: a proper list as `List([Integer(1)])` and `[]` as
    /// `List([])`, and an improper list with its tail as
    /// `ImproperList([Integer(1)], Atom(725))`. A fun writes its entry and
    /// its free variables, for example
    /// `Fun(Local(Lambda { .. }), [Atom(725)])`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = Term::tuple(vec![Term::static_atom("undef"), Term::nil()]);
    /// assert_eq!(format!("{term:?}"), "Tuple([Atom(725), List([])])");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_debug(self, None, formatter)
    }
}

/// Return the text of `term` for a message, with each atom name read from
/// `atoms`.
///
/// [`Debug for Term`] writes the atom index, because a bare term carries no
/// table and the caller of `Debug::fmt` cannot pass one. A production message
/// that holds the table calls this function, so its text keeps the atom name.
/// Every other part of the text is the same as the `Debug` text.
///
/// # Examples
///
/// ```rust,ignore
/// let table = AtomTable::new();
/// let term = Term::tuple(vec![Term::static_atom("undef"), Term::nil()]);
/// assert_eq!(debug_term(&term, &table), "Tuple([Atom(\"undef\"), List([])])");
/// ```
pub fn debug_term(term: &Term, atoms: &AtomTable) -> String {
    let mut text = String::new();
    write_debug(term, Some(atoms), &mut text).expect("writing to a String succeeds");
    text
}

/// Write the `Debug` text of `term` to `out`, with the atom name from `atoms`
/// when it is `Some` and the atom index when it is `None`.
fn write_debug(term: &Term, atoms: Option<&AtomTable>, out: &mut impl fmt::Write) -> fmt::Result {
    enum Item<'a> {
        Term(&'a Term),
        Text(&'static str),
    }
    let mut items = vec![Item::Term(term)];
    while let Some(item) = items.pop() {
        let term = match item {
            Item::Text(text) => {
                out.write_str(text)?;
                continue;
            }
            Item::Term(term) => term,
        };
        match term {
            Term::Atom(atom) => match atoms {
                Some(atoms) => write!(out, "Atom({:?})", atoms.name_of(atom))?,
                None => write!(out, "Atom({})", atom.index())?,
            },
            Term::Binary(bytes) => write!(out, "Binary({bytes:?})")?,
            Term::Integer(value) => write!(out, "Integer({value:?})")?,
            Term::BigInt(value) => write!(out, "Integer({value})")?,
            Term::Float(value) => write!(out, "Float({value:?})")?,
            Term::Pid(pid) => write!(out, "Pid({pid:?})")?,
            Term::Port(port) => write!(out, "Port({port:?})")?,
            Term::Reference(reference) => write!(out, "Reference({reference:?})")?,
            Term::MatchContext(context) => write!(out, "MatchContext({context:?})")?,
            Term::Nil | Term::Cons(_) => {
                let (elements, tail) = term.list_cells().expect("the term is a list");
                let elements: Vec<_> = elements.collect();
                match tail {
                    None => {
                        out.write_str("List([")?;
                        items.push(Item::Text("])"));
                    }
                    Some(tail) => {
                        out.write_str("ImproperList([")?;
                        items.push(Item::Text(")"));
                        items.push(Item::Term(tail));
                        items.push(Item::Text("], "));
                    }
                }
                for (index, element) in elements.iter().enumerate().rev() {
                    items.push(Item::Term(element));
                    if index > 0 {
                        items.push(Item::Text(", "));
                    }
                }
            }
            Term::Tuple(elements) => {
                out.write_str("Tuple([")?;
                items.push(Item::Text("])"));
                for (index, element) in elements.iter().enumerate().rev() {
                    items.push(Item::Term(element));
                    if index > 0 {
                        items.push(Item::Text(", "));
                    }
                }
            }
            Term::Fun(fun) => {
                let (target, free) = (fun.target(), fun.free());
                match atoms {
                    Some(atoms) => {
                        out.write_str("Fun(")?;
                        debug_target(target, atoms, out)?;
                        out.write_str(", [")?;
                    }
                    None => write!(out, "Fun({target:?}, [")?,
                }
                items.push(Item::Text("])"));
                for (index, element) in free.iter().enumerate().rev() {
                    items.push(Item::Term(element));
                    if index > 0 {
                        items.push(Item::Text(", "));
                    }
                }
            }
            Term::Map(map) => {
                out.write_str("Map([")?;
                items.push(Item::Text("])"));
                let pairs: Vec<_> = map.iter().collect();
                for (index, (key, value)) in pairs.into_iter().enumerate().rev() {
                    items.push(Item::Text(")"));
                    items.push(Item::Term(value));
                    items.push(Item::Text(", "));
                    items.push(Item::Term(key));
                    items.push(Item::Text("("));
                    if index > 0 {
                        items.push(Item::Text(", "));
                    }
                }
            }
        }
    }
    Ok(())
}

/// An identifier for a process in this runtime. The order of identifiers
/// is their creation order; the runtime uses it to visit the links of a
/// process in one fixed order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessId(pub(crate) u64);

/// An identifier for a port in this runtime: a counter value, which counts
/// from 0 in each run. PBeam takes it as the ERTS port data, which the
/// `Eterm` word of a local port holds above the port tag `0x7`
/// (`erl_term.h:82`, `697-703`), as a pid takes its counter value as the
/// pid data. The order of identifiers is their creation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(pub(crate) u64);

/// An identifier for a reference in this runtime: a counter value, and the
/// pid reference marker in its high bit. The order of identifiers is their
/// creation order; the runtime uses it to visit the monitors of a process in
/// one fixed order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reference(pub(crate) u64);

/// The bit of a reference value that marks a pid reference: the reference
/// that `erlang:monitor/3` makes for an alias option. ERTS marks the
/// reference numbers of a pid reference with `ERTS_REF1_PID_MARKER_BIT__`
/// (`erl_bif_unique.h:106-109`, `146-149`), and the mark stays when the
/// owner exits. The reference counter stays below this bit. The bit shows
/// in the number `N2` of [`Reference::numbers`].
const PID_REFERENCE_MARKER: u64 = 1 << 63;

/// The mask of the first reference number (`REF_MASK`, 18 bits).
const REF_NUMBER_0_MASK: u64 = 0x3_FFFF;
/// The mask of the counter bits in the second reference number
/// (`ERTS_REF1_NUM_MASK__`).
const REF_NUMBER_1_MASK: u64 = 0xFFFC_0000;
/// The scheduler thread number of a PBeam reference. ERTS gives the first
/// normal scheduler the number 1 (`init_scheduler_data` in
/// `erl_process.c`).
const REF_THREAD: u64 = 1;

impl Reference {
    /// Return the ordinary reference with the counter value `value`, or
    /// `None` when `value` has the pid reference marker bit.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Reference::ordinary(0), Some(Reference(0)));
    /// assert_eq!(Reference::ordinary(u64::MAX), None);
    /// ```
    pub(crate) fn ordinary(value: u64) -> Option<Self> {
        (value & PID_REFERENCE_MARKER == 0).then_some(Reference(value))
    }

    /// Return the pid reference with the counter value of the ordinary
    /// reference `self`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Reference(0).pid_reference().is_pid_reference());
    /// ```
    pub(crate) fn pid_reference(self) -> Self {
        Reference(self.0 | PID_REFERENCE_MARKER)
    }

    /// Return true for a pid reference, as `erts_is_pid_ref_numbers`
    /// (`erl_bif_unique.h:145-149`) does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Reference(0).is_pid_reference());
    /// ```
    pub(crate) fn is_pid_reference(self) -> bool {
        self.0 & PID_REFERENCE_MARKER != 0
    }

    /// Return the three ERTS reference numbers `[N0, N1, N2]` of the
    /// reference. ERTS makes them from a 64-bit counter value of a
    /// scheduler (`erts_set_ref_numbers`, `erl_bif_unique.h:117-131`): `N0` is the
    /// low 18 bits, `N1` is the next 14 counter bits with the scheduler
    /// number in its low bits, and `N2` is the high 32 bits. PBeam takes
    /// its reference value as the counter value of scheduler 1. ERTS prints
    /// the reference as `#Ref<0.N2.N1.N0>`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Reference(0).numbers(), [0, 1, 0]);
    /// assert_eq!(
    ///     Reference(0xF55F_5681_5BA8_8196).numbers(),
    ///     [33174, 1537736705, 4116665985]
    /// );
    /// ```
    pub(crate) fn numbers(self) -> [u32; 3] {
        let Reference(value) = self;
        [
            (value & REF_NUMBER_0_MASK) as u32,
            ((value & REF_NUMBER_1_MASK) | REF_THREAD) as u32,
            (value >> 32) as u32,
        ]
    }
}

impl Ord for Reference {
    /// Compare the counter values, so that the order is the creation order
    /// also for a pid reference.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.0 & !PID_REFERENCE_MARKER).cmp(&(other.0 & !PID_REFERENCE_MARKER))
    }
}

impl PartialOrd for Reference {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
#[path = "../tests/term/costs.rs"]
mod costs;

#[cfg(test)]
#[path = "../tests/term.rs"]
mod tests;

/// The comparison cost and the allocation count of `Term`, the measurement
/// of the pair-stack candidate of `docs/data-structure-audit.md` section
/// 4.2. The module counts the allocations of the process, so it loads into
/// the library test target.
#[cfg(test)]
#[path = "../tests/term/data_structure_costs.rs"]
mod data_structure_costs;
