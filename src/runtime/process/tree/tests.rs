use std::cmp::Ordering;

use super::{MonitorLinkTree, TreeKey};
use crate::term::{ProcessId, Reference};

/// Link sequences of a process on pinned ERTS 29.1.1 with one scheduler,
/// and the order of the link exit signals at its exit. A positive number
/// `i` is `link/1` of the member `i`, a negative number `-i` is
/// `unlink/1` of the member `i` after its unlink acknowledgment. Members
/// have pids in their number order. Measured with random sequences and
/// `exit_order_probe:links/2` (`fixtures/signal_queue`): the members that
/// do not trap exits end in the order of the signals.
#[rustfmt::skip]
const ERTS_ORDERS: &[(&[i64], &[u64])] = &[
    (&[-1, 3, 3, 9, 10, -11, 1, -8, -12, 12, 7, 6, 9, 3, 10, -3, -12, -8, 8, 9, 9, 4, 2, 4, -4, -11], &[1, 2, 8, 7, 6, 10, 9]),
    (&[-12, 10, -1, -11, 3, 12, -15, -15, -1, -14, 4, -1, 11, 4, -5, 9, -12, 2, 2, -10, -4, 13, 4, 4, -15, 8, 11, -3, 14, -15, 15, 6, 13, -6, 5, -15, 5, 2, 4, 9], &[2, 5, 8, 4, 11, 14, 13, 9]),
    (&[6, -4, -15, 12, 7, 2, 11, 2, 6, 8, 1, 18, 14, 13, -6, -4, -17, -2, 10, -3, -17, -17, -7, 14, -9, -5, 15, 18, -9, -11, 6, -1, -15, -15, -12, -14, -4, 16, 9, -15, -16, 10, 7, 4, 16, -1, 10, -16], &[4, 7, 6, 9, 10, 8, 18, 13]),
    (&[-8, -9, 18, -8, -2, -13, -6, -5, -5, 6, -11, -3, -17, -14, 4, 10, 6, -2, -3, 12, 14, 15, 5, 8, 3, -3, 17, -1, 1, 14, -8, 17, 16, -3, 4, 3, 13, -10, -18, -6, 18, 8, -17, -11, 18, 11, -15, -5, 1, -11, 13, 10, 2], &[1, 3, 2, 10, 8, 4, 13, 14, 18, 16, 12]),
    (&[1, 5, 2, 5, 3, -4, 2, 2, -4, 1, -3, 5, 2], &[1, 5, 2]),
    (&[11, -5, 4, 6, -7, 1, -7, 9, -1, -9, -6, -10, -12, 12, -13, 10, 14, -9, -3, 7, -2, 13, 4, 14, 3, 7, -4, 11, 6, 8, -6, 7, 2, 14, 7], &[2, 3, 8, 10, 7, 12, 14, 13, 11]),
    (&[3, 3, 7, -5, -7, -7, 8, 5, 3, 5, 7, 3, 5, 5, -1, -9, 3, -9, -5, -1, 5, 9], &[5, 3, 9, 8, 7]),
    (&[6, -4, 8, 4, -5, 2, -2, 8, 9, -5, 8, 4, 1, -8, -2, -8, 2, 3, 9, 5, 7], &[1, 3, 5, 4, 2, 7, 9, 6]),
    (&[-6, 1, -7, 6, 4, -3, 2, 2, 4, -2, -4, 7, -3, -8, 4, 4, -8, 6], &[4, 1, 7, 6]),
    (&[-6, -1, 7, 6, -6, 5, 8, -2, -1, 7, 5, 3, 1, 2, 8, -4, -1, -7, -7], &[2, 5, 8, 3]),
    (&[15, -2, 9, 4, 1, 6, 11, -4, -16, -9, -12, -17, 3, 18, -12, 1, 4, 6, 11, 7, 8, -7, -2, -15, -9, 17, 17, -4, -18, 2, -14, -7, 17, 10, 15, 18, -3, 11, -17, -12, 5, 7, -4, 17, 14, 16, 4, -15], &[1, 4, 5, 2, 7, 10, 8, 6, 14, 16, 18, 17, 11]),
    (&[5, -5, -3, 2, 1, -3, 3, -2, -3, -4, 4, -1], &[4]),
    (&[4, -6, -5, -4, 3, -3, 5, 1, -4, 1, 6, -1, 3, 1, 1, -5], &[1, 6, 3]),
    (&[10, -1, 11, 9, -1, -3, 9, -10, 9, 2, 4, -8, -7, 8, -6, 6, -11, -5, 9, 3, -4, 3, -6, -4, 10, -3, -9, -3, -3, -5, 3, 2, 3], &[3, 2, 10, 8]),
    (&[6, -8, 2, 9, 2, 3, -3, 5, -9, -9, 2, 9, -10, -10, 9, 1, 1, 2, 6, 2, 6, -6, 1], &[1, 2, 9, 5]),
    (&[5, -10, 1, -1, -18, -8, 4, 17, 10, -9, -12, 16, -4, -2, 18, -6, 16, 16, -4, 12, 7, 11, -4, -15, -15, -9, 8, -1, 13, 18, -11, -11, 12, -12, -11, 16, -6, -7, 1, -6, 5, 3, 12, 18, 11, -13, 7, 15, 18, 17], &[3, 1, 7, 8, 5, 11, 15, 12, 18, 17, 16, 10]),
    (&[12, 2, 5, 16, -13, 5, -10, -9, 8, -1, -13, 3, -1, -8, 15, -15, -15, 15, -7, 2, -11, -3, 9, 1, -14, 2, -2, -14, 18, -12, -13, -4, 3, 13, 17, 1, 4, -10, 11, -6, 18, -9], &[1, 4, 3, 13, 11, 16, 18, 17, 15, 5]),
    (&[8, -6, -4, 8, 6, -3, -4, 10, 8, 10, 3, 7, -13, 2, 5, 4, -5, 14, -14, -6, 14, 7, -14, 1, 8, -3, 3, 12, 8, -12, 7, -1, -4, -4, -11, -8, 13, 2, 6, 6], &[2, 6, 3, 13, 10, 7]),
    (&[10, 5, 6, 6, -7, 7, -11, -6, 2, 8, -5, 8, -2, 1, -13, -7, 10, 1, -5, -3, 8, -13, -6, 8, -10, -9, 9, 4, -1, 6, 9, 5, -6], &[4, 5, 9, 8]),
    (&[1, 1, -5, -5, 3, 10, -4, 1, -4, 10, -7, 5, -2, 8, -3, 6, -7, 8, 10, -4, -2, -6, 9, 6, -7, 8, -1, 5], &[5, 8, 6, 10, 9]),
    (&[-6, 5, 4, -6, 2, -8, -1, 6, -2, -5, 1, 1, -5, 1, -1, -7, 4, 3, -4], &[3, 6]),
    (&[3, 13, 14, 9, -6, -8, -11, -13, -5, 6, 4, 8, -17, 15, -2, -3, 14, 13, -16, -14, -6, 8, -17, -11, -14, -15, -12, -9, 6, 7, 9, -12, -6, -14, 6, 13, -17, 14, 11, 8], &[4, 7, 6, 11, 9, 14, 13, 8]),
    (&[-3, -15, -7, 17, 2, 15, -4, -5, -6, -16, 11, 12, 13, 12, -15, 9, 3, 15, 11, 3, -15, -5, 15, -14, 17, -4, 14, 2, -5, -3, -6, 7, -15, -8, -15], &[2, 9, 7, 12, 14, 17, 13, 11]),
    (&[6, -2, 6, -6, 7, 4, -7, 6, 7, -5, -7, 5, 5, 8, 8, 7, 4, 6, -2], &[4, 6, 8, 7, 5]),
    (&[3, -6, -1, 5, -5, 2, 5, 5, 2, 6, 4, -2, 1, -5, 1], &[1, 4, 6, 3]),
    (&[7, -9, -8, -13, 6, -2, 4, 3, 4, 5, 9, 4, 3, -6, -8, -13, -5, 8, -1, -5, 4, -10, 5, 6, 8, 1, 11, 7, 13, 4, -13], &[1, 3, 6, 5, 4, 8, 11, 9, 7]),
    (&[6, -9, 1, 16, 2, -1, 7, -17, -10, 1, -3, -9, 16, -10, 7, 6, 5, 7, -11, 12, 7, 9, -1, 6, 17, 2, -2, 10, -14, 16, 12, 15, -15, -12, 7, -11, 4, 11, 1, 5, 3, 17, 12, 7, 7], &[3, 1, 5, 4, 7, 6, 10, 12, 11, 17, 16, 9]),
    (&[18, -11, 18, 3, -7, 2, -14, 7, -6, 1, -4, 9, -10, 13, -8, 17, -12, 14, 7, 1, -10, 4, 18, 9, -14, 6, 4, 16, 10, 8, 14, 3, -7, 12, 8, 9, -11], &[1, 2, 4, 8, 6, 3, 12, 10, 14, 16, 13, 18, 17, 9]),
    (&[10, 1, -1, -10, 11, 6, -9, -3, 5, 6, 2, 4, 4, 7, 9, 10, 10, 7, 2, 7, 10, 1, -3, -10, 4, -5, 10, 5, -5], &[1, 4, 2, 7, 10, 11, 9, 6]),
    (&[-10, 8, 6, 11, 14, 11, 13, -2, -15, 17, 3, 15, -15, -16, 3, 9, 2, -4, 4, -11, 12, -4, 13, -12, -16, 17, -14, 17, 5, -7, -16, -3, -4, -16, 14, 14, -10, 9, -3, -8, -17, 5, -12, -9], &[2, 6, 5, 14, 13]),
    (&[14, 12, -14, 11, 5, -13, 14, 11, 3, -5, 11, -11, 1, -5, -6, 9, 12, 7, 6, -2, 10, -6, -1, 14, -7, 5, -7, 4, 8, 7, -14, 8], &[3, 5, 8, 7, 4, 10, 12, 9]),
    (&[3, 2, 2, 6, -4, -4, 2, -2, -4, 2, 1, 2, 1], &[1, 2, 6, 3]),
    (&[4, 3, 1, 5, -4, 2, 2, -3, -5, -4, -3, 5, 3, -3], &[1, 5, 2]),
    (&[1, -9, 7, 6, 4, 9, -1, 7, 3, -9, 7, 11, -3, -11, 10, 7, -11, 7, -11, -9, 6, 5, 6, 7, 2, 10, -8, 5, -5, 9, 3, 9], &[2, 4, 3, 7, 10, 9, 6]),
    (&[-5, 2, 2, 2, 3, 1, -6, -5, -2, -1, 6, 1, -5, 6, -5, 6, 5, 7], &[1, 5, 7, 6, 3]),
    (&[-5, 8, 11, -4, 7, -2, 10, 1, 7, 5, -6, 4, -8, 5, 5, -7, 8, 2, 2, 1, 2, -5, -5, -9, -1, -6, -10, 10, 9, -4, -1, 9], &[2, 9, 11, 10, 8]),
    (&[13, 13, -10, 13, 1, -4, 4, -2, -6, -10, 13, -4, -7, 9, 12, 6, 9, -13, 7, -12, 3, -5, -9, -4, -8, -3, 6, -13, 2, -1, -9, -13, -4, 11, 4, -9, -7, -8, -5], &[4, 2, 11, 6]),
    (&[11, 5, 5, 11, 7, 13, -4, 3, 7, 5, -1, -5, 14, 10, -12, 3, 1, 7, 7, 7, 1, 4, 5, -9, -7, 6, 7, 11, 12, 1, 3, -1, 9, 14, 9], &[4, 3, 6, 9, 7, 5, 12, 11, 14, 13, 10]),
    (&[14, -10, 7, -15, 2, 6, -5, 11, -13, 16, 2, 14, -9, -14, 12, -9, 15, -12, -8, -13, 3, 4, 3, 5, -14, 7, 8, -8, 12, -3, -13, -10, 5, -11], &[2, 6, 5, 4, 12, 16, 15, 7]),
    (&[5, 7, 5, 6, -4, 1, 5, 4, -5, 5, -6, -5, 4, 6, -4, -1, 4, -6, 1, 2, 4], &[2, 1, 7, 4]),
];

/// Return the keys of `tree` in post-order.
fn keys(tree: &MonitorLinkTree<ProcessId, u64>) -> Vec<u64> {
    tree.post_order().map(|(key, _)| key.0).collect()
}

/// Check the links and the red-black rules of `tree`, and return its
/// size.
fn check(tree: &MonitorLinkTree<ProcessId, u64>) -> usize {
    fn black_height(
        tree: &MonitorLinkTree<ProcessId, u64>,
        node: Option<usize>,
        parent: Option<usize>,
    ) -> usize {
        let Some(node) = node else { return 1 };
        let entry = &tree.nodes[node];
        assert_eq!(entry.parent, parent);
        assert!(!entry.red || !tree.is_red(parent));
        let left = black_height(tree, entry.left, Some(node));
        assert_eq!(left, black_height(tree, entry.right, Some(node)));
        left + usize::from(!entry.red)
    }
    assert!(!tree.is_red(tree.root));
    black_height(tree, tree.root, None);
    let mut keys = keys(tree);
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), tree.nodes.len());
    keys.len()
}

#[test]
fn walks_links_in_the_erts_post_order_after_inserts_and_deletes() {
    for (operations, expected) in ERTS_ORDERS {
        let mut tree = MonitorLinkTree::default();
        for &operation in *operations {
            let key = ProcessId(operation.unsigned_abs());
            if operation > 0 {
                tree.insert(key, 0);
            } else {
                tree.remove(&key);
            }
            check(&tree);
        }
        assert_eq!(keys(&tree), *expected, "{operations:?}");
    }
}

#[test]
fn walks_seven_ascending_links_as_erts() {
    let mut tree = MonitorLinkTree::default();
    for key in 1..=7 {
        tree.insert(ProcessId(key), 0);
    }
    assert_eq!(keys(&tree), [1, 3, 5, 7, 6, 4, 2]);
}

#[test]
fn continues_a_walk_at_its_position() {
    let mut tree = MonitorLinkTree::default();
    for key in 1..=7 {
        tree.insert(ProcessId(key), 0);
    }
    for done in 0..7 {
        let mut walk = tree.post_order();
        walk.by_ref().take(done).for_each(drop);
        let rest: Vec<u64> = tree
            .post_order_from(walk.position().unwrap())
            .map(|(key, _)| key.0)
            .collect();
        assert_eq!(rest, [1, 3, 5, 7, 6, 4, 2][done..], "{done}");
    }
    let mut walk = tree.post_order();
    walk.by_ref().for_each(drop);
    assert_eq!(walk.position(), None);
    assert_eq!(
        MonitorLinkTree::<ProcessId, u64>::default()
            .post_order()
            .position(),
        None
    );
}

#[test]
fn keeps_the_node_of_a_key_that_it_has() {
    let mut tree = MonitorLinkTree::default();
    assert!(tree.is_empty());
    assert_eq!(tree.insert(ProcessId(1), 0), None);
    assert_eq!(tree.insert(ProcessId(2), 0), None);
    assert_eq!(tree.insert(ProcessId(1), 5), Some(0));
    *tree.get_mut(&ProcessId(2)).unwrap() = 7;
    assert_eq!(tree.get(&ProcessId(2)), Some(&7));
    assert_eq!(tree.get_mut(&ProcessId(3)), None);
    assert_eq!(check(&tree), 2);
    assert_eq!(tree.remove(&ProcessId(3)), None);
    assert_eq!(tree.remove(&ProcessId(1)), Some(5));
    assert_eq!(tree.remove(&ProcessId(2)), Some(7));
    assert!(tree.is_empty());
}

#[test]
fn compares_references_as_erts_memory_bytes() {
    assert_eq!(Reference(1).compare(Reference(2)), Ordering::Less);
    assert_eq!(Reference(255).compare(Reference(256)), Ordering::Greater);
    assert_eq!(Reference(256).compare(Reference(256)), Ordering::Equal);
    // N0 holds the low 18 bits; the next bits are in N1 after the scheduler
    // number 1, so the pid marker of N1 compares before them.
    let pid = Reference(0).pid_reference();
    assert_eq!(
        super::memory_bytes(pid),
        [[0, 0, 0, 0], [1, 0, 1, 0], [0, 0, 0, 0]]
    );
    assert_eq!(pid.compare(Reference(1 << 32)), Ordering::Greater);
    assert_eq!(pid.compare(Reference(1 << 18)), Ordering::Less);
}
