mod atom;
mod fun;
mod hash;
mod list;
mod map;
mod match_context;
mod order;
mod print;

pub use atom::Atom;
#[cfg(test)]
pub(crate) use atom::test_atom;
pub(crate) use atom::{AtomTable, MAX_ATOM_CHARACTERS};
pub use fun::Fun;
pub(crate) use fun::{Lambda, Target};
pub(crate) use hash::internal_hash;
use list::{Cons, ConsRef};
pub use map::Map;
pub(crate) use map::{Direction, from_list_units};
pub use match_context::MatchContext;
pub(crate) use match_context::{segment_equals, segment_integer};
pub(crate) use print::print_term;

use std::{cell::RefCell, fmt, rc::Rc};

/// The smallest ERTS small integer on a 64-bit system: -(2^59).
pub(crate) const SMALL_INTEGER_MIN: i64 = -(1 << 59);
/// The largest ERTS small integer on a 64-bit system: 2^59 - 1.
const SMALL_INTEGER_MAX: i64 = (1 << 59) - 1;
/// The largest number of elements in a tuple: 2^24 - 1
/// (`ERTS_MAX_TUPLE_SIZE` and `MAX_ARITYVAL` in `erl_term.h:322-323`).
pub(crate) const MAX_TUPLE_SIZE: usize = (1 << 24) - 1;

/// A BEAM term. Lists, tuples, maps, and funs can nest to any depth, as in
/// ERTS, so `Drop`, `Clone`, `PartialEq`, and `Debug` use explicit work
/// stacks and do not recurse on the Rust stack.
pub enum Term {
    /// An atom: its index in the atom table of the program, and its name.
    Atom(Atom),
    Binary(Vec<u8>),
    Integer(i64),
    /// The empty list `[]`.
    Nil,
    /// A cons cell of a list: a head and a tail, for example `[1 | T]`. A
    /// proper list ends in `[]`, and an improper list ends in another term,
    /// for example `[1, 2 | x]`. Copies of a list share its cells. Code
    /// outside the term module uses the list API of `src/term/list.rs`.
    Cons(ConsRef),
    /// A BEAM tuple.
    Tuple(Vec<Term>),
    /// A BEAM map: a flatmap up to 32 keys and a hashmap above 32 keys, as
    /// in ERTS. `Map` keeps each map in its canonical form, so two maps with
    /// the same pairs are equal. A key is a binary, an atom, a small integer,
    /// `[]`, a list or a tuple whose elements are atoms, small integers,
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
    /// `bs_start_match4`, shares the match context and its position. The
    /// registers and stack slots of one process own the references, and
    /// the last reference that drops releases the match context. Only the
    /// binary-match instructions read it, `move` copies it, and a local
    /// call keeps it in its argument register. Each other instruction that
    /// gets a match context gives a runtime invariant error.
    MatchContext(Rc<RefCell<MatchContext>>),
}

/// The kind of a term that a clone copies element by element. A fun keeps
/// the entry of the source fun.
#[derive(Clone, Copy)]
enum Kind<'a> {
    Tuple,
    Fun(&'a Target),
}

/// The nested terms of a tuple or fun, in order. A fun gives its free
/// variables.
type Nested<'a> = std::slice::Iter<'a, Term>;

impl Kind<'_> {
    /// Build a term of this kind from its nested terms.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Kind::Tuple.build(vec![]), Term::Tuple(vec![]));
    /// ```
    fn build(self, terms: Vec<Term>) -> Term {
        match self {
            Kind::Tuple => Term::Tuple(terms),
            Kind::Fun(target) => Term::Fun(Fun {
                target: target.clone(),
                free: terms,
            }),
        }
    }
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

    /// Copy a term that has no nested terms. A map copy shares the form of
    /// the map, and a list copy shares its cons cells, so they do not copy
    /// the nested terms. For a tuple or fun, return its nested terms, kind,
    /// and count instead, so the caller copies them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::Integer(1).shallow_clone().ok(), Some(Term::Integer(1)));
    /// assert!(Term::Map(Map::new()).shallow_clone().is_ok());
    /// assert!(Term::list_from([Term::Integer(1)]).shallow_clone().is_ok());
    /// assert!(Term::Tuple(vec![]).shallow_clone().is_err());
    /// ```
    fn shallow_clone(&self) -> Result<Term, (Nested<'_>, Kind<'_>, usize)> {
        match self {
            Term::Nil => Ok(Term::Nil),
            Term::Cons(cell) => Ok(Term::Cons(cell.clone())),
            Term::Tuple(elements) => Err((elements.iter(), Kind::Tuple, elements.len())),
            Term::Map(map) => Ok(Term::Map(map.clone())),
            Term::Fun(fun) => Err((fun.free.iter(), Kind::Fun(&fun.target), fun.free.len())),
            Term::Atom(atom) => Ok(Term::Atom(atom.clone())),
            Term::Binary(bytes) => Ok(Term::Binary(bytes.clone())),
            Term::Integer(value) => Ok(Term::Integer(*value)),
            Term::Pid(pid) => Ok(Term::Pid(*pid)),
            Term::Port(port) => Ok(Term::Port(*port)),
            Term::Reference(reference) => Ok(Term::Reference(*reference)),
            Term::MatchContext(context) => Ok(Term::MatchContext(context.clone())),
        }
    }

    /// Move the nested terms of a list, tuple, map, or fun onto a work stack
    /// and leave the term empty. A cons term gives up its cell
    /// ([`ConsRef::take_last`]), and only the last holder of the cell moves
    /// its tail and then its head onto the stack. The head comes off the
    /// stack first, so a long list keeps the stack short.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut pending = Vec::new();
    /// Term::Tuple(vec![Term::Integer(1)]).take_nested(&mut pending);
    /// assert_eq!(pending, vec![Term::Integer(1)]);
    /// ```
    fn take_nested(&mut self, pending: &mut Vec<Term>) {
        match self {
            Term::Tuple(elements) => pending.append(elements),
            Term::Cons(cell) => {
                if let Some(Cons { head, tail }) = cell.take_last() {
                    pending.extend([tail, head]);
                }
            }
            Term::Fun(fun) => pending.append(&mut fun.free),
            Term::Map(map) => map.drain_into(pending),
            _ => {}
        }
    }
}

impl Drop for Term {
    /// Move nested terms onto a work stack, so dropping a deep term does not
    /// recurse.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let deep = (0..1_000_000).fold(Term::Tuple(vec![]), |inner, _| Term::Tuple(vec![inner]));
    /// drop(deep);
    /// ```
    fn drop(&mut self) {
        let mut pending = Vec::new();
        self.take_nested(&mut pending);
        while let Some(mut term) = pending.pop() {
            term.take_nested(&mut pending);
        }
    }
}

impl Clone for Term {
    /// Copy a term with a work stack of partly copied tuples and funs. A
    /// map copy shares the form of the map, and a list copy shares its cons
    /// cells.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = Term::Tuple(vec![Term::list_from([Term::Integer(1)])]);
    /// assert_eq!(term.clone(), term);
    /// ```
    fn clone(&self) -> Self {
        let (nested, kind, count) = match self.shallow_clone() {
            Ok(term) => return term,
            Err(container) => container,
        };
        let mut stack = vec![(nested, kind, Vec::with_capacity(count))];
        loop {
            let (source, _, copied) = stack.last_mut().expect("the clone stack is not empty");
            if let Some(term) = source.next() {
                match term.shallow_clone() {
                    Ok(term) => copied.push(term),
                    Err((nested, kind, count)) => {
                        stack.push((nested, kind, Vec::with_capacity(count)));
                    }
                }
                continue;
            }
            let (_, kind, copied) = stack.pop().expect("the clone stack is not empty");
            let term = kind.build(copied);
            match stack.last_mut() {
                Some((_, _, parent)) => parent.push(term),
                None => return term,
            }
        }
    }
}

impl PartialEq for Term {
    /// Compare two terms pair by pair with a work stack. Two cons cells are
    /// equal when their heads and their tails are equal, as `eq` in
    /// `utils.c:1137-1162` compares each head and then the last tail. Two
    /// terms that share one cons cell are equal with no walk. Two maps are equal
    /// when their pairs in canonical order are equal, as the structural ERTS
    /// comparison of two flatmaps or two hashmaps is. Two local funs are
    /// equal when their fun entries and their free variables are equal, and
    /// two external funs are equal when their modules, functions, and
    /// arities are equal, as `eq` in `utils.c:1241-1276` gives.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::list_from([Term::Integer(1)]), Term::list_from([Term::Integer(1)]));
    /// assert_ne!(Term::nil(), Term::Tuple(vec![]));
    /// ```
    fn eq(&self, other: &Self) -> bool {
        let mut pairs = vec![(self, other)];
        while let Some(pair) = pairs.pop() {
            let equal = match pair {
                (Term::Tuple(left), Term::Tuple(right)) => {
                    pairs.extend(left.iter().zip(right));
                    left.len() == right.len()
                }
                (Term::Nil, Term::Nil) => true,
                (Term::Cons(left), Term::Cons(right)) => {
                    // The tails go on the stack first, so the heads come off
                    // first and a long list keeps the stack short.
                    if !left.same_cell(right) {
                        pairs.extend([(&left.tail, &right.tail), (&left.head, &right.head)]);
                    }
                    true
                }
                (Term::Fun(left), Term::Fun(right)) => {
                    pairs.extend(left.free.iter().zip(&right.free));
                    left.target == right.target && left.free.len() == right.free.len()
                }
                (Term::Map(left), Term::Map(right)) => {
                    for ((left_key, left_value), (right_key, right_value)) in
                        left.iter().zip(right.iter())
                    {
                        pairs.push((left_key, right_key));
                        pairs.push((left_value, right_value));
                    }
                    left.len() == right.len()
                }
                (Term::Atom(left), Term::Atom(right)) => left == right,
                (Term::Binary(left), Term::Binary(right)) => left == right,
                (Term::Integer(left), Term::Integer(right)) => left == right,
                (Term::Pid(left), Term::Pid(right)) => left == right,
                (Term::Port(left), Term::Port(right)) => left == right,
                (Term::Reference(left), Term::Reference(right)) => left == right,
                // A match context is a heap object, so its identity is its
                // address. ERTS never compares a match context
                // (`get_term_type`, beam_validator.erl:2903-2908).
                (Term::MatchContext(left), Term::MatchContext(right)) => Rc::ptr_eq(left, right),
                _ => false,
            };
            if !equal {
                return false;
            }
        }
        true
    }
}

impl Eq for Term {}

impl fmt::Debug for Term {
    /// Write the text of a derived `Debug`, for example
    /// `Tuple([Atom("undef"), Integer(1)])` or `Map([(Atom("a"), Integer(1))])`,
    /// with a work stack. A list writes its elements: a proper list as
    /// `List([Integer(1)])` and `[]` as `List([])`, and an improper list
    /// with its tail as `ImproperList([Integer(1)], Atom("x"))`. A fun
    /// writes its entry and its free variables, for example
    /// `Fun(Local(Lambda { .. }), [Atom("tag")])`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = Term::Tuple(vec![Term::static_atom("undef"), Term::nil()]);
    /// assert_eq!(format!("{term:?}"), "Tuple([Atom(\"undef\"), List([])])");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        enum Item<'a> {
            Term(&'a Term),
            Text(&'static str),
        }
        let mut items = vec![Item::Term(self)];
        while let Some(item) = items.pop() {
            let term = match item {
                Item::Text(text) => {
                    formatter.write_str(text)?;
                    continue;
                }
                Item::Term(term) => term,
            };
            match term {
                Term::Atom(atom) => write!(formatter, "Atom({:?})", atom.name())?,
                Term::Binary(bytes) => write!(formatter, "Binary({bytes:?})")?,
                Term::Integer(value) => write!(formatter, "Integer({value:?})")?,
                Term::Pid(pid) => write!(formatter, "Pid({pid:?})")?,
                Term::Port(port) => write!(formatter, "Port({port:?})")?,
                Term::Reference(reference) => write!(formatter, "Reference({reference:?})")?,
                Term::MatchContext(context) => {
                    write!(formatter, "MatchContext({:?})", context.borrow())?
                }
                Term::Nil | Term::Cons(_) => {
                    let (elements, tail) = term.list_cells().expect("the term is a list");
                    let elements: Vec<_> = elements.collect();
                    match tail {
                        None => {
                            formatter.write_str("List([")?;
                            items.push(Item::Text("])"));
                        }
                        Some(tail) => {
                            formatter.write_str("ImproperList([")?;
                            items.push(Item::Text(")"));
                            items.push(Item::Term(tail));
                            items.push(Item::Text("], "));
                        }
                    }
                    for (index, element) in elements.into_iter().enumerate().rev() {
                        items.push(Item::Term(element));
                        if index > 0 {
                            items.push(Item::Text(", "));
                        }
                    }
                }
                Term::Tuple(elements) | Term::Fun(Fun { free: elements, .. }) => {
                    match term {
                        Term::Fun(fun) => write!(formatter, "Fun({:?}, [", fun.target)?,
                        _ => formatter.write_str("Tuple([")?,
                    }
                    items.push(Item::Text("])"));
                    for (index, element) in elements.iter().enumerate().rev() {
                        items.push(Item::Term(element));
                        if index > 0 {
                            items.push(Item::Text(", "));
                        }
                    }
                }
                Term::Map(map) => {
                    formatter.write_str("Map([")?;
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
#[path = "../tests/term.rs"]
mod tests;
