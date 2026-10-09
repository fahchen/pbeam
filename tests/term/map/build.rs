use super::{
    HxNode, common_depth, fit_in_bits, from_list_units, hash_64_to_32_shift, mask_slot, partition,
    sorted_unique_units, swizzle,
};
use crate::term::Term;

/// Return sorted nodes of the swizzled hashes, with the list positions
/// 0 onwards.
fn nodes(hashes: &[u64]) -> Vec<HxNode> {
    hashes
        .iter()
        .enumerate()
        .map(|(position, &hx)| HxNode {
            hx,
            position,
            skip: 1,
        })
        .collect()
}

/// Return the units of `hashmap_from_sorted_unique_array` and
/// `hashmap_from_chunked_array` for the sorted hashes.
fn build_units(hashes: &[u64]) -> usize {
    let mut units = Vec::new();
    sorted_unique_units(&mut nodes(hashes), &mut units);
    units.len()
}

#[test]
fn counts_the_loops_of_collision_groups() {
    // Equal 64-bit hashes of distinct keys make a collision group. The
    // counts follow the loops of `erl_map.c:857-1158` and their YCF
    // units, with the root slots 15, 8, and 1.
    let (a, b, c) = (
        0xF000_0000_0000_0000,
        0x8000_0000_0000_0000,
        0x1000_0000_0000_0000,
    );
    // One group: the sorted loop (1), the colliders (2 * 2), and the
    // chain of 15 nodes.
    assert_eq!(build_units(&[a, a]), 1 + 4 + 15);
    // A group of three: one step of the loop that finds its end (2),
    // and the colliders (3 * 2).
    assert_eq!(build_units(&[a, a, a]), 1 + 2 + 6 + 15);
    // A first group of two: the sorted loop (1), the colliders (4), the
    // chain below the root (15), and the root with two children (1).
    assert_eq!(build_units(&[a, a, b]), 1 + 4 + 15 + 1);
    // A last group of two: as above, with the chain after the loop.
    assert_eq!(build_units(&[a, b, b]), 2 + 4 + 15 + 1);
    // A middle group of two: the sorted loop (2), the colliders (4), one
    // step of the chunk loop (1), the chain in it (2 * 15), and the root
    // with three children (2).
    assert_eq!(build_units(&[a, b, b, c]), 2 + 4 + 1 + 30 + 2);
}

#[test]
fn partitions_with_three_parts_from_an_equal_item() {
    // The pivot 5 meets an equal item at once, so the slow path keeps
    // a pivot part: 9 goes before it and 1 after it
    // (`utils.c:3551-3587`). A higher hash comes first.
    let mut part = nodes(&[5, 5, 9, 1]);
    let mut units = Vec::new();
    let mut compare = |a: &HxNode, b: &HxNode| Ok(b.hx.cmp(&a.hx));
    assert_eq!(
        partition(&mut part, 0, &mut units, &mut compare).unwrap(),
        (1, 3)
    );
    let order: Vec<u64> = part.iter().map(|node| node.hx).collect();
    assert_eq!((order, units.len()), (vec![9, 5, 5, 1], 3));
}

#[test]
fn computes_the_erts_helpers_of_the_build() {
    assert_eq!(swizzle(0x0123_4567_89AB_CDEF), 0xFEDC_BA98_7654_3210);
    assert_eq!(mask_slot(0xF000_0000_0000_0001, 0), 0xF);
    assert_eq!(mask_slot(0xF000_0000_0000_0001, 15), 1);
    assert_eq!(
        common_depth(0xF100_0000_0000_0000, 0xF200_0000_0000_0000),
        1
    );
    assert_eq!(
        (fit_in_bits(1), fit_in_bits(15), fit_in_bits(16)),
        (1, 4, 5)
    );
    assert_eq!(hash_64_to_32_shift(0), 720_020_139);
}

#[test]
fn counts_the_units_of_one_key_as_erts() {
    // ERTS 29.1.1: maps:from_list/1 of 33 pairs {1, 1} costs 8, and of
    // 400 pairs 99, with 40 units for each reduction. With one key, the
    // sort takes the slow path at once, so the pivot does not matter.
    let units = |count| {
        let keys = vec![Term::Integer(1); count];
        let keys: Vec<&Term> = keys.iter().collect();
        let units = from_list_units(&keys).unwrap();
        // The check loop of the BIF adds one unit for each pair.
        count + units.iter().map(|&unit| unit as usize).sum::<usize>()
    };
    assert_eq!(units(33) / 40, 8);
    assert_eq!(units(400) / 40, 99);
}
