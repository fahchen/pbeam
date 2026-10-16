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
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};

/// The default `node_tab_delete_delay` of ERTS, 60 s
/// (`ERTS_NODE_TAB_DELAY_GC_DEFAULT`, `erl_node_tables.h:67`), in
/// nanoseconds of the run clock.
const DELETE_DELAY: i64 = 60_000_000_000;

/// The known nodes of a run. ERTS finds an entry in a hash table
/// (`erts_find_or_insert_dist_entry`, `erl_node_tables.c:352-371`) and
/// runs each delete timer on its own, so a send finds its entry by the
/// atom index and runs only the timers that ended.
#[derive(Debug, Default)]
pub(super) struct KnownNodes {
    /// For the entry of each node, by the atom index of the node name:
    /// whether a send used the entry after its delete timer started.
    used: HashMap<u32, bool>,
    /// The node name of each entry, from the oldest to the newest.
    order: BTreeMap<u64, Atom>,
    /// The delete timer of each entry: the run clock time at which it ends,
    /// and the order number of the entry. The first timer to end is first.
    timers: BinaryHeap<Reverse<(i64, u64)>>,
    /// The order number of the next new entry.
    next: u64,
    /// The count of timers that ended, for the tests of the cost of a send.
    #[cfg(test)]
    steps: usize,
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
        match self.used.get_mut(&name.index()) {
            Some(used) => *used = true,
            None => {
                let order = self.next;
                self.next += 1;
                self.used.insert(name.index(), false);
                self.order.insert(order, name.clone());
                self.timers.push(Reverse((now + DELETE_DELAY, order)));
            }
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
        self.order.values().cloned().collect()
    }

    /// Run each delete timer that ends at `now` or before, the first to end
    /// first: delete an entry that no send used after its timer started,
    /// and start the timer of each other entry again.
    fn expire(&mut self, now: i64) {
        while let Some(&Reverse((deadline, order))) = self.timers.peek() {
            if deadline > now {
                break;
            }
            self.timers.pop();
            #[cfg(test)]
            {
                self.steps += 1;
            }
            let name = &self.order[&order];
            let used = self
                .used
                .get_mut(&name.index())
                .expect("each timer has an entry");
            if *used {
                *used = false;
                self.timers.push(Reverse((deadline + DELETE_DELAY, order)));
            } else {
                self.used.remove(&name.index());
                self.order.remove(&order);
            }
        }
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
