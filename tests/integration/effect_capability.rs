//! Capability checks of the effect entries. Each BIF and native binding
//! that the column "Capability" of `docs/effect-inventory.md` gives a
//! capability raises `error:capability_denied` without that operation. A
//! test policy that holds back one operation or limits one constraint is
//! the oracle of each denial, which ERTS does not have. The default policy
//! gives the pinned ERTS results.

use std::{collections::BTreeSet, process::Command};

use pbeam_runtime::{
    AllowAll, BeamError, BeamProgram, Capability, CapabilityKind, Constraint, Operation, Policy,
    Term,
};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const EFFECT_PROBE: &[u8] = include_bytes!("../../fixtures/capability/effect_probe.beam");

/// The stand-in `prim_tty` with the NIF stubs of the pinned module, so the
/// probe calls the native bindings with no `prim_tty:init/1`.
const PRIM_TTY: &[u8] = include_bytes!("../../fixtures/prim_tty/nif/prim_tty.beam");

/// The effect inventory, which names the capability of each effect entry.
/// `scripts/ci_task.sh` lists this file, so that CI runs the full check for
/// a change to it.
const INVENTORY: &str = include_str!("../../docs/effect-inventory.md");

/// Each capability of the column "Capability" of the effect inventory, as
/// `(entry, kind:operation)`, with the `effect_probe` function that calls
/// the entry once.
const PROBES: [(&str, &str, &str); 27] = [
    ("erlang:exit/2", "process:exit_other", "exit_other"),
    ("erlang:link/1", "process:link", "link"),
    ("erlang:monitor/2", "process:monitor", "monitor2"),
    ("erlang:monitor/3", "process:monitor", "monitor3"),
    ("erlang:send/2", "process:send", "send2"),
    ("erlang:send/3", "process:send", "send3"),
    ("erlang:start_timer/3", "process:send", "start_timer"),
    ("erlang:spawn/3", "process:spawn", "spawn"),
    ("erlang:spawn_link/3", "process:spawn", "spawn_link"),
    ("erlang:spawn_link/3", "process:link", "spawn_link"),
    ("erlang:spawn_opt/4", "process:spawn", "spawn_opt"),
    ("erlang:spawn_opt/4", "process:link", "spawn_opt_link"),
    ("erlang:spawn_opt/4", "process:monitor", "spawn_opt_monitor"),
    ("erlang:halt/2", "runtime:halt", "halt_run"),
    ("erlang:load_nif/2", "runtime:load_nif", "load_nif"),
    ("erlang:system_flag/2", "runtime:system_flag", "system_flag"),
    ("erts_internal:open_port/2", "port:spawn", "open_port"),
    (
        "erts_internal:port_command/3",
        "port:command",
        "port_command",
    ),
    ("erts_internal:port_close/1", "port:close", "port_close"),
    ("os:getenv/1", "env:read", "getenv"),
    ("os:set_signal/2", "signal:handle", "handle_signal"),
    ("os:set_signal/2", "signal:ignore", "ignore_signal"),
    ("prim_tty:read_nif/3", "terminal:read", "read_nif"),
    ("prim_tty:tty_init/2", "terminal:raw_mode", "tty_init"),
    ("prim_tty:tty_select/2", "terminal:read", "tty_select"),
    (
        "prim_tty:tty_window_size/1",
        "terminal:window_size",
        "tty_window_size",
    ),
    ("prim_tty:write_nif/2", "terminal:write", "write_nif"),
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
const OPERATIONS: [(&str, CapabilityKind, Operation); 19] = [
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
    ("process:send", CapabilityKind::Process, Operation::Send),
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
    ("runtime:halt", CapabilityKind::Runtime, Operation::Halt),
    (
        "runtime:system_flag",
        CapabilityKind::Runtime,
        Operation::SystemFlag,
    ),
    (
        "runtime:load_nif",
        CapabilityKind::Runtime,
        Operation::LoadNif,
    ),
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
const KINDS: [CapabilityKind; 6] = [
    CapabilityKind::Process,
    CapabilityKind::Terminal,
    CapabilityKind::Port,
    CapabilityKind::Env,
    CapabilityKind::Signal,
    CapabilityKind::Runtime,
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

/// Return the constraint that permits only `names`.
fn only(names: &[&str]) -> Constraint {
    Constraint::Only(names.iter().map(|&name| name.to_owned()).collect())
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

/// Load the probe, the stand-in `prim_tty`, and the pinned `erlang.beam`,
/// which has `erlang:open_port/2`, `erlang:port_command/2`,
/// `erlang:port_close/1`, and `erlang:port_info/1`.
fn program() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    BeamProgram::load(&[&erlang, PRIM_TTY, EFFECT_PROBE]).unwrap()
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

/// Return the tuple of `elements`.
fn tuple(elements: &[Term]) -> Term {
    Term::Tuple(elements.to_vec())
}

/// Return the list of the atoms `names` of the program.
fn atoms(program: &BeamProgram, names: &[&str]) -> Term {
    Term::List(names.iter().map(|&name| atom(program, name)).collect())
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

#[test]
fn checks_the_send_instruction_once_with_erlang_send() {
    // The `send` instruction has no row in the section "BIFs", so the
    // column check above does not cover it.
    let program = program();
    let policy = without((CapabilityKind::Process, Operation::Send));
    assert_eq!(
        run(&program, "send_instruction", &policy),
        atom(&program, "denied")
    );
    assert_eq!(
        run(&program, "send_instruction", &AllowAll),
        atom(&program, "ok")
    );
}

#[test]
fn gives_the_erts_results_with_the_default_policy() {
    let program = program();
    let badarg = atom(&program, "badarg");
    let expected = Term::List(vec![
        atom(&program, "false"),
        atom(&program, "true"),
        atom(&program, "true"),
        Term::Binary(b"x".to_vec()),
        atom(&program, "true"),
        badarg.clone(),
        badarg.clone(),
        badarg,
    ]);
    assert_eq!(run(&program, "permitted", &AllowAll), expected);
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/capability",
            "-eval",
            "io:format(\"~w~n\", [effect_probe:permitted()]), halt().",
        ])
        .env_remove("PBEAM_EFFECT_PROBE")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "[false,true,true,<<120>>,true,badarg,badarg,badarg]\n"
    );
}

#[test]
fn compares_the_native_bytes_of_an_environment_variable_name() {
    let program = program();
    let policy = grants(
        CapabilityKind::Env,
        &[Operation::Read],
        only(&["PBEAM_EFFECT_PROBE"]),
    );
    assert_eq!(
        run(&program, "names", &policy),
        atoms(&program, &["ok", "ok", "denied", "denied", "denied"])
    );
}

#[test]
fn compares_the_program_name_of_a_port_as_the_caller_gives_it() {
    let program = program();
    let policy = grants(
        CapabilityKind::Port,
        CapabilityKind::Port.operations(),
        only(&["/bin/cat"]),
    );
    assert_eq!(
        run(&program, "executables", &policy),
        atoms(&program, &["ok", "ok", "ok", "denied", "denied"])
    );
}

#[test]
fn keeps_the_port_open_after_a_denied_close() {
    let program = program();
    let policy = without((CapabilityKind::Port, Operation::Close));
    assert_eq!(
        run(&program, "close_kept", &policy),
        Term::Tuple(vec![atom(&program, "denied"), atom(&program, "open")])
    );
}

#[test]
fn needs_both_signal_operations_for_the_default_disposition() {
    let program = program();
    let denied = atom(&program, "denied");
    for operation in [Operation::Handle, Operation::Ignore] {
        let policy = without((CapabilityKind::Signal, operation));
        assert_eq!(run(&program, "default_signal", &policy), denied);
    }
    let policy = grants(
        CapabilityKind::Signal,
        CapabilityKind::Signal.operations(),
        only(&["sigusr2"]),
    );
    assert_eq!(
        run(&program, "signals", &policy),
        atoms(&program, &["ok", "denied"])
    );
}

/// Return the `~w` text of `effect_probe:Function()` on the pinned ERTS
/// with the stand-in `prim_tty`. The load of the stand-in stops the
/// standard output of ERTS, so the result goes through a file.
fn erts_with_stand_in(function: &str) -> String {
    let root = std::env::current_dir().unwrap();
    let result = std::env::temp_dir().join(format!(
        "effect_probe_{function}_{}.txt",
        std::process::id()
    ));
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noinput",
            "-nostick",
            "-pa",
            root.join("fixtures/capability").to_str().unwrap(),
            "-eval",
            &format!(
                "{{module, prim_tty}} = code:load_abs(\"{}\"), \
                 ok = file:write_file(\"{}\", io_lib:format(\"~w~n\", [effect_probe:{function}()])), \
                 halt().",
                root.join("fixtures/prim_tty/nif/prim_tty").display(),
                result.display()
            ),
        ])
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = std::fs::read_to_string(&result).unwrap();
    std::fs::remove_file(&result).unwrap();
    text
}

#[test]
fn gives_the_erts_results_of_the_permitted_runtime_operations() {
    let program = program();
    let policy = grants(
        CapabilityKind::Runtime,
        &[Operation::SystemFlag, Operation::LoadNif],
        Constraint::Any,
    );
    let reload = b"NIF library already loaded (reload disallowed since OTP 20).";
    let expected = Term::List(vec![
        atom(&program, "undefined"),
        atom(&program, "true"),
        atom(&program, "ok"),
        Term::Tuple(vec![
            atom(&program, "error"),
            Term::Tuple(vec![
                atom(&program, "reload"),
                Term::List(
                    reload
                        .iter()
                        .map(|&byte| Term::Integer(byte.into()))
                        .collect(),
                ),
            ]),
        ]),
    ]);
    assert_eq!(run(&program, "runtime_permitted", &policy), expected);
    let bytes: Vec<_> = reload.iter().map(u8::to_string).collect();
    assert_eq!(
        erts_with_stand_in("runtime_permitted"),
        format!(
            "[undefined,true,ok,{{error,{{reload,[{}]}}}}]\n",
            bytes.join(",")
        )
    );
    let policy = grants(CapabilityKind::Runtime, &[Operation::Halt], Constraint::Any);
    assert_eq!(
        program.execute_with_policy("effect_probe", "halt_run", &[], &policy),
        Err(BeamError::Halt(3))
    );
    let status = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noinput",
            "-pa",
            "fixtures/capability",
            "-eval",
            "effect_probe:halt_run().",
        ])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3));
}

#[test]
fn checks_the_runtime_arguments_before_the_capability() {
    let program = program();
    let policy = grants(CapabilityKind::Runtime, &[], Constraint::Any);
    let badargs = Term::List(vec![atom(&program, "badarg"); 6]);
    assert_eq!(run(&program, "runtime_badargs", &policy), badargs);
    assert_eq!(
        erts_with_stand_in("runtime_badargs"),
        "[badarg,badarg,badarg,badarg,badarg,badarg]\n"
    );
}

#[test]
fn keeps_the_runtime_state_after_a_denied_call() {
    // A child with `#{runtime => #{operations => []}}` tries the call. The
    // parent then sees the start system logger and loads the library once.
    let program = program();
    let denied = atom(&program, "denied");
    assert_eq!(
        run(&program, "system_flag_kept", &AllowAll),
        Term::Tuple(vec![denied.clone(), atom(&program, "logger")])
    );
    assert_eq!(
        run(&program, "load_nif_kept", &AllowAll),
        Term::Tuple(vec![denied, atom(&program, "ok")])
    );
}

#[test]
fn gives_the_erts_results_of_the_permitted_process_operations() {
    let program = program();
    let atom = |name| atom(&program, name);
    let ok = |sign| tuple(&[atom("ok"), atom(sign)]);
    let expected = Term::List(vec![
        ok("linked"),
        Term::List(vec![ok("down"), ok("down")]),
        ok("killed"),
        ok("exit"),
        Term::List(vec![
            ok("got"),
            ok("got"),
            ok("got"),
            tuple(&[atom("error"), atom("badarg")]),
        ]),
        ok("fired"),
    ]);
    assert_eq!(run(&program, "process_permitted", &AllowAll), expected);
    assert_eq!(
        erts_with_stand_in("process_permitted"),
        "[{ok,linked},[{ok,down},{ok,down}],{ok,killed},{ok,exit},\
         [{ok,got},{ok,got},{ok,got},{error,badarg}],{ok,fired}]\n"
    );
}

#[test]
fn checks_the_process_arguments_before_the_capability() {
    let program = program();
    let policy = grants(CapabilityKind::Process, &[], Constraint::Any);
    let badargs = Term::List(vec![atom(&program, "badarg"); 8]);
    assert_eq!(run(&program, "process_badargs", &policy), badargs);
    assert_eq!(
        erts_with_stand_in("process_badargs"),
        "[badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg]\n"
    );
}

#[test]
fn keeps_the_process_state_after_a_denied_call() {
    let program = program();
    let atom = |name| atom(&program, name);
    let denied = |sign| tuple(&[atom("denied"), atom(sign)]);
    let without_process = |operation| without((CapabilityKind::Process, operation));
    assert_eq!(
        run(&program, "link_kept", &without_process(Operation::Link)),
        denied("unlinked")
    );
    assert_eq!(
        run(
            &program,
            "monitor_kept",
            &without_process(Operation::Monitor)
        ),
        Term::List(vec![denied("none"), denied("none")])
    );
    assert_eq!(
        run(
            &program,
            "exit_kept",
            &without_process(Operation::ExitOther)
        ),
        denied("boom")
    );
    let send = without_process(Operation::Send);
    assert_eq!(
        run(&program, "send_kept", &send),
        Term::List(vec![
            denied("none"),
            denied("none"),
            denied("none"),
            atom("denied"),
        ])
    );
    assert_eq!(run(&program, "timer_kept", &send), denied("none"));
}

#[test]
fn sends_an_exit_signal_to_the_caller_without_exit_other() {
    let program = program();
    let policy = without((CapabilityKind::Process, Operation::ExitOther));
    assert_eq!(
        run(&program, "exit_self", &policy),
        tuple(&[atom(&program, "ok"), atom(&program, "exit")])
    );
}
