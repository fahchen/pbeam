//! The layout of a BEAM map, as in ERTS OTP 29.1.1 (`erl_map.h`, `erl_map.c`).
//!
//! A map with at most 32 keys is a flatmap: its pairs are sorted by the
//! flatmap key order. A map with more than 32 keys is a hashmap: a hash
//! array mapped trie (HAMT) indexed by the ERTS map hash of each key. The
//! layout depends only on the set of keys, so two equal maps have the same
//! pairs in the same order.
//!
//! Supported keys:
//!
//! - hashmap: an atom, an integer in the ERTS small-integer range, `[]`,
//!   a list whose elements are such keys, and an improper list whose
//!   elements are such keys and whose tail is an atom or a small integer;
//! - flatmap: each hashmap key, and any integer.
//!
//! As in ERTS, a flatmap orders two atom keys by their atom index and each
//! other two keys in the term order, so an atom in a list compares by name.
//! A hashmap hashes a key with the ERTS internal hash, and an atom hashes by
//! its atom word `(index << 6) | 0xB`. Thus the pair order of a map is the
//! ERTS order, which BEAM code sees through operations such as
//! `maps:keys/1`. Other key types need the ERTS hash for bignums, tuples,
//! binaries, maps, funs, pids, and references, which PBeam does not have
//! yet. So does an improper list with such a tail, for example `[1|{}]`.

use std::cmp::Ordering;

use super::Term;
use crate::error::{BeamError, error};

/// The largest number of keys in a flatmap (`MAP_SMALL_MAP_LIMIT` in a
/// release build of ERTS). A map with more keys is a hashmap.
const SMALL_MAP_LIMIT: usize = 32;
/// The number of hash bits that select a slot at one HAMT level.
const SLOT_BITS: u32 = 4;
/// The number of HAMT levels that a 64-bit hash fills (`HAMT_MAX_LEVEL`).
const HAMT_MAX_LEVEL: u32 = u64::BITS / SLOT_BITS;
/// The mask of the hash bits that select a slot.
const SLOT_MASK: u64 = 0xF;
/// The width of the tag of an ERTS small integer word (`_TAG_IMMED1_SIZE`).
const SMALL_INTEGER_TAG_BITS: u32 = 4;
/// The tag bits of an ERTS small integer word (`_TAG_IMMED1_SMALL`).
const SMALL_INTEGER_TAG: u64 = 0xF;
/// The width of the tag of an ERTS atom word (`_TAG_IMMED2_SIZE`).
const ATOM_TAG_BITS: u32 = 6;
/// The tag bits of an ERTS atom word (`_TAG_IMMED2_ATOM`).
const ATOM_TAG: u64 = 0xB;
/// The ERTS word of `[]` (`NIL`, `erl_term.h:293`).
const NIL_WORD: u64 = 0x3B;
/// The type of an immediate term in the ERTS internal hash
/// (`IHASH_TYPE_IMMEDIATE`, `erl_term_hashing.c:1637`).
const IHASH_TYPE_IMMEDIATE: u64 = 1;
/// The type of a list head in the ERTS internal hash (`IHASH_TYPE_CAR`).
const IHASH_TYPE_CAR: u64 = 3;
/// The type of a list end in the ERTS internal hash (`IHASH_TYPE_CDR`).
const IHASH_TYPE_CDR: u64 = 4;
/// The type of a chunk of list bytes in the ERTS internal hash
/// (`IHASH_TYPE_STRING`).
const IHASH_TYPE_STRING: u64 = 5;
/// The largest number of list bytes in one chunk of the ERTS internal hash.
const IHASH_CHUNK_BYTES: u64 = 4;
/// The multiplier constants of the ERTS internal hash (`IHASH_C1`,
/// `IHASH_C2`, `erl_term_hashing.c:1665-1666`).
const IHASH_C1: u64 = 0x87C3_7B91_1142_53D5;
const IHASH_C2: u64 = 0x4CF5_AD43_2745_937F;
/// The number of path elements in one ERTS bignum digit
/// (`PATH_ELEMS_PER_DIGIT`, `erl_map.c:3635`). A path element has
/// `SLOT_BITS` bits.
const PATH_ELEMS_PER_DIGIT: usize = (u64::BITS / SLOT_BITS) as usize;

/// The direction of an `erts_internal:map_next/3` walk over a map.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Direction {
    /// The `iterator` mode: from the first pair of the map order onwards.
    Forward,
    /// The list mode: from the last pair of the map order backwards.
    Backward,
}

/// The pairs of one `erts_internal:map_next/3` call and the path of the
/// next call.
pub(crate) struct Chunk<'a> {
    /// The pairs of the chunk, in the map order.
    pub(crate) pairs: Vec<(&'a Term, &'a Term)>,
    /// The path of the first pair that the chunk does not hold, or `None`
    /// when the chunk ends the walk.
    pub(crate) next: Option<i64>,
}

/// The entries of one ERTS hashmap node, as the walk of
/// `erts_internal:map_next/3` sees them: a HAMT node, or a collision child,
/// which ERTS keeps as a collision node of leaves.
#[derive(Clone, Copy)]
enum Entries<'a> {
    /// The children of a HAMT node.
    Node(&'a Node),
    /// The pairs of a collision child.
    Collision(&'a [(Term, Term)]),
}

/// One entry of a hashmap node: a pair, or a node one level down.
enum Entry<'a> {
    /// A key and its value.
    Leaf(&'a Term, &'a Term),
    /// The entries one level down.
    Inner(Entries<'a>),
}

/// A BEAM map: a flatmap up to 32 keys and a HAMT above 32 keys, as in
/// ERTS. The layout is private, so every map is in its canonical form.
pub struct Map {
    /// The flatmap or hashmap form of the map.
    repr: Repr,
}

/// The two forms of a map. The number of keys selects the form.
enum Repr {
    /// At most 32 pairs, sorted by the flatmap key order.
    Flat(Vec<(Term, Term)>),
    /// A HAMT, only for 33 or more keys.
    Hash(Hamt),
}

/// A hash array mapped trie of map pairs.
struct Hamt {
    /// The node at level 0.
    root: Node,
    /// The number of keys in the trie.
    size: usize,
}

/// A HAMT node. Bit `i` of `bitmap` is set when slot `i` holds a child.
/// `children` holds one child per set bit, in ascending slot order.
struct Node {
    /// The occupied slots.
    bitmap: u16,
    /// The child of each occupied slot, in ascending slot order.
    children: Vec<Child>,
}

/// The child in one slot of a HAMT node.
enum Child {
    /// A key and its value.
    Leaf(Term, Term),
    /// A node one level down.
    Node(Node),
    /// The pairs of two or more keys with equal hashes, sorted in the term
    /// order. Only a node at the last level (`HAMT_MAX_LEVEL - 1`) holds a
    /// collision child, as an ERTS collision node is at level 16
    /// (`erl_map.h:205-210`).
    Collision(Vec<(Term, Term)>),
}

/// The state of the ERTS internal hash of a compound term
/// (`make_internal_hash`, `erl_term_hashing.c:1770-1782`): two 64-bit
/// MurmurHash3 lanes and the number of mix steps. The salt of a map hash is
/// 0, so both lanes start at 0.
#[derive(Default)]
struct InternalHash {
    /// The lane that `IHASH_MIX_ALPHA` changes.
    alpha: u64,
    /// The lane that `IHASH_MIX_BETA` changes.
    beta: u64,
    /// The number of mix steps (`hash_ticks`).
    ticks: u64,
}

/// One entry on the work stack of a list hash: the rest of a list whose
/// head is a list, or the marker of a list end (`IHASH_CDR_MARKER`).
enum Pending<'a> {
    /// The elements after a list head that the hash takes later, and the
    /// ERTS word of the tail of that list.
    Tail(&'a [Term], u64),
    /// The marker that a list head is the last element: the next `Tail`
    /// is empty.
    Cdr,
}

impl Map {
    /// Return an empty flatmap.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Map::new().len(), 0);
    /// ```
    pub(crate) fn new() -> Self {
        Map {
            repr: Repr::Flat(Vec::new()),
        }
    }

    /// Build a map from pairs in any order. Insert the pairs one by one;
    /// when a key occurs again, the last value wins, as `maps:from_list/1`
    /// does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = Map::from_pairs(vec![
    ///     (Term::static_atom("ok"), Term::Integer(1)),
    ///     (Term::Integer(2), Term::Integer(2)),
    /// ])?;
    /// let keys: Vec<_> = map.iter().map(|(key, _)| key.clone()).collect();
    /// assert_eq!(keys, [Term::Integer(2), Term::static_atom("ok")]);
    /// ```
    ///
    /// # Errors
    ///
    /// Return the first key that the map form does not support yet, for
    /// example a tuple key, or an integer outside the small-integer range in
    /// a map of more than 32 keys.
    pub fn from_pairs(pairs: Vec<(Term, Term)>) -> Result<Self, Term> {
        let mut map = Map::new();
        for (key, value) in pairs {
            map.insert(key, value)?;
        }
        Ok(map)
    }

    /// Return the number of keys.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))])?;
    /// assert_eq!(map.len(), 1);
    /// ```
    pub(crate) fn len(&self) -> usize {
        match &self.repr {
            Repr::Flat(pairs) => pairs.len(),
            Repr::Hash(hamt) => hamt.size,
        }
    }

    /// Return the value of a key, or `None` when the map does not hold the
    /// key. A flatmap compares each key with `==`, as ERTS does. A hashmap
    /// follows the slots of the key hash, and compares each key of a
    /// collision child with `==` (`erts_hashmap_get`, `erl_map.c:2424-2474`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))])?;
    /// assert_eq!(map.get(&Term::Integer(1)), Some(&Term::Integer(2)));
    /// assert_eq!(map.get(&Term::Tuple(vec![])), None);
    /// ```
    pub(crate) fn get(&self, key: &Term) -> Option<&Term> {
        let hamt = match &self.repr {
            Repr::Flat(pairs) => {
                return pairs
                    .iter()
                    .find(|(existing, _)| existing == key)
                    .map(|(_, value)| value);
            }
            Repr::Hash(hamt) => hamt,
        };
        // A key with no hash is not equal to any stored key.
        let hash = map_hash(key)?;
        let mut node = &hamt.root;
        let mut level = 0;
        loop {
            let child = node.child(slot(hash, level))?;
            match child {
                Child::Leaf(existing, value) => return (existing == key).then_some(value),
                Child::Node(inner) => node = inner,
                Child::Collision(pairs) => {
                    return pairs
                        .iter()
                        .find(|(existing, _)| existing == key)
                        .map(|(_, value)| value);
                }
            }
            level += 1;
        }
    }

    /// Return the value of a key for a change in place, or `None` when the
    /// map does not hold the key. The key lookup is the one of `get`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut map = Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))])?;
    /// *map.get_mut(&Term::Integer(1)).unwrap() = Term::Integer(3);
    /// assert_eq!(map.get(&Term::Integer(1)), Some(&Term::Integer(3)));
    /// assert!(map.get_mut(&Term::Integer(2)).is_none());
    /// ```
    pub(crate) fn get_mut(&mut self, key: &Term) -> Option<&mut Term> {
        let hamt = match &mut self.repr {
            Repr::Flat(pairs) => {
                return pairs
                    .iter_mut()
                    .find(|(existing, _)| existing == key)
                    .map(|(_, value)| value);
            }
            Repr::Hash(hamt) => hamt,
        };
        let hash = map_hash(key)?;
        let mut node = &mut hamt.root;
        let mut level = 0;
        loop {
            match node.child_mut(slot(hash, level))? {
                Child::Leaf(existing, value) => return (existing == key).then_some(value),
                Child::Node(inner) => node = inner,
                Child::Collision(pairs) => {
                    return pairs
                        .iter_mut()
                        .find(|(existing, _)| existing == key)
                        .map(|(_, value)| value);
                }
            }
            level += 1;
        }
    }

    /// Sort key and value pairs by the flatmap key order, as the ERTS loader
    /// sorts the pairs of a map instruction (`beam_load_map_key_sort`,
    /// `beam_transform_helpers.c:54-115`) with `erts_cmp_flatmap_keys`
    /// (`erl_utils.h:267-272`): two atoms compare by atom index, and each
    /// other two keys compare in the term order. `put_map_exact` then
    /// reports the first missing key in this order. The keys can be of any
    /// type, also a type that a PBeam map cannot hold.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut pairs = vec![
    ///     (Term::Tuple(vec![]), Term::Integer(0)),
    ///     (Term::static_atom("abs"), Term::Integer(1)),
    ///     (Term::static_atom("value"), Term::Integer(2)),
    /// ];
    /// Map::sort_pairs(&mut pairs)?;
    /// // value has atom index 745, abs has atom index 762, and an atom
    /// // comes before a tuple.
    /// assert_eq!(pairs[0].0, Term::static_atom("value"));
    /// assert_eq!(pairs[2].0, Term::Tuple(vec![]));
    /// ```
    ///
    /// # Errors
    ///
    /// Return a runtime error when the order depends on two different
    /// references, which have no PBeam term order. Compiled code cannot get
    /// this error: the ERTS loader sorts only literal keys, and a literal
    /// cannot hold a reference.
    pub(crate) fn sort_pairs(pairs: &mut [(Term, Term)]) -> Result<(), BeamError> {
        let mut failure = None;
        pairs.sort_by(|(left, _), (right, _)| {
            flatmap_order(left, right).unwrap_or_else(|error| {
                failure.get_or_insert(error);
                Ordering::Equal
            })
        });
        failure.map_or(Ok(()), Err)
    }

    /// Set a key to a value. Replace the value when the key exists. A
    /// flatmap that gets its 33rd key becomes a hashmap, as in ERTS.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut map = Map::new();
    /// map.insert(Term::Integer(1), Term::Integer(2))?;
    /// map.insert(Term::Integer(1), Term::Integer(3))?;
    /// assert_eq!(map.get(&Term::Integer(1)), Some(&Term::Integer(3)));
    /// assert!(map.insert(Term::Tuple(vec![]), Term::Integer(1)).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Return the first key that the map form does not support yet. The map
    /// does not change.
    pub(crate) fn insert(&mut self, key: Term, value: Term) -> Result<(), Term> {
        let pairs = match &mut self.repr {
            Repr::Flat(pairs) => pairs,
            Repr::Hash(hamt) => {
                let Some(hash) = map_hash(&key) else {
                    return Err(key);
                };
                hamt.insert(hash, key, value);
                return Ok(());
            }
        };
        if !matches!(key, Term::Integer(_)) && map_hash(&key).is_none() {
            return Err(key);
        }
        match pairs.binary_search_by(|(existing, _)| {
            flatmap_order(existing, &key).expect("supported map keys have a term order")
        }) {
            Ok(index) => pairs[index].1 = value,
            Err(index) if pairs.len() < SMALL_MAP_LIMIT => pairs.insert(index, (key, value)),
            Err(_) => {
                let hashes = pairs
                    .iter()
                    .map(|(existing, _)| existing)
                    .chain([&key])
                    .map(|candidate| map_hash(candidate).ok_or_else(|| candidate.clone()))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut hamt = Hamt {
                    root: Node::empty(),
                    size: 0,
                };
                let pairs = std::mem::take(pairs).into_iter().chain([(key, value)]);
                for (hash, (key, value)) in hashes.into_iter().zip(pairs) {
                    hamt.insert(hash, key, value);
                }
                self.repr = Repr::Hash(hamt);
            }
        }
        Ok(())
    }

    /// Iterate over the pairs in the ERTS order: flatmap key order for a
    /// flatmap, and depth-first ascending slot order for a hashmap, with the
    /// pairs of a collision child in the term order. BEAM code sees this
    /// order through operations such as `maps:keys/1`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let nil = || Term::List(vec![]);
    /// let map = Map::from_pairs(vec![(Term::Integer(2), nil()), (Term::Integer(1), nil())])?;
    /// let keys: Vec<_> = map.iter().map(|(key, _)| key.clone()).collect();
    /// assert_eq!(keys, [Term::Integer(1), Term::Integer(2)]);
    /// ```
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Term, &Term)> {
        // `pairs` holds the flatmap pairs, or the pairs of the current collision child.
        let (mut pairs, mut nodes) = match &self.repr {
            Repr::Flat(pairs) => (pairs.iter(), Vec::new()),
            Repr::Hash(hamt) => (Default::default(), vec![hamt.root.children.iter()]),
        };
        std::iter::from_fn(move || {
            loop {
                if let Some((key, value)) = pairs.next() {
                    return Some((key, value));
                }
                let Some(child) = nodes.last_mut()?.next() else {
                    nodes.pop();
                    continue;
                };
                match child {
                    Child::Leaf(key, value) => return Some((key, value)),
                    Child::Node(node) => nodes.push(node.children.iter()),
                    Child::Collision(collision) => pairs = collision.iter(),
                }
            }
        })
    }

    /// Move every key and value onto a work stack. `Term` uses this to drop
    /// a deep map without recursion.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut pending = Vec::new();
    /// Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))])?.drain_into(&mut pending);
    /// assert_eq!(pending, [Term::Integer(1), Term::Integer(2)]);
    /// ```
    pub(crate) fn drain_into(self, pending: &mut Vec<Term>) {
        let root = match self.repr {
            Repr::Flat(pairs) => {
                pending.extend(pairs.into_iter().flat_map(|(key, value)| [key, value]));
                return;
            }
            Repr::Hash(hamt) => hamt.root.children,
        };
        let mut nodes = vec![root];
        while let Some(children) = nodes.pop() {
            for child in children {
                match child {
                    Child::Leaf(key, value) => pending.extend([key, value]),
                    Child::Node(node) => nodes.push(node.children),
                    Child::Collision(pairs) => {
                        pending.extend(pairs.into_iter().flat_map(|(key, value)| [key, value]));
                    }
                }
            }
        }
    }

    /// Return the pairs of one `erts_internal:map_next/3` call from an
    /// integer path, as `erts_internal_map_next_3` does
    /// (`erl_map.c:3681-4135`). Return `None` when ERTS raises `badarg` for
    /// the path.
    ///
    /// A flatmap needs a small integer path from 0 to the map size, and it
    /// gives all pairs (`erl_map.c:3681-3708`). A hashmap reads the path as
    /// ERTS does: a small integer as its low 60 bits (`unsigned_val`), and an
    /// integer from 2^59 to 2^63 - 1 as the one digit of a positive bignum.
    /// A negative bignum or another term gives `None`. Each 4-bit element of
    /// the path, root first, selects an entry of a node: forward from the
    /// first entry for `Forward`, back from the last entry for `Backward`.
    /// After 16 elements, and after the last element, the path continues
    /// with element 0 down to a pair. An element that selects no entry gives
    /// `None`. The walk then takes up to `budget` pairs (at most
    /// the map size, at most 32 at path 0), but never part of a collision
    /// node (`erl_map.c:3902-3915`, `erl_map.c:3984-3997`). The next path
    /// selects the first pair that the chunk does not hold.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = Map::from_pairs((1..=40).map(|key| (Term::Integer(key), Term::Integer(key))).collect())?;
    /// let chunk = map.chunk(&Term::Integer(0), 1000, Direction::Backward)?.unwrap();
    /// // ERTS 29.1.1: the last 32 pairs and the path 43.
    /// assert_eq!((chunk.pairs.len(), chunk.next), (32, Some(43)));
    /// let rest = map.chunk(&Term::Integer(43), 1000, Direction::Backward)?.unwrap();
    /// assert_eq!((rest.pairs.len(), rest.next), (8, None));
    /// assert!(map.chunk(&Term::Integer(0xff), 1000, Direction::Backward)?.is_none());
    /// ```
    ///
    /// # Errors
    ///
    /// Return a runtime error when the next path has 15 elements and a
    /// value below 2^59, or 16 or more elements. ERTS returns a bignum for
    /// such a path (`erl_map.c:4058-4114`), which is not in normal form or
    /// is above 2^63 - 1, and which PBeam cannot hold.
    pub(crate) fn chunk(
        &self,
        path: &Term,
        budget: usize,
        direction: Direction,
    ) -> Result<Option<Chunk<'_>>, BeamError> {
        let hamt = match &self.repr {
            Repr::Flat(pairs) => {
                let size = pairs.len() as u64;
                let path = path.small_integer().map(unsigned_val);
                return Ok(path.filter(|path| *path <= size).map(|_| Chunk {
                    pairs: pairs.iter().map(|(key, value)| (key, value)).collect(),
                    next: None,
                }));
            }
            Repr::Hash(hamt) => hamt,
        };
        let (path, mut limit) = match path {
            Term::Integer(value) if path.small_integer().is_some() => {
                let path = unsigned_val(*value);
                let limit = if path == 0 {
                    SMALL_MAP_LIMIT
                } else {
                    usize::MAX
                };
                (path, limit)
            }
            // A positive bignum of one digit.
            Term::Integer(value) if *value > 0 => (*value as u64, usize::MAX),
            _ => return Ok(None),
        };
        limit = limit.min(budget).min(hamt.size);
        hamt.chunk(path, limit, direction)
    }
}

impl Hamt {
    /// Walk the trie from a path and take up to `limit` pairs, as the
    /// hashmap part of `erts_internal_map_next_3` does
    /// (`erl_map.c:3710-4134`). Keep the entries above the current one on a
    /// stack, with the position of the child that the walk is in. A
    /// position counts in the walk direction, so it is also the path
    /// element. Return `None` when the path selects no entry.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let chunk = hamt.chunk(0, 32, Direction::Forward)?.unwrap();
    /// assert_eq!(chunk.pairs.len(), 32);
    /// ```
    ///
    /// # Errors
    ///
    /// Return the runtime error of `encode_path` for a next path that PBeam
    /// cannot hold.
    fn chunk(
        &self,
        path: u64,
        limit: usize,
        direction: Direction,
    ) -> Result<Option<Chunk<'_>>, BeamError> {
        let mut above: Vec<(Entries, usize)> = Vec::new();
        let mut entries = Entries::Node(&self.root);
        let mut position = loop {
            let position = path_element(path, above.len());
            if position >= entries.len() {
                return Ok(None);
            }
            match entries.at(position, direction) {
                Entry::Leaf(..) => break position,
                Entry::Inner(inner) => {
                    above.push((entries, position));
                    entries = inner;
                }
            }
        };
        let (mut pairs, mut remaining) = (Vec::new(), limit);
        let next = loop {
            if position == entries.len() {
                let Some((outer, child)) = above.pop() else {
                    break None;
                };
                (entries, position) = (outer, child + 1);
                continue;
            }
            if remaining == 0 {
                let elements = above.iter().map(|(_, child)| *child);
                break Some(encode_path(elements.chain([position]).collect())?);
            }
            // A chunk takes a whole collision node: it may hold more than 16 pairs.
            if let (0, Entries::Collision(collision)) = (position, entries) {
                remaining = remaining.max(collision.len());
            }
            match entries.at(position, direction) {
                Entry::Leaf(key, value) => {
                    pairs.push((key, value));
                    (position, remaining) = (position + 1, remaining - 1);
                }
                Entry::Inner(inner) => {
                    above.push((entries, position));
                    (entries, position) = (inner, 0);
                }
            }
        };
        if direction == Direction::Backward {
            pairs.reverse();
        }
        Ok(Some(Chunk { pairs, next }))
    }
}

impl<'a> Entries<'a> {
    /// Return the number of entries.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Entries::Node(&Node::empty()).len(), 0);
    /// ```
    fn len(self) -> usize {
        match self {
            Entries::Node(node) => node.children.len(),
            Entries::Collision(pairs) => pairs.len(),
        }
    }

    /// Return the entry at a position in the walk direction: the index for
    /// `Forward`, and the index from the end for `Backward`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let pairs = [(Term::Integer(1), Term::Integer(1)), (Term::Integer(2), Term::Integer(2))];
    /// let entries = Entries::Collision(&pairs);
    /// assert!(matches!(entries.at(0, Direction::Backward), Entry::Leaf(Term::Integer(2), _)));
    /// ```
    fn at(self, position: usize, direction: Direction) -> Entry<'a> {
        let index = match direction {
            Direction::Forward => position,
            Direction::Backward => self.len() - 1 - position,
        };
        match self {
            Entries::Node(node) => match &node.children[index] {
                Child::Leaf(key, value) => Entry::Leaf(key, value),
                Child::Node(inner) => Entry::Inner(Entries::Node(inner)),
                Child::Collision(pairs) => Entry::Inner(Entries::Collision(pairs)),
            },
            Entries::Collision(pairs) => Entry::Leaf(&pairs[index].0, &pairs[index].1),
        }
    }
}

impl Hamt {
    /// Set a key with a hash to a value, as ERTS `insert_down` does. A new
    /// key goes into the first free slot on its hash path. When that slot
    /// holds another key, add nodes until the two hashes select different
    /// slots. A key with the hash of a collision child goes into it in the
    /// term order, or replaces the value of an equal key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut hamt = Hamt { root: Node::empty(), size: 0 };
    /// hamt.insert(map_hash(&Term::Integer(1)).unwrap(), Term::Integer(1), Term::Integer(2));
    /// assert_eq!(hamt.size, 1);
    /// ```
    fn insert(&mut self, hash: u64, key: Term, value: Term) {
        let mut node = &mut self.root;
        let mut level = 0;
        loop {
            let slot = slot(hash, level);
            let bit = 1 << slot;
            let index = (node.bitmap & (bit - 1)).count_ones() as usize;
            if node.bitmap & bit == 0 {
                node.bitmap |= bit;
                node.children.insert(index, Child::Leaf(key, value));
                self.size += 1;
                return;
            }
            let child = &mut node.children[index];
            match child {
                Child::Node(inner) => node = inner,
                Child::Leaf(existing, existing_value) => {
                    if *existing == key {
                        *existing_value = value;
                        return;
                    }
                    let existing_hash = map_hash(existing).expect("a stored key has a hash");
                    let existing = (
                        existing_hash,
                        std::mem::replace(existing, Term::List(Vec::new())),
                        std::mem::replace(existing_value, Term::List(Vec::new())),
                    );
                    *child = Node::split(level + 1, existing, (hash, key, value));
                    self.size += 1;
                    return;
                }
                // Each key here has the same 64-bit hash (erl_map.c:2598-2624).
                Child::Collision(pairs) => {
                    match pairs.binary_search_by(|(existing, _)| {
                        existing
                            .compare(&key)
                            .expect("supported map keys have a term order")
                    }) {
                        Ok(index) => pairs[index].1 = value,
                        Err(index) => {
                            pairs.insert(index, (key, value));
                            self.size += 1;
                        }
                    }
                    return;
                }
            }
            level += 1;
        }
    }
}

impl Node {
    /// Return a node with no children.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Node::empty().bitmap, 0);
    /// ```
    fn empty() -> Self {
        Node {
            bitmap: 0,
            children: Vec::new(),
        }
    }

    /// Return the child in a slot, or `None` when the slot is free.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Node::empty().child(3).is_none());
    /// ```
    fn child(&self, slot: u32) -> Option<&Child> {
        let bit = 1 << slot;
        if self.bitmap & bit == 0 {
            return None;
        }
        Some(&self.children[(self.bitmap & (bit - 1)).count_ones() as usize])
    }

    /// Return the child in a slot for a change in place, or `None` when the
    /// slot is free.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Node::empty().child_mut(3).is_none());
    /// ```
    fn child_mut(&mut self, slot: u32) -> Option<&mut Child> {
        let bit = 1 << slot;
        if self.bitmap & bit == 0 {
            return None;
        }
        Some(&mut self.children[(self.bitmap & (bit - 1)).count_ones() as usize])
    }

    /// Build the child at `level` for two different keys whose hashes
    /// select the same slots above `level`, as the `insert_subnodes` step of
    /// ERTS `insert_down` does (`erl_map.c:2633-2656`). Add one node with
    /// one child for each level where the two hashes still select the same
    /// slot, then put the two leaves in slot order. When the two hashes are
    /// equal, the last of these nodes is at level 15 and holds a collision
    /// child with the two pairs in the term order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The hashes 0x01 and 0x11 differ first at level 1.
    /// let child = Node::split(0, (0x01, one, one_value), (0x11, two, two_value));
    /// assert!(matches!(child, Child::Node(Node { bitmap: 0b10, .. })));
    /// // Equal hashes at level 16 give a collision child.
    /// let child = Node::split(16, (0x01, one, one_value), (0x01, two, two_value));
    /// assert!(matches!(child, Child::Collision(_)));
    /// ```
    fn split(level: u32, first: (u64, Term, Term), second: (u64, Term, Term)) -> Child {
        let mut depth = level;
        while depth < HAMT_MAX_LEVEL && slot(first.0, depth) == slot(second.0, depth) {
            depth += 1;
        }
        let path = first.0;
        let mut child = if depth == HAMT_MAX_LEVEL {
            let order = first
                .1
                .compare(&second.1)
                .expect("supported map keys have a term order");
            let (low, high) = if order == Ordering::Less {
                (first, second)
            } else {
                (second, first)
            };
            Child::Collision(vec![(low.1, low.2), (high.1, high.2)])
        } else {
            let (first_slot, second_slot) = (slot(first.0, depth), slot(second.0, depth));
            let (low, high) = if first_slot < second_slot {
                (first, second)
            } else {
                (second, first)
            };
            Child::Node(Node {
                bitmap: (1 << first_slot) | (1 << second_slot),
                children: vec![Child::Leaf(low.1, low.2), Child::Leaf(high.1, high.2)],
            })
        };
        for depth in (level..depth).rev() {
            child = Child::Node(Node {
                bitmap: 1 << slot(path, depth),
                children: vec![child],
            });
        }
        child
    }
}

/// Compare two flatmap keys in the flatmap key order, as
/// `erts_cmp_flatmap_keys` does (`erl_utils.h:267-272`): two atoms compare
/// by atom index, and each other two keys compare in the term order. Thus
/// an atom in a list compares by name.
///
/// # Examples
///
/// ```rust,ignore
/// use std::cmp::Ordering;
///
/// // true has index 1 and abs_path has index 26.
/// let (abs_path, true_) = (Term::static_atom("abs_path"), Term::static_atom("true"));
/// assert_eq!(flatmap_order(&true_, &abs_path)?, Ordering::Less);
/// let list = |atom: &Term| Term::List(vec![atom.clone()]);
/// assert_eq!(flatmap_order(&list(&true_), &list(&abs_path))?, Ordering::Greater);
/// assert_eq!(flatmap_order(&Term::Integer(1), &true_)?, Ordering::Less);
/// ```
///
/// # Errors
///
/// Return a runtime error when the order depends on two different
/// references, which have no PBeam term order.
fn flatmap_order(left: &Term, right: &Term) -> Result<Ordering, BeamError> {
    match (left, right) {
        (Term::Atom(left), Term::Atom(right)) => Ok(left.index().cmp(&right.index())),
        _ => left.compare(right),
    }
}

/// Return the slot that a hash selects at a HAMT level. Level 0 uses the
/// lowest four bits.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(slot(0x21, 0), 1);
/// assert_eq!(slot(0x21, 1), 2);
/// ```
fn slot(hash: u64, level: u32) -> u32 {
    ((hash >> (SLOT_BITS * level)) & SLOT_MASK) as u32
}

/// Return the value that ERTS `unsigned_val` reads from the word of a small
/// integer: the low 60 bits of the value, so -1 gives 2^60 - 1.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(unsigned_val(43), 43);
/// assert_eq!(unsigned_val(-1), (1 << 60) - 1);
/// ```
fn unsigned_val(value: i64) -> u64 {
    value as u64 & (u64::MAX >> SMALL_INTEGER_TAG_BITS)
}

/// Return the element of an `erts_internal:map_next/3` path at a node
/// depth. The first 16 elements come from the path digit, root first, and
/// each later element is 0 (`erl_map.c:3770-3812`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(path_element(0x2b, 0), 0xb);
/// assert_eq!(path_element(0x2b, 1), 2);
/// assert_eq!(path_element(u64::MAX, 16), 0);
/// ```
fn path_element(path: u64, depth: usize) -> usize {
    if depth >= PATH_ELEMS_PER_DIGIT {
        return 0;
    }
    ((path >> (SLOT_BITS as usize * depth)) & SLOT_MASK) as usize
}

/// Return the integer of the path elements of a next pair, root first, as
/// `erts_internal_map_next_3` builds it (`erl_map.c:4058-4114`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(encode_path(vec![0xb, 2])?, 0x2b);
/// assert!(encode_path(vec![0; 16]).is_err());
/// ```
///
/// # Errors
///
/// Return a runtime error for a path that ERTS returns as a bignum which
/// PBeam cannot hold. ERTS makes a bignum of `(n + 1) * 4` bits for a path
/// of `n >= 15` elements. With 15 elements, it is one digit, which is in
/// normal form only above 2^59 - 1. With 16 elements, the top digit is 0,
/// so the bignum is not in normal form: on ERTS 29.1.1, it is not equal to
/// the integer of the same value. With 17 elements, a path into a collision
/// node, the value is 2^64 or more, or the top digit is 0.
fn encode_path(elements: Vec<usize>) -> Result<i64, BeamError> {
    let depth = elements.len();
    let value = if depth < PATH_ELEMS_PER_DIGIT {
        let value = elements
            .iter()
            .rev()
            .fold(0, |value, element| (value << SLOT_BITS) | *element as i64);
        let bignum = depth + 1 >= PATH_ELEMS_PER_DIGIT;
        (!bignum || Term::Integer(value).small_integer().is_none()).then_some(value)
    } else {
        None
    };
    value.ok_or_else(|| {
        error(format!(
            "unsupported erts_internal:map_next/3 path of {depth} elements: \
             ERTS returns a bignum that is not in normal form or above 2^63 - 1"
        ))
    })
}

/// Return the ERTS map hash (`erts_map_hash`, `erl_term_hashing.c:2301-2319`)
/// of a key, or `None` for a key that a hashmap does not support yet. An
/// immediate key hashes as `ihash_mix64` of its ERTS word. A list key that
/// is not empty hashes with the list path of `make_internal_hash`, also an
/// improper list whose tail is an immediate.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(map_hash(&Term::Integer(1)), Some(7938660269479274665));
/// // true has index 1, so its word is 0x4B.
/// assert_eq!(map_hash(&Term::static_atom("true")), Some(14558535449716669319));
/// assert_eq!(map_hash(&Term::List(vec![])), Some(mix64(0x3B)));
/// assert!(map_hash(&Term::List(vec![Term::Integer(97)])).is_some());
/// assert_eq!(map_hash(&Term::Integer(1 << 59)), None);
/// assert_eq!(map_hash(&Term::Tuple(vec![])), None);
/// let improper = |tail| Term::ImproperList(vec![Term::Integer(1)], Box::new(tail));
/// assert!(map_hash(&improper(Term::Integer(2))).is_some());
/// assert_eq!(map_hash(&improper(Term::Tuple(vec![]))), None);
/// ```
fn map_hash(key: &Term) -> Option<u64> {
    match key {
        Term::List(elements) if !elements.is_empty() => list_hash(elements, NIL_WORD),
        Term::ImproperList(elements, tail) => list_hash(elements, immediate_word(tail)?),
        _ => immediate_word(key).map(mix64),
    }
}

/// Return the ERTS word of a key that ERTS stores as an immediate: an atom
/// is `(index << 6) | 0xB`, a small integer is `(value << 4) | 0xF`, and
/// `[]` is 0x3B (`erl_term.h:83-90`, 293). Return `None` for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(immediate_word(&Term::Integer(1)), Some(0x1F));
/// assert_eq!(immediate_word(&Term::static_atom("true")), Some(0x4B));
/// assert_eq!(immediate_word(&Term::List(vec![])), Some(0x3B));
/// assert_eq!(immediate_word(&Term::Integer(1 << 59)), None);
/// ```
fn immediate_word(term: &Term) -> Option<u64> {
    match term {
        Term::Atom(atom) => Some((u64::from(atom.index()) << ATOM_TAG_BITS) | ATOM_TAG),
        Term::List(elements) if elements.is_empty() => Some(NIL_WORD),
        _ => term
            .small_integer()
            .map(|value| ((value as u64) << SMALL_INTEGER_TAG_BITS) | SMALL_INTEGER_TAG),
    }
}

/// Return the ERTS internal hash of a list that is not empty, with the
/// elements `list` and the ERTS word `end` of its tail, as the list path of
/// `make_internal_hash` gives it (`erl_term_hashing.c:1783-1841`). The
/// bytes (small integers 0..255) at the start of the rest of a list go into
/// chunks of up to 4 bytes. After them, an immediate head mixes with
/// `IHASH_TYPE_CAR`, and a list head goes first while its tail waits on a
/// work stack, so a deep key does not recurse. Each list ends with its
/// tail, `[]` or the immediate tail of an improper list, which mixes as an
/// immediate (`erl_term_hashing.c:2211-2213`). Return `None` when an
/// element is not an atom, a small integer, a list of these, or an improper
/// list of these with an immediate tail.
///
/// # Examples
///
/// ```rust,ignore
/// let string = |bytes: &[u8]| bytes.iter().map(|byte| Term::Integer(i64::from(*byte))).collect();
/// assert_ne!(list_hash(&string(b"ab"), NIL_WORD), list_hash(&string(b"ba"), NIL_WORD));
/// // [97|1] differs from [97].
/// assert_ne!(list_hash(&string(b"a"), 0x1F), list_hash(&string(b"a"), NIL_WORD));
/// assert!(list_hash(&[Term::Tuple(vec![])], NIL_WORD).is_none());
/// ```
fn list_hash(list: &[Term], end: u64) -> Option<u64> {
    let mut hash = InternalHash::default();
    let mut pending = Vec::new();
    let (mut rest, mut end) = (list, end);
    loop {
        let (mut value, mut bytes) = (0, 0);
        while let [Term::Integer(byte @ 0..=255), tail @ ..] = rest {
            value = (value << 8) | *byte as u64;
            bytes += 1;
            if bytes == IHASH_CHUNK_BYTES {
                hash.mix_alpha(IHASH_TYPE_STRING | (bytes << 8) | (value << 32));
                (value, bytes) = (0, 0);
            }
            rest = tail;
        }
        if bytes > 0 {
            hash.mix_alpha(IHASH_TYPE_STRING | (bytes << 8) | (value << 32));
        }
        if let [head, tail @ ..] = rest {
            if let Some(word) = immediate_word(head) {
                hash.mix_alpha(IHASH_TYPE_IMMEDIATE | (IHASH_TYPE_CAR << 32));
                hash.mix_beta(word);
                if tail.is_empty() {
                    hash.mix_alpha(IHASH_TYPE_CDR);
                }
                rest = tail;
                continue;
            }
            let (elements, head_end) = match head {
                Term::List(elements) => (elements, NIL_WORD),
                Term::ImproperList(elements, head_tail) => (elements, immediate_word(head_tail)?),
                _ => return None,
            };
            pending.push(Pending::Tail(tail, end));
            if tail.is_empty() {
                pending.push(Pending::Cdr);
            }
            hash.mix_alpha(IHASH_TYPE_CAR);
            (rest, end) = (elements, head_end);
            continue;
        }
        // The list ends with its immediate tail (IHASH_MIX_IMMEDIATE, then pop_next).
        hash.mix_alpha(IHASH_TYPE_IMMEDIATE);
        hash.mix_beta(end);
        loop {
            match pending.pop() {
                None => return Some(hash.finish()),
                Some(Pending::Cdr) => hash.mix_beta(IHASH_TYPE_CDR),
                Some(Pending::Tail(tail, tail_end)) => {
                    (rest, end) = (tail, tail_end);
                    break;
                }
            }
        }
    }
}

impl InternalHash {
    /// Mix a word into the alpha lane, as `IHASH_MIX_ALPHA` does
    /// (`erl_term_hashing.c:1668-1680`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut hash = InternalHash::default();
    /// hash.mix_alpha(1);
    /// assert_eq!(hash.ticks, 1);
    /// ```
    fn mix_alpha(&mut self, word: u64) {
        let word = word
            .wrapping_mul(IHASH_C1)
            .rotate_left(31)
            .wrapping_mul(IHASH_C2);
        self.alpha = (self.alpha ^ word)
            .rotate_left(27)
            .wrapping_add(self.beta)
            .wrapping_mul(5)
            .wrapping_add(0x52DC_E729);
        self.ticks += 1;
    }

    /// Mix a word into the beta lane, as `IHASH_MIX_BETA` does
    /// (`erl_term_hashing.c:1684-1695`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut hash = InternalHash::default();
    /// hash.mix_beta(0x3B);
    /// assert_eq!(hash.ticks, 1);
    /// ```
    fn mix_beta(&mut self, word: u64) {
        let word = word
            .wrapping_mul(IHASH_C2)
            .rotate_left(33)
            .wrapping_mul(IHASH_C1);
        self.beta = (self.beta ^ word)
            .rotate_left(31)
            .wrapping_add(self.alpha)
            .wrapping_mul(5)
            .wrapping_add(0x3849_5AB5);
        self.ticks += 1;
    }

    /// Return the hash at the end, when the work stack is empty
    /// (`erl_term_hashing.c:2220-2236`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let hash = InternalHash::default().finish();
    /// assert_eq!(hash, 0);
    /// ```
    fn finish(self) -> u64 {
        let alpha = self.alpha ^ self.ticks;
        let beta = self.beta ^ self.ticks;
        let alpha = alpha.wrapping_add(beta);
        let beta = beta.wrapping_add(alpha);
        let (alpha, beta) = (mix64(alpha), mix64(beta));
        let alpha = alpha.wrapping_add(beta);
        let beta = beta.wrapping_add(alpha);
        alpha ^ beta
    }
}

/// Mix a 64-bit word, as ERTS `ihash_mix64` does (the MurmurHash3 fmix64
/// finalizer). The function is a bijection on `u64`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(mix64(0x1F), 7938660269479274665);
/// ```
fn mix64(mut hash: u64) -> u64 {
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    hash ^ (hash >> 33)
}

#[cfg(test)]
mod tests {
    use super::{
        Child, Direction, HAMT_MAX_LEVEL, Hamt, Map, Node, Repr, encode_path, map_hash, mix64,
    };
    use crate::term::{AtomTable, Reference, Term, test_atom as atom};

    fn integers(keys: impl IntoIterator<Item = i64>) -> Map {
        Map::from_pairs(
            keys.into_iter()
                .map(|key| (Term::Integer(key), atom("v")))
                .collect(),
        )
        .unwrap()
    }

    fn keys(map: &Map) -> Vec<Term> {
        map.iter().map(|(key, _)| key.clone()).collect()
    }

    fn list(keys: &[i64]) -> Vec<Term> {
        keys.iter().map(|key| Term::Integer(*key)).collect()
    }

    #[test]
    fn hashes_small_integers_as_erts() {
        assert_eq!(map_hash(&Term::Integer(1)), Some(7938660269479274665));
        assert_eq!(map_hash(&Term::Integer(-1)), Some(7256831767414464289));
        assert_eq!(mix64(0x1F), 7938660269479274665);
        assert!(map_hash(&Term::Integer((1 << 59) - 1)).is_some());
        assert!(map_hash(&Term::Integer(-(1 << 59))).is_some());
        assert_eq!(map_hash(&Term::Integer(1 << 59)), None);
        assert_eq!(map_hash(&Term::Integer(-(1 << 59) - 1)), None);
    }

    #[test]
    fn hashes_atoms_by_their_index_as_erts() {
        // true has index 1: its ERTS word is (1 << 6) | 0xB = 0x4B.
        assert_eq!(mix64(0x4B), 14558535449716669319);
        assert_eq!(
            map_hash(&Term::static_atom("true")),
            Some(14558535449716669319)
        );
        // abs_path has index 26.
        assert_eq!(
            map_hash(&Term::static_atom("abs_path")),
            Some(mix64((26 << 6) | 0xB))
        );
    }

    #[test]
    fn keeps_flatmap_keys_in_order_and_replaces_values() {
        // A fresh table for this test makes z before a, so z has the lower
        // atom index. A table shared with other tests could make a first.
        let mut table = AtomTable::new();
        let mut atom = |name: &str| Term::Atom(table.put(name).unwrap());
        let mut map = Map::from_pairs(vec![
            (atom("z"), atom("z")),
            (atom("a"), atom("old")),
            (Term::Integer(2), atom("v")),
            (Term::Integer(i64::MIN), atom("v")),
            (atom("a"), atom("a")),
        ])
        .unwrap();
        assert_eq!(
            keys(&map),
            [
                Term::Integer(i64::MIN),
                Term::Integer(2),
                atom("z"),
                atom("a")
            ]
        );
        // ERTS 29.1.1: maps:keys(#{true => V, abs_path => V, 1 => V}) is [1,true,abs_path],
        // because true has index 1 and abs_path has index 26.
        let static_atoms = Map::from_pairs(vec![
            (Term::static_atom("abs_path"), atom("v")),
            (Term::static_atom("true"), atom("v")),
            (Term::Integer(1), atom("v")),
        ])
        .unwrap();
        assert_eq!(
            keys(&static_atoms),
            [
                Term::Integer(1),
                Term::static_atom("true"),
                Term::static_atom("abs_path")
            ]
        );
        assert_eq!(map.get(&atom("a")), Some(&atom("a")));
        assert_eq!(map.get(&atom("b")), None);
        assert_eq!(map.get(&Term::Tuple(vec![])), None);
        map.insert(atom("z"), atom("new")).unwrap();
        assert_eq!(map.get(&atom("z")), Some(&atom("new")));
        assert_eq!(map.len(), 4);
        // A key type that the flatmap does not support leaves the map unchanged.
        assert_eq!(
            map.insert(Term::Tuple(vec![]), atom("v")),
            Err(Term::Tuple(vec![]))
        );
        assert_eq!(map.len(), 4);
    }

    #[test]
    fn orders_atoms_in_list_keys_by_name_in_a_flatmap() {
        // A fresh table for this test makes z before a, so z has the lower atom index.
        let mut table = AtomTable::new();
        let mut atom = |name: &str| Term::Atom(table.put(name).unwrap());
        let (z, a, v) = (atom("z"), atom("a"), atom("v"));
        let list = |term: &Term| Term::List(vec![term.clone()]);
        let map = Map::from_pairs(vec![
            (list(&z), v.clone()),
            (a.clone(), v.clone()),
            (list(&a), v.clone()),
            (z.clone(), v),
        ])
        .unwrap();
        // erts_cmp_flatmap_keys: two atoms by index, two lists in the term order.
        assert_eq!(keys(&map), [z.clone(), a.clone(), list(&a), list(&z)]);
        assert_eq!(map.get(&list(&z)), Some(&atom("v")));
    }

    #[test]
    fn rejects_list_keys_with_unsupported_elements() {
        assert_eq!(map_hash(&Term::List(vec![])), Some(mix64(0x3B)));
        let unsupported = [
            Term::List(vec![Term::Integer(1), Term::Tuple(vec![])]),
            Term::List(vec![Term::List(vec![Term::Tuple(vec![])])]),
            Term::List(vec![Term::Integer(1 << 59)]),
            // [1|{}], [1|<<>>], and [[1|{}]]: an improper list tail that is not an immediate.
            Term::ImproperList(vec![Term::Integer(1)], Box::new(Term::Tuple(vec![]))),
            Term::ImproperList(vec![Term::Integer(1)], Box::new(Term::Binary(vec![]))),
            Term::List(vec![Term::ImproperList(
                vec![Term::Integer(1)],
                Box::new(Term::Tuple(vec![])),
            )]),
        ];
        for key in unsupported {
            assert_eq!(map_hash(&key), None);
            let mut flat = integers([1]);
            assert_eq!(flat.insert(key.clone(), atom("v")), Err(key.clone()));
            assert_eq!(keys(&flat), list(&[1]));
            let mut hash = integers(1..=33);
            assert_eq!(hash.insert(key.clone(), atom("v")), Err(key.clone()));
            assert_eq!(hash.len(), 33);
            assert_eq!(hash.get(&key), None);
        }
    }

    #[test]
    fn hashes_improper_list_keys_with_their_tail_as_erts() {
        // ERTS 29.1.1: maps:keys of #{[1|2] => v, [1,2|3] => v, [[1|2]|3] => v,
        // [true|false] => v, [256|1] => v, [[1]|2] => v, [[]|-1] => v, 1 => v, ..., 26 => v}.
        let improper = |elements: Vec<Term>, tail| Term::ImproperList(elements, Box::new(tail));
        let int = Term::Integer;
        let one_two = || improper(vec![int(1)], int(2));
        let one_two_three = || improper(vec![int(1), int(2)], int(3));
        let nested = || improper(vec![one_two()], int(3));
        let booleans = || improper(vec![Term::static_atom("true")], Term::static_atom("false"));
        let large = || improper(vec![int(256)], int(1));
        let list_head = || improper(vec![Term::List(vec![int(1)])], int(2));
        let nil_head = || improper(vec![Term::List(vec![])], int(-1));
        let lists = [
            one_two(),
            one_two_three(),
            nested(),
            booleans(),
            large(),
            list_head(),
            nil_head(),
        ];
        let map = Map::from_pairs(
            lists
                .into_iter()
                .chain((1..=26).map(int))
                .map(|key| (key, atom("v")))
                .collect(),
        )
        .unwrap();
        assert_eq!(
            keys(&map),
            [
                int(18),
                list_head(),
                nested(),
                int(4),
                large(),
                int(12),
                int(19),
                int(13),
                int(2),
                int(7),
                int(8),
                int(10),
                int(23),
                int(9),
                booleans(),
                one_two(),
                int(15),
                one_two_three(),
                int(1),
                int(25),
                int(20),
                int(6),
                int(11),
                int(17),
                int(24),
                int(14),
                nil_head(),
                int(3),
                int(16),
                int(21),
                int(5),
                int(26),
                int(22),
            ]
        );
        assert_eq!(map.get(&one_two_three()), Some(&atom("v")));
        assert_eq!(map.get(&improper(vec![int(1), int(2)], int(4))), None);
    }

    #[test]
    fn becomes_a_hashmap_with_the_erts_order_above_32_keys() {
        let mut map = integers(1..=32);
        assert_eq!(keys(&map), list(&(1..=32).collect::<Vec<_>>()));
        // Replacing a value in a full flatmap keeps the flatmap.
        map.insert(Term::Integer(32), Term::Integer(0)).unwrap();
        assert_eq!(keys(&map), list(&(1..=32).collect::<Vec<_>>()));
        map.insert(Term::Integer(33), atom("v")).unwrap();
        assert_eq!(map.len(), 33);
        // ERTS 29.1.1: maps:keys of 1..33.
        assert_eq!(
            keys(&map),
            list(&[
                18, 4, 12, 19, 29, 13, 2, 7, 31, 8, 10, 23, 9, 15, 32, 1, 25, 28, 20, 6, 11, 17,
                24, 14, 33, 3, 16, 30, 21, 5, 27, 26, 22
            ])
        );
        assert_eq!(map.get(&Term::Integer(32)), Some(&Term::Integer(0)));
        map.insert(Term::Integer(33), Term::Integer(1)).unwrap();
        assert_eq!(map.get(&Term::Integer(33)), Some(&Term::Integer(1)));
        assert_eq!(map.len(), 33);
        assert_eq!(map.get(&Term::Integer(34)), None);
        assert_eq!(map.get(&atom("a")), None);

        // ERTS 29.1.1: maps:keys of 1..64, where every root slot is used.
        let full = integers((1..=64).rev());
        assert_eq!(
            keys(&full),
            list(&[
                54, 18, 4, 34, 46, 52, 12, 19, 41, 29, 13, 42, 55, 2, 7, 38, 56, 61, 31, 8, 44, 10,
                23, 9, 49, 64, 15, 35, 36, 32, 58, 1, 25, 28, 60, 20, 6, 43, 57, 11, 48, 17, 24,
                37, 40, 14, 33, 53, 3, 16, 30, 62, 21, 5, 27, 47, 63, 26, 51, 22, 50, 45, 59, 39
            ])
        );
        assert!((1..=64).all(|key| full.get(&Term::Integer(key)).is_some()));

        // The first three keys have hashes with the same low 60 bits, and so do the next two.
        // Each group splits only at level 15. ERTS 29.1.1: maps:keys of this map.
        let deep = [
            413866500823436792,
            -101714049538469044,
            -74925715755171563,
            329898321527737274,
            446694106686689078,
        ];
        assert_eq!(
            map_hash(&Term::Integer(deep[0])),
            Some(0x6600_a35a_0999_50d8)
        );
        assert_eq!(
            map_hash(&Term::Integer(deep[1])),
            Some(0x7600_a35a_0999_50d8)
        );
        assert_eq!(
            map_hash(&Term::Integer(deep[2])),
            Some(0xf600_a35a_0999_50d8)
        );
        assert_eq!(
            map_hash(&Term::Integer(deep[3])),
            Some(0x3b04_04f2_b094_90b8)
        );
        assert_eq!(
            map_hash(&Term::Integer(deep[4])),
            Some(0x7b04_04f2_b094_90b8)
        );
        let deep60 = integers(deep.into_iter().chain(100..=129));
        assert_eq!(
            keys(&deep60),
            list(&[
                128, 123, 110, 114, 100, 113, 104, 105, 115, 127, 116, 111, 101, 112, 125, 103,
                118, 109, 106, 117, deep[3], deep[4], deep[0], deep[1], deep[2], 126, 107, 108,
                119, 122, 129, 124, 121, 102, 120
            ])
        );
        assert!(
            deep.iter()
                .all(|key| deep60.get(&Term::Integer(*key)).is_some())
        );

        // ERTS 29.1.1: the small-integer limits in a hashmap.
        let (min, max) = (-(1i64 << 59), (1i64 << 59) - 1);
        let edges = integers([-1, min, max].into_iter().chain(1..=30));
        assert_eq!(
            keys(&edges),
            list(&[
                18, 4, -1, 12, 19, 29, 13, 2, 7, min, 8, 10, 23, 9, 15, 1, 25, 28, 20, 6, max, 11,
                17, 24, 14, 3, 16, 30, 21, 5, 27, 26, 22
            ])
        );
    }

    #[test]
    fn puts_atom_keys_in_a_hashmap_and_rejects_large_integers() {
        let a = atom("a");
        // The 33rd key turns a flatmap with an atom key into a hashmap.
        let mut map = integers(1..=31);
        map.insert(a.clone(), a.clone()).unwrap();
        map.insert(Term::Integer(33), a.clone()).unwrap();
        assert_eq!(map.len(), 33);
        assert_eq!(map.get(&a), Some(&a));
        assert_eq!(map.get(&atom("b")), None);
        // An atom key goes into a hashmap.
        let mut map = integers(1..=33);
        map.insert(a.clone(), Term::Integer(1)).unwrap();
        map.insert(a.clone(), Term::Integer(2)).unwrap();
        assert_eq!(map.len(), 34);
        assert_eq!(map.get(&a), Some(&Term::Integer(2)));
        // A hashmap has no hash for an integer outside the small-integer range yet.
        assert_eq!(
            map.insert(Term::Integer(1 << 59), a.clone()),
            Err(Term::Integer(1 << 59))
        );
        let mut map = integers(1..=31);
        map.insert(Term::Integer(1 << 59), a.clone()).unwrap();
        assert_eq!(
            map.insert(Term::Integer(33), a),
            Err(Term::Integer(1 << 59))
        );
        assert_eq!(map.len(), 32);
    }

    #[test]
    fn changes_the_value_of_an_existing_key_in_place() {
        let mut flat = integers([1, 2]);
        *flat.get_mut(&Term::Integer(2)).unwrap() = Term::Integer(0);
        assert_eq!(flat.get(&Term::Integer(2)), Some(&Term::Integer(0)));
        assert!(flat.get_mut(&Term::Integer(3)).is_none());
        // Each root slot of 1..64 is used, so a key can be in a node below the root.
        let mut hash = integers(1..=64);
        for key in 1..=64 {
            *hash.get_mut(&Term::Integer(key)).unwrap() = Term::Integer(-key);
        }
        assert!((1..=64).all(|key| hash.get(&Term::Integer(key)) == Some(&Term::Integer(-key))));
        assert!(hash.get_mut(&Term::Integer(65)).is_none());
        assert!(integers(1..=33).get_mut(&Term::Integer(34)).is_none());
        assert!(hash.get_mut(&Term::Integer(1 << 59)).is_none());
        assert!(hash.get_mut(&Term::Tuple(vec![])).is_none());
    }

    #[test]
    fn sorts_pairs_by_the_flatmap_key_order() {
        // ERTS 29.1.1: (id(#{}))#{abs := 1, value := 2} raises {badkey,value},
        // because value (index 745) comes before abs (index 762).
        let mut pairs = vec![
            (Term::static_atom("abs"), Term::Integer(1)),
            (Term::static_atom("value"), Term::Integer(2)),
            (Term::Integer(7), Term::Integer(3)),
        ];
        Map::sort_pairs(&mut pairs).unwrap();
        let sorted: Vec<_> = pairs.iter().map(|(key, _)| key.clone()).collect();
        assert_eq!(
            sorted,
            [
                Term::Integer(7),
                Term::static_atom("value"),
                Term::static_atom("abs")
            ]
        );
        // ERTS 29.1.1: (id(#{a => 1}))#{{value} := 1, {abs} := 2} raises
        // {badkey,{abs}}: in a tuple, atoms compare by name. A tuple comes
        // before a map, and a map before [].
        let mut compound = vec![
            (Term::List(vec![]), Term::Integer(1)),
            (Term::Map(Map::new()), Term::Integer(2)),
            (
                Term::Tuple(vec![Term::static_atom("value")]),
                Term::Integer(3),
            ),
            (
                Term::Tuple(vec![Term::static_atom("abs")]),
                Term::Integer(4),
            ),
            (Term::static_atom("abs"), Term::Integer(5)),
        ];
        Map::sort_pairs(&mut compound).unwrap();
        let values: Vec<_> = compound.iter().map(|(_, value)| value.clone()).collect();
        assert_eq!(values, [5, 4, 3, 2, 1].map(Term::Integer));
        let mut references = vec![
            (Term::Reference(Reference(1)), Term::Integer(1)),
            (Term::Reference(Reference(2)), Term::Integer(2)),
        ];
        assert!(
            Map::sort_pairs(&mut references)
                .unwrap_err()
                .to_string()
                .contains("unsupported term order of two references")
        );
    }

    #[test]
    fn builds_the_same_map_from_pairs_in_any_order() {
        let forward = integers(1..=40);
        let backward = integers((1..=40).rev());
        assert_eq!(keys(&forward), keys(&backward));
        let mut pending = Vec::new();
        forward.drain_into(&mut pending);
        assert_eq!(pending.len(), 80);
        let mut pending = Vec::new();
        integers([2, 1]).drain_into(&mut pending);
        let v = atom("v");
        assert_eq!(pending, [Term::Integer(1), v.clone(), Term::Integer(2), v]);
    }

    #[test]
    fn splits_a_leaf_into_nodes_until_the_hashes_differ() {
        let leaf = |key: i64| (key as u64, Term::Integer(key), Term::Integer(key));
        // 0x101 and 0x201 select the same slot at levels 0 and 1, and slots 1 and 2 at level 2.
        assert!(matches!(
            Node::split(0, leaf(0x201), leaf(0x101)),
            Child::Node(Node { bitmap: 0b10, children }) if matches!(
            &children[..],
            [Child::Node(Node { bitmap: 1, children })] if matches!(
                &children[..],
                [Child::Node(Node { bitmap: 0b110, children })] if matches!(
                    &children[..],
                    [Child::Leaf(Term::Integer(0x101), _), Child::Leaf(Term::Integer(0x201), _)]
                )
            )
            )
        ));
    }

    #[test]
    fn keeps_keys_with_equal_hashes_in_a_collision_child() {
        let key = |element: i64| Term::List(vec![Term::Integer(element)]);
        // Two keys with equal hashes at the last level give a collision child in the term order.
        let child = Node::split(
            HAMT_MAX_LEVEL,
            (0x5, key(2), Term::Integer(2)),
            (0x5, key(1), Term::Integer(1)),
        );
        assert!(matches!(
            &child,
            Child::Collision(pairs) if pairs[..] == [(key(1), Term::Integer(1)), (key(2), Term::Integer(2))]
        ));
        // Fabricated hashes: 0x5 for [1], [2], and [3], and 0x15 for [4]. A leaf split takes
        // the real hash of the stored key, so the trie starts from a split of [2] and [3].
        let root = Node {
            bitmap: 1 << 5,
            children: vec![Node::split(
                1,
                (0x5, key(3), Term::Integer(0)),
                (0x5, key(2), Term::Integer(2)),
            )],
        };
        let mut hamt = Hamt { root, size: 2 };
        hamt.insert(0x5, key(1), Term::Integer(1));
        hamt.insert(0x5, key(3), Term::Integer(3));
        hamt.insert(0x15, key(4), Term::Integer(4));
        assert_eq!(hamt.size, 4);
        // The only collision child is at level 16, as an ERTS collision node is.
        let mut nodes = vec![(0, &hamt.root)];
        let mut collision_levels = Vec::new();
        while let Some((level, node)) = nodes.pop() {
            for child in &node.children {
                match child {
                    Child::Leaf(..) => {}
                    Child::Node(inner) => nodes.push((level + 1, inner)),
                    Child::Collision(_) => collision_levels.push(level + 1),
                }
            }
        }
        assert_eq!(collision_levels, [HAMT_MAX_LEVEL]);
        let map = Map {
            repr: Repr::Hash(hamt),
        };
        let pairs: Vec<_> = map
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        assert_eq!(
            pairs,
            (1..=4)
                .map(|n| (key(n), Term::Integer(n)))
                .collect::<Vec<_>>()
        );
        let mut pending = Vec::new();
        map.drain_into(&mut pending);
        assert_eq!(
            pending,
            [4, 1, 2, 3].map(|n| [key(n), Term::Integer(n)]).concat()
        );

        // ERTS 29.1.1: these two lists have equal map hashes.
        let first = Term::List(list(&[246, 157, 203, 77, 19, 254, 19, 93]));
        let second = Term::List(list(&[161, 92, 65, 50, 231, 24, 153, 137]));
        assert_eq!(map_hash(&first), map_hash(&second));
        let mut map = integers(1..=32);
        map.insert(first.clone(), Term::Integer(1)).unwrap();
        map.insert(second.clone(), Term::Integer(2)).unwrap();
        map.insert(second.clone(), Term::Integer(3)).unwrap();
        assert_eq!(map.len(), 34);
        *map.get_mut(&first).unwrap() = Term::Integer(4);
        assert_eq!(map.get(&first), Some(&Term::Integer(4)));
        assert_eq!(map.get(&second), Some(&Term::Integer(3)));
    }

    /// Return the map of each key to itself.
    fn identity(keys: Vec<Term>) -> Map {
        Map::from_pairs(keys.into_iter().map(|key| (key.clone(), key)).collect()).unwrap()
    }

    /// Return the keys of the pairs of a chunk.
    fn chunk_keys(map: &Map, path: Term, direction: Direction) -> (Vec<Term>, Option<i64>) {
        let chunk = map.chunk(&path, 1000, direction).unwrap().unwrap();
        let keys = chunk.pairs.iter().map(|(key, _)| (*key).clone()).collect();
        (keys, chunk.next)
    }

    #[test]
    fn takes_a_whole_collision_node_in_a_chunk() {
        // `walks_collision_nodes_and_bignum_paths_as_pinned_erts` in
        // `tests/logger_start.rs` checks these maps and paths against the pinned ERTS.
        // ERTS 29.1.1: these two lists have equal map hashes, so they share a collision node.
        let a = Term::List(list(&[246, 157, 203, 77, 19, 254, 19, 93]));
        let b = Term::List(list(&[161, 92, 65, 50, 231, 24, 153, 137]));
        let before = [
            19, 213, 340, 110, 91, 164, 399, 139, 361, 212, 41, 114, 100, 29, 347, 365, 171, 140,
            13, 42, 149, 55, 2, 146, 372, 317, 191, 316, 202, 192, 233,
        ];
        let mut keys = list(&before);
        keys.extend([b.clone(), a.clone()]);
        keys.extend(list(&[312, 98, 333]));
        let map = identity(keys.clone());
        // ERTS 29.1.1 in a new process: map_next(0, Map, iterator) gives 33
        // pairs, because the 32nd pair starts the collision node, and the path 369.
        assert_eq!(
            chunk_keys(&map, Term::Integer(0), Direction::Forward),
            (keys[..33].to_vec(), Some(369))
        );
        // map_next(113, Map, iterator) selects the collision node at depth
        // 16, after the 16 elements of the path digit.
        assert_eq!(
            chunk_keys(&map, Term::Integer(113), Direction::Forward),
            (keys[31..].to_vec(), None)
        );
        // A path other than 0 has no limit of 32 pairs: -(2^59) reads as
        // 2^59, and 2^59 is a bignum. A leaf above the last element ends the path.
        for path in [-(1 << 59), 1 << 59] {
            assert_eq!(
                chunk_keys(&map, Term::Integer(path), Direction::Forward),
                (keys.clone(), None)
            );
        }
        // A negative bignum raises badarg.
        assert!(
            map.chunk(&Term::Integer(-(1 << 60)), 1000, Direction::Forward)
                .unwrap()
                .is_none()
        );
        // ERTS 29.1.1: map_next(0, Map, []) gives the last 33 pairs and the path 97.
        let after = [
            312, 98, 333, 7, 398, 285, 90, 343, 38, 325, 326, 284, 281, 56, 153, 61, 99, 113, 383,
            201, 237, 314, 65, 322, 137, 141, 308, 315, 230, 104, 187,
        ];
        let mut keys = list(&[202, 192, 233]);
        keys.extend([b, a]);
        keys.extend(list(&after));
        let map = identity(keys.clone());
        assert_eq!(
            chunk_keys(&map, Term::Integer(0), Direction::Backward),
            (keys[3..].to_vec(), Some(97))
        );
        // A flatmap needs a small integer path.
        assert!(
            integers([1])
                .chunk(&Term::Integer(1 << 59), 1000, Direction::Backward)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn fails_for_a_path_that_erts_returns_as_a_bignum_not_in_normal_form() {
        // `walks_collision_nodes_and_bignum_paths_as_pinned_erts` in
        // `tests/logger_start.rs` checks the paths 2^60 + 18 and 2^60 + 786
        // against the pinned ERTS.
        // The hashes of the last five keys share their low 60 bits in two
        // groups (see becomes_a_hashmap_with_the_erts_order_above_32_keys),
        // so each group is in a node at level 15.
        let deep = [
            329898321527737274,
            446694106686689078,
            413866500823436792,
            -101714049538469044,
            -74925715755171563,
        ];
        let before = [
            319, 169, 371, 359, 160, 388, 136, 324, 348, 287, 296, 357, 239, 179, 289, 263, 313,
            235, 226, 240, 211, 128, 247, 194, 384, 269, 168, 373, 265,
        ];
        let keys: Vec<_> = list(&before).into_iter().chain(list(&deep)).collect();
        let map = identity(keys.clone());
        // ERTS 29.1.1: map_next(0, Map, iterator) stops in the node at level
        // 15, so the path has 16 elements. ERTS returns 2^60 + 18 as a
        // bignum of two digits, which is not equal to 1152921504606846994.
        let failure = map.chunk(&Term::Integer(0), 1000, Direction::Forward);
        assert_eq!(
            failure.err().unwrap().to_string(),
            "unsupported erts_internal:map_next/3 path of 16 elements: ERTS returns a bignum \
             that is not in normal form or above 2^63 - 1"
        );
        // ERTS 29.1.1: map_next(1152921504606846994, Map, iterator) gives the last two pairs.
        assert_eq!(
            chunk_keys(&map, Term::Integer((1 << 60) + 18), Direction::Forward),
            (keys[32..].to_vec(), None)
        );
        let after = [
            126, 390, 155, 165, 228, 227, 242, 151, 208, 197, 350, 133, 301, 392, 196, 205, 332,
            353, 177, 225, 142, 223, 275, 200, 377, 145, 107, 108, 184, 255, 272,
        ];
        let keys: Vec<_> = list(&deep).into_iter().chain(list(&after)).collect();
        let map = identity(keys.clone());
        assert!(
            map.chunk(&Term::Integer(0), 1000, Direction::Backward)
                .is_err()
        );
        // ERTS 29.1.1: map_next(1152921504606847762, Map, []) gives the first four pairs.
        assert_eq!(
            chunk_keys(&map, Term::Integer((1 << 60) + 786), Direction::Backward),
            (keys[..4].to_vec(), None)
        );
        // A path of 15 elements is a bignum of one digit, in normal form only above 2^59 - 1.
        let mut elements = vec![0; 14];
        assert_eq!(encode_path(elements.clone()), Ok(0));
        elements.push(8);
        assert_eq!(encode_path(elements.clone()), Ok(1 << 59));
        *elements.last_mut().unwrap() = 7;
        assert!(encode_path(elements.clone()).is_err());
        elements.extend([1, 1]);
        assert!(encode_path(elements).is_err());
    }
}
