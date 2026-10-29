//! The process dictionary, as the linear hash table of ERTS
//! `erl_process_dict.c`. The list order of `get/0`, `get_keys/0,1`, and
//! `erase/0` comes from the bucket array and the order of the entries in each
//! bucket, and both come from the full sequence of puts and erases. Thus the
//! table does the ERTS bucket choice (`pd_hash_value_to_ix`), the prepend of
//! a new entry (`pd_hash_put`), the bucket splits of `grow`, and the bucket
//! merges of `shrink`, with the same thresholds and step counts.
//!
//! ERTS keeps one `{Key, Value}` tuple, or a list of them, in each bucket.
//! This table keeps the pairs in one array and links the pairs of a bucket
//! with an index chain, so a bucket costs no allocation of its own.

use crate::term::{LiveBytes, Term, internal_hash};

/// The number of buckets of a new table (`erts_pd_initial_size` in
/// `erl_init.c:140`). ERTS can change it with `+hpds`; PBeam has no such flag.
const INITIAL_SIZE: usize = 8;
/// The bucket count that `grow` does not pass (`MAX_HASH`,
/// `erl_process_dict.c:52`).
const MAX_HASH: usize = 1_342_177_280;
/// The end of a chain, and an empty bucket.
const NONE: u32 = u32::MAX;
/// The width of the tag of an ERTS small integer word (`_TAG_IMMED1_SIZE`).
const SMALL_INTEGER_TAG_BITS: u32 = 4;

/// Return the index of a pair as the `u32` of a chain: a dictionary has fewer
/// than `MAX_HASH` buckets and at most as many pairs as the chains hold.
fn link_index(index: usize) -> u32 {
    u32::try_from(index).expect("a pair index of a dictionary fits in u32")
}

/// Return the index of a pair from the `u32` of a chain.
fn slot(index: u32) -> usize {
    usize::try_from(index).expect("a u32 fits in usize")
}

/// Return a masked hash as a bucket index: the mask is below `MAX_HASH * 2`.
fn word(value: u64) -> usize {
    usize::try_from(value).expect("a u64 fits in usize on a 64-bit host")
}

/// Return the ERTS `sizeMask` of a bucket count.
fn size_mask(mask: usize) -> u64 {
    u64::try_from(mask).expect("a usize fits in u64")
}

/// The chain data of one pair: the `MAKE_HASH` value of its key and the index
/// of the next pair of its bucket.
#[derive(Clone, Copy, Debug)]
struct Link {
    hash: u64,
    next: u32,
}

/// The process dictionary of one process, as ERTS `ProcDict`.
///
/// An empty `heads` is the table that ERTS has not made yet
/// (`p->dictionary == NULL`): a new process, and a process after `erase/0`.
#[derive(Debug, Default)]
pub(in crate::runtime) struct Dictionary {
    /// The key and value pairs, in no order. `links` has the chain data of
    /// each pair at the same index. A process exit moves this array to the
    /// release work list.
    entries: Vec<(Term, Term)>,
    /// The chain data of each pair of `entries`.
    links: Vec<Link>,
    /// The first pair of each bucket, or [`NONE`]. The length is the ERTS
    /// `usedSlots` (`HASH_RANGE`).
    heads: Vec<u32>,
    /// The ERTS `sizeMask`: two times the home size, minus 1.
    size_mask: u64,
    /// The ERTS `splitPosition`: the next bucket that `grow` splits.
    split_position: usize,
}

impl Dictionary {
    /// Return the value of `key`, as ERTS `erts_pd_hash_get`
    /// (`erl_process_dict.c:463-498`) does. Keys compare with exact equality.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut dictionary = Dictionary::default();
    /// dictionary.put(Term::Integer(1), Term::Integer(2));
    /// assert_eq!(dictionary.get(&Term::Integer(1)), Some(&Term::Integer(2)));
    /// ```
    pub(in crate::runtime) fn get(&self, key: &Term) -> Option<&Term> {
        if self.heads.is_empty() {
            return None;
        }
        let (_, found) = self.find(self.bucket(make_hash(key)), key);
        found.map(|index| &self.entries[index].1)
    }

    /// Set the value of `key` and return the old value, as ERTS `pd_hash_put`
    /// (`erl_process_dict.c:628-760`) does. A new key goes to the front of its
    /// bucket. A new value that is not immediate moves the key to the front
    /// of a bucket of two or more pairs (`erl_process_dict.c:679-691`, `740`).
    /// A new key grows the table when the bucket count is not more than the
    /// number of pairs.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut dictionary = Dictionary::default();
    /// assert_eq!(dictionary.put(Term::Integer(1), Term::Integer(2)), None);
    /// assert_eq!(dictionary.put(Term::Integer(1), Term::Integer(3)), Some(Term::Integer(2)));
    /// ```
    pub(in crate::runtime) fn put(&mut self, key: Term, value: Term) -> Option<Term> {
        if self.heads.is_empty() {
            self.heads = vec![NONE; INITIAL_SIZE];
            self.size_mask = size_mask(INITIAL_SIZE * 2 - 1);
            self.split_position = 0;
        }
        let hash = make_hash(&key);
        let bucket = self.bucket(hash);
        let (previous, found) = self.find(bucket, &key);
        let old = if let Some(index) = found {
            let single = previous.is_none() && self.links[index].next == NONE;
            let old = std::mem::replace(&mut self.entries[index].1, value);
            if single || is_immediate(&self.entries[index].1) {
                return Some(old);
            }
            self.unlink(bucket, previous, index);
            self.links[index].next = self.heads[bucket];
            self.heads[bucket] = link_index(index);
            Some(old)
        } else {
            self.links.push(Link {
                hash,
                next: self.heads[bucket],
            });
            self.heads[bucket] = link_index(self.entries.len());
            self.entries.push((key, value));
            None
        };
        if self.heads.len() <= self.entries.len() {
            self.grow();
        }
        old
    }

    /// Remove `key` and return its value, as ERTS `pd_hash_erase`
    /// (`erl_process_dict.c:385-436`) does. Each erase on a made table, also
    /// of a missing key, shrinks the table when it has more than
    /// [`INITIAL_SIZE`] buckets and half of the buckets are more than the
    /// pairs.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut dictionary = Dictionary::default();
    /// dictionary.put(Term::Integer(1), Term::Integer(2));
    /// assert_eq!(dictionary.erase(&Term::Integer(1)), Some(Term::Integer(2)));
    /// assert_eq!(dictionary.erase(&Term::Integer(1)), None);
    /// ```
    pub(in crate::runtime) fn erase(&mut self, key: &Term) -> Option<Term> {
        if self.heads.is_empty() {
            return None;
        }
        let bucket = self.bucket(make_hash(key));
        let (previous, found) = self.find(bucket, key);
        let value = found.map(|index| {
            self.unlink(bucket, previous, index);
            self.remove(index)
        });
        let range = self.heads.len();
        if range > INITIAL_SIZE && range / 2 > self.entries.len() {
            self.shrink();
        }
        value
    }

    /// Return the pairs in the order of ERTS `pd_hash_get_all`
    /// (`erl_process_dict.c:578-626`): ERTS walks the buckets from the first
    /// and each bucket from its front, and puts each pair at the front of
    /// the result, so the result is the reverse of the walk.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut dictionary = Dictionary::default();
    /// dictionary.put(Term::Integer(1), Term::Integer(2));
    /// dictionary.put(Term::Integer(2), Term::Integer(3));
    /// assert_eq!(dictionary.pairs().count(), 2);
    /// ```
    pub(in crate::runtime) fn pairs(&self) -> impl Iterator<Item = &(Term, Term)> {
        self.order().into_iter().map(|index| &self.entries[index])
    }

    /// Remove all pairs and return them in the order of [`Dictionary::pairs`],
    /// as ERTS `erase_0` (`erl_process_dict.c:363-371`) does. The table goes
    /// back to the state of a new process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut dictionary = Dictionary::default();
    /// dictionary.put(Term::Integer(1), Term::Integer(2));
    /// assert_eq!(dictionary.erase_all().len(), 1);
    /// assert_eq!(dictionary.get(&Term::Integer(1)), None);
    /// ```
    pub(in crate::runtime) fn erase_all(&mut self) -> Vec<(Term, Term)> {
        let order = self.order();
        let mut entries = self.take_entries();
        order
            .into_iter()
            .map(|index| std::mem::replace(&mut entries[index], (Term::Nil, Term::Nil)))
            .collect()
    }

    /// Move the pairs out and leave the table of a new process, for the
    /// release work list of a process exit. The pairs are in no order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut dictionary = Dictionary::default();
    /// dictionary.put(Term::Integer(1), Term::Integer(2));
    /// assert_eq!(dictionary.take_entries().len(), 1);
    /// ```
    pub(in crate::runtime) fn take_entries(&mut self) -> Vec<(Term, Term)> {
        std::mem::take(self).entries
    }

    /// Return the bytes of the pair array, the chain array, and the bucket
    /// array: the capacity of each array times the size of one item.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut dictionary = Dictionary::default();
    /// assert_eq!(dictionary.allocated_bytes(), 0);
    /// dictionary.put(Term::Integer(1), Term::Integer(2));
    /// assert!(dictionary.allocated_bytes() > 0);
    /// ```
    pub(in crate::runtime) fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<(Term, Term)>()
            + self.links.capacity() * std::mem::size_of::<Link>()
            + self.heads.capacity() * std::mem::size_of::<u32>()
    }

    /// Add the terms of each pair to `live`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut live = LiveBytes::new();
    /// dictionary.account(&mut live);
    /// ```
    pub(in crate::runtime) fn account(&self, live: &mut LiveBytes) {
        for (key, value) in &self.entries {
            live.term(key);
            live.term(value);
        }
    }

    /// Return the indexes of the pairs in the order of [`Dictionary::pairs`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let first = dictionary.order()[0];
    /// ```
    fn order(&self) -> Vec<usize> {
        let mut walk = Vec::with_capacity(self.entries.len());
        for &head in &self.heads {
            let mut index = head;
            while index != NONE {
                walk.push(slot(index));
                index = self.links[slot(index)].next;
            }
        }
        walk.reverse();
        walk
    }

    /// Return the bucket of a `MAKE_HASH` value, as ERTS
    /// `pd_hash_value_to_ix` (`erl_process_dict.c:1008-1020`) does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // A new table has 8 buckets and the size mask 15.
    /// assert_eq!(dictionary.bucket(13), 5);
    /// ```
    fn bucket(&self, hash: u64) -> usize {
        let high = word(hash & self.size_mask);
        if high >= self.heads.len() {
            word(hash & (self.size_mask >> 1))
        } else {
            high
        }
    }

    /// Return the pair before `key` in `bucket` and the pair of `key`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (previous, found) = dictionary.find(bucket, &key);
    /// ```
    fn find(&self, bucket: usize, key: &Term) -> (Option<usize>, Option<usize>) {
        let mut previous = None;
        let mut index = self.heads[bucket];
        while index != NONE {
            let current = slot(index);
            if self.entries[current].0 == *key {
                return (previous, Some(current));
            }
            previous = Some(current);
            index = self.links[current].next;
        }
        (previous, None)
    }

    /// Remove the pair `index` from the chain of `bucket`, where `previous`
    /// is the pair before it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (previous, Some(index)) = dictionary.find(bucket, &key) else { return };
    /// dictionary.unlink(bucket, previous, index);
    /// ```
    fn unlink(&mut self, bucket: usize, previous: Option<usize>, index: usize) {
        let next = self.links[index].next;
        match previous {
            Some(previous) => self.links[previous].next = next,
            None => self.heads[bucket] = next,
        }
    }

    /// Remove the unlinked pair `index` from the arrays and return its value.
    /// The last pair moves to `index`, so the link to the last pair changes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// dictionary.unlink(bucket, previous, index);
    /// let value = dictionary.remove(index);
    /// ```
    fn remove(&mut self, index: usize) -> Term {
        let last = self.entries.len() - 1;
        if index != last {
            let bucket = self.bucket(self.links[last].hash);
            let mut current = self.heads[bucket];
            if current == link_index(last) {
                self.heads[bucket] = link_index(index);
            } else {
                while self.links[slot(current)].next != link_index(last) {
                    current = self.links[slot(current)].next;
                }
                self.links[slot(current)].next = link_index(index);
            }
        }
        self.links.swap_remove(index);
        self.entries.swap_remove(index).1
    }

    /// Split buckets, as ERTS `grow` (`erl_process_dict.c:852-954`) does: a
    /// quarter of the bucket count, at most 15 and at least 1. Each split
    /// walks the chain of the bucket at the split position and puts each pair
    /// at the front of the chain of the bucket that it hashes to now: that
    /// bucket, or the new last bucket.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The ninth pair of a new table grows it from 8 to 10 buckets.
    /// dictionary.grow();
    /// ```
    fn grow(&mut self) {
        let steps = ((self.heads.len() / 4) & 0xf).max(1);
        // ERTS does no step when the steps would reach MAX_HASH buckets.
        let steps = steps * usize::from(MAX_HASH - steps > self.heads.len());
        let mut home = self.heads.len() - self.split_position;
        for _ in 0..steps {
            if self.split_position == home {
                home *= 2;
                self.size_mask = size_mask(home * 2 - 1);
                self.split_position = 0;
            }
            let position = self.split_position;
            self.split_position += 1;
            self.heads.push(NONE);
            let (mut stay, mut moved) = (NONE, NONE);
            let mut index = self.heads[position];
            while index != NONE {
                let Link { hash, next } = self.links[slot(index)];
                if self.bucket(hash) == position {
                    self.links[slot(index)].next = stay;
                    stay = index;
                } else {
                    self.links[slot(index)].next = moved;
                    moved = index;
                }
                index = next;
            }
            self.heads[position] = stay;
            self.heads[position + home] = moved;
        }
    }

    /// Merge buckets, as ERTS `shrink` (`erl_process_dict.c:766-850`) does:
    /// 30 percent of the bucket count, but not below [`INITIAL_SIZE`]. Each
    /// step merges the last bucket into the bucket at the split position.
    /// When that bucket has one pair, the last chain goes after it. Else the
    /// pairs of the last chain go to its front one by one, so the last chain
    /// comes first and in reverse order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// dictionary.shrink();
    /// ```
    fn shrink(&mut self) {
        let range = self.heads.len();
        let steps = (range * 3 / 10).min(range - INITIAL_SIZE);
        for _ in 0..steps {
            if self.split_position == 0 {
                self.size_mask = size_mask(self.heads.len() - 1);
                self.split_position = self.heads.len() / 2;
            }
            self.split_position -= 1;
            let high = self.heads.pop().unwrap_or(NONE);
            let low = self.heads[self.split_position];
            if high == NONE {
                continue;
            }
            if low == NONE {
                self.heads[self.split_position] = high;
            } else if self.links[slot(low)].next == NONE {
                self.links[slot(low)].next = high;
            } else {
                let (mut front, mut index) = (low, high);
                while index != NONE {
                    let next = self.links[slot(index)].next;
                    self.links[slot(index)].next = front;
                    front = index;
                    index = next;
                }
                self.heads[self.split_position] = front;
            }
        }
    }
}

/// Return the ERTS `MAKE_HASH` value of a key (`erl_process_dict.c:58-62`):
/// the value bits of a small integer, the index of an atom, or else the ERTS
/// internal hash.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(make_hash(&Term::Integer(5)), 5);
/// assert_eq!(make_hash(&Term::Integer(-1)), (1 << 60) - 1);
/// assert_eq!(make_hash(&Term::static_atom("true")), 1);
/// ```
fn make_hash(key: &Term) -> u64 {
    match key {
        Term::Atom(atom) => u64::from(atom.index()),
        _ => match key.small_integer() {
            Some(value) => value.cast_unsigned() & (u64::MAX >> SMALL_INTEGER_TAG_BITS),
            None => internal_hash(key),
        },
    }
}

/// Return true for a term that ERTS keeps in one word (`is_immed`): an atom,
/// a small integer, a local pid, a local port, or `[]`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_immediate(&Term::Integer(1)));
/// assert!(!is_immediate(&Term::tuple(vec![])));
/// ```
fn is_immediate(term: &Term) -> bool {
    match term {
        Term::Atom(_) | Term::Pid(_) | Term::Port(_) | Term::Nil => true,
        _ => term.small_integer().is_some(),
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/process/dictionary.rs"]
mod tests;
