//! The bytes of the live term containers of one measured run.
//!
//! `LiveBytes` walks the guest terms that the runtime holds and adds the
//! bytes of each container once. A container is a heap array or object that
//! a term owns: the element array of a tuple, a cons cell, the form and the
//! nodes of a map, the backing of a binary, the thing of a fun, and the
//! state of a match context. Two terms that share one container share one
//! `Arc`; the walk visits the `Arc` once, so the number counts each
//! allocation once, as the counting allocator of
//! `docs/term-layout-design.md` section 5 counts each allocation once.
//!
//! The walk holds an explicit work stack of the terms that it must visit,
//! as the drop of a term and the release work list do, so a long list does
//! not recurse on the Rust stack. A container that one owner only holds
//! cannot be shared, so the walk counts it with no lookup in the list of
//! visited containers; a container with more than one owner goes into that
//! list.
//!
//! The walk is safe Rust. It identifies a shared container with
//! `Arc::ptr_eq` and a list of the containers that it already visited. It
//! never dereferences a raw pointer and never uses `unsafe`.

use std::{
    mem::{size_of, size_of_val},
    sync::Arc,
};

use num_bigint::BigInt;

use super::{
    Binary, Fun, Map, Record, Term,
    fun::FunThing,
    list::{Cons, ConsRef},
    map::{Child, Node, Repr},
    match_context::MatchState,
    record::{RecordDef, RecordThing},
    tuple::Tuple,
};

/// The bytes of the term containers that the walk has visited.
pub(crate) struct LiveBytes {
    /// The containers that the walk has already counted.
    seen: Vec<Shared>,
    /// The bytes of the containers that the walk counted.
    bytes: usize,
    /// The terms that the walk must visit, with the flag that one owner only
    /// holds the container of the term.
    pending: Vec<(Term, bool)>,
}

/// One shared container of a term, by its `Arc`.
enum Shared {
    Tuple(Arc<Vec<Term>>),
    Cons(Arc<Cons>),
    Map(Arc<Repr>),
    Node(Arc<Node>),
    /// One binary view descriptor, the allocation behind a [`Binary`]
    /// handle. Two handles that share one descriptor count it once.
    BinaryView(Binary),
    Binary(Arc<Vec<u8>>),
    Fun(Arc<FunThing>),
    Record(Arc<RecordThing>),
    RecordDef(Arc<RecordDef>),
    Match(Arc<MatchState>),
    BigInt(Arc<BigInt>),
}

/// Return true when one owner only holds the container of `term`, so the
/// walk cannot reach it twice. A term that is not a shared container gives
/// false; the walk uses the flag of a container only.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(owned_once(&Term::list_from([Term::Integer(1)])));
/// assert!(!owned_once(&Term::Integer(1)));
/// ```
fn owned_once(term: &Term) -> bool {
    match term {
        Term::Tuple(tuple) => Arc::strong_count(tuple.arc()) == 1,
        Term::Cons(cell) => Arc::strong_count(cell.arc()) == 1,
        Term::Map(map) => Arc::strong_count(map.arc()) == 1,
        Term::Fun(fun) => Arc::strong_count(fun.arc()) == 1,
        Term::Record(record) => Arc::strong_count(record.arc()) == 1,
        Term::MatchContext(state) => Arc::strong_count(state) == 1,
        Term::BigInt(value) => Arc::strong_count(value) == 1,
        Term::Atom(_)
        | Term::Binary(_)
        | Term::Integer(_)
        | Term::Float(_)
        | Term::Nil
        | Term::Pid(_)
        | Term::Port(_)
        | Term::Reference(_) => false,
    }
}

impl Shared {
    /// Return true when the two shared containers are the same allocation.
    fn same(&self, other: &Shared) -> bool {
        match (self, other) {
            (Shared::Tuple(left), Shared::Tuple(right)) => Arc::ptr_eq(left, right),
            (Shared::Cons(left), Shared::Cons(right)) => Arc::ptr_eq(left, right),
            (Shared::Map(left), Shared::Map(right)) => Arc::ptr_eq(left, right),
            (Shared::Node(left), Shared::Node(right)) => Arc::ptr_eq(left, right),
            (Shared::BinaryView(left), Shared::BinaryView(right)) => left.same_view(right),
            (Shared::Binary(left), Shared::Binary(right)) => Arc::ptr_eq(left, right),
            (Shared::Fun(left), Shared::Fun(right)) => Arc::ptr_eq(left, right),
            (Shared::Record(left), Shared::Record(right)) => Arc::ptr_eq(left, right),
            (Shared::RecordDef(left), Shared::RecordDef(right)) => Arc::ptr_eq(left, right),
            (Shared::Match(left), Shared::Match(right)) => Arc::ptr_eq(left, right),
            (Shared::BigInt(left), Shared::BigInt(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl LiveBytes {
    /// Return a walk with no counted bytes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(LiveBytes::new().bytes(), 0);
    /// ```
    pub(crate) fn new() -> Self {
        Self {
            seen: Vec::new(),
            bytes: 0,
            pending: Vec::new(),
        }
    }

    /// Return the bytes of the containers that the walk counted.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut live = LiveBytes::new();
    /// live.add(40);
    /// assert_eq!(live.bytes(), 40);
    /// ```
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Add the bytes of a container that the walk counts by itself, such as
    /// the frame array of a process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut live = LiveBytes::new();
    /// live.add(8);
    /// assert_eq!(live.bytes(), 8);
    /// ```
    pub(crate) fn add(&mut self, bytes: usize) {
        self.bytes += bytes;
    }

    /// Add the bytes of one `Arc`-shared container when the walk has not
    /// visited it, and return true when the caller must walk its contents.
    fn fresh(&mut self, shared: Shared) -> bool {
        if self.seen.iter().any(|seen| seen.same(&shared)) {
            return false;
        }
        self.seen.push(shared);
        true
    }

    /// Push `term` onto the work stack of the walk.
    fn push(&mut self, term: &Term) {
        let unique = owned_once(term);
        self.pending.push((term.clone(), unique));
    }

    /// Count the container of `term` and walk the terms that it holds. The
    /// terms of a long list go through the work stack, so the call does not
    /// recurse on the Rust stack.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut live = LiveBytes::new();
    /// live.term(&Term::list_from([Term::Integer(1)]));
    /// assert!(live.bytes() > 0);
    /// ```
    pub(crate) fn term(&mut self, term: &Term) {
        self.push(term);
        self.walk();
    }

    /// Walk the terms that wait on the work stack of the walk. A caller that
    /// pushes terms and reads [`Self::bytes`] with no later [`Self::term`]
    /// call, such as the walk of a map drain that is the last item of a
    /// release, calls this before it reads the number.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut live = LiveBytes::new();
    /// live.flush();
    /// assert_eq!(live.bytes(), 0);
    /// ```
    pub(crate) fn flush(&mut self) {
        self.walk();
    }

    /// Count the containers of the terms that wait on the work stack, and
    /// walk the terms that they hold. The terms of a long list go through the
    /// work stack, so the loop does not recurse on the Rust stack.
    fn walk(&mut self) {
        while let Some((term, unique)) = self.pending.pop() {
            match &term {
                Term::Tuple(tuple) => self.tuple(tuple, unique),
                Term::Cons(cell) => self.cons(cell, unique),
                Term::Map(map) => self.map(map, unique),
                Term::Binary(binary) => self.binary(binary),
                Term::Fun(fun) => self.fun(fun, unique),
                Term::Record(record) => self.record(record, unique),
                Term::MatchContext(state) => self.match_context(state, unique),
                Term::BigInt(value) => self.bigint(value, unique),
                Term::Atom(_)
                | Term::Integer(_)
                | Term::Float(_)
                | Term::Nil
                | Term::Pid(_)
                | Term::Port(_)
                | Term::Reference(_) => {}
            }
        }
    }

    /// Count the element array of `tuple` and walk its elements.
    fn tuple(&mut self, tuple: &Tuple, unique: bool) {
        if !unique && !self.fresh(Shared::Tuple(Arc::clone(tuple.arc()))) {
            return;
        }
        self.bytes += tuple.allocated_bytes();
        for element in tuple.arc().iter() {
            self.push(element);
        }
    }

    /// Count the cell of `cell` and walk its head and tail.
    fn cons(&mut self, cell: &ConsRef, unique: bool) {
        if !unique && !self.fresh(Shared::Cons(Arc::clone(cell.arc()))) {
            return;
        }
        self.bytes += cell.allocated_bytes();
        self.push(&cell.head);
        self.push(&cell.tail);
    }

    /// Count the form of `map` and walk its pairs and nodes.
    fn map(&mut self, map: &Map, unique: bool) {
        let repr = map.arc();
        if !unique && !self.fresh(Shared::Map(Arc::clone(repr))) {
            return;
        }
        self.bytes += map.allocated_bytes();
        match &**repr {
            Repr::Flat(pairs) => self.pairs(pairs),
            Repr::Hash(hamt) => self.children_of(&hamt.root.children),
        }
    }

    /// Push the keys and the values of an array of map pairs.
    fn pairs(&mut self, pairs: &[(Term, Term)]) {
        for (key, value) in pairs {
            self.push(key);
            self.push(value);
        }
    }

    /// Count the child nodes of `children` and push the pairs of each leaf.
    /// The caller counts the capacity of the array of `children` itself. The
    /// depth of a HAMT is at most `HAMT_MAX_LEVEL`, so this walk does not
    /// recurse on the Rust stack for a long list.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// live.children_of(&Node::empty().children);
    /// ```
    pub(in crate::term) fn children_of(&mut self, children: &[Child]) {
        for child in children {
            match child {
                Child::Leaf(key, value) => {
                    self.push(key);
                    self.push(value);
                }
                Child::Node(node) => {
                    let unique = Arc::strong_count(node) == 1;
                    if !unique && !self.fresh(Shared::Node(Arc::clone(node))) {
                        continue;
                    }
                    self.bytes += node.allocated_bytes();
                    self.children_of(&node.children);
                }
                Child::Collision(pairs) => {
                    self.bytes += pairs.capacity() * size_of::<(Term, Term)>();
                    self.pairs(pairs);
                }
            }
        }
    }

    /// Count the view descriptor of `binary`, its backing array, and the
    /// packed array of a view. The first visit of a descriptor counts the
    /// bytes behind the handle through [`Binary::view_bytes`]. The first
    /// visit of a backing array counts both arrays of the binary through
    /// [`Binary::allocated_bytes`] and marks each array as visited. A later
    /// visit of the same backing array counts a packed array that the walk
    /// has not visited yet. A binary holds no term, so the walk does not
    /// recurse here.
    fn binary(&mut self, binary: &Binary) {
        if self.fresh(Shared::BinaryView(binary.clone())) {
            self.bytes += binary.view_bytes();
        }
        if self.fresh(Shared::Binary(Arc::clone(binary.backing()))) {
            self.bytes += binary.allocated_bytes();
            if let Some(packed) = binary.packed_backing() {
                let _ = self.fresh(Shared::Binary(Arc::clone(packed)));
            }
            return;
        }
        if let Some(packed) = binary.packed_backing()
            && self.fresh(Shared::Binary(Arc::clone(packed)))
        {
            self.bytes += packed.capacity();
        }
    }

    /// Count the thing of `fun` and walk its free variables.
    fn fun(&mut self, fun: &Fun, unique: bool) {
        if !unique && !self.fresh(Shared::Fun(Arc::clone(fun.arc()))) {
            return;
        }
        self.bytes += fun.allocated_bytes();
        for free in fun.free() {
            self.push(free);
        }
    }

    /// Count the thing of `record` and walk its values. The walk counts a
    /// transient definition once and its tuple; module code holds each
    /// other definition, so the walk does not count it.
    fn record(&mut self, record: &Record, unique: bool) {
        if !unique && !self.fresh(Shared::Record(Arc::clone(record.arc()))) {
            return;
        }
        self.bytes += size_of::<RecordThing>() + size_of_val(record.values());
        for value in record.values() {
            self.push(value);
        }
        let def = record.def();
        if def.transient && self.fresh(Shared::RecordDef(Arc::clone(def))) {
            self.bytes += size_of::<RecordDef>()
                + size_of_val(&*def.keys)
                + size_of_val(&*def.order)
                + size_of_val(&*def.defaults);
            self.push(&def.tuple);
        }
    }

    /// Count the state of `state` and walk its binary.
    fn match_context(&mut self, state: &Arc<MatchState>, unique: bool) {
        if !unique && !self.fresh(Shared::Match(Arc::clone(state))) {
            return;
        }
        self.bytes += size_of::<MatchState>();
        self.binary(state.binary());
    }

    /// Count the value of the bignum `value` and its digit array. Two terms
    /// that share one `Arc` count it once. A bignum holds no term, so the walk
    /// does not recurse here.
    fn bigint(&mut self, value: &Arc<BigInt>, unique: bool) {
        if !unique && !self.fresh(Shared::BigInt(Arc::clone(value))) {
            return;
        }
        self.bytes += size_of::<BigInt>() + value.iter_u64_digits().len() * size_of::<u64>();
    }
}

#[cfg(test)]
#[path = "../../tests/term/live_bytes.rs"]
mod tests;
