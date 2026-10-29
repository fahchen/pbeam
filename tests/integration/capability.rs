//! Capability checks at spawn. The default policy gives the pinned ERTS
//! results. A test policy that holds back one operation or one module is
//! the oracle of each denial, which ERTS does not have.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{collections::BTreeSet, process::Command};

use pbeam_runtime::{
    AllowAll, BeamProgram, Capability, CapabilityKind, Constraint, Delegation, Operation, Policy,
    Term,
};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const CAPABILITY_PROBE: &[u8] = include_bytes!("../../fixtures/capability/capability_probe.beam");

/// A test policy: the root process gets these capabilities.
struct Grants(Vec<Capability>);

impl Policy for Grants {
    fn initial(&self) -> Vec<Capability> {
        self.0.clone()
    }
}

/// Return a policy with one `process` capability.
fn process(operations: &[Operation], modules: Constraint, delegation: Delegation) -> Grants {
    Grants(vec![
        Capability::new(CapabilityKind::Process, operations, modules, delegation).unwrap(),
    ])
}

/// Return the `process` operations except `denied`.
fn all_but(denied: Operation) -> Vec<Operation> {
    let operations = CapabilityKind::Process.operations();
    operations
        .iter()
        .copied()
        .filter(|&operation| operation != denied)
        .collect()
}

/// Return the constraint that permits only `modules`.
fn only(modules: &[&str]) -> Constraint {
    Constraint::Only(modules.iter().map(|&module| module.to_owned()).collect())
}

/// Load the probe and the pinned `erlang.beam`, which has
/// `erlang:spawn_monitor/3` and `erlang:spawn/1`.
fn program() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    BeamProgram::load(&[&erlang, CAPABILITY_PROBE]).unwrap()
}

/// Return the list of the probe results with these names, as `ok`,
/// `denied`, or `{error, badarg}`.
fn results(program: &BeamProgram, names: &[&str]) -> Term {
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    Term::list_from(names.iter().map(|&name| match name {
        "badarg" => Term::tuple(vec![atom("error"), atom("badarg")]),
        name => atom(name),
    }))
}

/// Run `capability_probe:Function()` with `policy`.
fn run(program: &BeamProgram, function: &str, policy: &dyn Policy) -> Term {
    program
        .execute_with_policy("capability_probe", function, &[], policy)
        .unwrap()
        .term
}

/// Run `capability_probe:Function()` on the pinned ERTS with one
/// scheduler and return the printed result.
fn erts(function: &str) -> String {
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/capability",
            "-eval",
            &format!("io:format(\"~w~n\", [capability_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn gives_the_erts_results_with_the_default_policy() {
    let program = program();
    let five_ok = results(&program, &["ok"; 5]);
    for function in ["main", "inherit"] {
        assert_eq!(
            program
                .execute("capability_probe", function, &[])
                .unwrap()
                .term,
            five_ok
        );
        assert_eq!(erts(function), "[ok,ok,ok,ok,ok]\n");
    }
    assert_eq!(
        program
            .execute("capability_probe", "modules", &[])
            .unwrap()
            .term,
        results(&program, &["ok", "ok"])
    );
    assert_eq!(erts("modules"), "[ok,ok]\n");
}

#[test]
fn denies_each_spawn_without_the_spawn_operation() {
    let program = program();
    let policy = process(
        &all_but(Operation::Spawn),
        Constraint::Any,
        Delegation::Delegable,
    );
    assert_eq!(
        run(&program, "main", &policy),
        results(&program, &["denied"; 5])
    );
}

#[test]
fn denies_a_linked_spawn_without_the_link_operation() {
    let program = program();
    let policy = process(
        &all_but(Operation::Link),
        Constraint::Any,
        Delegation::Delegable,
    );
    assert_eq!(
        run(&program, "main", &policy),
        results(&program, &["ok", "denied", "ok", "denied", "ok"])
    );
}

#[test]
fn denies_a_monitored_spawn_without_the_monitor_operation() {
    let program = program();
    let policy = process(
        &all_but(Operation::Monitor),
        Constraint::Any,
        Delegation::Delegable,
    );
    assert_eq!(
        run(&program, "main", &policy),
        results(&program, &["ok", "ok", "denied", "ok", "denied"])
    );
}

#[test]
fn checks_the_module_of_the_spawn_mfa() {
    let program = program();
    let ops = CapabilityKind::Process.operations();
    // `erlang:spawn/1` starts a fun as `erlang:apply/2` (`erlang.erl:10601-10602`),
    // so the check sees the module `erlang`, not the module of the fun.
    let probe_only = process(ops, only(&["capability_probe"]), Delegation::Delegable);
    assert_eq!(
        run(&program, "modules", &probe_only),
        results(&program, &["ok", "denied"])
    );
    let erlang_only = process(ops, only(&["erlang"]), Delegation::Delegable);
    assert_eq!(
        run(&program, "modules", &erlang_only),
        results(&program, &["denied", "ok"])
    );
    // An undefined start is checked by its module before it takes a pid.
    assert_eq!(
        run(&program, "undefined", &erlang_only),
        results(&program, &["denied"])
    );
}

#[test]
fn gives_a_child_only_the_delegable_capabilities() {
    let program = program();
    let use_only = process(
        CapabilityKind::Process.operations(),
        Constraint::Any,
        Delegation::NotDelegable,
    );
    assert_eq!(
        run(&program, "main", &use_only),
        results(&program, &["ok"; 5])
    );
    assert_eq!(
        run(&program, "inherit", &use_only),
        results(&program, &["denied"; 5])
    );
    assert_eq!(
        run(&program, "inherit", &AllowAll),
        results(&program, &["ok"; 5])
    );
    assert_eq!(
        run(&program, "undelegable", &AllowAll),
        results(&program, &["denied"; 5])
    );
    assert_eq!(
        run(&program, "main", &Grants(vec![])),
        results(&program, &["denied"; 5])
    );
}

#[test]
fn reduces_the_capabilities_of_a_child_on_request() {
    let program = program();
    assert_eq!(
        run(&program, "reduce", &AllowAll),
        results(&program, &["ok", "denied", "ok", "denied", "ok"])
    );
    assert_eq!(
        run(&program, "empty", &AllowAll),
        results(&program, &["denied"; 5])
    );
    // ERTS has no `capabilities` spawn option.
    assert_eq!(erts("reduce"), "{error,badarg}\n");
}

#[test]
fn denies_a_request_for_more_than_the_caller_can_delegate() {
    let program = program();
    let policy = process(
        &[Operation::Spawn, Operation::Monitor],
        only(&["capability_probe"]),
        Delegation::Delegable,
    );
    assert_eq!(
        run(&program, "widen", &policy),
        results(
            &program,
            &["ok", "denied", "ok", "denied", "denied", "denied"]
        )
    );
}

#[test]
fn raises_badarg_for_a_capabilities_option_that_is_not_valid() {
    let program = program();
    let mut expected = vec!["badarg"; 19];
    expected.push("ok");
    assert_eq!(
        run(&program, "malformed", &AllowAll),
        results(&program, &expected)
    );
}

#[test]
fn skips_each_spawn_check_and_reduction_in_a_run_with_no_policy() {
    // With no policy, the run skips each check, and a child gets no copy,
    // so a `{capabilities, Map}` option does not reduce it. With `AllowAll`,
    // each of these children is denied (see the tests above).
    let program = program();
    let unchecked = |function| program.execute("capability_probe", function, &[]).unwrap();
    for function in ["reduce", "empty", "undelegable"] {
        assert_eq!(
            unchecked(function).term,
            results(&program, &["ok"; 5]),
            "{function}"
        );
    }
    assert_eq!(unchecked("widen").term, results(&program, &["ok"; 6]));
    // The option is still checked: a `Map` that is not valid raises badarg.
    let mut expected = vec!["badarg"; 19];
    expected.push("ok");
    assert_eq!(unchecked("malformed").term, results(&program, &expected));
}

#[test]
fn makes_only_capabilities_that_their_kind_permits() {
    let names = BTreeSet::from([String::from("HOME")]);
    let error = Capability::new(
        CapabilityKind::Env,
        &[Operation::Spawn],
        Constraint::Any,
        Delegation::Delegable,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Spawn is not an operation of the env capability"
    );
    let error = Capability::new(
        CapabilityKind::Terminal,
        &[Operation::Write],
        Constraint::Only(names.clone()),
        Delegation::Delegable,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the terminal capability has no constraint field"
    );
    assert!(
        Capability::new(
            CapabilityKind::Env,
            &[Operation::Read],
            Constraint::Only(names),
            Delegation::NotDelegable
        )
        .is_ok()
    );
}
