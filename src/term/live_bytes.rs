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
//! The walk is safe Rust. It identifies a shared container with
//! `Arc::ptr_eq` and a list of the containers that it already visited. It
//! never dereferences a raw pointer and never uses `unsafe`.

use std::{mem::size_of, sync::Arc};

use super::{
    Binary, Fun, Map, Term,
    fun::FunThing,
    list::{Cons, ConsRef},
    map::{Child, Node, Repr},
    match_context::MatchState,
    tuple::Tuple,
};

/// The bytes of the term containers that the walk has visited.
pub(crate) struct LiveBytes {
    /// The containers that the walk has already counted.
    seen: Vec<Shared>,
    /// The bytes of the containers that the walk counted.
    bytes: usize,
}

/// One shared container of a term, by its `Arc`.
enum Shared {
    Tuple(Arc<Vec<Term>>),
    Cons(Arc<Cons>),
    Map(Arc<Repr>),
    Node(Arc<Node>),
    Binary(Arc<Vec<u8>>),
    Fun(Arc<FunThing>),
    Match(Arc<MatchState>),
}

impl Shared {
    /// Return true when the two shared containers are the same allocation.
    fn same(&self, other: &Shared) -> bool {
        match (self, other) {
            (Shared::Tuple(left), Shared::Tuple(right)) => Arc::ptr_eq(left, right),
            (Shared::Cons(left), Shared::Cons(right)) => Arc::ptr_eq(left, right),
            (Shared::Map(left), Shared::Map(right)) => Arc::ptr_eq(left, right),
            (Shared::Node(left), Shared::Node(right)) => Arc::ptr_eq(left, right),
            (Shared::Binary(left), Shared::Binary(right)) => Arc::ptr_eq(left, right),
            (Shared::Fun(left), Shared::Fun(right)) => Arc::ptr_eq(left, right),
            (Shared::Match(left), Shared::Match(right)) => Arc::ptr_eq(left, right),
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

    /// Count the container of `term` and walk the terms that it holds.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut live = LiveBytes::new();
    /// live.term(&Term::list_from([Term::Integer(1)]));
    /// assert!(live.bytes() > 0);
    /// ```
    pub(crate) fn term(&mut self, term: &Term) {
        match term {
            Term::Tuple(tuple) => self.tuple(tuple),
            Term::Cons(cell) => self.cons(cell),
            Term::Map(map) => self.map(map),
            Term::Binary(binary) => self.binary(binary),
            Term::Fun(fun) => self.fun(fun),
            Term::MatchContext(state) => self.match_context(state),
            Term::Atom(_)
            | Term::Integer(_)
            | Term::Nil
            | Term::Pid(_)
            | Term::Port(_)
            | Term::Reference(_) => {}
        }
    }

    /// Count the element array of `tuple` and walk its elements.
    fn tuple(&mut self, tuple: &Tuple) {
        if !self.fresh(Shared::Tuple(Arc::clone(tuple.arc()))) {
            return;
        }
        self.bytes += tuple.allocated_bytes();
        for element in tuple.arc().iter() {
            self.term(element);
        }
    }

    /// Count the cell of `cell` and walk its head and tail.
    fn cons(&mut self, cell: &ConsRef) {
        if !self.fresh(Shared::Cons(Arc::clone(cell.arc()))) {
            return;
        }
        self.bytes += cell.allocated_bytes();
        self.term(&cell.head);
        self.term(&cell.tail);
    }

    /// Count the form of `map` and walk its pairs and nodes.
    fn map(&mut self, map: &Map) {
        let repr = map.arc();
        if !self.fresh(Shared::Map(Arc::clone(repr))) {
            return;
        }
        self.bytes += map.allocated_bytes();
        match &**repr {
            Repr::Flat(pairs) => self.pairs(pairs),
            Repr::Hash(hamt) => self.children(&hamt.root),
        }
    }

    /// Count an array of map pairs and walk the keys and the values.
    fn pairs(&mut self, pairs: &[(Term, Term)]) {
        for (key, value) in pairs {
            self.term(key);
            self.term(value);
        }
    }

    /// Count the child nodes of `node` and walk the pairs of each leaf.
    fn children(&mut self, node: &Node) {
        for child in &node.children {
            match child {
                Child::Leaf(key, value) => {
                    self.term(key);
                    self.term(value);
                }
                Child::Node(node) => {
                    if !self.fresh(Shared::Node(Arc::clone(node))) {
                        continue;
                    }
                    self.bytes += node.allocated_bytes();
                    self.children(node);
                }
                Child::Collision(pairs) => {
                    self.bytes += pairs.capacity() * size_of::<(Term, Term)>();
                    self.pairs(pairs);
                }
            }
        }
    }

    /// Count the backing array of `binary` and the packed array of a view.
    /// The first visit of a backing array counts both arrays of the binary
    /// through [`Binary::allocated_bytes`] and marks each array as visited.
    /// A later visit of the same backing array counts only a packed array
    /// that the walk has not visited yet.
    fn binary(&mut self, binary: &Binary) {
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
    fn fun(&mut self, fun: &Fun) {
        if !self.fresh(Shared::Fun(Arc::clone(fun.arc()))) {
            return;
        }
        self.bytes += fun.allocated_bytes();
        for free in fun.free() {
            self.term(free);
        }
    }

    /// Count the state of `state` and walk its binary.
    fn match_context(&mut self, state: &Arc<MatchState>) {
        if !self.fresh(Shared::Match(Arc::clone(state))) {
            return;
        }
        self.bytes += size_of::<MatchState>();
        self.binary(state.binary());
    }
}

#[cfg(test)]
#[path = "../../tests/term/live_bytes.rs"]
mod tests;
