//! The expiry of not-connected dist entries that a send to another node
//! makes (`do_send`, `bif.c:2656-2666`). The shared dist table keeps their
//! order with the entries of connections.
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

/// The timers of send-only entries in the shared dist table. ERTS finds an
/// entry in a hash table (`erts_find_or_insert_dist_entry`,
/// `erl_node_tables.c:352-371`) and runs each delete timer on its own.
#[derive(Debug, Default)]
pub(super) struct KnownNodes {
    /// For the entry of each node, by the atom index of the node name:
    /// whether a send used the entry after its delete timer started.
    used: HashMap<u32, (bool, u64)>,
    /// The node name of each entry, from the oldest to the newest.
    order: BTreeMap<u64, Atom>,
    /// The delete timer of each entry: the run clock time at which it ends,
    /// and the order number of the entry. The first timer to end is first.
    timers: BinaryHeap<Reverse<(i64, u64)>>,
    /// The order number of the next new entry.
    #[cfg(test)]
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
    #[cfg(test)]
    pub(super) fn send(&mut self, name: &Atom, now: i64) {
        self.expire(now);
        let order = self.next;
        if self.send_ordered(name, order, now) {
            self.next += 1;
        }
    }

    /// Record a send using the shared distribution-list position `order`.
    /// Return true when the node was not present.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(known.send_ordered(&name, 1, now));
    /// ```
    pub(super) fn send_ordered(&mut self, name: &Atom, order: u64, now: i64) -> bool {
        if let Some((used, _)) = self.used.get_mut(&name.index()) {
            *used = true;
            false
        } else {
            self.used.insert(name.index(), (false, order));
            self.order.insert(order, name.clone());
            self.timers.push(Reverse((now + DELETE_DELAY, order)));
            true
        }
    }

    /// Remove a transient entry when a dist handle takes ownership of it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(known.take(&name), Some(1));
    /// ```
    pub(super) fn take(&mut self, name: &Atom) -> Option<u64> {
        let (_, order) = self.used.remove(&name.index())?;
        self.order.remove(&order);
        Some(order)
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
    #[cfg(test)]
    pub(super) fn names(&mut self, now: i64) -> Vec<Atom> {
        self.expire(now);
        self.order.values().cloned().collect()
    }

    /// Run each delete timer that ends at `now` or before, the first to end
    /// first: delete an entry that no send used after its timer started,
    /// and start the timer of each other entry again.
    pub(super) fn expire(&mut self, now: i64) -> Vec<u64> {
        let mut removed = Vec::new();
        while let Some(&Reverse((deadline, order))) = self.timers.peek() {
            if deadline > now {
                break;
            }
            self.timers.pop();
            let Some(name) = self.order.get(&order) else {
                continue;
            };
            #[cfg(test)]
            {
                self.steps += 1;
            }
            let (used, _) = self
                .used
                .get_mut(&name.index())
                .expect("each timer has an entry");
            if *used {
                *used = false;
                self.timers.push(Reverse((deadline + DELETE_DELAY, order)));
            } else {
                self.used.remove(&name.index());
                self.order.remove(&order);
                removed.push(order);
            }
        }
        removed
    }
}

impl Scheduler {
    /// Find or make the known node entry of `name` for a send to
    /// `{Name, Node}` with another node (see [`KnownNodes::send`]).
    pub(in crate::runtime) fn send_to_remote_node(&mut self, name: &Atom) {
        let now = self.timers.now();
        self.node().dist().send_known(name, now);
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/known_nodes.rs"]
mod tests;
