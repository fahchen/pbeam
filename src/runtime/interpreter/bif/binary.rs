//! The BIFs `binary:match/2`, `binary:matches/2`, and `binary:split/2,3`:
//! the Boyer-Moore and Aho-Corasick searches of ERTS `erl_bif_binary.c`,
//! with their reduction budget and their traps, and the match, matches, and
//! split results. The file also holds `binary:at/2`, `binary:part/2,3`, and
//! `binary:copy/1,2`.

use super::BifResult::Value;
use super::*;
use crate::term::Binary;

/// The option `global`: find each match (`BF_FLAG_GLOBAL`,
/// `erl_bif_binary.c:1118`).
const GLOBAL: u8 = 0x01;
/// The option `trim`: remove the empty parts at the end
/// (`BF_FLAG_SPLIT_TRIM`, `erl_bif_binary.c:1119`).
const TRIM: u8 = 0x02;
/// The option `trim_all`: remove each empty part
/// (`BF_FLAG_SPLIT_TRIM_ALL`, `erl_bif_binary.c:1120`).
const TRIM_ALL: u8 = 0x04;
/// PBeam only: the search gives the result of `binary:match/2`, not of
/// `binary:split/2,3`. ERTS keeps the result functions of the BIF in the
/// search context instead (`bf_context_init` with
/// `do_match_not_found_result` and `do_match_single_result`,
/// `erl_bif_binary.c:1559-1560`, `1647-1673`).
const MATCH: u8 = 0x08;
/// PBeam only: the search gives the result of `binary:matches/2`, not of
/// `binary:match/2`. ERTS keeps the result functions of the BIF in the
/// search context instead (`bf_context_init` with
/// `do_match_global_result`, `erl_bif_binary.c:1559-1560`, `1672-1733`).
const MATCHES: u8 = 0x10;

/// The bits of `binary:copy/1,2` that one reduction covers
/// (`BITSTRING_COPY_LOOP_FACTOR = ERTS_CACHE_LINE_SIZE * 8`,
/// `erl_bif_binary.c:2377`, `2445`). `ERTS_CACHE_LINE_SIZE` is
/// `ASSUMED_CACHE_LINE_SIZE` (`erl_alloc.h:242`), which the pinned build
/// leaves at the default of 64 bytes.
const COPY_BITS_PER_REDUCTION: u64 = 64 * 8;

/// The search steps for each reduction: `BM_LOOP_FACTOR` and
/// `AC_LOOP_FACTOR` (`erl_bif_binary.c:635`, `824`), both 10.
const LOOP_FACTOR: u64 = 10;

/// The bytes that the `memchr` skip of a Boyer-Moore search reads for each
/// search step (`MC_LOOP_FACTOR`, `erl_bif_binary.c:825`).
const MEMCHR_FACTOR: i64 = 8;

/// The root node of an Aho-Corasick trie. A transition to 0 is no
/// transition, because no node goes to the root.
const ROOT: usize = 0;

/// A compiled pattern, as `do_binary_match_compile` makes it
/// (`erl_bif_binary.c:1004-1101`): one needle gives a Boyer-Moore search,
/// and more needles give an Aho-Corasick search.
enum Pattern {
    /// The Boyer-Moore data of one needle (`BMData`).
    Bm(Bm),
    /// The Aho-Corasick trie of two or more needles (`ACTrie`).
    Ac(Vec<AcNode>),
}

/// The Boyer-Moore data of a needle, as `create_bmdata` makes it
/// (`erl_bif_binary.c:470-501`).
struct Bm {
    /// The needle.
    needle: Vec<u8>,
    /// The bad-character shift of each byte (`compute_badshifts`).
    badshift: Box<[i64; 256]>,
    /// The good-suffix shift of each needle position
    /// (`compute_goodshifts`).
    goodshift: Vec<i64>,
}

/// A node of an Aho-Corasick trie (`ACNode`, `erl_bif_binary.c:178-198`).
struct AcNode {
    /// The depth of the node: the length of the bytes that lead to it.
    depth: u64,
    /// The final mark: above 0 for the end of a needle, below 0 when a node
    /// on the failure chain ends a needle, and 0 for no needle.
    end: i32,
    /// The failure node.
    failure: usize,
    /// The node for each next byte, or [`ROOT`] for no transition.
    next: Box<[usize; 256]>,
}

/// The state of a search and of the result build, as
/// `BinaryFindContext` keeps it (`erl_bif_binary.c:252-310`). A trap gives
/// it to the next turn as a term ([`Find::to_term`]).
struct Find {
    /// The options `global`, `trim`, and `trim_all`, and the result kind
    /// [`MATCH`].
    flags: u8,
    /// The end of the search scope (`hsend`).
    end: u64,
    /// The next search position.
    pos: i64,
    /// The current node of an Aho-Corasick search.
    node: usize,
    /// The node and the start of the first match of an Aho-Corasick search
    /// so far (`candidate`, `candidate_start`).
    candidate: Option<(usize, u64)>,
    /// The start and the length of each match of a global search.
    matches: Vec<(u64, u64)>,
    /// The state of the result build of a global search, after the search.
    build: Option<Build>,
}

/// The state of the result build after a global search, as
/// `BinaryFindAllContext` keeps it (`erl_bif_binary.c:252-310`): the state
/// of `do_split_global_result` (`erl_bif_binary.c:1835-1933`) or of
/// `do_match_global_result` (`erl_bif_binary.c:1672-1733`). Both builds
/// keep their state in the fields `fa->head`, `fa->end_pos`, and
/// `fa->parts`, so the fields have one meaning for each kind of build.
struct Build {
    /// The index of the next match. A split build and the tuple phase of a
    /// matches build walk down from the last match, and the integer phase
    /// of a matches build walks up from the first one.
    head: i64,
    /// The end of the part before the current match of a split build, or
    /// the phase of a matches build: 0 makes the integers of each match,
    /// and 1 conses their tuples on the result.
    end_pos: u64,
    /// The parts so far of a split build, from the last part to the first,
    /// or the `{Position, Length}` tuples so far of a matches build, from
    /// the last tuple to the first.
    parts: Vec<Term>,
}

/// How a search of the first match ends (`BFReturn`,
/// `erl_bif_binary.c:245-250`).
enum Search {
    /// The search steps of the turn ended (`BF_RESTART`).
    Restart,
    /// No needle matches (`BF_NOT_FOUND`).
    NotFound,
    /// The first match, as its start and its length.
    Found(u64, u64),
}

/// How a global search ends (`BFReturn`, `erl_bif_binary.c:245-250`). The
/// matches of a global search are in [`Find::matches`], so a found search
/// needs no position.
enum SearchAll {
    /// The search steps of the turn ended (`BF_RESTART`).
    Restart,
    /// No needle matches (`BF_NOT_FOUND`).
    NotFound,
    /// The search found matches, which are in [`Find::matches`]
    /// (`BF_FOUND`).
    FoundAll,
}

impl Bm {
    /// Compile one needle, as `create_bmdata` does
    /// (`erl_bif_binary.c:347-419`, `470-501`). ERTS computes no shifts for
    /// a needle of one byte, and a search of such a needle never reads them.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bm = Bm::new(b"abab");
    /// assert_eq!(bm.badshift[b'a' as usize], 1);
    /// assert_eq!(bm.goodshift, vec![2, 2, 4, 1]);
    /// ```
    fn new(needle: &[u8]) -> Bm {
        let m = needle.len() as i64;
        let mut badshift = Box::new([m; 256]);
        for (i, byte) in needle.iter().enumerate().take(needle.len() - 1) {
            badshift[*byte as usize] = m - i as i64 - 1;
        }
        let suffixes = suffixes(needle);
        let mut goodshift = vec![m; needle.len()];
        let mut j = 0;
        for i in (-1..m).rev() {
            if i == -1 || suffixes[i as usize] == i + 1 {
                while j < m - 1 - i {
                    if goodshift[j as usize] == m {
                        goodshift[j as usize] = m - 1 - i;
                    }
                    j += 1;
                }
            }
        }
        for i in 0..m - 1 {
            goodshift[(m - 1 - suffixes[i as usize]) as usize] = m - 1 - i;
        }
        Bm {
            needle: needle.to_vec(),
            badshift,
            goodshift,
        }
    }

    /// Skip to the next position where the last needle byte matches, as the
    /// `memchr` step of `bm_find_first_match` and
    /// `bm_find_all_non_overlapping` does (`erl_bif_binary.c:858-869`,
    /// `941-955`). The step reads at most `1 + (reds - 1) * 8` bytes from
    /// the position `j`, and uses one search step for each 8 bytes up to the
    /// match or the end of the read. ERTS computes with unsigned integers,
    /// so the steps can wrap below 0 for a needle of more than 16 bytes.
    /// Return the new position and the steps that remain.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let bm = Bm::new(b"ab");
    /// assert_eq!(bm.skip(b"xxxxxxxxxxab", 0, 11, 100), (10, 99));
    /// ```
    fn skip(&self, haystack: &[u8], j: i64, available: i64, reds: u64) -> (i64, u64) {
        let last = self.needle.len() as i64 - 1;
        let limit = reds
            .wrapping_sub(1)
            .wrapping_mul(MEMCHR_FACTOR as u64)
            .wrapping_add(1);
        let read = if (available as u64) < limit {
            available
        } else {
            limit as i64
        };
        let start = (j + last) as usize;
        let window = &haystack[start..start + read as usize];
        match window
            .iter()
            .position(|byte| *byte == self.needle[last as usize])
        {
            None => (j + read, reds.wrapping_sub((read / MEMCHR_FACTOR) as u64)),
            Some(at) => {
                let at = at as i64;
                (
                    j + at,
                    reds.wrapping_sub(((last + at) / MEMCHR_FACTOR) as u64),
                )
            }
        }
    }

    /// Compare the needle with the haystack at `j` from its last byte, and
    /// return the needle position of the first difference, or -1 for a
    /// match.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Bm::new(b"ab").compare(b"xab", 1), -1);
    /// ```
    fn compare(&self, haystack: &[u8], j: i64) -> i64 {
        let mut i = self.needle.len() as i64 - 1;
        while i >= 0 && self.needle[i as usize] == haystack[(i + j) as usize] {
            i -= 1;
        }
        i
    }

    /// Return the shift after a difference at needle position `i` and
    /// haystack position `j`: the larger of the good-suffix shift and the
    /// bad-character shift (`erl_bif_binary.c:888`, `986`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Bm::new(b"ab").shift(b"xx", 1, 0), 2);
    /// ```
    fn shift(&self, haystack: &[u8], i: i64, j: i64) -> i64 {
        let blen = self.needle.len() as i64;
        let bad = self.badshift[haystack[(i + j) as usize] as usize] - blen + 1 + i;
        self.goodshift[i as usize].max(bad)
    }

    /// Find the first match from [`Find::pos`] to [`Find::end`], as
    /// `bm_find_first_match` does (`erl_bif_binary.c:834-890`). Each
    /// comparison uses one search step, and the skip uses more. When the
    /// steps reach 0, the search keeps its position and restarts.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut reds = 100;
    /// assert!(matches!(bm.first(&mut find, b"xab", &mut reds), Search::Found(1, 2)));
    /// ```
    fn first(&self, find: &mut Find, haystack: &[u8], reds: &mut u64) -> Search {
        let blen = self.needle.len() as i64;
        let len = find.end as i64;
        let available = len - (blen - 1) - find.pos;
        if available <= 0 {
            return Search::NotFound;
        }
        let (mut j, mut steps) = self.skip(haystack, find.pos, available, *reds);
        loop {
            if j > len - blen {
                *reds = steps;
                return Search::NotFound;
            }
            steps = steps.wrapping_sub(1);
            if steps == 0 {
                find.pos = j;
                return Search::Restart;
            }
            let i = self.compare(haystack, j);
            if i < 0 {
                *reds = steps;
                return Search::Found(j as u64, blen as u64);
            }
            j += self.shift(haystack, i, j);
        }
    }

    /// Find each match from [`Find::pos`] to [`Find::end`] that does not
    /// overlap a match before it, as `bm_find_all_non_overlapping` does
    /// (`erl_bif_binary.c:918-994`). The skip runs at the start and after
    /// each match.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut reds = 100;
    /// assert!(matches!(bm.all(&mut find, b"abxab", &mut reds), SearchAll::FoundAll));
    /// assert_eq!(find.matches, [(0, 2), (3, 2)]);
    /// ```
    fn all(&self, find: &mut Find, haystack: &[u8], reds: &mut u64) -> SearchAll {
        let blen = self.needle.len() as i64;
        let len = find.end as i64;
        let mut j = find.pos;
        let mut steps = *reds;
        let mut i = -1;
        loop {
            if i < 0 {
                let available = len - (blen - 1) - j;
                if available <= 0 {
                    break;
                }
                (j, steps) = self.skip(haystack, j, available, steps);
            }
            if j > len - blen {
                break;
            }
            steps = steps.wrapping_sub(1);
            if steps == 0 {
                find.pos = j;
                return SearchAll::Restart;
            }
            i = self.compare(haystack, j);
            if i < 0 {
                find.matches.push((j as u64, blen as u64));
                j += blen;
            } else {
                j += self.shift(haystack, i, j);
            }
        }
        *reds = steps;
        if find.matches.is_empty() {
            SearchAll::NotFound
        } else {
            SearchAll::FoundAll
        }
    }
}

/// Return the suffix lengths of a needle for the good-suffix shifts, as
/// `compute_suffixes` does (`erl_bif_binary.c:361-385`): at each position,
/// the length of the longest needle suffix that ends there.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(suffixes(b"abab"), vec![0, 2, 0, 4]);
/// ```
fn suffixes(x: &[u8]) -> Vec<i64> {
    let m = x.len() as i64;
    let mut suffixes = vec![0; x.len()];
    suffixes[x.len() - 1] = m;
    let (mut f, mut g) = (0, m - 1);
    for i in (0..m - 1).rev() {
        if i > g && suffixes[(i + m - 1 - f) as usize] < i - g {
            suffixes[i as usize] = suffixes[(i + m - 1 - f) as usize];
        } else {
            g = g.min(i);
            f = i;
            while g >= 0 && x[g as usize] == x[(g + m - 1 - f) as usize] {
                g -= 1;
            }
            suffixes[i as usize] = f - g;
        }
    }
    suffixes
}

/// Build the Aho-Corasick trie of the needles, as `create_acdata`,
/// `ac_add_one_pattern`, and `ac_compute_failure_functions` do
/// (`erl_bif_binary.c:438-464`, `518-612`). Each new node gets the next
/// index. A needle that ends at a node with no mark marks it with its
/// number, from 1. The failure nodes come from a breadth-first walk in byte
/// order. A node whose failure node has a mark, and that has no mark
/// itself, gets the mark -1.
///
/// # Examples
///
/// ```rust,ignore
/// let trie = ac_trie(&[b"a", b"ab"]);
/// assert_eq!(trie.len(), 3);
/// ```
fn ac_trie(needles: &[&[u8]]) -> Vec<AcNode> {
    let node = |depth| AcNode {
        depth,
        end: 0,
        failure: ROOT,
        next: Box::new([ROOT; 256]),
    };
    let mut trie = vec![node(0)];
    for (number, needle) in (1..).zip(needles) {
        let mut current = ROOT;
        for (depth, byte) in (1..).zip(needle.iter()) {
            let byte = *byte as usize;
            if trie[current].next[byte] == ROOT {
                trie.push(node(depth));
                trie[current].next[byte] = trie.len() - 1;
            }
            current = trie[current].next[byte];
        }
        if trie[current].end == 0 {
            trie[current].end = number;
        }
    }
    let mut queue: Vec<usize> = trie[ROOT]
        .next
        .iter()
        .copied()
        .filter(|&child| child != ROOT)
        .collect();
    let mut head = 0;
    while let Some(&parent) = queue.get(head) {
        head += 1;
        for byte in 0..256 {
            let child = trie[parent].next[byte];
            if child == ROOT {
                continue;
            }
            queue.push(child);
            // The root has no failure node while the walk runs.
            let mut failure = Some(trie[parent].failure);
            while let Some(node) = failure.filter(|&node| trie[node].next[byte] == ROOT) {
                failure = (node != ROOT).then_some(trie[node].failure);
            }
            // With no failure node, the root has no transition for the
            // byte, so the failure node is the root.
            trie[child].failure = failure.map_or(ROOT, |node| trie[node].next[byte]);
            if failure.is_some() && trie[child].end == 0 && trie[trie[child].failure].end != 0 {
                trie[child].end = -1;
            }
        }
    }
    trie
}

/// Return the node after the byte at `node`: follow the failure nodes
/// until a node has a transition for the byte or is the root, then take
/// the transition if it has one (`erl_bif_binary.c:664-669`, `754-759`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(ac_next(&ac_trie(&[b"a", b"b"]), ROOT, b'a'), 1);
/// ```
fn ac_next(trie: &[AcNode], mut node: usize, byte: u8) -> usize {
    let byte = byte as usize;
    while trie[node].next[byte] == ROOT && node != ROOT {
        node = trie[node].failure;
    }
    match trie[node].next[byte] {
        ROOT => node,
        next => next,
    }
}

/// Return the node on the failure chain of `node` that ends a needle: the
/// node itself when its mark is above 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(ac_end(&trie, node_with_mark_1), node_with_mark_1);
/// ```
fn ac_end(trie: &[AcNode], mut node: usize) -> usize {
    while trie[node].end < 0 {
        node = trie[node].failure;
    }
    node
}

/// Find the match that starts first, and the longest one at that start, as
/// `ac_find_first_match` does (`erl_bif_binary.c:637-699`). Each byte uses
/// one search step. The search stops when the current node starts after the
/// first match so far. When the steps are 0 before a byte, the search
/// keeps its state and restarts.
///
/// # Examples
///
/// ```rust,ignore
/// let mut reds = 100;
/// assert!(matches!(ac_first(&trie, &mut find, b"xab", &mut reds), Search::Found(1, 2)));
/// ```
fn ac_first(trie: &[AcNode], find: &mut Find, haystack: &[u8], reds: &mut u64) -> Search {
    let mut steps = *reds;
    let mut i = find.pos as u64;
    while i < find.end {
        if steps == 0 {
            find.pos = i as i64;
            return Search::Restart;
        }
        steps -= 1;
        find.node = ac_next(trie, find.node, haystack[i as usize]);
        i += 1;
        let node = &trie[find.node];
        if find
            .candidate
            .is_some_and(|(_, start)| i - node.depth > start)
        {
            break;
        }
        if node.end != 0 {
            let end = ac_end(trie, find.node);
            let start = i - trie[end].depth;
            let better = find.candidate.is_none_or(|(candidate, candidate_start)| {
                start < candidate_start
                    || (start == candidate_start && trie[candidate].depth < node.depth)
            });
            if better {
                find.candidate = Some((end, start));
            }
        }
    }
    *reds = steps;
    match find.candidate {
        None => Search::NotFound,
        Some((end, start)) => Search::Found(start, trie[end].depth),
    }
}

/// Find each match that does not overlap a match before it, as
/// `ac_find_all_non_overlapping` does (`erl_bif_binary.c:729-822`). At each
/// byte, each needle that ends there, on the failure chain from the
/// current node, replaces the matches that start after it, or at the same
/// start and are shorter, when it does not overlap the match before them.
/// The search uses one step before each byte, and restarts when the steps
/// become 0.
///
/// # Examples
///
/// ```rust,ignore
/// let mut reds = 100;
/// assert!(matches!(ac_all(&trie, &mut find, b"abab", &mut reds), SearchAll::FoundAll));
/// ```
fn ac_all(trie: &[AcNode], find: &mut Find, haystack: &[u8], reds: &mut u64) -> SearchAll {
    let mut steps = *reds;
    let mut i = find.pos as u64;
    while i < find.end {
        steps = steps.wrapping_sub(1);
        if steps == 0 {
            find.pos = i as i64;
            return SearchAll::Restart;
        }
        find.node = ac_next(trie, find.node, haystack[i as usize]);
        i += 1;
        let mut node = find.node;
        while trie[node].end != 0 {
            node = ac_end(trie, node);
            let (start, length) = (i - trie[node].depth, trie[node].depth);
            let matches = &mut find.matches;
            let mut kept = matches.len();
            while kept > 0
                && (matches[kept - 1].0 > start
                    || (matches[kept - 1].0 == start && matches[kept - 1].1 < length))
            {
                kept -= 1;
            }
            if kept == 0 || matches[kept - 1].0 + matches[kept - 1].1 <= start {
                matches.truncate(kept);
                matches.push((start, length));
            }
            node = trie[node].failure;
        }
    }
    *reds = steps;
    if find.matches.is_empty() {
        SearchAll::NotFound
    } else {
        SearchAll::FoundAll
    }
}

/// Compile a pattern of `binary:split/2,3`, as `maybe_binary_match_compile`
/// and `do_binary_match_compile` do (`erl_bif_binary.c:1004-1101`,
/// `1244-1271`): a binary, or a proper list of binaries, each with at least
/// one byte. One needle gives a Boyer-Moore search, and more needles give an
/// Aho-Corasick search. Return `None` for each other term. ERTS also takes a
/// compiled pattern `{bm, Ref}` or `{ac, Ref}` of `binary:compile_pattern/1`,
/// which PBeam does not run, so each tuple is not valid.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(compile(&Term::binary(b",".to_vec())), Some(Pattern::Bm(_))));
/// assert!(compile(&Term::binary(Vec::new())).is_none());
/// ```
fn compile(pattern: &Term) -> Option<Pattern> {
    let needles: Vec<&[u8]> = match pattern {
        Term::Binary(needle) => vec![&needle[..]],
        _ => pattern
            .proper_list()?
            .map(|needle| match needle {
                Term::Binary(needle) => Some(&needle[..]),
                _ => None,
            })
            .collect::<Option<_>>()?,
    };
    if needles.is_empty() || needles.iter().any(|needle| needle.is_empty()) {
        return None;
    }
    Some(match needles.as_slice() {
        [needle] => Pattern::Bm(Bm::new(needle)),
        _ => Pattern::Ac(ac_trie(&needles)),
    })
}

/// Read the options of `binary:split/3` for a subject of `size` bytes, as
/// `parse_split_opts_list` does (`erl_bif_binary.c:1346-1431`), and return
/// the flags and the scope start and end. `None` (`binary:split/2`) and
/// `[]` give no flag and the whole subject. Each list cell is `global`,
/// `trim`, `trim_all`, or `{scope, {Pos, Len}}`, and the last scope counts.
/// The tail of an improper list is not read. A negative `Len` ends the
/// scope at `Pos`. A scope outside the subject, and each other term, give
/// `None`. ERTS computes the scope with unsigned integers.
///
/// # Examples
///
/// ```rust,ignore
/// let scope = Term::tuple(vec![Term::static_atom("scope"), Term::tuple(vec![Term::Integer(3), Term::Integer(-2)])]);
/// assert_eq!(options(Some(&Term::list_from(vec![scope])), 3), Some((0, 1, 3)));
/// ```
fn options(options: Option<&Term>, size: u64) -> Option<(u8, u64, u64)> {
    let (mut flags, mut start, mut end) = (0, 0, size);
    let Some(options) = options else {
        return Some((flags, start, end));
    };
    for cell in options.list_iter()? {
        match cell {
            Term::Atom(atom) if atom.is_static("global") => flags |= GLOBAL,
            Term::Atom(atom) if atom.is_static("trim") => flags |= TRIM,
            Term::Atom(atom) if atom.is_static("trim_all") => flags |= TRIM_ALL,
            Term::Tuple(option) => {
                let [Term::Atom(scope), Term::Tuple(range)] = option.as_ref() else {
                    return None;
                };
                let [Term::Integer(pos), Term::Integer(len)] = range.as_ref() else {
                    return None;
                };
                if !scope.is_static("scope") || *pos < 0 {
                    return None;
                }
                let (mut pos, mut len) = (*pos as u64, *len as u64);
                if (len as i64) < 0 {
                    len = len.wrapping_neg();
                    if (len as i64) < 0 {
                        return None;
                    }
                    pos = pos.wrapping_sub(len);
                }
                let stop = pos.wrapping_add(len);
                if stop < pos || size < pos || size < stop {
                    return None;
                }
                (start, end) = (pos, stop);
            }
            _ => return None,
        }
    }
    Some((flags, start, end))
}

/// Return the result of a search with no match, as
/// `do_match_not_found_result` (`erl_bif_binary.c:1647-1652`) and
/// `do_split_not_found_result` (`erl_bif_binary.c:1737-1751`) do: `nomatch`
/// for `binary:match/2`, `[]` for each global match search, and the subject
/// in a list for `binary:split/2,3`, or `[]` for an empty subject with
/// `trim` or `trim_all`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(not_found(0, &Binary::from(&[1, 2][..])), Term::list_from(vec![Term::binary(vec![1, 2])]));
/// assert_eq!(not_found(TRIM, &Binary::from(&[][..])), Term::nil());
/// assert_eq!(not_found(MATCHES, &Binary::from(&[1][..])), Term::nil());
/// ```
fn not_found(flags: u8, subject: &Binary) -> Term {
    if flags & MATCHES != 0 {
        return Term::nil();
    }
    if flags & (TRIM | TRIM_ALL) != 0 && subject.is_empty() {
        return Term::nil();
    }
    // ERTS puts a sub-binary of the subject in the result (`binary_split`,
    // `erl_bif_binary.c:1599-1645`), so the part shares the bytes of the
    // subject.
    Term::list_from(vec![Term::Binary(subject.bits(0, subject.len() * 8))])
}

/// Return the result of the first match at `pos` with `len` bytes, as
/// `do_split_single_result` does (`erl_bif_binary.c:1753-1833`): the part
/// before the match and the part after it. With `trim` or `trim_all`, a
/// match at the end of the subject gives only the part before it, or `[]`
/// when that part is empty. With `trim_all`, a match at the start gives
/// only the part after it.
///
/// # Examples
///
/// ```rust,ignore
/// let parts = split_once(0, &Binary::from(&[97, 44, 98][..]), 1, 1);
/// assert_eq!(parts, Term::list_from(vec![Term::binary(b"a".to_vec()), Term::binary(b"b".to_vec())]));
/// ```
fn split_once(flags: u8, subject: &Binary, pos: u64, len: u64) -> Term {
    let (pos, after) = (pos as usize, (pos + len) as usize);
    let size = subject.len() * 8;
    // ERTS puts sub-binaries of the subject in the result
    // (`do_split_single_result`, `erl_bif_binary.c:1753-1833`), so each part
    // shares the bytes of the subject.
    let first = Term::Binary(subject.bits(0, pos * 8));
    if flags & (TRIM | TRIM_ALL) != 0 && after == subject.len() {
        return Term::list_from(if pos > 0 { vec![first] } else { Vec::new() });
    }
    let rest = Term::Binary(subject.bits(after * 8, size));
    Term::list_from(if flags & TRIM_ALL != 0 && pos == 0 {
        vec![rest]
    } else {
        vec![first, rest]
    })
}

/// Build the parts of a global search from the last match to the first, as
/// `do_split_global_result` does (`erl_bif_binary.c:1835-1933`). Each match
/// uses one search step before its part. With `trim`, the empty parts at
/// the end are left out, and with `trim_all`, each empty part is left out.
/// When the steps become 0, the build keeps its state and returns `None`.
/// A build that restarts after a part that is not empty leaves out no more
/// part for `trim` (`erl_bif_binary.c:1888-1890`).
///
/// # Examples
///
/// ```rust,ignore
/// find.matches = vec![(1, 1)];
/// let mut reds = 100;
/// assert_eq!(split_all(&mut find, &Binary::from(&[97, 44, 98][..]), &mut reds), Some(Term::list_from(vec![Term::binary(b"a".to_vec()), Term::binary(b"b".to_vec())])));
/// ```
fn split_all(find: &mut Find, subject: &Binary, reds: &mut u64) -> Option<Term> {
    let matches = &find.matches;
    let build = find.build.get_or_insert_with(|| Build {
        head: matches.len() as i64 - 1,
        end_pos: subject.len() as u64,
        parts: Vec::new(),
    });
    let mut trim = find.flags & (TRIM | TRIM_ALL);
    let mut steps = *reds;
    // ERTS puts sub-binaries of the subject in the result
    // (`do_split_global_result`, `erl_bif_binary.c:1835-1933`), so each part
    // shares the bytes of the subject. A part is empty when its end is at or
    // before its start: the build can pass the end of a part to the next one
    // after a trap, and `Binary::bits` needs a range that does not go
    // backwards.
    let part_at =
        |start: u64, end: u64| subject.bits(start as usize * 8, end.max(start) as usize * 8);
    while build.head >= 0 {
        steps = steps.wrapping_sub(1);
        if steps == 0 {
            if trim == 0 {
                find.flags &= !TRIM;
            }
            return None;
        }
        let (pos, len) = matches[build.head as usize];
        let part = part_at(pos + len, build.end_pos);
        if !(part.is_empty() && trim != 0) {
            build.parts.push(Term::Binary(part));
            trim &= !TRIM;
        }
        build.end_pos = pos;
        build.head -= 1;
    }
    *reds = steps;
    let first = part_at(0, matches[0].0);
    if !(first.is_empty() && trim != 0) {
        build.parts.push(Term::Binary(first));
    }
    let mut parts = std::mem::take(&mut build.parts);
    parts.reverse();
    Some(Term::list_from(parts))
}

/// Build the result of `binary:matches/2` from the matches of the search,
/// as `do_match_global_result` does (`erl_bif_binary.c:1672-1733`). The
/// integer phase visits each match once, and then the cons phase visits
/// each match once from the last match to the first. Each visit uses one
/// search step before it. When the steps become 0, the build keeps its
/// state and returns `None`.
///
/// # Examples
///
/// ```rust,ignore
/// find.matches = vec![(1, 2)];
/// let mut reds = 100;
/// let one = Term::list_from(vec![Term::tuple(vec![Term::Integer(1), Term::Integer(2)])]);
/// assert_eq!(match_all(&mut find, &mut reds), Some(one));
/// ```
fn match_all(find: &mut Find, reds: &mut u64) -> Option<Term> {
    let matches = &find.matches;
    let build = find.build.get_or_insert_with(|| Build {
        head: 0,
        end_pos: 0,
        parts: Vec::new(),
    });
    let mut steps = *reds;
    if build.end_pos == 0 {
        while (build.head as usize) < matches.len() {
            steps = steps.wrapping_sub(1);
            if steps == 0 {
                return None;
            }
            build.head += 1;
        }
        build.end_pos = 1;
        build.head = matches.len() as i64 - 1;
    }
    while build.head >= 0 {
        steps = steps.wrapping_sub(1);
        if steps == 0 {
            return None;
        }
        let (pos, len) = matches[build.head as usize];
        build.parts.push(Term::tuple(vec![
            Term::Integer(pos as i64),
            Term::Integer(len as i64),
        ]));
        build.head -= 1;
    }
    *reds = steps;
    let mut tuples = std::mem::take(&mut build.parts);
    tuples.reverse();
    Some(Term::list_from(tuples))
}

impl Find {
    /// Return the state as the term of a trap: `{Flags, End, Pos, Node,
    /// CandidateNode, CandidateStart, Head, EndPos, Matches, Parts}`. A
    /// missing candidate has the node -1, and a search before its result
    /// build has the head -2. `Matches` holds the start and the length of
    /// each match in order. `Head`, `EndPos`, and `Parts` hold the next
    /// match, the part end, and the parts so far of a `binary:split/2,3`
    /// build, or the next match, the phase, and the tuples so far of a
    /// `binary:matches/2` build.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let term = find.to_term();
    /// assert!(matches!(term, Term::Tuple(fields) if fields.len() == 10));
    /// ```
    fn to_term(&self) -> Term {
        let (node, start) = self
            .candidate
            .map_or((-1, 0), |(node, start)| (node as i64, start));
        let (head, end_pos, parts) = self.build.as_ref().map_or((-2, 0, Vec::new()), |build| {
            (build.head, build.end_pos, build.parts.clone())
        });
        let matches = self
            .matches
            .iter()
            .flat_map(|(pos, len)| [Term::Integer(*pos as i64), Term::Integer(*len as i64)]);
        let integers = [
            i64::from(self.flags),
            self.end as i64,
            self.pos,
            self.node as i64,
            node,
            start as i64,
            head,
            end_pos as i64,
        ];
        let mut fields: Vec<Term> = integers.into_iter().map(Term::Integer).collect();
        fields.push(Term::list_from(matches));
        fields.push(Term::list_from(parts));
        Term::tuple(fields)
    }

    /// Return the state of a trap term of [`Find::to_term`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let Term::Tuple(fields) = find.to_term() else { unreachable!() };
    /// assert_eq!(Find::from_term(&fields).pos, find.pos);
    /// ```
    ///
    /// # Panics
    ///
    /// Panic for a term that [`Find::to_term`] does not make. Only a
    /// `binary:split/2,3` trap gives the state to the trap export.
    fn from_term(state: &[Term]) -> Find {
        let integer = |term: &Term| term.small_integer().expect("a state integer");
        let list = |term: &'_ Term| -> Vec<Term> {
            term.proper_list()
                .expect("a binary_find_trap state list")
                .cloned()
                .collect()
        };
        let [
            flags,
            end,
            pos,
            node,
            candidate,
            start,
            head,
            end_pos,
            matches,
            parts,
        ] = <&[Term; 10]>::try_from(state).expect("a binary_find_trap state has 10 fields");
        let matches = list(matches)
            .chunks(2)
            .map(|pair| (integer(&pair[0]) as u64, integer(&pair[1]) as u64))
            .collect();
        let candidate =
            usize::try_from(integer(candidate)).map(|node| (node, integer(start) as u64));
        // The kind of the result follows the flags of the state: the parts
        // of a split build and the tuples of a matches build share the same
        // fields, and the build function of the kind reads them.
        let build = (integer(head) >= -1).then(|| Build {
            head: integer(head),
            end_pos: integer(end_pos) as u64,
            parts: list(parts),
        });
        Find {
            flags: integer(flags) as u8,
            end: integer(end) as u64,
            pos: integer(pos),
            node: integer(node) as usize,
            candidate: candidate.ok(),
            matches,
            build,
        }
    }
}

/// Run `binary:split/2,3` as ERTS `binary_split` does
/// (`erl_bif_binary.c:1599-1645`): read the subject, the options, and the
/// pattern, then search. A subject that is not a binary, options that are
/// not valid, and a pattern that is not valid raise `badarg` with no cost.
/// When the search steps of the turn end, the call uses all reductions and
/// traps to the export `erlang:binary_find_trap/3` ([`find_trap`]).
///
/// # Examples
///
/// ```rust,ignore
/// let parts = split(&mut process, &Term::binary(b"a,b".to_vec()), &Term::binary(b",".to_vec()), None)?;
/// assert!(matches!(parts, BifResult::Value(parts) if parts.proper_list().is_some_and(|parts| parts.len() == 2)));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn split(
    process: &mut ProcessState,
    subject: &Term,
    pattern: &Term,
    options_term: Option<&Term>,
) -> Result<BifResult, BeamError> {
    find_first_turn(process, subject, pattern, options_term, 0)
}

/// Run `binary:match/2` as ERTS `binary_match` does with no options
/// (`erl_bif_binary.c:1540-1582`): a subject that is not a binary and a
/// pattern that is not valid raise `badarg` with no cost. Then the first
/// match of the search gives `{Position, Length}`, and no match gives
/// `nomatch` (`erl_bif_binary.c:1647-1673`). A pattern of one binary runs
/// the Boyer-Moore search, and two or more binaries run the Aho-Corasick
/// search, which gives the first match that starts first, and of these the
/// longest. When the search steps of the turn end, the call uses all
/// reductions and traps to the export `erlang:binary_find_trap/3`
/// ([`find_trap`]).
///
/// # Examples
///
/// ```rust,ignore
/// // {1, 2}
/// let result = match_2(&mut process, &Term::binary(b"abcab".to_vec()), &Term::binary(b"bc".to_vec()))?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(match_2(&mut process, &Term::binary(b"ab".to_vec()), &Term::binary(vec![]))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn match_2(
    process: &mut ProcessState,
    subject: &Term,
    pattern: &Term,
) -> Result<BifResult, BeamError> {
    find_first_turn(process, subject, pattern, None, MATCH)
}

/// Run `binary:matches/2` as ERTS `binary_matches_2` does
/// (`erl_bif_binary.c:1589-1592`): it is `binary_match` with the flag
/// `BF_FLAG_GLOBAL` (`erl_bif_binary.c:1554-1571`), so the search keeps
/// each match that does not overlap a match before it, and the result is
/// their list of `{Position, Length}` tuples, or `[]` with no match
/// (`do_match_global_result`, `erl_bif_binary.c:1672-1733`). The same
/// `badarg` rules as `binary:match/2` apply: a subject that is not a binary
/// and a pattern that is not valid raise `badarg`. When the search steps or
/// the result build use all reductions of the turn, the call traps to
/// `erlang:binary_find_trap/3` ([`find_trap`]).
///
/// # Examples
///
/// ```rust,ignore
/// // [{0, 2}, {2, 2}]
/// let result = matches_2(&mut process, &Term::binary(b"aaaa".to_vec()), &Term::binary(b"aa".to_vec()))?;
/// assert!(matches!(result, BifResult::Value(parts) if parts.proper_list().is_some_and(|list| list.len() == 2)));
/// assert!(matches!(matches_2(&mut process, &Term::binary(b"ab".to_vec()), &Term::binary(vec![]))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn matches_2(
    process: &mut ProcessState,
    subject: &Term,
    pattern: &Term,
) -> Result<BifResult, BeamError> {
    find_first_turn(process, subject, pattern, None, GLOBAL | MATCHES)
}

/// Check the arguments of `binary:match/2` or `binary:split/2,3`, and run
/// the first turn of its search with the flags of the options and `kind`
/// ([`MATCH`] or 0).
///
/// # Examples
///
/// ```rust,ignore
/// let result = find_first_turn(&mut process, &subject, &pattern, None, MATCH)?;
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
fn find_first_turn(
    process: &mut ProcessState,
    subject: &Term,
    pattern: &Term,
    options_term: Option<&Term>,
    kind: u8,
) -> Result<BifResult, BeamError> {
    let Term::Binary(bytes) = subject else {
        return Ok(BifResult::badarg());
    };
    let (Some((flags, start, end)), Some(compiled)) =
        (options(options_term, bytes.len() as u64), compile(pattern))
    else {
        return Ok(BifResult::badarg());
    };
    let mut find = Find {
        flags: flags | kind,
        end,
        pos: start as i64,
        node: ROOT,
        candidate: None,
        matches: Vec::new(),
        build: None,
    };
    Ok(match find_turn(process, &compiled, &mut find, bytes)? {
        Some(parts) => Value(parts),
        None => {
            process.bump_all_reductions();
            find_trap_result(subject, &find, pattern)
        }
    })
}

/// Continue `binary:match/2` or `binary:split/2,3` in the export `erlang:binary_find_trap/3`, as
/// `binary_find_trap` does (`erl_bif_binary.c:1935-1952`), with the budget
/// of the next turn. ERTS keeps the compiled pattern, and PBeam compiles the
/// pattern again with no cost. When the search steps end again, the call
/// traps again with no reduction bump.
///
/// # Examples
///
/// ```rust,ignore
/// let result = find_trap(&mut process, &subject, b"a,b", &state, &pattern)?;
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn find_trap(
    process: &mut ProcessState,
    subject: &Term,
    bytes: &Binary,
    state: &[Term],
    pattern: &Term,
) -> Result<BifResult, BeamError> {
    let compiled = compile(pattern).expect("the first call checked the pattern");
    let mut find = Find::from_term(state);
    Ok(match find_turn(process, &compiled, &mut find, bytes)? {
        Some(parts) => Value(parts),
        None => find_trap_result(subject, &find, pattern),
    })
}

/// Return the trap of `binary:match/2` or `binary:split/2,3` to `erlang:binary_find_trap/3`
/// with the subject, the state, and the pattern.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(find_trap_result(&subject, &find, &pattern), BifResult::TrapBif { function: "binary_find_trap", .. }));
/// ```
fn find_trap_result(subject: &Term, find: &Find, pattern: &Term) -> BifResult {
    BifResult::TrapBif {
        module: "erlang",
        function: "binary_find_trap",
        arguments: vec![subject.clone(), find.to_term(), pattern.clone()],
    }
}

/// Run one turn of a search and of its result, as `do_binary_find` does
/// (`erl_bif_binary.c:110-121`, `1433-1538`). The turn has 10 search steps
/// for each reduction of the budget, and at least 1. A search turn with no
/// restart bumps one reduction for each 10 steps that it used, and returns
/// the match, the match list, or the parts. A restart returns `None` with
/// no bump.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(find_turn(&mut process, &compiled, &mut find, b"a,b")?.is_some());
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
fn find_turn(
    process: &mut ProcessState,
    pattern: &Pattern,
    find: &mut Find,
    subject: &Binary,
) -> Result<Option<Term>, BeamError> {
    let initial = (process.bif_reductions_left()? as u64 * LOOP_FACTOR).max(1);
    let mut reds = initial;
    let bytes: &[u8] = subject;
    let parts = if find.build.is_some() {
        // A result build in progress continues with the matches that the
        // search kept: the integers and the tuples of `binary:matches/2`, or
        // the parts of `binary:split/2,3`.
        if find.flags & MATCHES != 0 {
            match_all(find, &mut reds)
        } else {
            split_all(find, subject, &mut reds)
        }
    } else {
        // `binary:matches/2` and `binary:split/3` with the option `global`
        // run a global search, and each other search runs a first-match
        // search.
        if find.flags & GLOBAL != 0 {
            let search = match pattern {
                Pattern::Bm(bm) => bm.all(find, bytes, &mut reds),
                Pattern::Ac(trie) => ac_all(trie, find, bytes, &mut reds),
            };
            match search {
                SearchAll::Restart => None,
                SearchAll::NotFound => Some(not_found(find.flags, subject)),
                SearchAll::FoundAll if find.flags & MATCHES != 0 => match_all(find, &mut reds),
                SearchAll::FoundAll => split_all(find, subject, &mut reds),
            }
        } else {
            let search = match pattern {
                Pattern::Bm(bm) => bm.first(find, bytes, &mut reds),
                Pattern::Ac(trie) => ac_first(trie, find, bytes, &mut reds),
            };
            match search {
                Search::Restart => None,
                Search::NotFound if find.flags & MATCH != 0 => Some(Term::static_atom("nomatch")),
                Search::NotFound => Some(not_found(find.flags, subject)),
                Search::Found(pos, len) if find.flags & MATCH != 0 => Some(Term::tuple(vec![
                    Term::Integer(pos as i64),
                    Term::Integer(len as i64),
                ])),
                Search::Found(pos, len) => Some(split_once(find.flags, subject, pos, len)),
            }
        }
    };
    if parts.is_some() {
        let used = initial.wrapping_sub(reds) / LOOP_FACTOR;
        process.bump_reductions(i32::try_from(used).unwrap_or(i32::MAX));
    }
    Ok(parts)
}

/// Run `binary:at/2` as ERTS `binary_at_2` does
/// (`erl_bif_binary.c:2335-2362`): return the byte at the zero-based
/// position `index` of the subject as an integer. A subject that is not a
/// bitstring, an index that is not a non-negative integer, and an index at
/// or after the end of the subject raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // 19
/// let byte = at(&Term::binary(vec![5, 19, 72]), &Term::Integer(1));
/// assert!(matches!(byte, BifResult::Value(Term::Integer(19))));
/// assert!(matches!(at(&Term::binary(vec![5]), &Term::Integer(1)), BifResult::Raise { .. }));
/// ```
pub(super) fn at(subject: &Term, index: &Term) -> BifResult {
    let Term::Binary(bytes) = subject else {
        return BifResult::badarg();
    };
    let Some(index) = to_uint(index).and_then(|index| usize::try_from(index).ok()) else {
        return BifResult::badarg();
    };
    match bytes.get(index) {
        Some(byte) => Value(Term::Integer(i64::from(*byte))),
        None => BifResult::badarg(),
    }
}

/// Return the value of a term as an `Uint`, as ERTS `term_to_Uint` gives it
/// (`erl_utils.c`): an integer that is not negative and that fits in a
/// 64-bit unsigned word. Return `None` for each other term, which includes a
/// negative integer, an integer above 2^64 - 1, and each term that is not an
/// integer.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(to_uint(&Term::Integer(2)), Some(2));
/// assert_eq!(to_uint(&Term::Integer(-1)), None);
/// ```
fn to_uint(term: &Term) -> Option<u64> {
    u64::try_from(&*term.bigint()?).ok()
}

/// Run `binary:part/2` as ERTS `binary_binary_part_2` does
/// (`erl_bif_binary.c:2026-2041`): take the `{Position, Length}` tuple of
/// the second argument and give the part of the subject that
/// [`part_3`] gives. A second argument that is not a tuple of two elements
/// raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// // <<2, 3>>
/// let part = part_2(&Term::binary(vec![1, 2, 3]), &Term::tuple(vec![Term::Integer(1), Term::Integer(2)]));
/// assert!(matches!(part, BifResult::Value(Term::Binary(bytes)) if bytes == [2, 3]));
/// assert!(matches!(part_2(&Term::binary(vec![1]), &Term::Integer(0)), BifResult::Raise { .. }));
/// ```
pub(super) fn part_2(subject: &Term, poslen: &Term) -> BifResult {
    let Term::Tuple(parts) = poslen else {
        return BifResult::badarg();
    };
    let [position, length] = parts.as_ref() else {
        return BifResult::badarg();
    };
    part_3(subject, position, length)
}

/// Run `binary:part/3` as ERTS `binary_binary_part_3` does
/// (`erl_bif_binary.c:2021-2024`) and `erlang:binary_part/3` does: return
/// the `length` bytes of the subject from byte `position`, as
/// [`erlang::binary_part`] does with `erts_binary_part`
/// (`erl_bif_binary.c:1954-2012`). A negative length takes the bytes before
/// the position. The part shares the bytes of the subject, as an ERTS
/// sub-binary refers to its source.
///
/// # Examples
///
/// ```rust,ignore
/// // <<3, 4>>
/// let part = part_3(&Term::binary(vec![1, 2, 3, 4]), &Term::Integer(2), &Term::Integer(2));
/// assert!(matches!(part, BifResult::Value(Term::Binary(bytes)) if bytes == [3, 4]));
/// assert!(matches!(part_3(&Term::binary(vec![1]), &Term::Integer(0), &Term::Integer(2)), BifResult::Raise { .. }));
/// ```
pub(super) fn part_3(subject: &Term, position: &Term, length: &Term) -> BifResult {
    erlang::binary_part(subject, position, length)
}

/// Run `binary:copy/1,2` as ERTS `binary_copy_1` and `binary_copy_2` do
/// (`erl_bif_binary.c:2400-2447`, `2498-2505`): return a binary of the
/// bytes of the subject `count` times, as a new binary also for the count
/// of 1. The count of 0 and an empty subject give the empty binary. A
/// subject that is not a bitstring and a count that is not a non-negative
/// integer that fits in an `Uint` raise `badarg`. A count of at least
/// `2^64 - 1` divided by the bit size of the subject raises
/// `system_limit`, as `erl_bif_binary.c:2423` checks before the copy. The
/// copy uses one reduction for each 512 bits of the result
/// (`erl_bif_binary.c:2377`, `2445`). The budget of the turn stops at
/// 0, so a copy whose charge is larger than the remaining budget
/// charges that budget only: ERTS resumes its copy after a trap and
/// charges the next turn too (row "Binary copy reductions" of
/// `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// // <<1, 2, 1, 2>>
/// let copy = copy(&mut process, &Term::binary(vec![1, 2]), &Term::Integer(2));
/// assert!(matches!(copy, BifResult::Value(Term::Binary(bytes)) if bytes == [1, 2, 1, 2]));
/// assert!(matches!(copy(&mut process, &Term::binary(vec![1]), &Term::Integer(-1)), BifResult::Raise { .. }));
/// ```
pub(super) fn copy(process: &mut ProcessState, subject: &Term, count: &Term) -> BifResult {
    let Term::Binary(bytes) = subject else {
        return BifResult::badarg();
    };
    let Some(count) = to_uint(count) else {
        return BifResult::badarg();
    };
    // A binary holds at most `usize::MAX` bytes, so its bit size fits in the
    // low 61 bits of a `u64` and the sums below cannot wrap a `u64` for a
    // count under the ERTS limit.
    let bits = bytes.len() as u64 * 8;
    if count == 0 || bits == 0 {
        return Value(Term::binary(Vec::new()));
    }
    if count >= u64::MAX / bits {
        return BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::static_atom("system_limit"),
        };
    }
    let bit_size = bits * count;
    let mut copied = Vec::with_capacity((bit_size / 8) as usize);
    for _ in 0..count {
        copied.extend_from_slice(bytes);
    }
    process.bump_reductions(i32::try_from(bit_size / COPY_BITS_PER_REDUCTION).unwrap_or(i32::MAX));
    Value(Term::binary(copied))
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/binary.rs"]
mod tests;
