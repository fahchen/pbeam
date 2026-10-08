//! The work units of the ERTS hashmap build of `maps:from_list/1`
//! (`erl_map.c:321-594`, `764-1182`; `utils.c:3499-3689`).
//!
//! ERTS builds a hashmap of more than 32 pairs with code that the Yielding
//! C Fun (YCF) tool makes yieldable (`MAPS_YCF_FLAGS` and `UTILS_YCF_FLAGS`
//! in `erts/emulator/Makefile.in:730-751`). A function given with `-f`
//! uses one unit at the start of each loop body, and more in a nested loop
//! ([`loop_steps`]). `YCF_CONSUME_REDS(N)` uses `N` units. The BIF converts the units to reductions (see the BIF
//! `maps:from_list/1`). This module gives the units of the build in the
//! order that ERTS uses them. It does not build the map: [`Map::insert`]
//! gives the same map.
//!
//! `erts_qsort` picks each pivot from a hash of an address: of the compare
//! function, of the array, and of a scheduler random number for more than
//! 128 items (`utils.c:3522-3526`, `3686-3687`). These values change
//! between ERTS runs, so the ERTS units of the sort change between runs.
//! PBeam uses the address 0 for the compare function and for the start of
//! the array, the offset of each part in the array, and the seed 1.

use std::cmp::Ordering;

use super::{HAMT_MAX_LEVEL, SLOT_BITS, SLOT_MASK};
use crate::error::BeamError;
use crate::term::{Term, hash::internal_hash};

/// The size of an ERTS `hxnode_t` in bytes: the hash, `skip`, `i`, and
/// `val` words (`erl_map.c:81-86`). The pivot hash reads the address of a
/// part of the array.
const HXNODE_BYTES: u64 = 32;
/// The largest part that `erts_qsort_helper` sorts with its selection sort
/// (`qsort_cut_off`, `utils.c:3635`).
const QSORT_CUT_OFF: usize = 16;

/// One `hxnode_t` of the build: the swizzled hash, the list position of the
/// pair, the index of its key, and the number of nodes of its collision
/// group.
#[derive(Clone, Copy)]
struct HxNode {
    /// The map hash with its 4-bit slots in reverse order
    /// (`swizzle_map_hash`, `erl_map.c:4170-4179`), so the root slot is in
    /// the top bits.
    hx: u64,
    /// The position of the pair in the list (`i`).
    position: usize,
    /// The number of nodes of the group that starts here (`skip`).
    skip: usize,
}

/// Return the units of the hashmap build of `maps:from_list/1` for a list
/// of more than 32 pairs, in the order that ERTS uses them:
///
/// 1. One unit for each pair of the hash loop of
///    `hashmap_from_validated_list` (`erl_map.c:514`).
/// 2. The units of `erts_qsort` of the hashes ([`qsort_units`]).
/// 3. The loops of `hashmap_from_unsorted_array` that remove the pairs of a
///    key that comes again (`erl_map.c:777-823`). The last pair of a key
///    wins.
/// 4. The loops of `hashmap_from_sorted_unique_array` and
///    `hashmap_from_chunked_array`, when more than one key remains
///    (`erl_map.c:857-1158`).
///
/// The hash of each key is the ERTS internal hash, as `erts_map_hash` gives
/// it in a release build (`erl_term_hashing.c:2311-2319`). It also covers
/// the keys that the PBeam hashmap does not support yet, so a list of more
/// than 32 pairs of such a key that gives a flatmap has its ERTS cost.
///
/// `keys` are the keys of the pairs in list order. The check loop of
/// `maps_from_list_1_helper` (`erl_map.c:332`) comes before these units,
/// and the BIF adds it.
///
/// # Examples
///
/// ```rust,ignore
/// let keys: Vec<Term> = (1..=40).map(Term::Integer).collect();
/// let units = from_list_units(&keys.iter().collect::<Vec<_>>())?;
/// // 40 hash units come first.
/// assert!(units[..40].iter().all(|&unit| unit == 1));
/// ```
///
/// # Errors
///
/// Return a runtime error when two keys with equal hashes have no PBeam
/// term order.
pub(crate) fn from_list_units(keys: &[&Term]) -> Result<Vec<u32>, BeamError> {
    let mut nodes: Vec<HxNode> = keys
        .iter()
        .enumerate()
        .map(|(position, key)| HxNode {
            hx: swizzle(internal_hash(key)),
            position,
            skip: 1,
        })
        .collect();
    let mut units = vec![1; keys.len()];
    // hxnodecmp: a higher hash comes first (`erl_map.c:1171-1182`).
    qsort_units(&mut nodes, 0, &mut units, &mut |a, b| Ok(b.hx.cmp(&a.hx)))?;
    let unique = unique_units(&mut nodes, keys, &mut units)?;
    if unique > 1 {
        nodes.truncate(unique);
        sorted_unique_units(&mut nodes, &mut units);
    }
    Ok(units)
}

/// Return the hash with its 4-bit slots in reverse order, as
/// `swizzle_map_hash` does (`erl_map.c:4170-4179`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(swizzle(0x0123_4567_89AB_CDEF), 0xFEDC_BA98_7654_3210);
/// ```
fn swizzle(hash: u64) -> u64 {
    let bytes = hash.swap_bytes();
    ((bytes & 0xF0F0_F0F0_F0F0_F0F0) >> 4) | ((bytes & 0x0F0F_0F0F_0F0F_0F0F) << 4)
}

/// Return the slot of a swizzled hash at a level (`maskval`,
/// `erl_map.c:109`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(mask_slot(0xF000_0000_0000_0001, 0), 0xF);
/// assert_eq!(mask_slot(0xF000_0000_0000_0001, 15), 1);
/// ```
fn mask_slot(hx: u64, level: u32) -> u64 {
    (hx >> ((HAMT_MAX_LEVEL - 1 - level) * SLOT_BITS)) & SLOT_MASK
}

/// Return the number of equal leading slots of two swizzled hashes
/// (`cdepth`, `erl_map.c:108`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(common_depth(0xF100_0000_0000_0000, 0xF200_0000_0000_0000), 1);
/// ```
fn common_depth(first: u64, second: u64) -> u32 {
    (first ^ second).leading_zeros() / SLOT_BITS
}

/// Return the hash of `erts_sched_local_random_hash_64_to_32_shift`
/// (`erl_process.h:3037-3047`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(hash_64_to_32_shift(0), 720_020_139);
/// ```
fn hash_64_to_32_shift(key: u64) -> u32 {
    let mut key = (!key).wrapping_add(key << 18);
    key ^= key >> 31;
    key = key.wrapping_add(key << 2).wrapping_add(key << 4);
    key ^= key >> 11;
    key = key.wrapping_add(key << 6);
    key ^= key >> 22;
    key as u32
}

/// Return the number of bits of a part size, as `erts_fit_in_bits_int64`
/// gives it for a size of 1 or more (`utils.c:369-390`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!((fit_in_bits(1), fit_in_bits(15), fit_in_bits(16)), (1, 4, 5));
/// ```
fn fit_in_bits(size: usize) -> u32 {
    usize::BITS - size.leading_zeros()
}

/// Sort `nodes`, which start at node `offset` of the array, as `erts_qsort`
/// does, and add its units (`utils.c:3499-3689`):
///
/// - A part of at most 16 nodes: the selection sort of
///   `erts_qsort_insertion_sort`, then `YCF_CONSUME_REDS(n * bits(n) * 2)`.
/// - A larger part: one unit for each loop step of
///   `erts_qsort_partion_array`, then the first part and the second part
///   with the seed plus 1. The pivot is
///   `hash(seed + compare + base - n) % n` (see the module documentation
///   for the values).
///
/// # Examples
///
/// ```rust,ignore
/// qsort_units(&mut nodes, 0, &mut units, &mut |a, b| Ok(b.hx.cmp(&a.hx)))?;
/// ```
///
/// # Errors
///
/// Return the error of `compare`.
fn qsort_units(
    nodes: &mut [HxNode],
    offset: usize,
    units: &mut Vec<u32>,
    compare: &mut impl FnMut(&HxNode, &HxNode) -> Result<Ordering, BeamError>,
) -> Result<(), BeamError> {
    // The parts that remain, as (start, length, seed). ERTS recurses, first
    // part first; the stack keeps that order.
    let mut parts = vec![(0, nodes.len(), 1_u64)];
    while let Some((start, length, seed)) = parts.pop() {
        let part = &mut nodes[start..start + length];
        if length <= QSORT_CUT_OFF {
            selection_sort(part, compare)?;
            units.push(length as u32 * fit_in_bits(length) * 2);
            continue;
        }
        let base = (offset + start) as u64 * HXNODE_BYTES;
        let key = seed.wrapping_add(base).wrapping_sub(length as u64);
        let pivot = hash_64_to_32_shift(key) as usize % length;
        let (first_end, second_start) = partition(part, pivot, units, compare)?;
        parts.push((start + second_start, length - second_start, seed + 1));
        parts.push((start, first_end, seed + 1));
    }
    Ok(())
}

/// Sort a part as `erts_qsort_insertion_sort` does (`utils.c:3590-3618`):
/// put the smallest remaining node first, then the next, with a swap only
/// for a smaller node. It uses no units.
///
/// # Examples
///
/// ```rust,ignore
/// selection_sort(&mut nodes[..4], &mut compare)?;
/// ```
///
/// # Errors
///
/// Return the error of `compare`.
fn selection_sort(
    part: &mut [HxNode],
    compare: &mut impl FnMut(&HxNode, &HxNode) -> Result<Ordering, BeamError>,
) -> Result<(), BeamError> {
    for start in 0..part.len().saturating_sub(1) {
        let mut smallest = start;
        for current in start + 1..part.len() {
            if compare(&part[current], &part[smallest])?.is_lt() {
                smallest = current;
            }
        }
        part.swap(start, smallest);
    }
    Ok(())
}

/// Partition a part around the node at `pivot` as
/// `erts_qsort_partion_array` does (`utils.c:3499-3588`), with one unit for
/// each loop step. Return the end of the first part and the start of the
/// second part.
///
/// The fast path keeps two parts. At the first node equal to the pivot, the
/// slow path continues with three parts.
///
/// # Examples
///
/// ```rust,ignore
/// let (first_end, second_start) = partition(part, 3, &mut units, &mut compare)?;
/// ```
///
/// # Errors
///
/// Return the error of `compare`.
fn partition(
    part: &mut [HxNode],
    pivot: usize,
    units: &mut Vec<u32>,
    compare: &mut impl FnMut(&HxNode, &HxNode) -> Result<Ordering, BeamError>,
) -> Result<(usize, usize), BeamError> {
    part.swap(0, pivot);
    let mut second = part.len();
    let mut current = 1;
    while current != second {
        units.push(1);
        match compare(&part[current], &part[0])? {
            Ordering::Less => current += 1,
            Ordering::Equal => return slow_partition(part, current, second, units, compare),
            Ordering::Greater => {
                second -= 1;
                part.swap(current, second);
            }
        }
    }
    part.swap(0, second - 1);
    Ok((second - 1, second))
}

/// Continue a partition with three parts from the first node equal to the
/// pivot, as the slow path of `erts_qsort_partion_array` does
/// (`utils.c:3551-3587`). Return the start and the end of the pivot part.
///
/// # Examples
///
/// ```rust,ignore
/// let (pivot_start, pivot_end) = slow_partition(part, 5, 9, &mut units, &mut compare)?;
/// ```
///
/// # Errors
///
/// Return the error of `compare`.
fn slow_partition(
    part: &mut [HxNode],
    equal: usize,
    mut last: usize,
    units: &mut Vec<u32>,
    compare: &mut impl FnMut(&HxNode, &HxNode) -> Result<Ordering, BeamError>,
) -> Result<(usize, usize), BeamError> {
    let mut pivot_start = equal - 1;
    let mut pivot_end = equal + 1;
    part.swap(0, pivot_start);
    while pivot_end != last {
        units.push(1);
        match compare(&part[pivot_end], &part[pivot_end - 1])? {
            Ordering::Equal => pivot_end += 1,
            Ordering::Less => {
                part.swap(pivot_start, pivot_end);
                pivot_start += 1;
                pivot_end += 1;
            }
            Ordering::Greater => {
                last -= 1;
                part.swap(pivot_end, last);
            }
        }
    }
    Ok((pivot_start, pivot_end))
}

/// Add the units of the loop steps of one loop of a YCF `-f` function.
/// YCF puts `YCF_CONSUME_REDS(1)` at the start of each loop body
/// (`consume_reds_calls_inserter`, `ycf_yield_fun.c:648-679`). It inserts
/// into a loop body, and then again into the new body of each enclosing
/// loop, so a loop in `nesting` enclosing loops uses `2^nesting` units for
/// each step. The YCF output of the pinned source shows this: for example,
/// the innermost loop of `hashmap_from_unsorted_array` has 4
/// `YCF_CONSUME_REDS(1)` statements. Each unit has its own yield check.
///
/// # Examples
///
/// ```rust,ignore
/// let mut units = Vec::new();
/// loop_steps(&mut units, 3, 1);
/// assert_eq!(units, [1; 6]);
/// ```
fn loop_steps(units: &mut Vec<u32>, steps: usize, nesting: u32) {
    units.extend(std::iter::repeat_n(1, steps << nesting));
}

/// Remove the nodes of a key that comes again from the sorted `nodes`, as
/// `hashmap_from_unsorted_array` does (`erl_map.c:777-823`), and add the
/// units of its loops ([`loop_steps`]). A group of equal hashes is sorted
/// by key with `erts_qsort`, and the node of the last pair of each key
/// stays. Return the number of nodes that stay at the start of `nodes`.
///
/// # Examples
///
/// ```rust,ignore
/// let unique = unique_units(&mut nodes, &keys, &mut units)?;
/// ```
///
/// # Errors
///
/// Return a runtime error when two keys of equal hashes have no PBeam term
/// order.
fn unique_units(
    nodes: &mut [HxNode],
    keys: &[&Term],
    units: &mut Vec<u32>,
) -> Result<usize, BeamError> {
    let count = nodes.len();
    let (mut index, mut kept) = (0, 0);
    while index + 1 < count {
        loop_steps(units, 1, 0);
        if nodes[index].hx == nodes[index + 1].hx {
            let mut end = index + 2;
            while end < count && nodes[index].hx == nodes[end].hx {
                loop_steps(units, 1, 1);
                end += 1;
            }
            // hxnodecmpkey: the term order of the keys (`erl_map.c:1160-1169`).
            let mut by_key = |a: &HxNode, b: &HxNode| keys[a.position].compare(keys[b.position]);
            qsort_units(&mut nodes[index..end], index, units, &mut by_key)?;
            while index < end {
                loop_steps(units, 1, 1);
                let mut last = index;
                index += 1;
                while index < end && keys[nodes[index].position] == keys[nodes[last].position] {
                    loop_steps(units, 1, 2);
                    if nodes[index].position > nodes[last].position {
                        last = index;
                    }
                    index += 1;
                }
                nodes[kept] = nodes[last];
                kept += 1;
            }
            continue;
        }
        nodes[kept] = nodes[index];
        kept += 1;
        index += 1;
    }
    if index < count {
        nodes[kept] = nodes[index];
        kept += 1;
    }
    Ok(kept)
}

/// Add the units of `hashmap_from_sorted_unique_array` and
/// `hashmap_from_chunked_array` for the sorted nodes of distinct keys
/// (`erl_map.c:857-1158`). Nodes with equal hashes form one collision
/// group.
///
/// # Examples
///
/// ```rust,ignore
/// sorted_unique_units(&mut nodes, &mut units);
/// ```
fn sorted_unique_units(nodes: &mut [HxNode], units: &mut Vec<u32>) {
    let count = nodes.len();
    let mut index = 0;
    let mut groups = 1;
    while index + 1 < count {
        loop_steps(units, 1, 0);
        if nodes[index].hx == nodes[index + 1].hx {
            let mut end = index + 2;
            while end < count && nodes[index].hx == nodes[end].hx {
                loop_steps(units, 1, 1);
                end += 1;
            }
            let colliders = end - index;
            loop_steps(units, colliders, 1);
            nodes[index].skip = colliders;
            index = end;
            if index < count {
                groups += 1;
            }
            continue;
        }
        nodes[index].skip = 1;
        groups += 1;
        index += 1;
    }
    chunked_units(nodes, groups, units);
}

/// Add the units of `hashmap_from_chunked_array` (`erl_map.c:924-1158`) for
/// `groups` groups of nodes ([`loop_steps`]). The walk keeps the bitmaps of
/// the nodes on a stack, as ERTS does, because the number of children of a
/// node gives the steps of a loop. A child word on the stack is 0.
///
/// # Examples
///
/// ```rust,ignore
/// chunked_units(&nodes, groups, &mut units);
/// ```
fn chunked_units(nodes: &[HxNode], groups: usize, units: &mut Vec<u32>) {
    let top = HAMT_MAX_LEVEL - 1;
    if groups == 1 {
        // A chain of nodes down to the collision node (`erl_map.c:965-983`).
        loop_steps(units, top as usize, 0);
        return;
    }
    let mut stack: Vec<u64> = Vec::new();
    let pop = |stack: &mut Vec<u64>| stack.pop().expect("the build pushed a word");
    let mut index = 0;
    let mut level = 0;
    let mut previous = nodes[index].hx;
    let mut current = nodes[index + nodes[index].skip].hx;
    let mut slot = mask_slot(previous, level);
    while slot == mask_slot(current, level) {
        loop_steps(units, 1, 0);
        stack.push(1 << slot);
        level += 1;
        slot = mask_slot(previous, level);
    }
    if nodes[index].skip > 1 {
        loop_steps(units, (top - level) as usize, 0);
    }
    stack.extend([0, 1 << slot]);
    for _ in 0..groups - 2 {
        loop_steps(units, 1, 0);
        let mut bitmap = pop(&mut stack);
        index += nodes[index].skip;
        let next = nodes[index + nodes[index].skip].hx;
        let next_level = common_depth(current, next);
        if nodes[index].skip > 1 {
            loop_steps(units, (top - level.max(next_level)) as usize, 1);
        }
        if level < next_level {
            while level < next_level {
                loop_steps(units, 1, 1);
                stack.push(bitmap | (1 << mask_slot(current, level)));
                level += 1;
                bitmap = 0;
            }
            stack.extend([0, 1 << mask_slot(current, level)]);
        } else if level == next_level {
            stack.extend([0, bitmap | (1 << mask_slot(current, level))]);
        } else {
            while next_level != level {
                loop_steps(units, 1, 1);
                bitmap |= 1 << mask_slot(current, level);
                let children = bitmap.count_ones() as usize - 1;
                loop_steps(units, children, 2);
                for _ in 0..children {
                    pop(&mut stack);
                }
                bitmap = pop(&mut stack);
                level -= 1;
            }
            stack.extend([0, bitmap]);
        }
        previous = current;
        current = next;
        level = next_level;
    }
    let mut last_level = common_depth(previous, current);
    index += nodes[index].skip;
    if nodes[index].skip > 1 {
        loop_steps(units, (top - last_level) as usize, 0);
    }
    let mut bitmap = pop(&mut stack);
    while last_level > 0 {
        loop_steps(units, 1, 0);
        bitmap |= 1 << mask_slot(current, last_level);
        let children = bitmap.count_ones() as usize - 1;
        loop_steps(units, children, 1);
        for _ in 0..children {
            pop(&mut stack);
        }
        bitmap = pop(&mut stack);
        last_level -= 1;
    }
    bitmap |= 1 << mask_slot(current, 0);
    loop_steps(units, bitmap.count_ones() as usize - 1, 0);
}

#[cfg(test)]
mod tests {
    use super::{
        HxNode, common_depth, fit_in_bits, from_list_units, hash_64_to_32_shift, mask_slot,
        partition, sorted_unique_units, swizzle,
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
}
