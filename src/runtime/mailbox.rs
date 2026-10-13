use std::collections::VecDeque;

use super::signal::Signal;
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
pub(super) type NodeId = usize;

/// A `next` field that points to a node, like an ERTS `ErtsMessage **`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    /// The head of the queue.
    First,
    /// The `next` field of a node.
    After(NodeId),
}

/// The content of one node of the signal queue.
#[derive(Debug)]
enum Entry {
    /// A message.
    Message(Term),
    /// A receive marker. Its slot in `Mailbox::markers` has its state.
    Marker,
    /// A non-message signal that the process did not handle yet. It is only
    /// in the middle or the outer part. The value is `None` after the
    /// handling took the signal out of the node, until the handling puts a
    /// message in the node or removes it, and in a node that left the
    /// queue.
    Signal(Option<Signal>),
}

impl Entry {
    /// Return the message of a message entry, or `None` for another entry.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Entry::Message(Term::Integer(1)).message(), Some(&Term::Integer(1)));
    /// assert_eq!(Entry::Marker.message(), None);
    /// ```
    fn message(&self) -> Option<&Term> {
        match self {
            Entry::Message(term) => Some(term),
            _ => None,
        }
    }
}

/// One node of the signal queue.
#[derive(Debug)]
struct Node {
    /// The content of the node.
    entry: Entry,
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
    /// True when the save pointer goes past the marker when the handling
    /// moves it into the inner part (`set_save` in ERTS).
    set_save: bool,
    /// True for the marker of `set_save_end`, which leaves the queue when
    /// the handling reaches it (`in_sigq < 0` in ERTS).
    remove: bool,
}

/// One handling pass over the middle part, as the `next_nm_sig` cursor of
/// `erts_proc_sig_handle_incoming` (`erl_proc_sig_queue.c:6229-7176`).
#[derive(Debug)]
pub(super) struct Walk {
    /// The `next` field before the next node to look at.
    at: Link,
    /// True when a receive marker moved the save pointer into the middle
    /// part during this pass (`save_in_msgq` is 0 in ERTS).
    save_in_middle: bool,
}

/// What the handling pass found next in the middle part.
#[derive(Debug, PartialEq)]
pub(super) enum Step {
    /// A signal. The node holds no signal until
    /// [`Mailbox::convert`] or [`Mailbox::remove`].
    Signal(NodeId, Signal),
    /// A receive marker, which is now in the inner part or left the queue.
    Marker,
}

/// The signal queue of a process, as the private and public signal queues
/// of ERTS (`erl_proc_sig_queue.h:43-74`). Messages, receive markers, and
/// non-message signals are nodes of one linked list in arrival order. Two
/// boundaries split the list in three parts:
///
/// - The inner part, up to `fetched` (`sig_qs.first` to `sig_qs.last`),
///   holds the messages and receive markers that a receive reads.
/// - The middle part, up to `received` (`sig_qs.cont` to
///   `sig_qs.cont_last`), holds the fetched messages and signals that the
///   process did not handle yet.
/// - The outer part holds the messages and signals that arrived and are not
///   fetched (`sig_inq`).
///
/// A message or a signal that arrives goes to the end of the list. A fetch
/// (`erts_proc_sig_fetch__`, `erl_proc_sig_queue.c:1332-1440`) moves the
/// outer part directly into the inner part when the middle part is empty
/// and the outer part has no signal; else it moves the outer part into the
/// middle part. The handling of signals ([`Mailbox::walk`]) moves the
/// messages before each handled signal into the inner part. The save
/// pointer is the receive cursor: it is the `next` field that points to the
/// current message.
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
    /// The end of the inner part, as `sig_qs.last` in ERTS.
    fetched: Link,
    /// The end of the middle part, as `sig_qs.cont_last` in ERTS.
    received: Link,
    /// The number of messages in the outer part
    /// (`ERTS_PSFLG_MSG_SIG_IN_Q`).
    outer_messages: usize,
    /// The number of signals in the outer part (`ERTS_PSFLG_NMSG_SIG_IN_Q`).
    outer_signals: usize,
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
            fetched: Link::First,
            received: Link::First,
            outer_messages: 0,
            outer_signals: 0,
            save: Link::First,
            markers: VecDeque::new(),
            next_marker_id: FIRST_MARKER_ID,
        }
    }
}

impl Mailbox {
    /// Add a message to the end of the outer part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::static_atom("ok"));
    /// mailbox.fetch();
    /// assert_eq!(mailbox.current(), Some(&Term::static_atom("ok")));
    /// ```
    pub(super) fn push(&mut self, term: Term) {
        self.append(Entry::Message(term));
        self.outer_messages += 1;
    }

    /// Add a non-message signal to the end of the outer part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push_signal(Signal::Flush);
    /// assert!(mailbox.has_control_signals());
    /// ```
    pub(super) fn push_signal(&mut self, signal: Signal) {
        self.append(Entry::Signal(Some(signal)));
        self.outer_signals += 1;
    }

    /// Fetch the outer part, as `erts_proc_sig_fetch`
    /// (`erl_proc_sig_queue.h:1997-2028`) does. When the middle part is
    /// empty and the outer part has no signal, the messages go directly to
    /// the end of the inner part (`erl_proc_sig_queue.c:1378-1386`). Else
    /// the outer part goes to the end of the middle part. A fetch has no
    /// cost.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::Integer(1));
    /// assert_eq!(mailbox.current(), None);
    /// mailbox.fetch();
    /// assert_eq!(mailbox.current(), Some(&Term::Integer(1)));
    /// ```
    pub(super) fn fetch(&mut self) {
        if self.fetched == self.received && self.outer_signals == 0 {
            self.fetched = self.last;
        }
        self.received = self.last;
        self.outer_messages = 0;
        self.outer_signals = 0;
    }

    /// Return true when the process has signals to handle: the middle part
    /// is not empty, or the outer part has a signal (`ERTS_PSFLG_SIG_Q` or
    /// `ERTS_PSFLG_NMSG_SIG_IN_Q`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Mailbox::default().has_control_signals());
    /// ```
    pub(super) fn has_control_signals(&self) -> bool {
        self.fetched != self.received || self.outer_signals > 0
    }

    /// Return true when the middle part is not empty (`sig_qs.cont`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Mailbox::default().has_middle());
    /// ```
    pub(super) fn has_middle(&self) -> bool {
        self.fetched != self.received
    }

    /// Return true when the middle or the outer part is not empty, as
    /// `ERTS_PROC_HAS_INCOMING_SIGNALS` (`erl_proc_sig_queue.h:396-400`)
    /// and the fail state of `erts_try_lock_sig_free_proc`
    /// (`erl_process.c:14835-14883`) tell.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::Integer(1));
    /// assert!(mailbox.has_incoming());
    /// ```
    pub(super) fn has_incoming(&self) -> bool {
        self.fetched != self.last
    }

    /// Return true when the outer part has a message
    /// (`ERTS_PSFLG_MSG_SIG_IN_Q`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::Integer(1));
    /// assert!(mailbox.has_outer_messages());
    /// ```
    pub(super) fn has_outer_messages(&self) -> bool {
        self.outer_messages > 0
    }

    /// Return true when the inner part holds a message. A receive marker is
    /// not a message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Mailbox::default().has_messages());
    /// ```
    pub(super) fn has_messages(&self) -> bool {
        self.message_count() > 0
    }

    /// Return the number of messages in the inner part, as the ERTS message
    /// count `mq_len` (`erl_proc_sig_queue.h:100-110`). A receive marker is
    /// not a message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::Integer(1));
    /// assert_eq!(mailbox.message_count(), 0);
    /// mailbox.fetch();
    /// assert_eq!(mailbox.message_count(), 1);
    /// ```
    pub(super) fn message_count(&self) -> usize {
        self.messages_until(self.fetched)
    }

    /// Return the number of messages before the node `node`, as `mq_len` is
    /// during the handling of the signal in `node`: the handling adds the
    /// messages before each signal (`erl_proc_sig_queue.c:6333`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(mailbox.messages_before(node), 2);
    /// ```
    pub(super) fn messages_before(&self, node: NodeId) -> usize {
        self.messages_until(self.nodes[node].prev)
    }

    /// Return the number of messages from the start of the queue up to the
    /// `next` field `end`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(mailbox.messages_until(Link::First), 0);
    /// ```
    fn messages_until(&self, end: Link) -> usize {
        let mut count = 0;
        let mut link = Link::First;
        while link != end
            && let Some(node) = self.target(link)
        {
            count += usize::from(self.nodes[node].entry.message().is_some());
            link = Link::After(node);
        }
        count
    }

    /// Return the message at the save pointer in the inner part, as
    /// `loop_rec` reads it (`erts_msgq_peek_msg`). A message that is not
    /// fetched, or that is in the middle part, is not a current message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Mailbox::default().current(), None);
    /// ```
    pub(super) fn current(&self) -> Option<&Term> {
        self.nodes[self.current_node()?].entry.message()
    }

    /// Return the node at the save pointer when it is in the inner part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Mailbox::default().current_node(), None);
    /// ```
    fn current_node(&self) -> Option<NodeId> {
        if self.save == self.fetched {
            return None;
        }
        self.target(self.save)
    }

    /// Move the save pointer past the current message, as `loop_rec_end`
    /// does. Return false when there is no current message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.push(Term::Integer(1));
    /// mailbox.fetch();
    /// assert!(mailbox.save_next());
    /// assert_eq!(mailbox.current(), None);
    /// ```
    pub(super) fn save_next(&mut self) -> bool {
        let Some(node) = self.current_node() else {
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
    /// mailbox.fetch();
    /// assert!(mailbox.remove_current());
    /// assert_eq!(mailbox.current(), None);
    /// ```
    pub(super) fn remove_current(&mut self) -> bool {
        let Some(node) = self.current_node() else {
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

    /// Move the save pointer to the end of the inner part after a fetch, as
    /// `erts_msgq_set_save_end` (`erl_proc_sig_queue.h:2397-2421`) does
    /// before a process-info request. When the middle part is not empty, a
    /// marker goes to its end instead, and the save pointer moves past the
    /// marker when the handling reaches it
    /// (`erts_msgq_recv_marker_create_insert_set_save`,
    /// `erl_proc_sig_queue.c:4365-4384`). No marker slot that the marker can
    /// use leaves the save pointer as it is.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.set_save_end();
    /// assert_eq!(mailbox.current(), None);
    /// ```
    pub(super) fn set_save_end(&mut self) {
        self.fetch();
        if !self.has_middle() {
            self.save = self.fetched;
            return;
        }
        if self.allocate_marker_slot() {
            self.clear_pending_set_save();
            let node = self.append(Entry::Marker);
            self.received = self.last;
            self.markers.push_back(Marker {
                id: Some(Term::nil()),
                node,
                pass: 0,
                set_save: true,
                remove: true,
            });
            self.save = self.fetched;
        }
    }

    /// Fetch, then insert a receive marker at the end of the queue and
    /// return its identifier, as `recv_marker_reserve` does
    /// (`erts_msgq_recv_marker_insert`, `erl_proc_sig_queue.h:2226-2236`).
    /// The marker goes to the inner part when the middle part is empty, else
    /// to the end of the middle part (`recv_marker_insert`,
    /// `erl_proc_sig_queue.c:4290-4348`). A queue with no message and no
    /// signal gets no marker and returns `undefined`. When all slots are in
    /// use, a marker in the inner part is used again, preferably a cleared
    /// one, else the least recently allocated one; when that marker is in
    /// the middle part, no marker is made and the result is `undefined`
    /// (`recv_marker_reuse`, `erl_proc_sig_queue.c:4147-4240`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(mailbox.reserve_marker(), Term::static_atom("undefined"));
    /// ```
    pub(super) fn reserve_marker(&mut self) -> Term {
        self.fetch();
        if self.first.is_none() || !self.allocate_marker_slot() {
            return Term::static_atom("undefined");
        }
        let id = Term::Integer(self.next_marker_id);
        self.next_marker_id += 1;
        let inner = !self.has_middle();
        let at_end = self.save == self.last;
        let node = self.append(Entry::Marker);
        self.received = self.last;
        let mut pass = 0;
        if inner {
            self.fetched = self.last;
            if at_end {
                // The save pointer now points to the marker; move it past.
                pass = 1;
                self.save = self.last;
            }
        }
        self.markers.push_back(Marker {
            id: Some(id.clone()),
            node,
            pass,
            set_save: false,
            remove: false,
        });
        id
    }

    /// Free a marker slot for a new marker when all slots are in use, as
    /// `recv_marker_reuse` (`erl_proc_sig_queue.c:4147-4240`) does. Return
    /// false when the least recently allocated marker is in the middle part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(mailbox.allocate_marker_slot());
    /// ```
    fn allocate_marker_slot(&mut self) -> bool {
        if self.markers.len() < MARKER_SLOTS {
            return true;
        }
        if !self.in_inner(self.markers[0].node) {
            return false;
        }
        let mut index = 0;
        for (position, marker) in self.markers.iter().enumerate() {
            if !self.in_inner(marker.node) {
                break;
            }
            if marker.id.is_none() {
                index = position;
                break;
            }
        }
        let marker = self.markers.remove(index).expect("marker slot index");
        self.unlink(marker.node);
        true
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
        if let Some(index) = self.marker(marker) {
            let slot = &mut self.markers[index];
            if matches!(reference, Term::Reference(_)) {
                slot.id = Some(reference.clone());
            } else {
                slot.id = None;
                slot.pass = MARKER_PASS_MAX;
            }
        }
    }

    /// Move the save pointer past the marker bound to a reference, as
    /// `recv_marker_use` does (`erts_msgq_recv_marker_set_save__`,
    /// `erl_proc_sig_queue.h:2150-2200`). A marker in the middle part moves
    /// the save pointer to the end of the inner part, and the handling moves
    /// it past the marker when the marker enters the inner part. Only one
    /// marker waits for this at a time.
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
        let Some(index) = self.marker(reference) else {
            return;
        };
        self.clear_pending_set_save();
        let node = self.markers[index].node;
        if self.in_inner(node) {
            self.save = self.pass_markers(Link::After(node), false);
        } else {
            self.markers[index].set_save = true;
            self.save = self.fetched;
        }
    }

    /// Clear the `set_save` flag of the marker in the middle part that waits
    /// to move the save pointer, as
    /// `ERTS_PROC_SIG_RECV_MARK_CLEAR_PENDING_SET_SAVE__`
    /// (`erl_proc_sig_queue.h:1892-1903`) does: one marker waits at a time.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.clear_pending_set_save();
    /// ```
    fn clear_pending_set_save(&mut self) {
        for marker in &mut self.markers {
            marker.set_save = false;
        }
    }

    /// Clear a marker, as `recv_marker_clear` does for an identifier or a
    /// reference. The next receive that passes the marker removes it from
    /// the queue.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.clear_marker(&Term::Reference(reference));
    /// ```
    pub(super) fn clear_marker(&mut self, id: &Term) {
        if !matches!(id, Term::Integer(_) | Term::Reference(_)) {
            return;
        }
        if let Some(index) = self.marker(id) {
            let slot = &mut self.markers[index];
            slot.id = None;
            slot.pass = MARKER_PASS_MAX;
        }
    }

    /// Return the index of the marker slot with an identifier.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(mailbox.marker(&Term::Integer(0)).is_none());
    /// ```
    fn marker(&self, id: &Term) -> Option<usize> {
        self.markers
            .iter()
            .position(|marker| marker.id.as_ref() == Some(id))
    }

    /// Return true when `node` is in the inner part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(mailbox.in_inner(node));
    /// ```
    fn in_inner(&self, node: NodeId) -> bool {
        let mut link = self.fetched;
        while let Some(next) = self.target(link) {
            if next == node {
                return false;
            }
            link = Link::After(next);
        }
        true
    }

    /// Start a handling pass at the start of the middle part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut walk = mailbox.walk();
    /// ```
    pub(super) fn walk(&self) -> Walk {
        Walk {
            at: self.fetched,
            save_in_middle: false,
        }
    }

    /// Return the next signal or receive marker of the middle part, and pass
    /// the messages before it, as `erts_proc_sig_handle_incoming` goes to
    /// the next non-message signal (`erl_proc_sig_queue.c:6312-6330`). A
    /// signal leaves its node with no signal. A receive marker moves into
    /// the inner part, and a marker with `set_save` moves the save pointer
    /// past itself; the marker of `set_save_end` leaves the queue and moves
    /// the save pointer to its place (`erl_proc_sig_queue.c:6893-6947`).
    /// Return `None` at the end of the middle part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut walk = mailbox.walk();
    /// assert!(matches!(mailbox.next_signal(&mut walk), Some(Step::Signal(_, Signal::Flush))));
    /// ```
    pub(super) fn next_signal(&mut self, walk: &mut Walk) -> Option<Step> {
        loop {
            if walk.at == self.received {
                return None;
            }
            let node = self.target(walk.at).expect("the middle part has a node");
            match &mut self.nodes[node].entry {
                Entry::Message(_) => walk.at = Link::After(node),
                Entry::Marker => return Some(self.handle_marker(walk, node)),
                Entry::Signal(signal) => {
                    let signal = signal.take().expect("the handling takes a signal once");
                    return Some(Step::Signal(node, signal));
                }
            }
        }
    }

    /// Move the receive marker in the node `node` of the middle part into
    /// the inner part, and move the save pointer past it when it has
    /// `set_save`. The marker of `set_save_end` leaves the queue, and the
    /// save pointer moves to its place (`erl_proc_sig_queue.c:6893-6947`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(mailbox.handle_marker(&mut walk, node), Step::Marker);
    /// ```
    fn handle_marker(&mut self, walk: &mut Walk, node: NodeId) -> Step {
        let index = self
            .markers
            .iter()
            .position(|marker| marker.node == node)
            .expect("receive marker node has a slot");
        let marker = &mut self.markers[index];
        let set_save = std::mem::take(&mut marker.set_save);
        if marker.remove {
            if set_save {
                self.save = walk.at;
                walk.save_in_middle = true;
            }
            self.markers.remove(index);
            self.unlink(node);
        } else {
            if set_save {
                self.save = Link::After(node);
                walk.save_in_middle = true;
            }
            walk.at = Link::After(node);
        }
        Step::Marker
    }

    /// Put the message `term` in the place of the signal that
    /// [`Mailbox::next_signal`] took from `node`, as
    /// `convert_prepared_sig_to_msg` does. The pass continues after it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.convert(&mut walk, node, exit_message);
    /// ```
    pub(super) fn convert(&mut self, walk: &mut Walk, node: NodeId, term: Term) {
        self.nodes[node].entry = Entry::Message(term);
        walk.at = Link::After(node);
    }

    /// Remove the node of a signal that [`Mailbox::next_signal`] took, as
    /// `remove_nm_sig` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.remove(node);
    /// ```
    pub(super) fn remove(&mut self, node: NodeId) {
        self.unlink(node);
    }

    /// Return true when the handling pass can go past its limit, as
    /// `stretch_limit` (`erl_proc_sig_queue.c:7180-7256`) does: the inner
    /// part has no message after the save pointer, and the middle part is
    /// not empty. ERTS also passes receive markers after the save pointer;
    /// the save pointer of PBeam is never before a receive marker of the
    /// inner part, because each move of the save pointer passes the
    /// markers. When a receive marker moved the save pointer into the middle
    /// part in this pass, the limit grows while a node follows it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!mailbox.can_stretch(&walk));
    /// ```
    pub(super) fn can_stretch(&self, walk: &Walk) -> bool {
        if walk.save_in_middle {
            return self.target(self.save).is_some();
        }
        self.save == self.fetched && self.has_middle()
    }

    /// End a handling pass: move the messages before the next signal that
    /// the pass did not handle into the inner part, or the whole middle part
    /// when no such signal is left (`erl_proc_sig_queue.c:7035-7068`). Then
    /// move the save pointer past receive markers
    /// (`erl_proc_sig_queue.c:7144-7149`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.end_walk(walk);
    /// ```
    pub(super) fn end_walk(&mut self, walk: Walk) {
        let mut link = walk.at;
        while link != self.received {
            let node = self.target(link).expect("the middle part has a node");
            if self.nodes[node].entry.message().is_none() {
                break;
            }
            link = Link::After(node);
        }
        self.fetched = link;
        self.save = self.pass_markers(self.save, false);
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
    /// let node = mailbox.append(Entry::Message(Term::Integer(1)));
    /// ```
    fn append(&mut self, entry: Entry) -> NodeId {
        let entry = Node {
            entry,
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

    /// Remove a node from the queue. A save pointer or a boundary at the
    /// `next` field of the node moves to the `next` field that pointed to
    /// the node.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// mailbox.unlink(node);
    /// ```
    fn unlink(&mut self, node: NodeId) {
        let Node { prev, next, .. } = self.nodes[node];
        self.nodes[node].entry = Entry::Signal(None);
        self.set_target(prev, next);
        match next {
            Some(next) => self.nodes[next].prev = prev,
            None => self.last = prev,
        }
        for link in [&mut self.save, &mut self.fetched, &mut self.received] {
            if *link == Link::After(node) {
                *link = prev;
            }
        }
        self.free.push(node);
    }

    /// Move a `next` field of the inner part past the receive markers that
    /// follow it, as `msgq_pass_recv_markers` does. Leading markers, and
    /// markers passed more than `MARKER_PASS_MAX` times, leave the queue.
    /// The pass stops at the end of the inner part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let save = mailbox.pass_markers(Link::First, true);
    /// ```
    fn pass_markers(&mut self, mut link: Link, leading: bool) -> Link {
        while link != self.fetched
            && let Some(node) = self.target(link)
            && matches!(self.nodes[node].entry, Entry::Marker)
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

    /// Return the messages in queue order, in all parts.
    #[cfg(test)]
    pub(super) fn messages(&self) -> Vec<Term> {
        let mut messages = Vec::new();
        let mut node = self.first;
        while let Some(current) = node {
            messages.extend(self.nodes[current].entry.message().cloned());
            node = self.nodes[current].next;
        }
        messages
    }

    /// Move the terms of at most `budget` entries of the mailbox to
    /// `pending`, and return the number of entries that it removed. An entry
    /// is the message of a node, the terms of a signal, or the identifier of
    /// a receive marker. Only a mailbox of a process that no guest code can
    /// read again gives up its terms (see `Scheduler::release_process`).
    ///
    /// The mailbox loses the nodes that it gives, so a caller reads the count
    /// to know its progress and reads [`Mailbox::is_empty`] to know when no
    /// entry is left.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut mailbox = Mailbox::default();
    /// mailbox.push(Term::Integer(1));
    /// let mut pending = Vec::new();
    /// assert_eq!(mailbox.take_terms_bounded(&mut pending, 256), 1);
    /// assert_eq!(pending, vec![Term::Integer(1)]);
    /// assert!(mailbox.is_empty());
    /// ```
    pub(super) fn take_terms_bounded(&mut self, pending: &mut Vec<Term>, budget: usize) -> usize {
        let mut taken = 0;
        while taken < budget {
            if let Some(marker) = self.markers.pop_back() {
                pending.extend(marker.id);
                taken += 1;
                continue;
            }
            let Some(node) = self.nodes.pop() else {
                break;
            };
            taken += 1;
            match node.entry {
                Entry::Message(term) => pending.push(term),
                Entry::Marker => {}
                Entry::Signal(signal) => {
                    if let Some(mut signal) = signal {
                        signal.take_terms(pending);
                    }
                }
            }
        }
        taken
    }

    /// Return true when no entry of the mailbox is left.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Mailbox::default().is_empty());
    /// ```
    pub(super) fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.markers.is_empty()
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/mailbox.rs"]
mod tests;
