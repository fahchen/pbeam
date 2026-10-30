//! The heap flags of a process and the node-wide defaults of new processes:
//! `min_heap_size`, `min_bin_vheap_size`, `fullsweep_after`, and
//! `max_heap_size`. PBeam has no garbage collector and does not measure a
//! process heap, so the values only change what `process_flag/2`,
//! `system_flag/2`, and `process_info/2` report (row "Heap limit" in
//! `docs/erts-differences.md`).

use std::sync::LazyLock;

use crate::error::BeamError;
use crate::term::{AtomTable, Map, Term};

/// The heap flags of one process, or the node defaults that a new process
/// takes: ERTS `min_heap_size`, `min_vheap_size`, `max_gen_gcs`, and the
/// max heap size with its flags (`erl_process.h:1093-1145`), or `H_MIN_SIZE`,
/// `BIN_VH_MIN_SIZE`, `erts_max_gen_gcs`, `H_MAX_SIZE`, and `H_MAX_FLAGS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) struct HeapFlags {
    /// The minimum heap size in words.
    pub(in crate::runtime) min_heap_size: i64,
    /// The minimum binary virtual heap size in words.
    pub(in crate::runtime) min_bin_vheap_size: i64,
    /// The number of minor collections before a full sweep, a `Uint16`.
    pub(in crate::runtime) fullsweep_after: u16,
    /// The maximum heap size in words, 0 for no limit.
    pub(in crate::runtime) max_heap_size: i64,
    /// The `kill`, `error_logger`, and `include_shared_binaries` flags of
    /// the max heap size.
    pub(in crate::runtime) max_heap_settings: MaxHeapFlags,
}

/// The flags of the max heap size: `MAX_HEAP_SIZE_KILL`,
/// `MAX_HEAP_SIZE_LOG`, and `MAX_HEAP_SIZE_INCLUDE_OH_BINS`
/// (`erl_process.h:1039-1041`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) struct MaxHeapFlags {
    pub(in crate::runtime) kill: bool,
    pub(in crate::runtime) error_logger: bool,
    pub(in crate::runtime) include_shared_binaries: bool,
}

impl Default for HeapFlags {
    /// The start values of ERTS (`erl_init.c:808-811`): `H_DEFAULT_SIZE`
    /// 233, `VH_DEFAULT_SIZE` 46422, `fullsweep_after` 65535, no max heap
    /// size, and the flags `kill` and `error_logger`.
    fn default() -> Self {
        HeapFlags {
            min_heap_size: 233,
            min_bin_vheap_size: 46422,
            fullsweep_after: u16::MAX,
            max_heap_size: 0,
            max_heap_settings: MaxHeapFlags {
                kill: true,
                error_logger: true,
                include_shared_binaries: false,
            },
        }
    }
}

/// The heap sizes of ERTS `erts_init_gc` (`erl_gc.c:265-291`) for 8-byte
/// words: 12 and 38, then each size is the sum of the two sizes before it
/// plus 1 up to index 22, then each size grows by a fifth while it is at
/// most `2^53 / 8`.
static HEAP_SIZES: LazyLock<Vec<i64>> = LazyLock::new(|| {
    let mut sizes = vec![12_i64, 38];
    for i in 2..23 {
        sizes.push(sizes[i - 1] + sizes[i - 2] + 1);
    }
    let max = (1_i64 << 53) / 8;
    while sizes.len() < 154 {
        let last = sizes[sizes.len() - 1];
        let next = last + last / 5;
        if next > max {
            break;
        }
        sizes.push(next);
    }
    sizes
});

/// Return the smallest heap size of ERTS that is `size` or more, as
/// `erts_next_heap_size(size, 0)` (`erl_gc.c:310-338`) does, or `None` where
/// ERTS finds no size: for `size` equal to or above the largest heap size,
/// ERTS stops the emulator with "no next heap size found".
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(next_heap_size(0), Some(12));
/// assert_eq!(next_heap_size(300), Some(376));
/// assert_eq!(next_heap_size(1 << 55), None);
/// ```
pub(in crate::runtime) fn next_heap_size(size: i64) -> Option<i64> {
    let last = *HEAP_SIZES.last().expect("the heap size table is not empty");
    (size < last).then(|| {
        let index = HEAP_SIZES.partition_point(|&heap| heap < size);
        HEAP_SIZES[index]
    })
}

/// Read a `max_heap_size` value as ERTS `erts_max_heap_size`
/// (`erl_gc.c:3824-3873`) does: a small integer `Size`, or a map with a
/// small integer `size` and the optional boolean keys `kill`,
/// `error_logger`, and `include_shared_binaries`. A flag that the map does
/// not give takes its value from `defaults`, the node flags `H_MAX_FLAGS`.
/// A negative size, a key value that is not valid, and each other term give
/// `None`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(max_heap_size(&Term::Integer(0), defaults), Some((0, defaults)));
/// assert_eq!(max_heap_size(&Term::Integer(-1), defaults), None);
/// ```
pub(in crate::runtime) fn max_heap_size(
    value: &Term,
    defaults: MaxHeapFlags,
) -> Option<(i64, MaxHeapFlags)> {
    let mut flags = defaults;
    let size = match value {
        Term::Map(map) => {
            let size = map.get(&Term::static_atom("size"))?.small_integer()?;
            for (key, flag) in [
                ("kill", &mut flags.kill),
                ("error_logger", &mut flags.error_logger),
                (
                    "include_shared_binaries",
                    &mut flags.include_shared_binaries,
                ),
            ] {
                match map.get(&Term::static_atom(key)) {
                    None => {}
                    Some(Term::Atom(value)) if value.is_static("true") => *flag = true,
                    Some(Term::Atom(value)) if value.is_static("false") => *flag = false,
                    Some(_) => return None,
                }
            }
            size
        }
        _ => value.small_integer()?,
    };
    (size >= 0).then_some((size, flags))
}

/// Return the map of a max heap size, as `erts_max_heap_size_map`
/// (`erl_gc.c:3806-3822`) builds it: the keys `error_logger`,
/// `include_shared_binaries`, `kill`, and `size`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = max_heap_size_map(&atoms, 0, HeapFlags::default().max_heap_settings);
/// ```
pub(in crate::runtime) fn max_heap_size_map(
    atoms: &AtomTable,
    size: i64,
    flags: MaxHeapFlags,
) -> Term {
    let boolean = |flag: bool| Term::static_atom(if flag { "true" } else { "false" });
    let map = Map::from_pairs(
        vec![
            (
                Term::static_atom("error_logger"),
                boolean(flags.error_logger),
            ),
            (
                Term::static_atom("include_shared_binaries"),
                boolean(flags.include_shared_binaries),
            ),
            (Term::static_atom("kill"), boolean(flags.kill)),
            (Term::static_atom("size"), Term::Integer(size)),
        ],
        atoms,
    )
    // ponytail: each key is an atom, so `from_pairs` cannot fail.
    .expect("the max_heap_size map holds atom keys");
    Term::Map(map)
}

/// Return the list of the `garbage_collection` item of `process_info/1`
/// and `process_info/2`, as `process_info_aux` builds it
/// (`erl_bif_info.c:2072-2094`): `max_heap_size`, `min_bin_vheap_size`,
/// `min_heap_size`, `fullsweep_after`, and `minor_gcs`, which is 0 because
/// PBeam does no garbage collection.
///
/// # Examples
///
/// ```rust,ignore
/// let list = garbage_collection_list(&atoms, HeapFlags::default())?;
/// ```
///
/// # Errors
///
/// A full atom table cannot hold the atom `minor_gcs`.
pub(in crate::runtime) fn garbage_collection_list(
    atoms: &AtomTable,
    flags: HeapFlags,
) -> Result<Term, BeamError> {
    let max_heap_size = max_heap_size_map(atoms, flags.max_heap_size, flags.max_heap_settings);
    let minor_gcs = atoms.put("minor_gcs")?;
    Ok(Term::list_from(vec![
        Term::tuple(vec![Term::static_atom("max_heap_size"), max_heap_size]),
        Term::tuple(vec![
            Term::static_atom("min_bin_vheap_size"),
            Term::Integer(flags.min_bin_vheap_size),
        ]),
        Term::tuple(vec![
            Term::static_atom("min_heap_size"),
            Term::Integer(flags.min_heap_size),
        ]),
        Term::tuple(vec![
            Term::static_atom("fullsweep_after"),
            Term::Integer(i64::from(flags.fullsweep_after)),
        ]),
        Term::tuple(vec![Term::Atom(minor_gcs), Term::Integer(0)]),
    ]))
}
