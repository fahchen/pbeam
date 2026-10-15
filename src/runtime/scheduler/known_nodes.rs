//! The known nodes of a run: the not connected dist entries that ERTS makes
//! for a send to `{Name, Node}` with another node (`do_send`,
//! `bif.c:2656-2666`). PBeam has no connection, so each such entry is
//! not connected, and `erlang:nodes/1,2` shows it with the node type
//! `known`.
//!
//! ERTS puts a new entry at the head of `erts_not_connected_dist_entries`
//! (`erl_node_tables.c:216-227`), and `nodes` reverses that list
//! (`dist.c:6384-6411`), so the result gives the entries from the oldest to
//! the newest. A send to a node that has an entry uses that entry and does
//! not move it.
//!
//! The send releases the entry at once. When its reference count gets to
//! zero, ERTS starts a timer of `node_tab_delete_delay`, 60 s by default
//! (`erl_node_tables.c:429-470`, `1073-1084`). When the timer ends, ERTS
//! deletes the entry if no lookup used it after the timer started, and
//! else starts the timer again (`erl_node_tables.c:472-537`). An entry is
//! visible only through `nodes`, so PBeam does the same steps when it
//! reads or changes the entries, by the run clock.

use super::Scheduler;
use crate::term::Atom;

/// The default `node_tab_delete_delay` of ERTS, 60 s
/// (`ERTS_NODE_TAB_DELAY_GC_DEFAULT`, `erl_node_tables.h:67`), in
/// nanoseconds of the run clock.
const DELETE_DELAY: i64 = 60_000_000_000;

/// One not connected dist entry.
#[derive(Debug)]
struct KnownNode {
    /// The node name.
    name: Atom,
    /// The run clock time at which the delete timer ends.
    deadline: i64,
    /// Whether a send used the entry after the delete timer started.
    used: bool,
}

/// The known nodes of a run, from the oldest to the newest.
#[derive(Debug, Default)]
pub(super) struct KnownNodes {
    entries: Vec<KnownNode>,
}

impl KnownNodes {
    /// Find or make the entry of `name` at the run clock time `now`, as
    /// `erts_find_or_insert_dist_entry` and `erts_deref_dist_entry` do for
    /// one remote send.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut known = KnownNodes::default();
    /// known.send(&a, 0);
    /// known.send(&b, 0);
    /// known.send(&a, 1);
    /// assert_eq!(known.names(1), vec![a, b]);
    /// ```
    pub(super) fn send(&mut self, name: &Atom, now: i64) {
        self.expire(now);
        match self.entries.iter_mut().find(|entry| entry.name == *name) {
            Some(entry) => entry.used = true,
            None => self.entries.push(KnownNode {
                name: name.clone(),
                deadline: now + DELETE_DELAY,
                used: false,
            }),
        }
    }

    /// Return the names of the entries at the run clock time `now`, from the
    /// oldest to the newest.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut known = KnownNodes::default();
    /// known.send(&a, 0);
    /// assert_eq!(known.names(DELETE_DELAY - 1), vec![a]);
    /// assert!(known.names(DELETE_DELAY).is_empty());
    /// ```
    pub(super) fn names(&mut self, now: i64) -> Vec<Atom> {
        self.expire(now);
        self.entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// Run each delete timer that ends at `now` or before: delete an entry
    /// that no send used after its timer started, and start the timer of
    /// each other entry again.
    fn expire(&mut self, now: i64) {
        self.entries.retain_mut(|entry| {
            while entry.deadline <= now {
                if !entry.used {
                    return false;
                }
                entry.used = false;
                entry.deadline += DELETE_DELAY;
            }
            true
        });
    }
}

impl Scheduler {
    /// Find or make the known node entry of `name` for a send to
    /// `{Name, Node}` with another node (see [`KnownNodes::send`]).
    pub(in crate::runtime) fn send_to_remote_node(&mut self, name: &Atom) {
        let now = self.timers.now();
        self.known_nodes.send(name, now);
    }

    /// Return the known nodes of the run, from the oldest to the newest (see
    /// [`KnownNodes::names`]).
    pub(in crate::runtime) fn known_nodes(&mut self) -> Vec<Atom> {
        let now = self.timers.now();
        self.known_nodes.names(now)
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/known_nodes.rs"]
mod tests;
