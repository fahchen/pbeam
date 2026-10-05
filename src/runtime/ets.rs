//! ETS tables of the node, as the ERTS table store (`erl_db.c`). All
//! processes of a run share one store.

use std::collections::HashMap;

use crate::term::{ProcessId, Reference, Term};

/// The ETS tables of the node. ERTS identifies each table by a table
/// reference that `make_btid` (`erl_db.c:2811`) gives it, also a named
/// table, and finds a named table by its atom in the meta name table. An
/// atom name is unique in one atom table, so its text identifies it, as in
/// `Scheduler::names`. PBeam supports only `set` tables.
#[derive(Debug, Default)]
pub(super) struct EtsTables {
    /// Each live table by its table reference.
    tables: HashMap<Reference, Table>,
    /// The table reference of each named table by its name.
    names: HashMap<String, Reference>,
}

/// One ETS `set` table.
#[derive(Debug)]
pub(super) struct Table {
    /// The process that owns the table, as `tb->common.owner`.
    owner: ProcessId,
    /// Which other processes can read and write the table.
    access: Access,
    /// The 1-based key position, as the C `int` `tb->common.keypos`
    /// (`erl_db_util.h:346`). `ets:new/2` accepts each small integer above
    /// 0, and the C assignment keeps only its low 32 bits, so the value can
    /// be 0 or negative.
    keypos: i32,
    /// The elements of each object. ERTS keeps a `set` in a hash table and
    /// finds a key with the `EQ` exact equality (`erl_db_hash.c:766-789`).
    /// `Term` has no hash, so this store uses a linear search with the exact
    /// equality of `Term`. Each object has a key: an element at `keypos`.
    objects: Vec<Vec<Term>>,
}

/// The access mode of a table: `tb->common.status & (DB_PUBLIC |
/// DB_PROTECTED | DB_PRIVATE)`.
#[derive(Debug, Clone, Copy)]
pub(super) enum Access {
    /// Each process can read and write the table.
    Public,
    /// Each process can read the table. Only the owner can write it.
    Protected,
    /// Only the owner can read and write the table.
    Private,
}

/// What a BIF does with a table: the `what` argument of `db_get_table`
/// (`erl_db.c:800-912`).
#[derive(Debug, Clone, Copy)]
pub(super) enum Operation {
    /// `DB_READ`, which a `public` or a `protected` table allows
    /// (`erl_db_util.h:449`).
    Read,
    /// `DB_WRITE`, which only a `public` table allows (`erl_db_util.h:450`).
    Write,
}

impl EtsTables {
    /// Return whether a live table has the name `name`. A name that is in
    /// use makes `insert_named_tab` (`erl_db.c:1000-1047`) fail, and
    /// `ets:new/2` then raises `badarg` (`erl_db.c:2821-2831`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// assert!(!tables.name_in_use("ac_tab"));
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Public, 1));
    /// assert!(tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn name_in_use(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// Add the table `table` with the reference `id`. A named table also
    /// gets its name. The caller makes sure that the name is not in use.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), None, Table::new(ProcessId(0), Access::Protected, 1));
    /// tables.create(Reference(1), Some("ac_tab"), Table::new(ProcessId(0), Access::Public, 1));
    /// assert!(tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn create(&mut self, id: Reference, name: Option<&str>, table: Table) {
        self.tables.insert(id, table);
        if let Some(name) = name {
            self.names.insert(name.to_owned(), id);
        }
    }

    /// Return the table reference of the live table with the name `name`, or
    /// `None` for a free name. `ets:whereis/1` (`erl_db.c:2849-2872`) asks
    /// only for `DB_INFO` (`erl_db_util.h:451`), which each process has, so
    /// the caller and the access mode do not matter.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("logger"), Table::new(ProcessId(0), Access::Private, 1));
    /// assert_eq!(tables.whereis("logger"), Some(Reference(0)));
    /// assert_eq!(tables.whereis("missing"), None);
    /// ```
    pub(super) fn whereis(&self, name: &str) -> Option<Reference> {
        self.names.get(name).copied()
    }

    /// Return the table that `id` identifies, when the process `caller` can
    /// do `operation` on it, as `db_get_table` (`erl_db.c:800-912`) does. An
    /// atom finds a named table, and a reference finds its table. Return
    /// `None` for each other term, a free name, a deleted table, and a
    /// process that is not the owner and that the access mode does not
    /// allow (`handle_lacking_permission`, `erl_db.c:769-798`). The ETS BIFs
    /// then raise `badarg`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Protected, 1));
    /// let name = Term::static_atom("ac_tab");
    /// assert!(tables.table(&name, ProcessId(1), Operation::Read).is_some());
    /// assert!(tables.table(&name, ProcessId(1), Operation::Write).is_none());
    /// assert!(tables.table(&name, ProcessId(0), Operation::Write).is_some());
    /// assert!(tables.table(&Term::Integer(1), ProcessId(0), Operation::Read).is_none());
    /// ```
    pub(super) fn table(
        &mut self,
        id: &Term,
        caller: ProcessId,
        operation: Operation,
    ) -> Option<&mut Table> {
        let id = match id {
            Term::Atom(name) => *self.names.get(name.name())?,
            Term::Reference(id) => *id,
            _ => return None,
        };
        let table = self.tables.get_mut(&id)?;
        let allowed = table.owner == caller
            || matches!(
                (table.access, operation),
                (Access::Public, _) | (Access::Protected, Operation::Read)
            );
        allowed.then_some(table)
    }

    /// Delete each table that the process `owner` owns, and free the names
    /// of these tables, as `erts_db_process_exiting` (`erl_db.c:5031-5093`)
    /// does when the owner exits. PBeam has no heir, so no table goes to
    /// another process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(1), Access::Public, 1));
    /// tables.delete_owned(ProcessId(1));
    /// assert!(!tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn delete_owned(&mut self, owner: ProcessId) {
        self.tables.retain(|_, table| table.owner != owner);
        self.names
            .retain(|_, table| self.tables.contains_key(table));
    }
}

impl Table {
    /// Return an empty table that the process `owner` owns, with the access
    /// mode `access` and the truncated key position `keypos`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Protected, 1);
    /// assert_eq!(table.keypos(), 1);
    /// ```
    pub(super) fn new(owner: ProcessId, access: Access, keypos: i32) -> Table {
        Table {
            owner,
            access,
            keypos,
            objects: Vec::new(),
        }
    }

    /// Return the key position, as the C `int` of ERTS.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 2).keypos(), 2);
    /// ```
    pub(super) fn keypos(&self) -> i32 {
        self.keypos
    }

    /// Add the object with the elements `object`. An object with an exactly
    /// equal key replaces the old object, as `db_put_hash`
    /// (`erl_db_hash.c:1387-1402`) does in a `set` table. The caller makes
    /// sure that the key position is at least 1 and at most the size of the
    /// object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1);
    /// table.insert(vec![Term::static_atom("k"), Term::Integer(1)]);
    /// table.insert(vec![Term::static_atom("k"), Term::Integer(2)]);
    /// assert_eq!(table.object(&Term::static_atom("k")), Some(&[Term::static_atom("k"), Term::Integer(2)][..]));
    /// ```
    pub(super) fn insert(&mut self, object: Vec<Term>) {
        let key = self.key(&object);
        match self.objects.iter().position(|other| self.key(other) == key) {
            Some(index) => self.objects[index] = object,
            None => self.objects.push(object),
        }
    }

    /// Return the elements of the object with a key that is exactly equal to
    /// `key`, or `None` when the table has no such object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1);
    /// assert_eq!(table.object(&Term::static_atom("k")), None);
    /// table.insert(vec![Term::static_atom("k")]);
    /// assert_eq!(table.object(&Term::static_atom("k")), Some(&[Term::static_atom("k")][..]));
    /// ```
    pub(super) fn object(&self, key: &Term) -> Option<&[Term]> {
        self.objects
            .iter()
            .find(|object| self.key(object) == key)
            .map(Vec::as_slice)
    }

    /// Remove the object with a key that is exactly equal to `key`, as
    /// `db_erase_hash` (`erl_db_hash.c:1613-1664`) does in a `set` table. A
    /// key that the table does not hold changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1);
    /// table.insert(vec![Term::static_atom("k")]);
    /// table.delete(&Term::static_atom("missing"));
    /// table.delete(&Term::static_atom("k"));
    /// assert_eq!(table.object(&Term::static_atom("k")), None);
    /// ```
    pub(super) fn delete(&mut self, key: &Term) {
        if let Some(index) = self
            .objects
            .iter()
            .position(|object| self.key(object) == key)
        {
            self.objects.remove(index);
        }
    }

    /// Return the key of `object`: its element at the key position.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Public, 2);
    /// assert_eq!(table.key(&[Term::static_atom("v"), Term::static_atom("k")]), &Term::static_atom("k"));
    /// ```
    fn key<'a>(&self, object: &'a [Term]) -> &'a Term {
        let index = usize::try_from(self.keypos).expect("a stored object has a key position") - 1;
        &object[index]
    }
}

#[cfg(test)]
mod tests {
    use super::{Access, EtsTables, Table};
    use crate::term::{ProcessId, Reference, Term};

    #[test]
    fn deletes_only_the_tables_of_the_owner() {
        let mut tables = EtsTables::default();
        let table = |owner| Table::new(ProcessId(owner), Access::Protected, 1);
        tables.create(Reference(0), None, table(1));
        tables.create(Reference(1), Some("ets_probe_owned"), table(1));
        tables.create(Reference(2), Some("ets_probe_named"), table(2));
        tables.create(Reference(3), None, table(2));

        tables.delete_owned(ProcessId(1));
        assert!(!tables.name_in_use("ets_probe_owned"));
        assert!(tables.name_in_use("ets_probe_named"));
        let mut live: Vec<_> = tables.tables.keys().copied().collect();
        live.sort_by_key(|table| table.0);
        assert_eq!(live, vec![Reference(2), Reference(3)]);
    }

    #[test]
    fn finds_a_named_table_of_each_access_mode() {
        let mut tables = EtsTables::default();
        tables.create(
            Reference(0),
            None,
            Table::new(ProcessId(1), Access::Public, 1),
        );
        tables.create(
            Reference(1),
            Some("ets_probe_private"),
            Table::new(ProcessId(1), Access::Private, 1),
        );
        assert_eq!(tables.whereis("ets_probe_private"), Some(Reference(1)));
        assert_eq!(tables.whereis("ets_probe_missing"), None);
        tables.delete_owned(ProcessId(1));
        assert_eq!(tables.whereis("ets_probe_private"), None);
    }

    #[test]
    fn deletes_only_the_object_with_an_exactly_equal_key() {
        let mut table = Table::new(ProcessId(0), Access::Public, 2);
        let object = |value, key| vec![Term::Integer(value), key];
        let tuple = |value| Term::Tuple(vec![Term::Integer(value)]);
        table.insert(object(1, tuple(1)));
        table.insert(object(2, tuple(2)));
        table.delete(&tuple(3));
        table.delete(&tuple(1));
        assert_eq!(table.object(&tuple(1)), None);
        assert_eq!(table.object(&tuple(2)), Some(&object(2, tuple(2))[..]));
    }
}
