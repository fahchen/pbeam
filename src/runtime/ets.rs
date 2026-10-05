//! ETS tables of the node, as the ERTS table store (`erl_db.c`). All
//! processes of a run share one store.

use std::collections::HashMap;

use crate::term::{ProcessId, Reference};

/// The ETS tables of the node. ERTS identifies each table by a table
/// reference that `make_btid` (`erl_db.c:2811`) gives it, also a named
/// table, and finds a named table by its atom in the meta name table. An
/// atom name is unique in one atom table, so its text identifies it, as in
/// `Scheduler::names`. PBeam supports only `set` tables and does not store
/// objects yet.
#[derive(Debug, Default)]
pub(super) struct EtsTables {
    /// Each live table by its table reference.
    tables: HashMap<Reference, Table>,
    /// The table reference of each named table by its name.
    names: HashMap<String, Reference>,
}

/// One ETS table.
#[derive(Debug)]
struct Table {
    /// The process that owns the table, as `tb->common.owner`.
    owner: ProcessId,
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
    /// tables.create(Reference(0), ProcessId(0), Some("ac_tab"));
    /// assert!(tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn name_in_use(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// Add the table `table` that the process `owner` owns. A named table
    /// also gets its name. The caller makes sure that the name is not in
    /// use.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), ProcessId(0), None);
    /// tables.create(Reference(1), ProcessId(0), Some("ac_tab"));
    /// assert!(tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn create(&mut self, table: Reference, owner: ProcessId, name: Option<&str>) {
        self.tables.insert(table, Table { owner });
        if let Some(name) = name {
            self.names.insert(name.to_owned(), table);
        }
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
    /// tables.create(Reference(0), ProcessId(1), Some("ac_tab"));
    /// tables.delete_owned(ProcessId(1));
    /// assert!(!tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn delete_owned(&mut self, owner: ProcessId) {
        self.tables.retain(|_, table| table.owner != owner);
        self.names
            .retain(|_, table| self.tables.contains_key(table));
    }
}

#[cfg(test)]
mod tests {
    use super::EtsTables;
    use crate::term::{ProcessId, Reference};

    #[test]
    fn deletes_only_the_tables_of_the_owner() {
        let mut tables = EtsTables::default();
        tables.create(Reference(0), ProcessId(1), None);
        tables.create(Reference(1), ProcessId(1), Some("ets_probe_owned"));
        tables.create(Reference(2), ProcessId(2), Some("ets_probe_named"));
        tables.create(Reference(3), ProcessId(2), None);

        tables.delete_owned(ProcessId(1));
        assert!(!tables.name_in_use("ets_probe_owned"));
        assert!(tables.name_in_use("ets_probe_named"));
        let mut live: Vec<_> = tables.tables.keys().copied().collect();
        live.sort_by_key(|table| table.0);
        assert_eq!(live, vec![Reference(2), Reference(3)]);
    }
}
