use std::collections::BTreeSet;

use super::{LIBRARIES, Linked};
use crate::runtime::Job;
use crate::runtime::interpreter::bif::BIFS;

/// The effect inventory of transition 1 of
/// `docs/capability-security-migration.md`. `scripts/ci_task.sh` lists this
/// file, so that CI runs the full check for a change to it.
const INVENTORY: &str = include_str!("../../../docs/effect-inventory.md");

/// The source of the BIF table, which marks each BIF that PBeam does not
/// implement with `=> unsupported`.
const BIF_TABLE: &str = include_str!("../../../src/runtime/interpreter/bif/table.rs");

/// Return the entry keys of the section `## {section}` of the effect
/// inventory: the first code span of each table row, and each
/// `module:name/arity` of a list line ``- `module`: `name/arity`, ...``.
fn inventory_entries(section: &str) -> BTreeSet<String> {
    let heading = format!("## {section}");
    let mut entries = BTreeSet::new();
    let lines = INVENTORY
        .lines()
        .skip_while(|line| *line != heading)
        .skip(1)
        .take_while(|line| !line.starts_with("## "));
    for line in lines {
        if let Some(row) = line.strip_prefix("| `") {
            entries.insert(row[..row.find('`').unwrap()].to_owned());
        } else if let Some(item) = line.strip_prefix("- `") {
            let (module, functions) = item.split_once("`: ").unwrap();
            for function in functions.split(", ") {
                entries.insert(format!("{module}:{}", function.trim_matches('`')));
            }
        }
    }
    entries
}

/// Assert that the section `## {section}` of the effect inventory lists
/// exactly the entries of `expected`.
fn assert_section(section: &str, expected: &BTreeSet<String>) {
    let listed = inventory_entries(section);
    let missing: Vec<_> = expected.difference(&listed).collect();
    let extra: Vec<_> = listed.difference(expected).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "docs/effect-inventory.md section {section}: missing {missing:?}, not in the code {extra:?}"
    );
}

/// The effect inventory has an entry for each BIF of `BIFS` and each
/// function of a static NIF library, in the section of its kind: a BIF
/// with a body, an `unsupported` BIF, a native binding, or a library
/// function without a binding. No section lists an entry that the code
/// does not have.
#[test]
fn effect_inventory_lists_each_bif_and_native_function() {
    let bifs: BTreeSet<String> = BIFS
        .iter()
        .map(|(module, name, arity)| format!("{module}:{name}/{arity}"))
        .collect();
    let unsupported: BTreeSet<String> = BIF_TABLE
        .lines()
        .filter_map(|line| line.trim().strip_suffix(" => unsupported,"))
        .map(|mfa| mfa.replace('"', ""))
        .collect();
    assert!(unsupported.is_subset(&bifs), "{unsupported:?}");
    let implemented = bifs.difference(&unsupported).cloned().collect();
    assert_section("BIFs", &implemented);
    assert_section("Unsupported BIFs", &unsupported);

    let mut bound = BTreeSet::new();
    let mut unbound = BTreeSet::new();
    for library in LIBRARIES {
        for &(name, arity) in library.functions {
            let key = format!("{}:{name}/{arity}", library.module);
            if has_binding(library.module, name, arity) {
                bound.insert(key);
            } else {
                unbound.insert(key);
            }
        }
    }
    assert_section("Native bindings", &bound);
    assert_section("Native functions without a binding", &unbound);
}

/// The authority labels of an entry that has no effect outside its own
/// process.
const NO_EFFECT: [&str; 2] = ["`none`", "`process.self`"];

/// Each row of the sections "BIFs" and "Native bindings" of the effect
/// inventory names the capability that its entry checks, in its column
/// "Capability". An entry whose authority is only `none` or `process.self`
/// has no effect, and its cell is `none`. Each other entry is an effect
/// entry: its cell lists each `kind:operation` that it checks, is
/// `` `ambient`: Reason `` with a reason when it has no check by decision,
/// or is `` `later`: Task `` with the name of the task, the epic, or, when
/// no task exists, the capability kind that adds its check. The integration
/// test `effect_capability` calls each listed entry
/// without its operation and gets `error:capability_denied`, and calls each
/// `ambient` and `later` entry with no capability and gets the pinned ERTS
/// result.
#[test]
fn effect_inventory_names_the_capability_of_each_effect_entry() {
    let mut rows = 0;
    for section in ["BIFs", "Native bindings"] {
        let heading = format!("## {section}");
        let lines = INVENTORY
            .lines()
            .skip_while(|line| *line != heading)
            .skip(1)
            .take_while(|line| !line.starts_with("## "));
        for row in lines.filter(|line| line.starts_with("| `")) {
            rows += 1;
            let cells: Vec<_> = row.split(" | ").collect();
            let effect = cells[1]
                .split(", ")
                .any(|label| !NO_EFFECT.contains(&label));
            // A reason or a task name can hold ", ", so the values `ambient`
            // and `later` come before the split of a list.
            if let Some(text) = cells[2]
                .strip_prefix("`ambient`: ")
                .or_else(|| cells[2].strip_prefix("`later`: "))
            {
                assert!(
                    effect && !text.trim().is_empty(),
                    "docs/effect-inventory.md: {row}"
                );
                continue;
            }
            let capabilities: Vec<_> = cells[2].split(", ").collect();
            let named = capabilities.iter().all(|capability| {
                let name = capability.trim_matches('`');
                capability.len() == name.len() + 2
                    && name
                        .split_once(':')
                        .is_some_and(|(kind, operation)| !kind.is_empty() && !operation.is_empty())
            });
            let valid = match capabilities.as_slice() {
                ["`none`"] => !effect,
                _ => effect && named,
            };
            assert!(valid, "docs/effect-inventory.md: {row}");
        }
    }
    // `effect_inventory_lists_each_bif_and_native_function` checks that
    // the sections hold a row for each entry.
    assert!(rows > 0);
}

/// `library` and `binding` find an entry with binary searches, so the
/// libraries and the bindings of each library are strictly increasing:
/// sorted, with no duplicate entry. Each binding is a function of its
/// library. The lookup matches the exact MFA.
#[test]
fn registry_is_sorted_and_matches_the_exact_mfa() {
    assert!(
        LIBRARIES
            .windows(2)
            .all(|pair| pair[0].module < pair[1].module)
    );
    for library in LIBRARIES {
        let bindings = library.bindings;
        assert!(
            bindings
                .windows(2)
                .all(|pair| (pair[0].name, pair[0].arity) < (pair[1].name, pair[1].arity))
        );
        assert!(
            bindings
                .iter()
                .all(|binding| library.functions.contains(&(binding.name, binding.arity)))
        );
    }
    assert!(has_binding("prim_tty", "write_nif", 2));
    assert!(!has_binding("prim_tty", "write_nif", 1));
    assert!(!has_binding("prim_tty", "write", 2));
    assert!(has_binding("prim_file", "read_info_nif", 2));
    assert!(has_binding("prim_file", "write_nif", 2));
    assert!(!has_binding("prim_file", "sync_nif", 2));
    assert!(has_binding("zlib", "inflate_nif", 4));
    assert!(!has_binding("zlib", "deflateParams_nif", 3));
    assert!(has_binding("zstd", "compress_stream_nif", 3));
    assert!(!has_binding("zstd", "create_cdict_nif", 2));
}

/// Return whether the library of `module` has a binding of exactly
/// `name/arity`.
fn has_binding(module: &str, name: &str, arity: u32) -> bool {
    Linked::of(module).is_some_and(|linked| linked.binds(name, arity))
}

/// The NIF functions of the static libraries whose pinned-OTP `ErlNifFunc`
/// has a dirty job flag (erl_nif.h:239-243), with the job of the flag. Each
/// other function has no flag and runs on a normal scheduler.
const DIRTY_FUNCTIONS: &[(&str, &str, u32, Job)] = &[
    // prim_file_nif.c:179-218: each function except `get_handle_nif/1`
    // (prim_file_nif.c:215).
    ("prim_file", "open_nif", 2, Job::IoBound),
    ("prim_file", "close_nif", 1, Job::IoBound),
    ("prim_file", "read_nif", 2, Job::IoBound),
    ("prim_file", "write_nif", 2, Job::IoBound),
    ("prim_file", "pread_nif", 3, Job::IoBound),
    ("prim_file", "pwrite_nif", 3, Job::IoBound),
    ("prim_file", "seek_nif", 3, Job::IoBound),
    ("prim_file", "sync_nif", 2, Job::IoBound),
    ("prim_file", "truncate_nif", 1, Job::IoBound),
    ("prim_file", "allocate_nif", 3, Job::IoBound),
    ("prim_file", "advise_nif", 4, Job::IoBound),
    ("prim_file", "read_handle_info_nif", 1, Job::IoBound),
    ("prim_file", "make_hard_link_nif", 2, Job::IoBound),
    ("prim_file", "make_soft_link_nif", 2, Job::IoBound),
    ("prim_file", "rename_nif", 2, Job::IoBound),
    ("prim_file", "read_info_nif", 2, Job::IoBound),
    ("prim_file", "set_permissions_nif", 2, Job::IoBound),
    ("prim_file", "set_owner_nif", 3, Job::IoBound),
    ("prim_file", "set_time_nif", 4, Job::IoBound),
    ("prim_file", "read_link_nif", 1, Job::IoBound),
    ("prim_file", "list_dir_nif", 1, Job::IoBound),
    ("prim_file", "make_dir_nif", 1, Job::IoBound),
    ("prim_file", "del_file_nif", 1, Job::IoBound),
    ("prim_file", "del_dir_nif", 1, Job::IoBound),
    ("prim_file", "get_device_cwd_nif", 1, Job::IoBound),
    ("prim_file", "set_cwd_nif", 1, Job::IoBound),
    ("prim_file", "get_cwd_nif", 0, Job::IoBound),
    ("prim_file", "ipread_s32bu_p32bu_nif", 3, Job::IoBound),
    ("prim_file", "read_file_nif", 1, Job::IoBound),
    ("prim_file", "delayed_close_nif", 1, Job::IoBound),
    ("prim_file", "altname_nif", 1, Job::IoBound),
    ("prim_file", "file_desc_to_ref_nif", 1, Job::IoBound),
    // crypto.c:105.
    ("crypto", "pkey_sign_heavy_nif", 5, Job::CpuBound),
    // prim_tty_nif.c:161, 163.
    ("prim_tty", "write_nif", 2, Job::IoBound),
    ("prim_tty", "read_nif", 3, Job::IoBound),
    // zstd_nif.c:1086-1087, 1093-1094, 1108-1109.
    ("zstd", "load_compress_dictionary_nif", 2, Job::CpuBound),
    ("zstd", "load_decompress_dictionary_nif", 2, Job::CpuBound),
    ("zstd", "create_cdict_nif", 2, Job::CpuBound),
    ("zstd", "create_ddict_nif", 1, Job::CpuBound),
];

/// Each static binding has the job of the flag of its pinned-OTP
/// `ErlNifFunc`: `crypto` (crypto.c:69-103) except `pkey_sign_heavy_nif/5`, `prim_buffer`
/// (prim_buffer_nif.c:87-95), `zlib` (zlib_nif.c:148-176), and `zstd`
/// (zstd_nif.c:1082-1116) have no flag on a bound function.
#[test]
fn each_binding_has_the_job_of_its_pinned_flag() {
    for &(module, name, arity, _) in DIRTY_FUNCTIONS {
        let library = LIBRARIES.iter().find(|library| library.module == module);
        assert!(
            library.is_some_and(|library| library.functions.contains(&(name, arity))),
            "{module}:{name}/{arity}"
        );
    }
    let prim_file = LIBRARIES
        .iter()
        .find(|library| library.module == "prim_file");
    assert!(prim_file.is_some_and(|library| library.functions.contains(&("get_handle_nif", 1))));
    for library in LIBRARIES {
        for binding in library.bindings {
            let expected = DIRTY_FUNCTIONS
                .iter()
                .find(|&&(module, name, arity, _)| {
                    (module, name, arity) == (library.module, binding.name, binding.arity)
                })
                .map_or(Job::Normal, |&(.., job)| job);
            assert_eq!(
                binding.job, expected,
                "{}:{}/{}",
                library.module, binding.name, binding.arity
            );
        }
    }
}

/// Return an empty instance of the module `name`, for a direct call of
/// `call`.
pub(crate) fn instance(name: &str) -> crate::beam::BeamModule {
    use crate::beam::code::Instruction;
    crate::beam::BeamModule::new(
        name.into(),
        std::collections::HashMap::new(),
        Vec::new(),
        vec![Instruction::End],
    )
    .unwrap()
}
