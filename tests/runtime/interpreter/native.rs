use std::collections::BTreeSet;

use super::{LIBRARIES, binding};
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
            if binding(library.module, name, arity).is_some() {
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
    assert!(binding("prim_tty", "write_nif", 2).is_some());
    assert!(binding("prim_tty", "write_nif", 1).is_none());
    assert!(binding("prim_tty", "write", 2).is_none());
    assert!(binding("prim_file", "read_info_nif", 2).is_some());
    assert!(binding("prim_file", "write_nif", 2).is_some());
    assert!(binding("prim_file", "sync_nif", 2).is_none());
    assert!(binding("zlib", "inflate_nif", 4).is_some());
    assert!(binding("zlib", "deflateParams_nif", 3).is_none());
}

/// A registered native function returns its result and a duplicate
/// registration fails.
#[test]
fn native_registry_registers_and_rejects_duplicates() {
    use crate::BeamError;
    use crate::Term;
    use crate::runtime::interpreter::bif::BifResult;
    use crate::runtime::register_native;
    fn add(
        _scheduler: &mut crate::runtime::Scheduler,
        arguments: &[crate::Term],
    ) -> Result<BifResult, BeamError> {
        match (arguments.first(), arguments.get(1)) {
            (Some(Term::Integer(left)), Some(Term::Integer(right))) => {
                Ok(BifResult::Value(Term::Integer(left + right)))
            }
            _ => Ok(BifResult::badarg()),
        }
    }
    let module = "native_probe_dup";
    let _ = crate::runtime::unregister_native(module, "add", 2);
    register_native(module, "add", 2, add).unwrap();
    assert!(register_native(module, "add", 2, add).is_err());
    assert!(crate::runtime::lookup_native(module, "add", 2).is_some());
    assert!(crate::runtime::lookup_native(module, "add", 1).is_none());
    crate::runtime::unregister_native(module, "add", 2);
}

/// Unregistering removes the binding.
#[test]
fn native_registry_unregisters() {
    use crate::runtime::{lookup_native, register_native, unregister_native};
    fn f(
        _s: &mut crate::runtime::Scheduler,
        _a: &[crate::Term],
    ) -> Result<crate::runtime::BifResult, crate::BeamError> {
        Ok(crate::runtime::BifResult::Value(crate::Term::Integer(0)))
    }
    let _ = unregister_native("m_unreg", "f", 1);
    register_native("m_unreg", "f", 1, f).unwrap();
    assert!(lookup_native("m_unreg", "f", 1).is_some());
    assert!(unregister_native("m_unreg", "f", 1));
    assert!(lookup_native("m_unreg", "f", 1).is_none());
    assert!(!unregister_native("m_unreg", "f", 1));
}
