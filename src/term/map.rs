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
//! - hashmap: a binary, an atom, an integer in the ERTS small-integer
//!   range, `[]`, a list or a tuple whose elements are such keys other than
//!   binaries, and an improper list whose elements are such elements and
//!   whose tail is an atom, a small integer, or such a tuple;
//! - flatmap: each hashmap key, and any integer.
//!
//! As in ERTS, a flatmap orders two atom keys by their atom index and each
//! other two keys in the term order, so an atom in a list or a tuple
//! compares by name. A hashmap hashes a key with the ERTS internal hash, and
//! an atom hashes by its atom word `(index << 6) | 0xB`. Thus the pair order
//! of a map is the ERTS order, which BEAM code sees through operations such
//! as `maps:keys/1`. Other key types need the hashmap insertion of bignums,
//! maps, funs, pids, and references, which PBeam does not have yet. So does
//! a list or a tuple with a binary element, for example `[<<>>]` and
//! `{<<>>}`.
#![expect(
    clippy::arc_with_non_send_sync,
    reason = "a term is not Send and Sync until #459, and a map uses Arc now so that #459 does not change it again"
)]

use std::cmp::Ordering;
use std::sync::Arc;

use super::{Term, hash::internal_hash};
use crate::error::{BeamError, error};

mod build;

pub(crate) use build::from_list_units;

/// The largest number of keys in a flatmap (`MAP_SMALL_MAP_LIMIT` in a
/// release build of ERTS). A map with more keys is a hashmap.
const SMALL_MAP_LIMIT: usize = 32;
/// The number of hash bits that select a slot at one HAMT level.
const SLOT_BITS: u32 = 4;
/// The number of HAMT levels that a 64-bit hash fills (`HAMT_MAX_LEVEL`).
const HAMT_MAX_LEVEL: u32 = u64::BITS / SLOT_BITS;
/// The mask of the hash bits that select a slot.
const SLOT_MASK: u64 = 0xF;
/// The bitmap of a node that uses all 16 slots. ERTS gives such a root the
/// `HEAD_ARRAY` header instead of `HEAD_BITMAP` (`hashmap_from_*` in
/// `erl_map.c`).
const FULL_BITMAP: u16 = 0xFFFF;
/// The width of the tag of an ERTS small integer word (`_TAG_IMMED1_SIZE`).
const SMALL_INTEGER_TAG_BITS: u32 = 4;
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
    /// The pairs that the call counts for its cost: `orig_elems - elems`
    /// (`erl_map.c:4126`). A chunk that takes a whole collision node with
    /// more pairs than the limit counts the limit. A flatmap chunk counts 0.
    pub(crate) counted: usize,
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
///
/// A copy of a map shares the form of the map, as an ERTS copy of a map
/// term in a register or a stack slot is the same boxed pointer. A change
/// through one copy first copies a shared form, so the other copies do not
/// change. A hashmap copies only the HAMT nodes on the path of the key, and
/// the new map shares each other node with the old map, as
/// `erts_hashmap_insert` does (`erl_map.c:2477-2503`).
#[derive(Clone)]
pub struct Map {
    /// The flatmap or hashmap form of the map. All copies of the map own
    /// one reference to it. `insert` and `get_mut` copy it first when
    /// another copy also holds it (`Arc::make_mut`). The last copy that
    /// drops releases it.
    repr: Arc<Repr>,
}

/// The two forms of a map. The number of keys selects the form.
#[derive(Clone)]
enum Repr {
    /// At most 32 pairs, sorted by the flatmap key order.
    Flat(Vec<(Term, Term)>),
    /// A HAMT, only for 33 or more keys.
    Hash(Hamt),
}

/// A hash array mapped trie of map pairs.
#[derive(Clone)]
struct Hamt {
    /// The node at level 0.
    root: Node,
    /// The number of keys in the trie.
    size: usize,
}

/// The form of a map, as the ERTS internal hash reads it.
pub(super) enum Form<'a> {
    /// The pairs of a flatmap, in flatmap key order.
    Flat(&'a [(Term, Term)]),
    /// The number of keys and the root node of a hashmap.
    Hash { size: usize, root: &'a Node },
}

/// A HAMT node. Bit `i` of `bitmap` is set when slot `i` holds a child.
/// `children` holds one child per set bit, in ascending slot order. A copy
/// of a node copies the keys and values of its leaves, and shares its child
/// nodes.
#[derive(Clone)]
pub(super) struct Node {
    /// The occupied slots.
    bitmap: u16,
    /// The child of each occupied slot, in ascending slot order.
    children: Vec<Child>,
}

/// The child in one slot of a HAMT node.
#[derive(Clone)]
pub(super) enum Child {
    /// A key and its value.
    Leaf(Term, Term),
    /// A node one level down. Each map that holds the node owns one
    /// reference to it. A change copies it first when another map also
    /// holds it (`Arc::make_mut`), so a change copies only the nodes on the
    /// path of its key.
    Node(Arc<Node>),
    /// The pairs of two or more keys with equal hashes, sorted in the term
    /// order. Only a node at the last level (`HAMT_MAX_LEVEL - 1`) holds a
    /// collision child, as an ERTS collision node is at level 16
    /// (`erl_map.h:205-210`).
    Collision(Vec<(Term, Term)>),
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
            repr: Arc::new(Repr::Flat(Vec::new())),
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
    /// example a map key, or an integer outside the small-integer range in
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
        match &*self.repr {
            Repr::Flat(pairs) => pairs.len(),
            Repr::Hash(hamt) => hamt.size,
        }
    }

    /// Return true for a hashmap: a map of more than 32 keys.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Map::new().is_hashmap());
    /// ```
    pub(crate) fn is_hashmap(&self) -> bool {
        matches!(*self.repr, Repr::Hash(_))
    }

    /// Return the flatmap pairs or the hashmap root, so that the ERTS
    /// internal hash reads the map in its ERTS layout.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))])?;
    /// assert!(matches!(map.form(), Form::Flat([(Term::Integer(1), Term::Integer(2))])));
    /// ```
    pub(super) fn form(&self) -> Form<'_> {
        match &*self.repr {
            Repr::Flat(pairs) => Form::Flat(pairs),
            Repr::Hash(hamt) => Form::Hash {
                size: hamt.size,
                root: &hamt.root,
            },
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
    /// assert_eq!(map.get(&Term::tuple(vec![])), None);
    /// ```
    pub(crate) fn get(&self, key: &Term) -> Option<&Term> {
        let hamt = match &*self.repr {
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
    /// map does not hold the key. The key lookup is the one of `get`. When
    /// another copy also holds the form of the map, copy the form first, so
    /// the change does not show in the other copy. A hashmap copies only
    /// the shared nodes on the path of the key.
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
        let hamt = match Arc::make_mut(&mut self.repr) {
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
                Child::Node(inner) => node = Arc::make_mut(inner),
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
    ///     (Term::tuple(vec![]), Term::Integer(0)),
    ///     (Term::static_atom("abs"), Term::Integer(1)),
    ///     (Term::static_atom("value"), Term::Integer(2)),
    /// ];
    /// Map::sort_pairs(&mut pairs)?;
    /// // value has atom index 745, abs has atom index 762, and an atom
    /// // comes before a tuple.
    /// assert_eq!(pairs[0].0, Term::static_atom("value"));
    /// assert_eq!(pairs[2].0, Term::tuple(vec![]));
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
    /// flatmap that gets its 33rd key becomes a hashmap, as in ERTS. When
    /// another copy also holds the form of the map, copy the form first, so
    /// the change does not show in the other copy: a flatmap copies all its
    /// pairs, as `erts_maps_put` does (`erl_map.c:2105-2229`), and a hashmap
    /// copies only the shared nodes on the path of the key, in O(log N)
    /// time, as `erts_hashmap_insert` does (`erl_map.c:2477-2503`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut map = Map::new();
    /// map.insert(Term::Integer(1), Term::Integer(2))?;
    /// map.insert(Term::Integer(1), Term::Integer(3))?;
    /// assert_eq!(map.get(&Term::Integer(1)), Some(&Term::Integer(3)));
    /// map.insert(Term::tuple(vec![Term::Integer(1)]), Term::Integer(4))?;
    /// assert_eq!(map.get(&Term::tuple(vec![Term::Integer(1)])), Some(&Term::Integer(4)));
    /// assert!(map.insert(Term::Map(Map::new()), Term::Integer(1)).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Return the first key that the map form does not support yet. The map
    /// does not change.
    pub(crate) fn insert(&mut self, key: Term, value: Term) -> Result<(), Term> {
        let pairs = match Arc::make_mut(&mut self.repr) {
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
                self.repr = Arc::new(Repr::Hash(hamt));
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
    /// let nil = || Term::nil();
    /// let map = Map::from_pairs(vec![(Term::Integer(2), nil()), (Term::Integer(1), nil())])?;
    /// let keys: Vec<_> = map.iter().map(|(key, _)| key.clone()).collect();
    /// assert_eq!(keys, [Term::Integer(1), Term::Integer(2)]);
    /// ```
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Term, &Term)> {
        // `pairs` holds the flatmap pairs, or the pairs of the current collision child.
        let (mut pairs, mut nodes) = match &*self.repr {
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

    /// Move every key and value onto a work stack and leave the map empty.
    /// Take the pairs of the form and of each HAMT node only from its last
    /// holder: a form or a node that another map also holds stays with that
    /// map, which moves the pairs when it drops last. `Term` uses this to
    /// drop a deep map without recursion.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut pending = Vec::new();
    /// let mut map = Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))])?;
    /// let mut copy = map.clone();
    /// map.drain_into(&mut pending);
    /// assert!(pending.is_empty());
    /// drop(map);
    /// copy.drain_into(&mut pending);
    /// assert_eq!(pending, [Term::Integer(1), Term::Integer(2)]);
    /// ```
    pub(crate) fn drain_into(&mut self, pending: &mut Vec<Term>) {
        let repr = std::mem::replace(&mut self.repr, Arc::new(Repr::Flat(Vec::new())));
        // Of the holders that drop an `Arc` at the same time, exactly one gets the value
        // (`Arc::into_inner`). `Arc::get_mut` can fail for each of them, so the last drop
        // would then recurse into the value.
        let Some(repr) = Arc::into_inner(repr) else {
            return;
        };
        let root = match repr {
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
                    Child::Node(node) => {
                        nodes.extend(Arc::into_inner(node).map(|node| node.children))
                    }
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
        let hamt = match &*self.repr {
            Repr::Flat(pairs) => {
                let size = pairs.len() as u64;
                let path = path.small_integer().map(unsigned_val);
                return Ok(path.filter(|path| *path <= size).map(|_| Chunk {
                    pairs: pairs.iter().map(|(key, value)| (key, value)).collect(),
                    next: None,
                    counted: 0,
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

    /// Return true when the two maps are copies of one map: they hold the
    /// same form, as two ERTS map terms that are the same boxed pointer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))])?;
    /// assert!(map.same_form(&map.clone()));
    /// assert!(!map.same_form(&Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))])?));
    /// ```
    pub(crate) fn same_form(&self, other: &Map) -> bool {
        Arc::ptr_eq(&self.repr, &other.repr)
    }

    /// Return the number of steps of the ERTS hashmap merge for
    /// `maps:merge(self, other)`, or `None` when ERTS does not run it
    /// (`maps_merge_2`, `erl_map.c:1257-1284`). Two flatmaps merge with
    /// `flatmap_merge`, and an empty flatmap with a hashmap gives the
    /// hashmap (`erl_map.c:1440`). Each other pair runs `hashmap_merge`
    /// (`erl_map.c:1558-1851`), after the flatmap of a pair becomes a tree
    /// of its keys (`map_merge_mixed`, `erl_map.c:1425-1466`). See
    /// [`merge_walk`] for the steps.
    ///
    /// The caller must merge the two maps first: each key of a merge that
    /// gives a hashmap then has a map hash.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = |keys: std::ops::Range<i64>| Map::from_pairs(keys.map(|key| (Term::Integer(key), Term::Integer(key))).collect());
    /// assert_eq!(map(0..10)?.merge_steps(&map(5..20)?), None);
    /// assert_eq!(map(0..0)?.merge_steps(&map(0..40)?), None);
    /// // ERTS 29.1.1: maps:merge/2 of two maps of the keys 1..40 costs 8.
    /// assert_eq!(map(1..41)?.merge_steps(&map(1..41)?).map(|steps| steps / 8), Some(8));
    /// // Two copies of one hashmap share the root node.
    /// let big = map(1..41)?;
    /// assert_eq!(big.merge_steps(&big.clone()), Some(0));
    /// ```
    pub(crate) fn merge_steps(&self, other: &Map) -> Option<usize> {
        let both_flat = matches!((&*self.repr, &*other.repr), (Repr::Flat(_), Repr::Flat(_)));
        if both_flat || self.len() == 0 || other.len() == 0 {
            return None;
        }
        fn hashes(map: &Map) -> Vec<(u64, &Term)> {
            map.iter()
                .map(|(key, _)| {
                    (
                        map_hash(key).expect("a key of a merged hashmap has a hash"),
                        key,
                    )
                })
                .collect()
        }
        // The tree of a flatmap is new (`map_merge_mixed`), so it shares no node.
        let roots = match (&*self.repr, &*other.repr) {
            (Repr::Hash(first), Repr::Hash(second)) => Some((&first.root, &second.root)),
            _ => None,
        };
        Some(merge_walk(&hashes(self), &hashes(other), roots, 0))
    }
}

/// Return the steps that ERTS `hashmap_merge` uses below a slot path that
/// both maps use, at `level`. `first` and `second` hold the hash and the key
/// of each key of the two maps under the path. `nodes` holds the HAMT nodes
/// of the two maps at the path, when both maps have a node there. The walk
/// uses one step (`--reds`, `erl_map.c:1736`) when it starts to build a
/// node, and one step when a merged child returns to its node
/// (`erl_map.c:1706-1727`). A HAMT node has a child in a slot when a key
/// hash has that slot path, so the steps depend only on the key hashes and
/// on the nodes that the two maps share:
///
/// - The two maps hold the same node: ERTS takes it with no step
///   (`erl_map.c:1608-1612`).
/// - At the collision level 16, the walk merges the collision nodes with no
///   step (`erl_map.c:1728-1734`).
/// - One key in each map, and the two keys are equal: the child of the
///   second map is the result, with no step (`erl_map.c:1617-1629`).
/// - Else the build costs 1, and each slot that both maps use one level down
///   costs 1 for the return of its child and the steps of that child
///   (`erl_map.c:1742-1766`).
///
/// # Examples
///
/// ```rust,ignore
/// let (one, two) = (Term::Integer(1), Term::Integer(2));
/// assert_eq!(merge_walk(&[(7, &one)], &[(7, &one)], None, 1), 0);
/// // The root, and the return of slot 7, and the build at level 1.
/// assert_eq!(merge_walk(&[(7, &one)], &[(0x17, &two)], None, 0), 3);
/// assert_eq!(merge_walk(&[(7, &one)], &[(7, &two)], None, 16), 0);
/// let node = Node::empty();
/// assert_eq!(merge_walk(&[(7, &one)], &[(0x17, &two)], Some((&node, &node)), 0), 0);
/// ```
fn merge_walk(
    first: &[(u64, &Term)],
    second: &[(u64, &Term)],
    nodes: Option<(&Node, &Node)>,
    level: u32,
) -> usize {
    if nodes.is_some_and(|(first, second)| std::ptr::eq(first, second)) || level == HAMT_MAX_LEVEL {
        return 0;
    }
    if let ([(_, first)], [(_, second)]) = (first, second)
        && first == second
    {
        return 0;
    }
    fn slots<'a>(keys: &[(u64, &'a Term)], level: u32) -> [Vec<(u64, &'a Term)>; 16] {
        let mut slots: [Vec<(u64, &Term)>; 16] = Default::default();
        for &(hash, key) in keys {
            slots[slot(hash, level) as usize].push((hash, key));
        }
        slots
    }
    // The nodes of the two maps in a slot, when both maps have a node there.
    let below = |slot| {
        let (first, second) = nodes?;
        match (first.child(slot)?, second.child(slot)?) {
            (Child::Node(first), Child::Node(second)) => Some((&**first, &**second)),
            _ => None,
        }
    };
    let children = slots(first, level).into_iter().zip(slots(second, level));
    let shared = (0..).zip(children);
    let shared = shared.filter(|(_, (first, second))| !first.is_empty() && !second.is_empty());
    1 + shared
        .map(|(slot, (first, second))| 1 + merge_walk(&first, &second, below(slot), level + 1))
        .sum::<usize>()
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
        // A whole collision node above the limit leaves `remaining` at 0.
        let counted = limit - remaining;
        Ok(Some(Chunk {
            pairs,
            next,
            counted,
        }))
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
                Child::Node(inner) => node = Arc::make_mut(inner),
                Child::Leaf(existing, existing_value) => {
                    if *existing == key {
                        *existing_value = value;
                        return;
                    }
                    let existing_hash = map_hash(existing).expect("a stored key has a hash");
                    let existing = (
                        existing_hash,
                        std::mem::replace(existing, Term::nil()),
                        std::mem::replace(existing_value, Term::nil()),
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

    /// Return true when the node uses all 16 slots. A hashmap root with all
    /// slots has the ERTS `HEAD_ARRAY` header.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!Node::empty().is_full());
    /// ```
    pub(super) fn is_full(&self) -> bool {
        self.bitmap == FULL_BITMAP
    }

    /// Return the children in ascending slot order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Node::empty().children().is_empty());
    /// ```
    pub(super) fn children(&self) -> &[Child] {
        &self.children
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
            Child::Node(Arc::new(Node {
                bitmap: (1 << first_slot) | (1 << second_slot),
                children: vec![Child::Leaf(low.1, low.2), Child::Leaf(high.1, high.2)],
            }))
        };
        for depth in (level..depth).rev() {
            child = Child::Node(Arc::new(Node {
                bitmap: 1 << slot(path, depth),
                children: vec![child],
            }));
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
/// let list = |atom: &Term| Term::list_from([atom.clone()]);
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
/// of a key, or `None` for a key that a hashmap does not support yet. The
/// map hash is the ERTS internal hash, so an atom hashes as its word
/// `(index << 6) | 0xB`, a small integer as its word `(value << 4) | 0xF`, and
/// a list key that is not empty with the list path of `make_internal_hash`,
/// also an improper list whose tail is an atom, a small integer, or a
/// supported tuple. A tuple key hashes with the tuple path
/// (`erl_term_hashing.c:1878-1893`): its type and arity, then its elements.
/// A binary key hashes with the binary path of `make_internal_hash`. A
/// binary in a list or a tuple key is not supported yet.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(map_hash(&Term::Integer(1)), Some(7938660269479274665));
/// // true has index 1, so its word is 0x4B.
/// assert_eq!(map_hash(&Term::static_atom("true")), Some(14558535449716669319));
/// assert_eq!(map_hash(&Term::nil()), Some(internal_hash(&Term::nil())));
/// assert!(map_hash(&Term::list_from([Term::Integer(97)])).is_some());
/// assert_eq!(map_hash(&Term::Integer(1 << 59)), None);
/// assert_eq!(map_hash(&Term::tuple(vec![])), Some(4510301282025278020));
/// assert_eq!(map_hash(&Term::binary(vec![])), Some(10873612744977862784));
/// assert_eq!(map_hash(&Term::list_from([Term::binary(vec![])])), None);
/// assert_eq!(map_hash(&Term::tuple(vec![Term::binary(vec![])])), None);
/// let improper = |tail| Term::cons(Term::Integer(1), tail);
/// assert!(map_hash(&improper(Term::Integer(2))).is_some());
/// assert_eq!(map_hash(&improper(Term::Map(Map::new()))), None);
/// ```
fn map_hash(key: &Term) -> Option<u64> {
    if let Term::Binary(_) = key {
        return Some(internal_hash(key));
    }
    let mut pending = vec![key];
    while let Some(term) = pending.pop() {
        match term {
            Term::Atom(_) | Term::Nil => {}
            Term::Cons(cell) => pending.extend([&cell.tail, &cell.head]),
            Term::Tuple(elements) => pending.extend(elements.iter()),
            _ => {
                term.small_integer()?;
            }
        }
    }
    Some(internal_hash(key))
}

#[cfg(test)]
#[path = "../../tests/term/map.rs"]
mod tests;
