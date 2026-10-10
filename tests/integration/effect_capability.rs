//! Capability checks of the effect entries. Each BIF and native binding
//! that the column "Capability" of `docs/effect-inventory.md` gives a
//! capability raises `error:capability_denied` without that operation. A
//! test policy that holds back one operation is the oracle of each denial,
//! which ERTS does not have.

use std::collections::BTreeSet;

use pbeam_runtime::{BeamProgram, Capability, CapabilityKind, Constraint, Operation, Policy, Term};

const EFFECT_PROBE: &[u8] = include_bytes!("../../fixtures/capability/effect_probe.beam");

/// The effect inventory, which names the capability of each effect entry.
/// `scripts/ci_task.sh` lists this file, so that CI runs the full check for
/// a change to it.
const INVENTORY: &str = include_str!("../../docs/effect-inventory.md");

/// Each capability of the column "Capability" of the effect inventory, as
/// `(entry, kind:operation)`, with the `effect_probe` function that calls
/// the entry once.
const PROBES: [(&str, &str, &str); 6] = [
    ("erlang:spawn/3", "process:spawn", "spawn"),
    ("erlang:spawn_link/3", "process:spawn", "spawn_link"),
    ("erlang:spawn_link/3", "process:link", "spawn_link"),
    ("erlang:spawn_opt/4", "process:spawn", "spawn_opt"),
    ("erlang:spawn_opt/4", "process:link", "spawn_opt_link"),
    ("erlang:spawn_opt/4", "process:monitor", "spawn_opt_monitor"),
];

/// A test policy: the root process gets these capabilities.
struct Grants(Vec<Capability>);

impl Policy for Grants {
    fn initial(&self) -> Vec<Capability> {
        self.0.clone()
    }
}

/// The kind and the operation of each `kind:operation` name of the
/// inventory.
const OPERATIONS: [(&str, CapabilityKind, Operation); 15] = [
    ("process:spawn", CapabilityKind::Process, Operation::Spawn),
    ("process:link", CapabilityKind::Process, Operation::Link),
    (
        "process:monitor",
        CapabilityKind::Process,
        Operation::Monitor,
    ),
    (
        "process:exit_other",
        CapabilityKind::Process,
        Operation::ExitOther,
    ),
    ("terminal:write", CapabilityKind::Terminal, Operation::Write),
    ("terminal:read", CapabilityKind::Terminal, Operation::Read),
    (
        "terminal:raw_mode",
        CapabilityKind::Terminal,
        Operation::RawMode,
    ),
    (
        "terminal:window_size",
        CapabilityKind::Terminal,
        Operation::WindowSize,
    ),
    ("port:spawn", CapabilityKind::Port, Operation::Spawn),
    ("port:command", CapabilityKind::Port, Operation::Command),
    ("port:close", CapabilityKind::Port, Operation::Close),
    ("env:read", CapabilityKind::Env, Operation::Read),
    ("env:write", CapabilityKind::Env, Operation::Write),
    ("signal:handle", CapabilityKind::Signal, Operation::Handle),
    ("signal:ignore", CapabilityKind::Signal, Operation::Ignore),
];

/// Return the kind and the operation of `kind:operation`.
fn operation(name: &str) -> (CapabilityKind, Operation) {
    let &(_, kind, operation) = OPERATIONS
        .iter()
        .find(|entry| entry.0 == name)
        .unwrap_or_else(|| panic!("{name} is not a capability operation"));
    (kind, operation)
}

/// The capability kinds.
const KINDS: [CapabilityKind; 5] = [
    CapabilityKind::Process,
    CapabilityKind::Terminal,
    CapabilityKind::Port,
    CapabilityKind::Env,
    CapabilityKind::Signal,
];

/// Return a policy with a delegable capability of each kind: one of `kind`
/// with `operations` and `constraint`, and one of each other kind with each
/// operation and no constraint.
fn grants(kind: CapabilityKind, operations: &[Operation], constraint: Constraint) -> Grants {
    Grants(
        KINDS
            .into_iter()
            .map(|other| {
                if other == kind {
                    Capability::new(kind, operations, constraint.clone(), true)
                } else {
                    Capability::new(other, other.operations(), Constraint::Any, true)
                }
                .unwrap()
            })
            .collect(),
    )
}

/// Return the policy of [`grants`] with each operation except `denied`.
fn without(denied: (CapabilityKind, Operation)) -> Grants {
    let (kind, denied) = denied;
    let operations: Vec<_> = kind
        .operations()
        .iter()
        .copied()
        .filter(|&operation| operation != denied)
        .collect();
    grants(kind, &operations, Constraint::Any)
}

/// Return the `(entry, kind:operation)` pairs of the column "Capability" of
/// the sections "BIFs" and "Native bindings" of the effect inventory. A
/// cell is `none`, `later`, or a list of `kind:operation` code spans.
fn inventory_capabilities() -> BTreeSet<(String, String)> {
    let mut pairs = BTreeSet::new();
    let mut section = "";
    for line in INVENTORY.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            section = heading;
        } else if matches!(section, "BIFs" | "Native bindings")
            && let Some(row) = line.strip_prefix("| `")
        {
            let cells: Vec<_> = row.split(" | ").collect();
            let entry = &cells[0][..cells[0].find('`').unwrap()];
            for capability in cells[2].split(", ") {
                let capability = capability.trim_matches('`');
                if capability.contains(':') {
                    pairs.insert((entry.to_owned(), capability.to_owned()));
                }
            }
        }
    }
    pairs
}

/// Load the probe.
fn program() -> BeamProgram {
    BeamProgram::load(&[EFFECT_PROBE]).unwrap()
}

/// Run `effect_probe:Function()` with `policy`.
fn run(program: &BeamProgram, function: &str, policy: &dyn Policy) -> Term {
    program
        .execute_with_policy("effect_probe", function, &[], policy)
        .unwrap()
}

/// Return the atom `name` of the program.
fn atom(program: &BeamProgram, name: &str) -> Term {
    Term::Atom(program.atom(name).unwrap())
}

#[test]
fn each_effect_entry_with_a_capability_denies_without_its_operation() {
    let probes: BTreeSet<_> = PROBES
        .iter()
        .map(|&(entry, capability, _)| (entry.to_owned(), capability.to_owned()))
        .collect();
    assert_eq!(inventory_capabilities(), probes);
    let program = program();
    let denied = atom(&program, "denied");
    for (entry, capability, probe) in PROBES {
        let policy = without(operation(capability));
        assert_eq!(
            run(&program, probe, &policy),
            denied,
            "{entry} without {capability}"
        );
    }
}
