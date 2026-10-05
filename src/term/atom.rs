//! Atoms and the atom table, as in ERTS OTP 29.1.1 (`atom.c`, `index.c`).
//!
//! ERTS gives each atom an index in one atom table for each emulator. The
//! table starts with the 1150 static atoms (`init_atom_table`). A loaded
//! module puts its `AtU8` atoms and then its `LitT` atoms. An atom that
//! exists keeps its index. A new atom gets the next index. ERTS never frees
//! or reuses an index. The atom word is `(index << 6) | 0xB`, so a flatmap
//! orders atom keys by index, and a hashmap hashes an atom key by its index.

mod static_names;

use std::{collections::HashMap, sync::LazyLock};

use static_names::STATIC_ATOM_NAMES;

use crate::error::{BeamError, error};
#[cfg(test)]
use crate::term::Term;

/// The largest number of atoms in the atom table (`ATOM_LIMIT` in `atom.h`,
/// the default of the `+t` emulator flag).
const ATOM_LIMIT: usize = 1 << 20;

/// The index of each static atom, made once for the process.
static STATIC_INDICES: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(|| {
    (0..)
        .zip(STATIC_ATOM_NAMES.iter().copied())
        .map(|(index, name)| (name, index))
        .collect()
});

/// An atom term: its index in the atom table of one program, and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    /// The ERTS atom-table index. The atom word is `(index << 6) | 0xB`.
    index: u32,
    /// The atom name. Name-based code, such as BIF dispatch and `Debug`,
    /// reads it without the table.
    name: String,
}

impl Atom {
    /// Return the name of the atom.
    ///
    /// # Examples
    ///
    /// ```
    /// use pbeam_runtime::Atom;
    ///
    /// assert_eq!(Atom::from_static("ok").unwrap().name(), "ok");
    /// ```
    pub fn name(&self) -> &str {
        &self.name
    }

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
        STATIC_INDICES.get(name).map(|&index| Atom {
            index,
            name: name.to_owned(),
        })
    }
}

/// The atom table of one program or one run (`erts_atom_table` in ERTS).
#[derive(Debug, Clone)]
pub(crate) struct AtomTable {
    /// The index of each atom. The next new atom gets index `indices.len()`.
    indices: HashMap<String, u32>,
}

impl Default for AtomTable {
    /// Return a table with only the static atoms, as `AtomTable::new` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(AtomTable::default().get("ok"), Atom::from_static("ok"));
    /// ```
    fn default() -> Self {
        AtomTable::new()
    }
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
    pub(crate) fn new() -> Self {
        let indices = STATIC_INDICES
            .iter()
            .map(|(&name, &index)| (name.to_owned(), index))
            .collect();
        AtomTable { indices }
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
        let count = self.indices.len();
        if count >= ATOM_LIMIT {
            return Err(error(format!(
                "no more index entries in atom_tab (max={ATOM_LIMIT})"
            )));
        }
        let index = u32::try_from(count).expect("ATOM_LIMIT fits in u32");
        self.indices.insert(name.to_owned(), index);
        Ok(Atom {
            index,
            name: name.to_owned(),
        })
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
        self.indices.get(name).map(|&index| Atom {
            index,
            name: name.to_owned(),
        })
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

#[cfg(test)]
mod tests {
    use super::{ATOM_LIMIT, Atom, AtomTable, STATIC_ATOM_NAMES};

    #[test]
    fn starts_with_the_erts_static_atoms() {
        let table = AtomTable::new();
        assert_eq!(table.indices.len(), 1150);
        // ERTS 29.1.1 crash dump: false is 0, ok is 506, get_definition is 1149.
        for (name, index) in [("false", 0), ("true", 1), ("", 5), ("ok", 506)] {
            assert_eq!(table.get(name).unwrap().index, index);
            assert_eq!(Atom::from_static(name), table.get(name));
        }
        assert_eq!(table.get("get_definition").unwrap().index, 1149);
        assert_eq!(
            table.get("get_definition").unwrap().name(),
            "get_definition"
        );
        assert_eq!(table.get("pbeam_zeta"), None);
        assert_eq!(Atom::from_static("pbeam_zeta"), None);
        assert_eq!(STATIC_ATOM_NAMES[1149], "get_definition");
    }

    #[test]
    fn gives_a_new_atom_the_next_index_and_keeps_existing_indices() {
        let mut table = AtomTable::new();
        let zeta = table.put("pbeam_zeta").unwrap();
        assert_eq!(zeta.index, 1150);
        assert_eq!(table.put("pbeam_alpha").unwrap().index, 1151);
        assert_eq!(table.put("pbeam_zeta").unwrap(), zeta);
        assert_eq!(table.get("pbeam_zeta"), Some(zeta));
        assert_eq!(table.put("ok").unwrap().index, 506);
        assert_eq!(table.indices.len(), 1152);
    }

    #[test]
    fn stops_at_the_erts_atom_limit() {
        let mut table = AtomTable::new();
        for index in table.indices.len()..ATOM_LIMIT {
            table.put(&index.to_string()).unwrap();
        }
        assert_eq!(table.put("1048575").unwrap().index, 1048575);
        // An existing atom still resolves at the limit.
        assert_eq!(table.put("ok").unwrap().index, 506);
        assert_eq!(
            table.put("pbeam_zeta").unwrap_err().to_string(),
            "no more index entries in atom_tab (max=1048576)"
        );
        assert_eq!(table.indices.len(), ATOM_LIMIT);
    }
}
