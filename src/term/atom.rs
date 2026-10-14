//! Atoms and the atom table, as in ERTS OTP 29.1.1 (`atom.c`, `index.c`).
//!
//! ERTS gives each atom an index in one atom table for each emulator. The
//! table starts with the 1150 static atoms (`init_atom_table`). A loaded
//! module puts its `AtU8` atoms and then its `LitT` atoms. An atom that
//! exists keeps its index. A new atom gets the next index. ERTS never frees
//! or reuses an index. The atom word is `(index << 6) | 0xB`, so a flatmap
//! orders atom keys by index, and a hashmap hashes an atom key by its index.
//!
//! An ERTS atom term is one tagged word that holds the table index
//! (`make_atom` and `_unchecked_atom_val` in `erl_term.h:300`, `:303`), and
//! the name stays in the atom table (`erts_atom_table` in `atom.c:38`). A
//! copy of an atom therefore copies one word and allocates nothing. PBeam
//! follows that rule: an [`Atom`] holds only the index, and the
//! [`AtomTable`] that owns the index holds the name.

mod static_names;

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

use static_names::STATIC_ATOM_NAMES;

use crate::{
    error::{BeamError, error},
    term::{Fun, Map, Target, Term},
};

/// The largest number of atoms in the atom table (`ATOM_LIMIT` in `atom.h`,
/// the default of the `+t` emulator flag).
const ATOM_LIMIT: usize = 1 << 20;

/// The largest number of characters in an atom name (`MAX_ATOM_CHARACTERS`
/// in `atom.h`).
pub(crate) const MAX_ATOM_CHARACTERS: usize = 255;

/// The index of each static atom, made once for the process.
static STATIC_INDICES: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(|| {
    (0..)
        .zip(STATIC_ATOM_NAMES.iter().copied())
        .map(|(index, name)| (name, index))
        .collect()
});

/// An atom term: its index in the atom table of one program.
///
/// The atom holds no name. Read a name through the [`AtomTable`] that owns
/// the index, with [`AtomTable::name_of`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    /// The ERTS atom-table index. The atom word is `(index << 6) | 0xB`.
    index: u32,
}

impl Atom {
    /// Return the atom-table index of the atom.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Atom::from_static("ok").unwrap().index(), 506);
    /// ```
    pub(crate) fn index(&self) -> u32 {
        self.index
    }

    /// Return a static atom without a table. A static atom has the same
    /// index in every atom table. Return `None` for a name that is not
    /// static.
    ///
    /// # Examples
    ///
    /// ```
    /// use pbeam_runtime::Atom;
    ///
    /// assert!(Atom::from_static("ok").is_some());
    /// assert!(Atom::from_static("pbeam_zeta").is_none());
    /// ```
    pub fn from_static(name: &str) -> Option<Atom> {
        STATIC_INDICES.get(name).map(|&index| Atom { index })
    }

    /// Return the index of a static atom name, with no atom table.
    ///
    /// [`Atom::from_static`] gives an atom with this index. This function
    /// returns the index alone, so a comparison allocates nothing. A static
    /// atom has the same index in every atom table, so the index is enough
    /// to compare a name without the table.
    ///
    /// # Panics
    ///
    /// Panics when `name` is not a static atom name. Each caller compares a
    /// name that the pinned ERTS holds as a static atom.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Atom::static_index("ok"), Atom::from_static("ok").unwrap().index());
    /// ```
    fn static_index(name: &str) -> u32 {
        *STATIC_INDICES.get(name).expect("the name is a static atom")
    }

    /// Return true when this atom is the static atom `name`.
    ///
    /// `name` must be a static atom name, as [`Atom::static_index`] requires.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let ok = Atom::from_static("ok").unwrap();
    /// assert!(ok.is_static("ok"));
    /// assert!(!ok.is_static("error"));
    /// ```
    pub(crate) fn is_static(&self, name: &str) -> bool {
        self.index == Atom::static_index(name)
    }

    /// Return true when this atom is one of the static atom names `names`.
    ///
    /// Each name must be a static atom name, as [`Atom::is_static`] requires.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let ok = Atom::from_static("ok").unwrap();
    /// assert!(ok.is_one_of(&["ok", "error"]));
    /// assert!(!ok.is_one_of(&["error"]));
    /// ```
    pub(crate) fn is_one_of(&self, names: &[&str]) -> bool {
        names.iter().any(|name| self.is_static(name))
    }
}

/// The identity of the program that owns an [`AtomTable`]. Two tables have
/// the same identity only when one is a copy of the other, and a copy keeps
/// the identity of its source. The import between two tables compares the
/// identity of the two programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProgramId(u64);

impl ProgramId {
    /// Return the identity of a new program.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_ne!(ProgramId::next(), ProgramId::next());
    /// ```
    fn next() -> Self {
        /// The number of programs that this process made.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        ProgramId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// The atom table of one program or one run (`erts_atom_table` in ERTS).
///
/// A caller receives the table of a program from [`BeamProgram::atoms`] and
/// passes it to [`Map::from_pairs`], so a map that the caller builds orders
/// its keys as the program does. A table of the library holds every atom of
/// one program; the host cannot add an atom to it.
///
/// [`BeamProgram::atoms`]: crate::BeamProgram::atoms
/// [`Map::from_pairs`]: crate::Map::from_pairs
#[derive(Debug, Clone)]
pub struct AtomTable {
    /// The index of each atom, keyed by its name.
    indices: HashMap<Arc<str>, u32>,
    /// The name of each atom, by its index. The next new atom gets index
    /// `names.len()`.
    names: Vec<Arc<str>>,
    /// The program that owns the table. A copy keeps the identity, and the
    /// import compares it with the identity of the destination table.
    program: ProgramId,
}

impl AtomTable {
    /// Return a table with the 1150 static atoms at indices 0 to 1149, as
    /// `init_atom_table` makes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = AtomTable::new();
    /// assert_eq!(table.get("false"), Atom::from_static("false"));
    /// ```
    pub fn new() -> Self {
        let mut indices = HashMap::with_capacity(STATIC_ATOM_NAMES.len());
        let mut names = Vec::with_capacity(STATIC_ATOM_NAMES.len());
        for (index, &name) in STATIC_ATOM_NAMES.iter().enumerate() {
            let name: Arc<str> = Arc::from(name);
            indices.insert(
                Arc::clone(&name),
                u32::try_from(index).expect("STATIC_ATOM_NAMES fits in u32"),
            );
            names.push(name);
        }
        AtomTable {
            indices,
            names,
            program: ProgramId::next(),
        }
    }

    /// Return the atom of a name. Create the atom with the next index when
    /// it is new, as `erts_atom_put_index` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = AtomTable::new();
    /// let zeta = table.put("pbeam_zeta")?;
    /// assert_eq!(zeta.index, 1150);
    /// assert_eq!(table.put("pbeam_zeta")?, zeta);
    /// assert_eq!(table.put("ok")?.index, 506);
    /// ```
    ///
    /// # Errors
    ///
    /// A new atom when the table holds 1048576 atoms is the error
    /// `no more index entries in atom_tab (max=1048576)`. ERTS exits the
    /// emulator with this message (`index_put_entry`).
    pub(crate) fn put(&mut self, name: &str) -> Result<Atom, BeamError> {
        if let Some(atom) = self.get(name) {
            return Ok(atom);
        }
        let count = self.names.len();
        if count >= ATOM_LIMIT {
            return Err(error(format!(
                "no more index entries in atom_tab (max={ATOM_LIMIT})"
            )));
        }
        let index = u32::try_from(count).expect("ATOM_LIMIT fits in u32");
        let shared: Arc<str> = Arc::from(name);
        self.indices.insert(Arc::clone(&shared), index);
        self.names.push(shared);
        Ok(Atom { index })
    }

    /// Return the atom of a name only when it exists.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = AtomTable::new();
    /// assert_eq!(table.get("get_definition").unwrap().index, 1149);
    /// assert_eq!(table.get("pbeam_zeta"), None);
    /// ```
    pub(crate) fn get(&self, name: &str) -> Option<Atom> {
        self.indices.get(name).map(|&index| Atom { index })
    }

    /// Return the name of the atom at `index`, or `None` when the table
    /// holds no atom at that index. The import of a host term uses it to
    /// resolve the index of an atom of the source table. The index is all
    /// that an atom term holds, so an index outside the table is not a
    /// value of the table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = AtomTable::new();
    /// assert_eq!(table.name(506), Some("ok"));
    /// assert_eq!(table.name(1048576), None);
    /// ```
    pub(crate) fn name(&self, index: u32) -> Option<&str> {
        self.names
            .get(usize::try_from(index).ok()?)
            .map(Arc::as_ref)
    }

    /// Return the name of `atom` from this table. Read it with
    /// `AtomTable::name` at the index of the atom.
    ///
    /// An atom term holds an index of the table that owns it, so an index
    /// that this table does not hold is a broken invariant and not a value.
    /// A caller that holds a term of another table imports it first.
    ///
    /// # Panics
    ///
    /// This function panics when this table does not hold the index of
    /// `atom`. The message names the index.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = AtomTable::new();
    /// let ok = table.get("ok").unwrap();
    /// assert_eq!(table.name_of(&ok), "ok");
    /// ```
    pub fn name_of(&self, atom: &Atom) -> &str {
        let index = atom.index();
        self.name(index)
            .unwrap_or_else(|| panic!("atom index {index} is outside the atom table"))
    }

    /// Return the term of `source` with every atom of its source table
    /// replaced by the atom of the same name in this table. The walk reads
    /// each source atom through [`AtomTable::name`], so it also checks that
    /// each index is a value of the source table. A `Term::Atom` of another
    /// table enters this table only through this function.
    ///
    /// The walk remaps the `module` of a `Lambda` and the `module` and
    /// `function` of a `Target::External`, because their atoms are outside
    /// the free variables of the fun. It builds a new term and never
    /// changes `source` or the term. It remembers each compound by the
    /// identity of its storage, so a term that holds one subtree more than
    /// once keeps that sharing in the imported term and the walk visits
    /// each compound one time.
    ///
    /// The memory covers the walk. It does not cover the rebuild of a map:
    /// the `Map::from_pairs` call validates and hashes each key, and that
    /// pass visits a key with a shared subtree again for each occurrence. A
    /// map whose key is built as `T{n} = {T{n-1}, T{n-1}}` has storage in
    /// `n` and costs time that grows exponentially in `n`, so this function
    /// does not promise linear time for every term.
    ///
    /// # Errors
    ///
    /// * An atom index that the source table does not hold gives the error
    ///   `atom index {index} is outside the atom table of the host term`.
    /// * A `Target::Local` whose source program is not the program of this
    ///   table gives `local fun of module {module} belongs to another
    ///   program`, because a local fun holds the entry label of its
    ///   function and this table can name a module that holds different
    ///   code.
    /// * A destination table at `ATOM_LIMIT` gives the error of
    ///   [`AtomTable::put`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let source = AtomTable::new();
    /// let mut destination = source.clone();
    /// let term = Term::Atom(source.get("ok").unwrap());
    /// assert_eq!(destination.import_term(&source, &term)?, Term::Atom(destination.get("ok").unwrap()));
    /// ```
    pub(crate) fn import_term(
        &mut self,
        source: &AtomTable,
        term: &Term,
    ) -> Result<Term, BeamError> {
        let mut work = vec![ImportStep::Term(term)];
        // The imported terms, in the order that the walk finishes them. A
        // node takes the results of its children from the end.
        let mut ready: Vec<Term> = Vec::new();
        // The imported term of each shared compound, keyed by the identity
        // of its storage. The same source compound is imported one time, so
        // a term that holds one subtree twice keeps that sharing and the
        // walk takes O(size) time.
        let mut imported: HashMap<usize, Term> = HashMap::new();
        while let Some(step) = work.pop() {
            match step {
                ImportStep::Term(term) => match term {
                    Term::Atom(atom) => ready.push(Term::Atom(self.import_atom(source, atom)?)),
                    Term::Tuple(elements) => {
                        let key = elements.identity();
                        if let Some(term) = imported.get(&key) {
                            ready.push(term.clone());
                        } else {
                            work.push(ImportStep::Tuple {
                                key,
                                len: elements.len(),
                            });
                            work.extend(elements.iter().rev().map(ImportStep::Term));
                        }
                    }
                    Term::Cons(cell) => {
                        let key = cell.identity();
                        if let Some(term) = imported.get(&key) {
                            ready.push(term.clone());
                        } else {
                            work.push(ImportStep::Cons { key });
                            work.push(ImportStep::Term(&cell.tail));
                            work.push(ImportStep::Term(&cell.head));
                        }
                    }
                    Term::Map(map) => {
                        let key = map.identity();
                        if let Some(term) = imported.get(&key) {
                            ready.push(term.clone());
                        } else {
                            work.push(ImportStep::Map {
                                key,
                                len: map.len(),
                            });
                            for (key, value) in map.iter().collect::<Vec<_>>().into_iter().rev() {
                                work.push(ImportStep::Term(value));
                                work.push(ImportStep::Term(key));
                            }
                        }
                    }
                    Term::Fun(fun) => {
                        let key = fun.identity();
                        if let Some(term) = imported.get(&key) {
                            ready.push(term.clone());
                        } else {
                            match fun.target() {
                                Target::External {
                                    module,
                                    function,
                                    arity,
                                } => {
                                    let module = self.import_atom(source, module)?;
                                    let function = self.import_atom(source, function)?;
                                    work.push(ImportStep::Fun {
                                        key,
                                        module,
                                        function,
                                        arity: *arity,
                                    });
                                }
                                Target::Local(lambda) => {
                                    let module = lambda.module();
                                    let name = source
                                        .name(module.index())
                                        .ok_or_else(|| outside_atom_table(module.index()))?;
                                    if source.program != self.program {
                                        return Err(error(format!(
                                            "local fun of module {name} belongs to another program"
                                        )));
                                    }
                                    // A local fun holds the entry label of its
                                    // function, and the code of its module is
                                    // not part of the host term. A run can load
                                    // different code under one module name on
                                    // demand, and the destination run can load
                                    // such code after this import, so the
                                    // import cannot check that the label names
                                    // the same function of the destination.
                                    return Err(error(format!(
                                        "local fun of module {name} has no verified code in this program"
                                    )));
                                }
                            }
                        }
                    }
                    Term::Binary(_)
                    | Term::Integer(_)
                    | Term::Nil
                    | Term::Pid(_)
                    | Term::Port(_)
                    | Term::Reference(_)
                    | Term::MatchContext(_) => ready.push(term.clone()),
                },
                ImportStep::Tuple { key, len } => {
                    let elements = ready.split_off(ready.len() - len);
                    let term = Term::tuple(elements);
                    imported.insert(key, term.clone());
                    ready.push(term);
                }
                ImportStep::Cons { key } => {
                    let tail = ready.pop().expect("a cons cell has a tail result");
                    let head = ready.pop().expect("a cons cell has a head result");
                    let term = Term::cons(head, tail);
                    imported.insert(key, term.clone());
                    ready.push(term);
                }
                ImportStep::Fun {
                    key,
                    module,
                    function,
                    arity,
                } => {
                    // An external fun holds no free variable.
                    let term = Term::Fun(Fun::external(module, function, arity));
                    imported.insert(key, term.clone());
                    ready.push(term);
                }
                ImportStep::Map { key, len } => {
                    let flat = ready.split_off(ready.len() - 2 * len);
                    let mut pairs = Vec::with_capacity(len);
                    let mut values = flat.into_iter();
                    while let (Some(pair_key), Some(pair_value)) = (values.next(), values.next()) {
                        pairs.push((pair_key, pair_value));
                    }
                    // The source map is canonical, so its keys are valid
                    // keys, and a key keeps its shape across the import.
                    let map = Map::from_pairs(pairs, self)
                        .expect("a canonical map of one table imports to a canonical map");
                    let term = Term::Map(map);
                    imported.insert(key, term.clone());
                    ready.push(term);
                }
            }
        }
        Ok(ready
            .pop()
            .expect("the import leaves the result of the root"))
    }

    /// Return the atom of `source` that the atom `atom` names, in this
    /// table.
    ///
    /// # Errors
    ///
    /// An index that `source` does not hold gives the outside-table error,
    /// and a full destination table gives the error of [`AtomTable::put`].
    fn import_atom(&mut self, source: &AtomTable, atom: &Atom) -> Result<Atom, BeamError> {
        let name = source
            .name(atom.index())
            .ok_or_else(|| outside_atom_table(atom.index()))?;
        self.put(name)
    }
}

impl Default for AtomTable {
    /// Return the table of [`AtomTable::new`], which holds the static atoms.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(AtomTable::default().get("ok"), AtomTable::new().get("ok"));
    /// ```
    fn default() -> Self {
        Self::new()
    }
}

/// One step of the import walk of [`AtomTable::import_term`]: import a term,
/// or build a compound from the results of its children. Each build carries
/// the identity of the source compound, so the walk remembers the imported
/// term of a compound that it meets again.
enum ImportStep<'a> {
    /// Import this term and push its result.
    Term(&'a Term),
    /// Make a tuple of the last `len` results, and remember it at `key`.
    Tuple { key: usize, len: usize },
    /// Make a cons cell of the last two results, and remember it at `key`.
    Cons { key: usize },
    /// Make the external fun of this module, function, and arity, and
    /// remember it at `key`.
    Fun {
        key: usize,
        module: Atom,
        function: Atom,
        arity: u32,
    },
    /// Make a map of the last `len` pairs of results, and remember it at
    /// `key`.
    Map { key: usize, len: usize },
}

/// Return the error of an atom index that the source table does not hold.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(outside_atom_table(1048576).to_string(), "atom index 1048576 is outside the atom table of the host term");
/// ```
fn outside_atom_table(index: u32) -> BeamError {
    error(format!(
        "atom index {index} is outside the atom table of the host term"
    ))
}

/// A term of one program together with the atom table that owns each of its
/// atoms. A host result and a host argument are `HostTerm` values. A copy is
/// cheap: the term copy is O(1), and the table is shared through an `Arc`.
/// The `term` field is the value, and `HostTerm::atom` reads a name of its
/// table.
#[derive(Clone)]
pub struct HostTerm {
    /// The term.
    pub term: Term,
    /// The atom table that owns each atom of the term, at any depth.
    pub(crate) atoms: Arc<AtomTable>,
}

impl fmt::Debug for HostTerm {
    /// Write the term with the `Debug` text of [`Term`]. A run result keeps
    /// the text that it had when the entry points returned a bare term, and
    /// the atom table of the host term is not printed.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(format!("{:?}", host_term), format!("{:?}", host_term.term));
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.term.fmt(formatter)
    }
}

impl PartialEq for HostTerm {
    /// Compare the two host terms by their semantic identity. The atom
    /// table is storage metadata, and the same name holds a different index
    /// in another program, so the comparison imports the term of `other`
    /// into a copy of the table of `self` and compares the two terms of one
    /// table. A term that the copy cannot import is not equal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(program.host_term(Term::static_atom("ok")), program.host_term(Term::static_atom("ok")));
    /// ```
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.atoms, &other.atoms) {
            return self.term.eq_shared(&other.term);
        }
        let mut table = (*self.atoms).clone();
        match table.import_term(&other.atoms, &other.term) {
            Ok(imported) => self.term.eq_shared(&imported),
            Err(_) => false,
        }
    }
}

impl Eq for HostTerm {}

impl HostTerm {
    /// Return the atom of `name` in the atom table of this host term, or
    /// `None` when the table does not hold the name. A run adds each name
    /// that it creates to the table of its result, so this function reads
    /// that name after the run.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(host_term.atom("ok").is_some());
    /// assert_eq!(host_term.atom("pbeam_unknown"), None);
    /// ```
    pub fn atom(&self, name: &str) -> Option<Atom> {
        self.atoms.get(name)
    }

    /// Return the atom table that owns the atoms of this host term, which is
    /// the table of the run that made the term. A run adds each name that it
    /// creates to that table, for example with `erlang:list_to_atom/1`, and
    /// the program table holds none of those names. A host that builds a map
    /// from an atom of a run passes this table to [`Map::from_pairs`],
    /// because the order rule reads each name through the owning table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = Map::from_pairs(vec![(key, value)], host_term.atoms())?;
    /// ```
    pub fn atoms(&self) -> &AtomTable {
        &self.atoms
    }

    /// Return this term with every atom of its table replaced by the atom
    /// of the same name in `destination`, as
    /// [`AtomTable::import_term`] describes.
    ///
    /// Use this function to pass a host term of one program to another
    /// program. A term of the program that owns `destination` needs no
    /// import.
    ///
    /// # Errors
    ///
    /// The errors of [`AtomTable::import_term`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let imported = host_term.import_into(&mut scheduler.atoms)?;
    /// ```
    pub(crate) fn import_into(&self, destination: &mut AtomTable) -> Result<Term, BeamError> {
        destination.import_term(&self.atoms, &self.term)
    }
}

#[cfg(test)]
thread_local! {
    /// The atom table of the unit tests in one test thread.
    static TEST_ATOMS: std::cell::RefCell<AtomTable> = std::cell::RefCell::new(AtomTable::new());
}

/// Return the atom term of a name for a unit test. A static name gets its
/// static index. A new name gets the next index in the table of the test
/// thread. libtest reuses a thread for many tests, so another test can make
/// the same name first: use this only where atom order does not matter, and
/// use a fresh `AtomTable` in a test that checks atom order.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(test_atom("ok"), Term::static_atom("ok"));
/// assert_ne!(test_atom("a"), test_atom("b"));
/// ```
#[cfg(test)]
pub(crate) fn test_atom(name: &str) -> Term {
    let atom = TEST_ATOMS.with(|table| table.borrow_mut().put(name));
    Term::Atom(atom.expect("a test makes fewer atoms than ATOM_LIMIT"))
}

/// Return a copy of the atom table of the unit tests in this thread. A test
/// that builds a program by hand uses it as the table of the run, so the
/// atoms of [`test_atom`] keep their index in the run table.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = test_atom_table();
/// assert_eq!(atoms.name(test_atom("ok").index()), Some("ok"));
/// ```
#[cfg(test)]
pub(crate) fn test_atom_table() -> AtomTable {
    TEST_ATOMS.with(|table| table.borrow().clone())
}

#[cfg(test)]
#[path = "../../tests/term/atom.rs"]
mod tests;
