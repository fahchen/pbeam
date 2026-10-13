//! The red-black tree that keeps the links and the monitor tree of a
//! process, as the ERTS `mon_lnk` tree (`erl_monitor_link.c:239-291`) that
//! `erl_rbtree.h` makes. The order of the signals that a process sends at
//! its exit comes from the shape of this tree, and the shape comes from
//! the full sequence of inserts and deletes. Thus the tree does the ERTS
//! insert and delete with the same rebalancing, and gives the ERTS
//! post-order walk. The same tree keeps the BIF timers to a process, as
//! the ERTS `proc_btm` tree (`erl_hl_timer.c:579-617`), which `erl_rbtree.h`
//! also makes.

use std::cmp::Ordering;

use super::LinkId;
use crate::term::{PortId, ProcessId, Reference};

/// The key order of a [`MonitorLinkTree`], as ERTS `ml_cmp_keys`
/// (`erl_monitor_link.c:90-233`).
pub(in crate::runtime) trait TreeKey: Copy {
    /// Compare `self` with `other` in the ERTS tree key order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ProcessId(1).compare(ProcessId(2)), Ordering::Less);
    /// ```
    fn compare(self, other: Self) -> Ordering;
}

impl TreeKey for ProcessId {
    /// ERTS compares two local pids as their `Eterm` words
    /// (`erl_monitor_link.c:157`). The pid data is in the high bits of the
    /// word, so the order is the order of the pid data, which is the PBeam
    /// counter value.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ProcessId(3).compare(ProcessId(2)), Ordering::Greater);
    /// ```
    fn compare(self, other: Self) -> Ordering {
        self.cmp(&other)
    }
}

impl TreeKey for LinkId {
    /// ERTS compares a local pid and a local port as their `Eterm` words
    /// (`erl_monitor_link.c:157`): the pid data or port data above the
    /// 4-bit tag, which is `0x3` for a pid and `0x7` for a port
    /// (`erl_term.h:81-82`). Thus the number orders first, and a pid comes
    /// before a port with the same number.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let pid = LinkId::Process(ProcessId(2));
    /// assert_eq!(pid.compare(LinkId::Port(PortId(2))), Ordering::Less);
    /// assert_eq!(pid.compare(LinkId::Port(PortId(1))), Ordering::Greater);
    /// ```
    fn compare(self, other: Self) -> Ordering {
        let word = |id| match id {
            LinkId::Process(ProcessId(number)) => (number, 0x3),
            LinkId::Port(PortId(number)) => (number, 0x7),
        };
        word(self).cmp(&word(other))
    }
}

impl TreeKey for Reference {
    /// ERTS compares two local references with `sys_memcmp` of their three
    /// 32-bit numbers (`erl_monitor_link.c:167-181`), that is, byte by byte
    /// in little-endian memory order: the low byte of `N0` first. Thus the
    /// order is not the creation order when the low byte goes past 255.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Reference(1).compare(Reference(2)), Ordering::Less);
    /// assert_eq!(Reference(255).compare(Reference(256)), Ordering::Greater);
    /// ```
    fn compare(self, other: Self) -> Ordering {
        memory_bytes(self).cmp(&memory_bytes(other))
    }
}

/// The reference of a BIF timer, as the key of the ERTS `proc_btm` tree of
/// the BIF timers to a process (`erl_hl_timer.c:579-617`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) struct TimerReference(pub(in crate::runtime) Reference);

impl TreeKey for TimerReference {
    /// ERTS compares the reference numbers `N2`, then `N1`, then `N0`
    /// (`refn_is_lt`, `erl_hl_timer.c:319-331`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let less = TimerReference(Reference(255)).compare(TimerReference(Reference(256)));
    /// assert_eq!(less, Ordering::Less);
    /// ```
    fn compare(self, other: Self) -> Ordering {
        let [low, middle, high] = self.0.numbers();
        let [other_low, other_middle, other_high] = other.0.numbers();
        (high, middle, low).cmp(&(other_high, other_middle, other_low))
    }
}

/// Return the bytes of the ERTS reference numbers `[N0, N1, N2]` of
/// `reference` in little-endian memory order. Arrays compare element by
/// element, so these bytes compare as `sys_memcmp` does. ERTS marks a pid
/// reference with bit 16 of `N1` (`ERTS_REF1_PID_MARKER_BIT__`,
/// `erl_bif_unique.h:106-109`). [`Reference::numbers`] gives the mark in
/// the high bit of `N2`, so this function moves it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(memory_bytes(Reference(0x1_0203)), [[3, 2, 1, 0], [1, 0, 0, 0], [0, 0, 0, 0]]);
/// assert_eq!(memory_bytes(Reference(0).pid_reference())[1], [1, 0, 1, 0]);
/// ```
fn memory_bytes(reference: Reference) -> [[u8; 4]; 3] {
    let [n0, mut n1, mut n2] = reference.numbers();
    if reference.is_pid_reference() {
        n1 |= 1 << 16;
        n2 &= !(1 << 31);
    }
    [n0, n1, n2].map(u32::to_le_bytes)
}

/// A child side of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    /// The left child.
    Left,
    /// The right child.
    Right,
}

impl Side {
    /// Return the other side.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Side::Left.opposite(), Side::Right);
    /// ```
    fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// One node of the tree: an entry and its tree links, as the `tree` part of
/// an ERTS `ErtsMonLnkNode`. The links are indexes in
/// `MonitorLinkTree::nodes`.
#[derive(Debug)]
struct Node<K, V> {
    /// The key: the pid of a link, or the reference of a monitor or alias.
    key: K,
    /// The value of the entry.
    value: V,
    /// The parent node, or `None` for the root.
    parent: Option<usize>,
    /// The left child, with less keys.
    left: Option<usize>,
    /// The right child, with greater keys.
    right: Option<usize>,
    /// The color: true for red, false for black (`ERTS_ML_TPFLG_RED`).
    red: bool,
}

/// A map from keys to values that keeps the shape of the ERTS red-black
/// tree for each sequence of inserts and deletes, and walks its entries in
/// the ERTS post-order. The nodes are in one vector, with no free slots.
#[derive(Debug)]
pub(in crate::runtime) struct MonitorLinkTree<K, V> {
    /// The nodes, in no order. The tree owns them, and a delete takes its
    /// node out (see [`MonitorLinkTree::free`]).
    nodes: Vec<Node<K, V>>,
    /// The root node, or `None` for an empty tree.
    root: Option<usize>,
}

impl<K, V> Default for MonitorLinkTree<K, V> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            root: None,
        }
    }
}

impl<K: TreeKey, V> MonitorLinkTree<K, V> {
    /// Return the bytes of the node vector of the tree: the capacity of the
    /// vector times the size of one node. The number excludes the header of
    /// the `Vec`, as the accounting of task
    /// [#559](https://github.com/fahchen/pbeam/issues/559) excludes it.
    /// The keys and the values are slots of the node.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tree = MonitorLinkTree::default();
    /// tree.insert(ProcessId(1), 7);
    /// assert!(tree.allocated_bytes() > 0);
    /// ```
    pub(in crate::runtime) fn allocated_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node<K, V>>()
    }

    /// Return the values of the tree, in no order, for the live-byte walk.
    /// A tree with no entry gives no value.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tree = MonitorLinkTree::default();
    /// tree.insert(ProcessId(1), 7);
    /// assert_eq!(tree.values().copied().collect::<Vec<_>>(), vec![7]);
    /// ```
    pub(in crate::runtime) fn values(&self) -> impl Iterator<Item = &V> {
        self.nodes.iter().map(|node| &node.value)
    }

    /// Return true when the tree has no entry. Only tests use it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(MonitorLinkTree::<ProcessId, u64>::default().is_empty());
    /// ```
    #[cfg(test)]
    pub(in crate::runtime) fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// Return the value of `key`, as `mon_lnk_rbt_lookup`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tree.insert(ProcessId(1), 0);
    /// assert_eq!(tree.get(&ProcessId(1)), Some(&0));
    /// assert_eq!(tree.get(&ProcessId(2)), None);
    /// ```
    pub(in crate::runtime) fn get(&self, key: &K) -> Option<&V> {
        self.find(*key).map(|index| &self.nodes[index].value)
    }

    /// Return the value of `key` for a change in place, which keeps the
    /// shape of the tree.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tree.insert(ProcessId(1), 0);
    /// *tree.get_mut(&ProcessId(1)).unwrap() = 7;
    /// assert_eq!(tree.get(&ProcessId(1)), Some(&7));
    /// ```
    pub(in crate::runtime) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.find(*key).map(|index| &mut self.nodes[index].value)
    }

    /// Insert `value` at `key`, as the ERTS `insert_aux__`
    /// (`erl_rbtree.h:1055-1130`) and `insert_fixup__`
    /// (`erl_rbtree.h:938-1053`) do. A key that the tree has keeps its node
    /// and gets the new value, as a change of the node that
    /// `lookup_create` finds; the old value returns.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(tree.insert(ProcessId(1), 0), None);
    /// assert_eq!(tree.insert(ProcessId(1), 5), Some(0));
    /// ```
    pub(in crate::runtime) fn insert(&mut self, key: K, value: V) -> Option<V> {
        let mut parent = None;
        let mut at = self.root;
        let mut side = Side::Left;
        while let Some(index) = at {
            side = match key.compare(self.nodes[index].key) {
                Ordering::Equal => {
                    return Some(std::mem::replace(&mut self.nodes[index].value, value));
                }
                Ordering::Less => Side::Left,
                Ordering::Greater => Side::Right,
            };
            parent = Some(index);
            at = self.child(index, side);
        }
        let node = self.nodes.len();
        self.nodes.push(Node {
            key,
            value,
            parent,
            left: None,
            right: None,
            red: parent.is_some(),
        });
        match parent {
            None => self.root = Some(node),
            Some(parent) => {
                self.set_child(parent, side, Some(node));
                if self.nodes[parent].red {
                    self.insert_fixup(node);
                }
            }
        }
        None
    }

    /// Remove `key` and return its value, as the ERTS `delete`
    /// (`erl_rbtree.h:704-932`) does. A node with two children is replaced
    /// by its successor. Return `None` when the tree has no such key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tree.insert(ProcessId(1), 0);
    /// assert_eq!(tree.remove(&ProcessId(1)), Some(0));
    /// assert_eq!(tree.remove(&ProcessId(1)), None);
    /// ```
    pub(in crate::runtime) fn remove(&mut self, key: &K) -> Option<V> {
        let z = self.find(*key)?;
        let spliced = match (self.nodes[z].left, self.nodes[z].right) {
            (Some(_), Some(right)) => self.leftmost(right),
            _ => z,
        };
        let child = self.nodes[spliced].left.or(self.nodes[spliced].right);
        let spliced_is_black = !self.nodes[spliced].red;
        let mut parent = self.nodes[spliced].parent;
        if let Some(child) = child {
            self.nodes[child].parent = parent;
        }
        self.replace_in_parent(spliced, child);
        if spliced != z {
            self.replace(z, spliced);
            if parent == Some(z) {
                parent = Some(spliced);
            }
        }
        if spliced_is_black {
            self.delete_fixup(child, parent);
        }
        Some(self.free(z))
    }

    /// Return the entries in the order of the ERTS walk
    /// `foreach_unordered__` (`erl_rbtree.h:1342-1452`), which
    /// `foreach_destroy` uses at the exit of a process: each subtree before
    /// its parent, the left subtree first.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// for key in 1..=3 {
    ///     tree.insert(ProcessId(key), 0);
    /// }
    /// let keys: Vec<_> = tree.post_order().map(|(key, _)| key.0).collect();
    /// assert_eq!(keys, [1, 3, 2]);
    /// ```
    pub(in crate::runtime) fn post_order(&self) -> PostOrder<'_, K, V> {
        PostOrder {
            tree: self,
            next: self.root.map(|root| self.first_in_post_order(root)),
        }
    }

    /// Return the post-order walk that continues at `position`, as a
    /// yielding ERTS walk continues at the node in its yield state
    /// (`erl_rbtree.h:1392-1396`). The tree must not change after the walk
    /// gave `position`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut walk = tree.post_order();
    /// walk.next();
    /// let position = walk.position().unwrap();
    /// assert!(tree.post_order_from(position).eq(walk));
    /// ```
    pub(in crate::runtime) fn post_order_from(
        &self,
        position: WalkPosition,
    ) -> PostOrder<'_, K, V> {
        PostOrder {
            tree: self,
            next: Some(position.0),
        }
    }

    /// Return the node of `key`, or `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(tree.find(ProcessId(9)), None);
    /// ```
    fn find(&self, key: K) -> Option<usize> {
        let mut at = self.root;
        while let Some(index) = at {
            at = match key.compare(self.nodes[index].key) {
                Ordering::Equal => return Some(index),
                Ordering::Less => self.nodes[index].left,
                Ordering::Greater => self.nodes[index].right,
            };
        }
        None
    }

    /// Return the child of `node` on `side`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(tree.child(root, Side::Left), tree.nodes[root].left);
    /// ```
    fn child(&self, node: usize, side: Side) -> Option<usize> {
        match side {
            Side::Left => self.nodes[node].left,
            Side::Right => self.nodes[node].right,
        }
    }

    /// Set the child of `node` on `side`. The parent of the child does not
    /// change.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tree.set_child(parent, Side::Right, Some(node));
    /// ```
    fn set_child(&mut self, node: usize, side: Side, child: Option<usize>) {
        match side {
            Side::Left => self.nodes[node].left = child,
            Side::Right => self.nodes[node].right = child,
        }
    }

    /// Return true for a red node, and false for a black node or no node.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!tree.is_red(None));
    /// ```
    fn is_red(&self, node: Option<usize>) -> bool {
        node.is_some_and(|node| self.nodes[node].red)
    }

    /// Put `new` in the place of `old` in the parent of `old`, or at the
    /// root.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tree.replace_in_parent(old, Some(new));
    /// ```
    fn replace_in_parent(&mut self, old: usize, new: Option<usize>) {
        match self.nodes[old].parent {
            None => self.root = new,
            Some(parent) if self.nodes[parent].left == Some(old) => {
                self.nodes[parent].left = new;
            }
            Some(parent) => self.nodes[parent].right = new,
        }
    }

    /// Rotate the subtree of `node` to `side`, as `left_rotate__` and
    /// `right_rotate__` (`erl_rbtree.h:564-637`): the child of `node` on
    /// the other side takes its place.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tree.rotate(node, Side::Left);
    /// ```
    fn rotate(&mut self, node: usize, side: Side) {
        let other = side.opposite();
        let up = self.child(node, other).expect("a rotation has a child");
        let inner = self.child(up, side);
        self.set_child(node, other, inner);
        if let Some(inner) = inner {
            self.nodes[inner].parent = Some(node);
        }
        self.nodes[up].parent = self.nodes[node].parent;
        self.replace_in_parent(node, Some(up));
        self.set_child(up, side, Some(node));
        self.nodes[node].parent = Some(up);
    }

    /// Put `with` in the place of `node` with its children and its color,
    /// as `replace__` (`erl_rbtree.h:646-682`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tree.replace(z, successor);
    /// ```
    fn replace(&mut self, node: usize, with: usize) {
        let Node {
            parent,
            left,
            right,
            red,
            ..
        } = self.nodes[node];
        self.replace_in_parent(node, Some(with));
        for child in [left, right].into_iter().flatten() {
            self.nodes[child].parent = Some(with);
        }
        let with = &mut self.nodes[with];
        (with.parent, with.left, with.right, with.red) = (parent, left, right, red);
    }

    /// Return the node with the least key in the subtree of `node`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let successor = tree.leftmost(tree.nodes[z].right.unwrap());
    /// ```
    fn leftmost(&self, mut node: usize) -> usize {
        while let Some(left) = self.nodes[node].left {
            node = left;
        }
        node
    }

    /// Restore the red-black rules after the insert of the red `node` under
    /// a red parent, as `insert_fixup__` (`erl_rbtree.h:938-1053`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tree.insert_fixup(node);
    /// ```
    fn insert_fixup(&mut self, mut node: usize) {
        loop {
            let parent = self.nodes[node].parent.expect("a red node has a parent");
            let grand = self.nodes[parent]
                .parent
                .expect("a red parent has a parent");
            let side = if self.nodes[grand].left == Some(parent) {
                Side::Left
            } else {
                Side::Right
            };
            let uncle = self.child(grand, side.opposite());
            if let Some(uncle) = uncle.filter(|&uncle| self.nodes[uncle].red) {
                self.nodes[uncle].red = false;
                self.nodes[parent].red = false;
                self.nodes[grand].red = true;
                node = grand;
            } else {
                let mut parent = parent;
                if self.child(parent, side.opposite()) == Some(node) {
                    node = parent;
                    self.rotate(node, side);
                    parent = self.nodes[node]
                        .parent
                        .expect("the rotation gives a parent");
                }
                self.nodes[parent].red = false;
                self.nodes[grand].red = true;
                self.rotate(grand, side.opposite());
                break;
            }
            match self.nodes[node].parent {
                Some(parent) if self.nodes[parent].red => {}
                _ => break,
            }
        }
        let root = self.root.expect("the tree has a root");
        self.nodes[root].red = false;
    }

    /// Restore the red-black rules after a black node left the tree, as the
    /// fixup part of the ERTS `delete` (`erl_rbtree.h:772-928`). `node` has
    /// the extra black; `None` is the empty place under `parent` that ERTS
    /// fills with its `null_x` node. The sibling of an empty place always
    /// exists, so the empty place is the side of `parent` with no child.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tree.delete_fixup(child, parent);
    /// ```
    fn delete_fixup(&mut self, mut node: Option<usize>, mut parent: Option<usize>) {
        while let Some(up) = parent.filter(|_| !self.is_red(node)) {
            let side = if self.nodes[up].left == node {
                Side::Left
            } else {
                Side::Right
            };
            let other = side.opposite();
            let mut sibling = self.child(up, other).expect("a black place has a sibling");
            if self.nodes[sibling].red {
                self.nodes[sibling].red = false;
                self.nodes[up].red = true;
                self.rotate(up, side);
                sibling = self.child(up, other).expect("a black place has a sibling");
            }
            let near = self.child(sibling, side);
            let far = self.child(sibling, other);
            if !self.is_red(near) && !self.is_red(far) {
                self.nodes[sibling].red = true;
                node = Some(up);
                parent = self.nodes[up].parent;
                continue;
            }
            if !self.is_red(far) {
                let near = near.expect("a red child exists");
                self.nodes[near].red = false;
                self.nodes[sibling].red = true;
                self.rotate(sibling, other);
                sibling = self.child(up, other).expect("a black place has a sibling");
            }
            if self.nodes[up].red {
                self.nodes[up].red = false;
                self.nodes[sibling].red = true;
            }
            let far = self.child(sibling, other).expect("a red child exists");
            self.nodes[far].red = false;
            self.rotate(up, side);
            node = self.root;
            break;
        }
        if let Some(node) = node {
            self.nodes[node].red = false;
        }
    }

    /// Take the node `index` out of the vector and return its value. The
    /// tree no longer refers to the node. The last node moves to `index`,
    /// and its links move with it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = tree.free(z);
    /// ```
    fn free(&mut self, index: usize) -> V {
        let node = self.nodes.swap_remove(index);
        let moved = self.nodes.len();
        if index != moved {
            match self.nodes[index].parent {
                None => self.root = Some(index),
                Some(parent) if self.nodes[parent].left == Some(moved) => {
                    self.nodes[parent].left = Some(index);
                }
                Some(parent) => self.nodes[parent].right = Some(index),
            }
            let Node { left, right, .. } = self.nodes[index];
            for child in [left, right].into_iter().flatten() {
                self.nodes[child].parent = Some(index);
            }
        }
        node.value
    }

    /// Return the first node of the post-order walk of the subtree of
    /// `node`: go left while possible, else right, to a leaf.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let first = tree.first_in_post_order(root);
    /// ```
    fn first_in_post_order(&self, mut node: usize) -> usize {
        while let Some(child) = self.nodes[node].left.or(self.nodes[node].right) {
            node = child;
        }
        node
    }
}

/// The post-order walk of a [`MonitorLinkTree`]; see
/// [`MonitorLinkTree::post_order`].
pub(in crate::runtime) struct PostOrder<'a, K, V> {
    /// The tree of the walk.
    tree: &'a MonitorLinkTree<K, V>,
    /// The node that the walk gives next, or `None` at the end.
    next: Option<usize>,
}

/// The node where a post-order walk continues, as the node in the yield
/// state of a yielding ERTS walk (`erl_rbtree.h:1392-1396`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::runtime) struct WalkPosition(usize);

impl<K, V> PostOrder<'_, K, V> {
    /// Return the position of the entry that the walk gives next, or `None`
    /// at the end. [`MonitorLinkTree::post_order_from`] continues the walk
    /// there.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(MonitorLinkTree::<ProcessId, u64>::default().post_order().position(), None);
    /// ```
    pub(in crate::runtime) fn position(&self) -> Option<WalkPosition> {
        self.next.map(WalkPosition)
    }
}

impl<'a, K: TreeKey, V> Iterator for PostOrder<'a, K, V> {
    type Item = (&'a K, &'a V);

    /// Return the next entry. After a node comes the first node of the
    /// right subtree of its parent when the node is a left child, else the
    /// parent (`erl_rbtree.h:1427-1449`).
    fn next(&mut self) -> Option<Self::Item> {
        let tree = self.tree;
        let index = self.next?;
        let node = &tree.nodes[index];
        self.next = node.parent.map(|parent| match tree.nodes[parent].right {
            Some(right) if right != index => tree.first_in_post_order(right),
            _ => parent,
        });
        Some((&node.key, &node.value))
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/process/tree.rs"]
mod tests;
