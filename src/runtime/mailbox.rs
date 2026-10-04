use std::collections::VecDeque;

use crate::term::Term;

/// Number of receive marker slots in a process, as
/// `ERTS_RECV_MARKER_BLOCK_SIZE` in ERTS.
const MARKER_SLOTS: usize = 8;
/// A receive marker leaves the queue after this number of passes, as
/// `ERTS_RECV_MARKER_PASS_MAX` in ERTS.
const MARKER_PASS_MAX: u8 = 4;
/// First receive marker identifier. ERTS uses `MIN_SMALL` plus a counter of
/// the process.
const FIRST_MARKER_ID: i64 = -(1 << 59);

/// Index of a node in the node arena of a mailbox.
type NodeId = usize;

/// A `next` field that points to a node, like an ERTS `ErtsMessage **`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    /// The head of the queue.
    First,
    /// The `next` field of a node.
    After(NodeId),
}

/// One entry of the message queue: a message or a receive marker.
#[derive(Debug)]
struct Node {
    /// The message term. `None` marks a receive marker.
    message: Option<Term>,
    /// The `next` field that points to this node.
    prev: Link,
    /// The node after this node.
    next: Option<NodeId>,
}

/// A receive marker slot.
#[derive(Debug)]
struct Marker {
    /// Marker identifier or bound reference. `None` after `recv_marker_clear`.
    id: Option<Term>,
    /// The marker node in the queue.
    node: NodeId,
    /// Number of times a receive passed the marker.
    pass: u8,
}

/// A process mailbox that follows the ERTS message queue
/// (`erl_proc_sig_queue.h`). Messages and receive markers are nodes of one
/// linked list. The save pointer is the receive cursor: it is the `next`
/// field that points to the current message.
#[derive(Debug)]
pub(super) struct Mailbox {
    /// Node arena. A removed node goes to `free` for reuse.
    nodes: Vec<Node>,
    /// Arena slots that hold no live node.
    free: Vec<NodeId>,
    /// The first node of the queue.
    first: Option<NodeId>,
    /// The `next` field of the last node, where the next node goes.
    last: Link,
    /// The `next` field that points to the current message (`save` in ERTS).
    save: Link,
    /// Receive markers in allocation order. The first marker is the least
    /// recently allocated one.
    markers: VecDeque<Marker>,
    /// Identifier for the next receive marker.
    next_marker_id: i64,
}

impl Default for Mailbox {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            free: Vec::new(),
            first: None,
            last: Link::First,
            save: Link::First,
            markers: VecDeque::new(),
            next_marker_id: FIRST_MARKER_ID,
        }
    }
}

impl Mailbox {
    /// Add a message to the end of the queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::static_atom("ok"));
    /// assert_eq!(mailbox.current(), Some(&Term::static_atom("ok")));
    /// ```
    pub(super) fn push(&mut self, term: Term) {
        self.append(Some(term));
    }

    /// Return the message at the save pointer, as `loop_rec` reads it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Mailbox::default().current(), None);
    /// ```
    pub(super) fn current(&self) -> Option<&Term> {
        self.target(self.save)
            .and_then(|node| self.nodes[node].message.as_ref())
    }

    /// Move the save pointer past the current message, as `loop_rec_end`
    /// does. Return false when there is no current message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::Integer(1));
    /// assert!(mailbox.save_next());
    /// assert_eq!(mailbox.current(), None);
    /// ```
    pub(super) fn save_next(&mut self) -> bool {
        let Some(node) = self.target(self.save) else {
            return false;
        };
        self.save = self.pass_markers(Link::After(node), false);
        true
    }

    /// Remove the current message and move the save pointer to the first
    /// message, as `remove_message` does. Return false when there is no
    /// current message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::Integer(1));
    /// assert!(mailbox.remove_current());
    /// assert_eq!(mailbox.current(), None);
    /// ```
    pub(super) fn remove_current(&mut self) -> bool {
        let Some(node) = self.target(self.save) else {
            return false;
        };
        self.unlink(node);
        self.set_save_first();
        true
    }

    /// Move the save pointer to the first message and remove the receive
    /// markers at the front of the queue, as ERTS does when a receive ends.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.set_save_first();
    /// ```
    pub(super) fn set_save_first(&mut self) {
        self.save = self.pass_markers(Link::First, true);
    }

    /// Insert a receive marker at the end of the queue and return its
    /// identifier, as `recv_marker_reserve` does. An empty queue gets no
    /// marker and returns `undefined`. When all slots are in use, the least
    /// recently allocated marker is used again, preferably a cleared one.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(mailbox.reserve_marker(), Term::static_atom("undefined"));
    /// ```
    pub(super) fn reserve_marker(&mut self) -> Term {
        if self.first.is_none() {
            return Term::static_atom("undefined");
        }
        if self.markers.len() == MARKER_SLOTS {
            let index = self
                .markers
                .iter()
                .position(|marker| marker.id.is_none())
                .unwrap_or(0);
            let marker = self.markers.remove(index).expect("marker slot index");
            self.unlink(marker.node);
        }
        let id = Term::Integer(self.next_marker_id);
        self.next_marker_id += 1;
        let at_end = self.save == self.last;
        let node = self.append(None);
        let mut pass = 0;
        if at_end {
            // The save pointer now points to the marker; move it past.
            pass = 1;
            self.save = self.last;
        }
        self.markers.push_back(Marker {
            id: Some(id.clone()),
            node,
            pass,
        });
        id
    }

    /// Bind a reserved marker to a reference, as `recv_marker_bind` does. A
    /// value that is not a reference clears the marker.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.bind_marker(&marker, &Term::Reference(reference));
    /// ```
    pub(super) fn bind_marker(&mut self, marker: &Term, reference: &Term) {
        if !matches!(marker, Term::Integer(_)) {
            return;
        }
        if let Some(slot) = self.marker(marker) {
            if matches!(reference, Term::Reference(_)) {
                slot.id = Some(reference.clone());
            } else {
                slot.id = None;
                slot.pass = MARKER_PASS_MAX;
            }
        }
    }

    /// Move the save pointer past the marker bound to a reference, as
    /// `recv_marker_use` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.use_marker(&Term::Reference(reference));
    /// ```
    pub(super) fn use_marker(&mut self, reference: &Term) {
        if !matches!(reference, Term::Reference(_)) {
            return;
        }
        if let Some(node) = self.marker(reference).map(|slot| slot.node) {
            self.save = self.pass_markers(Link::After(node), false);
        }
    }

    /// Clear a marker, as `recv_marker_clear` does. The next receive that
    /// passes the marker removes it from the queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.clear_marker(&Term::Reference(reference));
    /// ```
    pub(super) fn clear_marker(&mut self, id: &Term) {
        if let Some(slot) = self.marker(id) {
            slot.id = None;
            slot.pass = MARKER_PASS_MAX;
        }
    }

    /// Return the marker slot with an identifier.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(mailbox.marker(&Term::Integer(0)).is_none());
    /// ```
    fn marker(&mut self, id: &Term) -> Option<&mut Marker> {
        self.markers
            .iter_mut()
            .find(|marker| marker.id.as_ref() == Some(id))
    }

    /// Return the node that a `next` field points to.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(mailbox.target(Link::First), mailbox.first);
    /// ```
    fn target(&self, link: Link) -> Option<NodeId> {
        match link {
            Link::First => self.first,
            Link::After(node) => self.nodes[node].next,
        }
    }

    /// Set the node that a `next` field points to.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.set_target(Link::First, None);
    /// ```
    fn set_target(&mut self, link: Link, node: Option<NodeId>) {
        match link {
            Link::First => self.first = node,
            Link::After(previous) => self.nodes[previous].next = node,
        }
    }

    /// Add a node to the end of the queue and return its identifier.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let node = mailbox.append(Some(Term::Integer(1)));
    /// ```
    fn append(&mut self, message: Option<Term>) -> NodeId {
        let entry = Node {
            message,
            prev: self.last,
            next: None,
        };
        let node = match self.free.pop() {
            Some(node) => {
                self.nodes[node] = entry;
                node
            }
            None => {
                self.nodes.push(entry);
                self.nodes.len() - 1
            }
        };
        self.set_target(self.last, Some(node));
        self.last = Link::After(node);
        node
    }

    /// Remove a node from the queue. A save pointer at the `next` field of
    /// the node moves to the `next` field that pointed to the node.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.unlink(node);
    /// ```
    fn unlink(&mut self, node: NodeId) {
        let Node { prev, next, .. } = self.nodes[node];
        self.nodes[node].message = None;
        self.set_target(prev, next);
        match next {
            Some(next) => self.nodes[next].prev = prev,
            None => self.last = prev,
        }
        if self.save == Link::After(node) {
            self.save = prev;
        }
        self.free.push(node);
    }

    /// Move a `next` field past the receive markers that follow it, as
    /// `msgq_pass_recv_markers` does. Leading markers, and markers passed
    /// more than `MARKER_PASS_MAX` times, leave the queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let save = mailbox.pass_markers(Link::First, true);
    /// ```
    fn pass_markers(&mut self, mut link: Link, leading: bool) -> Link {
        while let Some(node) = self.target(link)
            && self.nodes[node].message.is_none()
        {
            let index = self
                .markers
                .iter()
                .position(|marker| marker.node == node)
                .expect("receive marker node has a slot");
            let marker = &mut self.markers[index];
            marker.pass += 1;
            if leading || marker.pass > MARKER_PASS_MAX {
                self.markers.remove(index);
                self.unlink(node);
            } else {
                link = Link::After(node);
            }
        }
        link
    }

    /// Return the messages in queue order.
    #[cfg(test)]
    pub(super) fn messages(&self) -> Vec<Term> {
        let mut messages = Vec::new();
        let mut node = self.first;
        while let Some(current) = node {
            messages.extend(self.nodes[current].message.clone());
            node = self.nodes[current].next;
        }
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::{MARKER_SLOTS, Mailbox};
    use crate::term::test_atom;
    use crate::term::{Reference, Term};

    fn int(value: i64) -> Term {
        Term::Integer(value)
    }

    fn mailbox(values: &[i64]) -> Mailbox {
        let mut mailbox = Mailbox::default();
        for value in values {
            mailbox.push(int(*value));
        }
        mailbox
    }

    #[test]
    fn walks_and_removes_messages() {
        let mut mailbox = mailbox(&[1, 2, 3]);
        assert!(mailbox.save_next());
        assert_eq!(mailbox.current(), Some(&int(2)));
        assert!(mailbox.remove_current());
        assert_eq!(mailbox.messages(), vec![int(1), int(3)]);
        assert_eq!(mailbox.current(), Some(&int(1)));
        assert!(mailbox.save_next() && mailbox.save_next());
        assert!(!mailbox.save_next() && !mailbox.remove_current());
        mailbox.push(int(4));
        assert_eq!(mailbox.current(), Some(&int(4)));
        assert!(mailbox.remove_current());
        assert_eq!(mailbox.messages(), vec![int(1), int(3)]);
    }

    #[test]
    fn starts_a_receive_after_a_bound_marker() {
        let mut mailbox = Mailbox::default();
        assert_eq!(mailbox.reserve_marker(), test_atom("undefined"));
        mailbox.push(int(1));
        let marker = mailbox.reserve_marker();
        assert_eq!(marker, int(-(1 << 59)));
        let reference = Term::Reference(Reference(7));
        mailbox.bind_marker(&int(5), &reference);
        mailbox.bind_marker(&reference, &reference);
        mailbox.bind_marker(&marker, &reference);
        mailbox.push(int(2));
        mailbox.use_marker(&int(5));
        mailbox.use_marker(&Term::Reference(Reference(8)));
        assert_eq!(mailbox.current(), Some(&int(1)));
        mailbox.use_marker(&reference);
        assert_eq!(mailbox.current(), Some(&int(2)));

        assert!(mailbox.remove_current());
        mailbox.clear_marker(&reference);
        assert!(mailbox.save_next());
        mailbox.set_save_first();
        assert_eq!(mailbox.current(), Some(&int(1)));
        assert!(mailbox.remove_current());
        assert_eq!(mailbox.first, None);
        assert!(mailbox.markers.is_empty());
    }

    #[test]
    fn moves_the_save_pointer_past_a_marker_at_the_end() {
        let mut mailbox = mailbox(&[1]);
        assert!(mailbox.save_next());
        let marker = mailbox.reserve_marker();
        mailbox.push(int(2));
        assert_eq!(mailbox.current(), Some(&int(2)));
        mailbox.bind_marker(&marker, &test_atom("none"));
        assert!(mailbox.markers[0].id.is_none());

        // A marker at the front of the queue leaves when a receive ends.
        mailbox.set_save_first();
        assert!(mailbox.remove_current());
        assert!(mailbox.markers.is_empty());
        assert_eq!(mailbox.current(), Some(&int(2)));
    }

    #[test]
    fn reuses_marker_slots_and_drops_passed_markers() {
        let mut mailbox = mailbox(&[0]);
        let oldest = mailbox.reserve_marker();
        mailbox.push(int(1));
        let markers: Vec<Term> = (1..MARKER_SLOTS)
            .map(|_| mailbox.reserve_marker())
            .collect();
        let reference = Term::Reference(Reference(0));
        mailbox.bind_marker(&oldest, &reference);
        mailbox.use_marker(&reference);
        assert_eq!(mailbox.current(), Some(&int(1)));

        // The oldest marker holds the save pointer. Its reuse moves the
        // save pointer back to the same message.
        mailbox.reserve_marker();
        assert_eq!(mailbox.current(), Some(&int(1)));
        assert!(
            mailbox
                .markers
                .iter()
                .all(|marker| marker.id != Some(reference.clone()))
        );

        // A cleared marker is used again before the oldest marker.
        mailbox.clear_marker(&markers[3]);
        mailbox.reserve_marker();
        assert_eq!(mailbox.markers.len(), MARKER_SLOTS);
        assert!(mailbox.markers.iter().all(|marker| marker.id.is_some()));
        assert_eq!(mailbox.markers[0].id, Some(markers[0].clone()));

        // A marker leaves the queue after more than four passes.
        for _ in 0..5 {
            mailbox.set_save_first();
            assert!(mailbox.save_next() && mailbox.save_next());
        }
        assert!(mailbox.markers.is_empty());
        assert_eq!(mailbox.messages(), vec![int(0), int(1)]);
    }
}
