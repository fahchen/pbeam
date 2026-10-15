//! Capability checks of the effect entries. Each BIF and native binding
//! that the column "Capability" of `docs/effect-inventory.md` gives a
//! capability raises `error:capability_denied` without that operation. A
//! test policy that holds back one operation or limits one constraint is
//! the oracle of each denial, which ERTS does not have. The default policy
//! gives the pinned ERTS results.

use std::{
    collections::BTreeSet,
    ffi::CStr,
    process::{Command, Stdio},
};

use pbeam_runtime::{
    AllowAll, AtomTable, BeamError, BeamProgram, Capability, CapabilityKind, Constraint, Operation,
    Policy, Term,
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
const PROBES: [(&str, &str, &str); 46] = [
    ("erlang:exit/2", "process:exit_other", "exit_other"),
    ("erlang:link/1", "process:link", "link"),
    ("erlang:monitor/2", "process:monitor", "monitor2"),
    ("erlang:monitor/3", "process:monitor", "monitor3"),
    ("erlang:send/2", "process:send", "send2"),
    ("erlang:send/3", "process:send", "send3"),
    ("erlang:start_timer/3", "process:send", "start_timer"),
    ("erlang:process_info/2", "process:inspect", "process_info"),
    (
        "erlang:is_process_alive/1",
        "process:inspect",
        "is_process_alive",
    ),
    (
        "erts_internal:is_process_alive/2",
        "process:inspect",
        "is_alive_request",
    ),
    (
        "erts_internal:group_leader/2",
        "process:group_leader",
        "group_leader2",
    ),
    (
        "erts_internal:group_leader/3",
        "process:group_leader",
        "group_leader3",
    ),
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
    ("erts_internal:port_info/1", "port:inspect", "port_info"),
    ("os:getenv/1", "env:read", "getenv"),
    ("os:set_signal/2", "signal:handle", "handle_signal"),
    ("os:set_signal/2", "signal:ignore", "ignore_signal"),
    ("persistent_term:get/0", "persistent_term:read", "pt_get0"),
    ("persistent_term:get/1", "persistent_term:read", "pt_get1"),
    ("persistent_term:get/2", "persistent_term:read", "pt_get2"),
    ("persistent_term:put/2", "persistent_term:write", "pt_put"),
    ("prim_tty:isatty/1", "terminal:query", "isatty"),
    ("prim_tty:read_nif/3", "terminal:read", "read_nif"),
    ("prim_tty:setlocale/1", "terminal:setup", "setlocale"),
    ("prim_tty:setupterm_nif/0", "terminal:setup", "setupterm"),
    ("prim_tty:tigetflag_nif/1", "terminal:query", "tigetflag"),
    ("prim_tty:tigetnum_nif/1", "terminal:query", "tigetnum"),
    ("prim_tty:tty_create/1", "terminal:query", "tty_create"),
    ("prim_tty:tty_init/2", "terminal:raw_mode", "tty_init"),
    ("prim_tty:tty_select/2", "terminal:read", "tty_select"),
    (
        "prim_tty:tty_window_size/1",
        "terminal:window_size",
        "tty_window_size",
    ),
    ("prim_tty:write_nif/2", "terminal:write", "write_nif"),
    ("erlang:finish_loading/1", "code:load", "finish_loading"),
    ("erlang:module_loaded/1", "code:inspect", "module_loaded"),
    (
        "erlang:get_module_info/2",
        "code:inspect",
        "get_module_info",
    ),
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
const OPERATIONS: [(&str, CapabilityKind, Operation); 28] = [
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
    (
        "process:inspect",
        CapabilityKind::Process,
        Operation::Inspect,
    ),
    (
        "process:group_leader",
        CapabilityKind::Process,
        Operation::GroupLeader,
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
    ("terminal:setup", CapabilityKind::Terminal, Operation::Setup),
    ("terminal:query", CapabilityKind::Terminal, Operation::Query),
    ("port:spawn", CapabilityKind::Port, Operation::Spawn),
    ("port:command", CapabilityKind::Port, Operation::Command),
    ("port:close", CapabilityKind::Port, Operation::Close),
    ("port:inspect", CapabilityKind::Port, Operation::Inspect),
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
    (
        "persistent_term:read",
        CapabilityKind::PersistentTerm,
        Operation::Read,
    ),
    (
        "persistent_term:write",
        CapabilityKind::PersistentTerm,
        Operation::Write,
    ),
    ("code:load", CapabilityKind::Code, Operation::Load),
    ("code:inspect", CapabilityKind::Code, Operation::Inspect),
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
const KINDS: [CapabilityKind; 8] = [
    CapabilityKind::Process,
    CapabilityKind::Terminal,
    CapabilityKind::Port,
    CapabilityKind::Env,
    CapabilityKind::Signal,
    CapabilityKind::Runtime,
    CapabilityKind::PersistentTerm,
    CapabilityKind::Code,
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

/// Each entry that the column "Capability" of the effect inventory marks
/// `ambient`. `effect_probe:ambient/0` calls each entry once.
const AMBIENT: [&str; 29] = [
    "erlang:cancel_timer/2",
    "erlang:demonitor/1",
    "erlang:demonitor/2",
    "erlang:function_exported/3",
    "erlang:list_to_atom/1",
    "erlang:monotonic_time/1",
    "erlang:node/0",
    "erlang:node/1",
    "erlang:register/2",
    "erlang:system_info/1",
    "erlang:universaltime_to_localtime/1",
    "erlang:unlink/1",
    "erlang:unregister/1",
    "erlang:whereis/1",
    "erts_internal:prepare_loading/2",
    "ets:delete/2",
    "ets:info/2",
    "ets:insert/2",
    "ets:lookup/2",
    "ets:lookup_element/4",
    "ets:member/2",
    "ets:new/2",
    "ets:whereis/1",
    "file:native_name_encoding/0",
    "os:system_time/1",
    "prim_file:internal_name2native/1",
    "prim_file:internal_native2name/1",
    "prim_file:read_info_nif/2",
    "re:compile/2",
];

/// Each entry that the column "Capability" of the effect inventory marks
/// `later`: it has no check until its named task adds one.
/// `effect_probe:ambient/0` also calls each entry once.
const LATER: [&str; 4] = [
    "prim_file:get_cwd_nif/0",
    "prim_file:list_dir_nif/1",
    "prim_file:read_file_nif/1",
    "prim_file:read_link_nif/1",
];

/// Return the entry and the cell of the column "Capability" of each row of
/// the sections "BIFs" and "Native bindings" of the effect inventory.
fn inventory_cells() -> Vec<(&'static str, &'static str)> {
    let mut cells = Vec::new();
    let mut section = "";
    for line in INVENTORY.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            section = heading;
        } else if matches!(section, "BIFs" | "Native bindings")
            && let Some(row) = line.strip_prefix("| `")
        {
            let row: Vec<_> = row.split(" | ").collect();
            cells.push((&row[0][..row[0].find('`').unwrap()], row[2]));
        }
    }
    cells
}

/// Return the `(entry, kind:operation)` pairs of the column "Capability" of
/// the sections "BIFs" and "Native bindings" of the effect inventory. A
/// cell is `none`, `` `ambient`: Reason ``, `` `later`: Task ``, or a list
/// of `kind:operation` code spans.
fn inventory_capabilities() -> BTreeSet<(String, String)> {
    let mut pairs = BTreeSet::new();
    for (entry, cell) in inventory_cells() {
        if cell.starts_with("`ambient`") || cell.starts_with("`later`") {
            continue;
        }
        for capability in cell.split(", ") {
            let capability = capability.trim_matches('`');
            if capability.contains(':') {
                pairs.insert((entry.to_owned(), capability.to_owned()));
            }
        }
    }
    pairs
}

/// Return the entries that the column "Capability" of the effect inventory
/// marks with `value`, for example `` `ambient` ``.
fn inventory_marked(value: &str) -> BTreeSet<&'static str> {
    inventory_cells()
        .into_iter()
        .filter(|(_, cell)| cell.starts_with(value))
        .map(|(entry, _)| entry)
        .collect()
}

/// Load the probe, the stand-in `prim_tty`, the pinned `erlang.beam`, which
/// has `erlang:open_port/2`, `erlang:port_command/2`, `erlang:port_close/1`,
/// and `erlang:port_info/1`, and the pinned `prim_file.beam`, whose
/// `read_file_info/1`, `read_file/1`, `list_dir/1`, `read_link/1`, and
/// `get_cwd/0` call the `prim_file` native bindings.
fn program() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    let prim_file = pinned_beam("prim_file");
    BeamProgram::load(&[&erlang, &prim_file, PRIM_TTY, EFFECT_PROBE]).unwrap()
}

/// Run `effect_probe:Function()` with `policy`.
fn run(program: &BeamProgram, function: &str, policy: &dyn Policy) -> Term {
    program
        .execute_with_policy("effect_probe", function, &[], policy)
        .unwrap()
        .term
}

/// Run `effect_probe:Function()` with `policy` and return the `~w` text of
/// the result, with each atom name from the table of the run.
fn run_text(program: &BeamProgram, function: &str, policy: &dyn Policy) -> String {
    let result = program
        .execute_with_policy("effect_probe", function, &[], policy)
        .unwrap();
    text(&result.term, result.atoms())
}

/// Return the atom `name` of the program.
fn atom(program: &BeamProgram, name: &str) -> Term {
    Term::Atom(program.atom(name).unwrap())
}

/// Return the tuple of `elements`.
fn tuple(elements: &[Term]) -> Term {
    Term::tuple(elements.to_vec())
}

/// Return the list of the atoms `names` of the program.
fn atoms(program: &BeamProgram, names: &[&str]) -> Term {
    Term::list_from(names.iter().map(|&name| atom(program, name)))
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
fn calls_each_ambient_and_later_entry_with_no_capability() {
    assert_eq!(inventory_marked("`ambient`"), BTreeSet::from(AMBIENT));
    assert_eq!(inventory_marked("`later`"), BTreeSet::from(LATER));
    // The root process holds no capability, so no policy can permit the
    // calls: each `ambient` and `later` entry has no check.
    let program = program();
    let expected = "[true,true,false,true,true,true,integer,integer,date_time,unix,\
                    nonode@nohost,nonode@nohost,true,{error,badfile},false,\
                    effect_probe_ambient_atom,ok,effect_probe_ambient,true,[{key,value}],value,\
                    true,true,set,reference,atom,2,ok,1,error,ok,error,ok]";
    assert_eq!(run_text(&program, "ambient", &Grants(vec![])), expected);
    assert_eq!(erts_with_stand_in("ambient"), format!("{expected}\n"));
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
    let expected = Term::list_from(vec![
        atom(&program, "false"),
        atom(&program, "true"),
        atom(&program, "true"),
        Term::binary(b"x".to_vec()),
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
        Term::tuple(vec![atom(&program, "denied"), atom(&program, "open")])
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

#[test]
fn compares_the_module_name_of_a_code_entry() {
    // The `modules` constraint of `code` compares the module atom of
    // `erlang:module_loaded/1` and `erlang:get_module_info/2`, and each
    // module of the list of `erlang:finish_loading/1`. An empty list has no
    // module name, so it needs the `load` operation only.
    let program = program();
    let policy = grants(
        CapabilityKind::Code,
        CapabilityKind::Code.operations(),
        only(&["effect_probe"]),
    );
    assert_eq!(
        run(&program, "code_names", &policy),
        atoms(&program, &["ok", "denied", "ok", "denied", "ok"])
    );
}

#[test]
fn checks_the_key_of_get_module_info_before_the_code_capability() {
    // The argument checks of `erlang:get_module_info/2` come before its
    // capability check, as they do in `erlang:process_info/2` (row
    // "Capability checks" of `docs/erts-differences.md`): a key that is not
    // an atom raises `badarg` in a process with no capability. Pinned ERTS
    // raises `badarg` for it too (`erl_bif_info.c:5986-6008`).
    let program = program();
    let expected = "{error,badarg}";
    assert_eq!(run_text(&program, "code_badarg", &Grants(vec![])), expected);
    assert_eq!(erts_with_stand_in("code_badarg"), format!("{expected}\n"));
}

/// Return the `~w` text of `effect_probe:Function()` on the pinned ERTS
/// with the stand-in `prim_tty`. The load of the stand-in stops the
/// standard output of ERTS, so the result goes through a file.
fn erts_with_stand_in(function: &str) -> String {
    erts_result(function, |_| {})
}

/// Return the `~w` text of `effect_probe:Function()` on the pinned ERTS, as
/// [`erts_with_stand_in`] does, with the standard input and output of the
/// test process. Thus ERTS sees the same fd 0 and fd 1 as the PBeam run in
/// the test process.
fn erts_with_standard_streams(function: &str) -> String {
    erts_result(function, |command| {
        command.stdin(Stdio::inherit()).stdout(Stdio::inherit());
    })
}

/// Return the `~w` text of `effect_probe:Function()` on the pinned ERTS
/// with the stand-in `prim_tty`, after `configure` changes the `erl`
/// command.
fn erts_result(function: &str, configure: impl FnOnce(&mut Command)) -> String {
    let root = std::env::current_dir().unwrap();
    let result = std::env::temp_dir().join(format!(
        "effect_probe_{function}_{}.txt",
        std::process::id()
    ));
    let mut command = Command::new("erl");
    command
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
        .current_dir(std::env::temp_dir());
    configure(&mut command);
    let output = command.output().unwrap();
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
    let expected = Term::list_from(vec![
        atom(&program, "undefined"),
        atom(&program, "true"),
        atom(&program, "ok"),
        Term::tuple(vec![
            atom(&program, "error"),
            Term::tuple(vec![
                atom(&program, "reload"),
                Term::list_from(reload.iter().map(|&byte| Term::Integer(byte.into()))),
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
    let badargs = Term::list_from(vec![atom(&program, "badarg"); 6]);
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
        Term::tuple(vec![denied.clone(), atom(&program, "logger")])
    );
    assert_eq!(
        run(&program, "load_nif_kept", &AllowAll),
        Term::tuple(vec![denied, atom(&program, "ok")])
    );
}

#[test]
fn skips_each_effect_check_in_a_run_with_no_policy() {
    // A child with `#{Kind => #{operations => []}}` makes the call. With no
    // policy, the run skips each check, so the call has its effect, as on
    // ERTS: the parent then sees the changed state.
    let program = program();
    let unchecked = |function| program.execute("effect_probe", function, &[]).unwrap();
    let ok = atom(&program, "ok");
    assert_eq!(
        unchecked("system_flag_kept").term,
        tuple(&[ok.clone(), atom(&program, "undefined")])
    );
    let stored = |value| tuple(&[ok.clone(), Term::Integer(value)]);
    assert_eq!(
        unchecked("persistent_term_kept").term,
        Term::list_from(vec![stored(1), stored(2)])
    );
}

#[test]
fn gives_the_erts_results_of_the_permitted_process_operations() {
    let program = program();
    let atom = |name| atom(&program, name);
    let ok = |sign| tuple(&[atom("ok"), atom(sign)]);
    let expected = Term::list_from(vec![
        ok("linked"),
        Term::list_from(vec![ok("down"), ok("down")]),
        ok("killed"),
        ok("exit"),
        Term::list_from(vec![
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
    let badargs = Term::list_from(vec![atom(&program, "badarg"); 8]);
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
        Term::list_from(vec![denied("none"), denied("none")])
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
        Term::list_from(vec![
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

/// Return the policy of [`grants`] with a `process` capability of
/// `operations`.
fn process_grants(operations: &[Operation]) -> Grants {
    grants(CapabilityKind::Process, operations, Constraint::Any)
}

#[test]
fn gives_the_erts_results_of_the_permitted_process_inspection() {
    let program = program();
    let policy = process_grants(&[
        Operation::Spawn,
        Operation::Monitor,
        Operation::Send,
        Operation::Inspect,
        Operation::GroupLeader,
    ]);
    let expected = "[[[],{message_queue_len,0},true,ok,true,true,ok,true],true,\
                    [undefined,false,ok,false,badarg,ok,badarg]]";
    assert_eq!(
        run_text(&program, "process_inspect_permitted", &policy),
        expected
    );
    assert_eq!(
        erts_with_stand_in("process_inspect_permitted"),
        format!("{expected}\n")
    );
}

#[test]
fn checks_the_inspection_arguments_before_the_capability() {
    let program = program();
    let policy = process_grants(&[Operation::Spawn, Operation::Monitor]);
    let badargs = Term::list_from(vec![atom(&program, "badarg"); 9]);
    assert_eq!(run(&program, "process_inspect_badargs", &policy), badargs);
    assert_eq!(
        erts_with_stand_in("process_inspect_badargs"),
        "[badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg]\n"
    );
}

#[test]
fn inspects_the_caller_without_inspect_and_group_leader() {
    let program = program();
    let policy = process_grants(&[]);
    let expected = "[[],true,ok,true,true,ok,true]";
    assert_eq!(
        run_text(&program, "process_inspect_self", &policy),
        expected
    );
    assert_eq!(
        erts_with_stand_in("process_inspect_self"),
        format!("{expected}\n")
    );
}

#[test]
fn keeps_the_process_state_after_a_denied_inspection() {
    // A child of the probe holds no `inspect` and no `group_leader`.
    let program = program();
    assert_eq!(
        run_text(&program, "inspect_kept", &AllowAll),
        "[denied,denied,{denied,none}]"
    );
    assert_eq!(
        run_text(&program, "group_leader_kept", &AllowAll),
        "{[denied,denied],same}"
    );
}

#[test]
fn gives_the_erts_results_of_the_permitted_persistent_term_operations() {
    let program = program();
    let atom = |name| atom(&program, name);
    let key = tuple(&[atom("effect_probe"), atom("permitted")]);
    let expected = Term::list_from(vec![
        atom("none"),
        atom("badarg"),
        atom("ok"),
        atom("ok"),
        Term::Integer(1),
        Term::Integer(1),
        tuple(&[key, Term::Integer(1)]),
    ]);
    let policy = grants(
        CapabilityKind::PersistentTerm,
        &[Operation::Read, Operation::Write],
        Constraint::Any,
    );
    assert_eq!(
        run(&program, "persistent_term_permitted", &policy),
        expected
    );
    assert_eq!(
        erts_with_stand_in("persistent_term_permitted"),
        "[none,badarg,ok,ok,1,1,{{effect_probe,permitted},1}]\n"
    );
}

#[test]
fn keeps_the_persistent_terms_after_a_denied_put() {
    // A child with `#{persistent_term => #{operations => []}}` tries a put
    // of a new key, then of a changed value. The parent then sees no value,
    // then the old value.
    let program = program();
    let denied = atom(&program, "denied");
    assert_eq!(
        run(&program, "persistent_term_kept", &AllowAll),
        Term::list_from(vec![
            tuple(&[denied.clone(), atom(&program, "none")]),
            tuple(&[denied, Term::Integer(1)]),
        ])
    );
}

/// Return a term as `io:format("~w")` prints it, for the atoms, the
/// integers, and the tuples of the terminal probe results.
///
/// The atom names come from `atoms`, the table that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::static_atom("ok")]), &atoms), "[ok]");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(integer) => integer.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn gives_the_erts_results_of_the_permitted_terminal_setup_and_queries() {
    // The results of `isatty/1`, `setupterm_nif/0`, `tigetnum_nif/1`, and
    // `tigetflag_nif/1` depend on the standard streams and on `TERM`, so
    // ERTS runs with the streams and the environment of this test process.
    let program = program();
    let policy = grants(
        CapabilityKind::Terminal,
        &[Operation::Setup, Operation::Query],
        Constraint::Any,
    );
    let pbeam = run_text(&program, "terminal_permitted", &policy);
    assert_eq!(
        format!("{pbeam}\n"),
        erts_with_standard_streams("terminal_permitted")
    );
    assert!(pbeam.contains(",reference,"), "{pbeam}");
}

#[test]
fn checks_the_terminal_fds_and_codes_before_the_capability() {
    let program = program();
    let policy = grants(CapabilityKind::Terminal, &[], Constraint::Any);
    let badargs = Term::list_from(vec![atom(&program, "badarg"); 5]);
    assert_eq!(run(&program, "terminal_badargs", &policy), badargs);
    assert_eq!(
        erts_with_stand_in("terminal_badargs"),
        "[badarg,badarg,badarg,badarg,badarg]\n"
    );
}

/// Return the `LC_CTYPE` locale of the test process.
fn ctype_locale() -> String {
    // SAFETY: a null locale only reads the current locale, and the result
    // is a NUL-terminated string that stays valid until the next locale
    // call.
    unsafe {
        CStr::from_ptr(libc::setlocale(libc::LC_CTYPE, std::ptr::null()))
            .to_string_lossy()
            .into_owned()
    }
}

#[test]
fn keeps_the_locale_after_a_denied_setlocale() {
    // No test of this file permits `prim_tty:setlocale/1`, so the locale
    // stays the C start locale of the process. A permitted call sets it from
    // `LANG`.
    let program = program();
    let policy = without((CapabilityKind::Terminal, Operation::Setup));
    assert_eq!(
        run(&program, "setlocale", &policy),
        atom(&program, "denied")
    );
    assert_eq!(ctype_locale(), "C");
}

#[test]
fn gives_the_erts_results_of_the_permitted_port_information() {
    let program = program();
    let atom = |name| atom(&program, name);
    let item = |key, value| tuple(&[atom(key), value]);
    let expected = Term::list_from(vec![
        atoms(
            &program,
            &[
                "name",
                "links",
                "id",
                "connected",
                "input",
                "output",
                "os_pid",
            ],
        ),
        item(
            "name",
            Term::list_from(b"/bin/cat".iter().map(|&byte| Term::Integer(byte.into()))),
        ),
        atom("true"),
        atom("true"),
        item("input", Term::Integer(0)),
        item("output", Term::Integer(0)),
    ]);
    let policy = grants(
        CapabilityKind::Port,
        &[Operation::Spawn, Operation::Inspect],
        only(&["/bin/cat"]),
    );
    assert_eq!(run(&program, "inspect_permitted", &policy), expected);
    assert_eq!(
        erts_with_stand_in("inspect_permitted"),
        "[[name,links,id,connected,input,output,os_pid],\
         {name,[47,98,105,110,47,99,97,116]},true,true,{input,0},{output,0}]\n"
    );
}

#[test]
fn gives_undefined_for_a_closed_port_without_inspect() {
    let program = program();
    let policy = without((CapabilityKind::Port, Operation::Inspect));
    assert_eq!(
        run(&program, "inspect_closed", &policy),
        atoms(&program, &["undefined", "undefined"])
    );
    assert_eq!(
        erts_with_stand_in("inspect_closed"),
        "[undefined,undefined]\n"
    );
}

#[test]
fn compares_the_program_name_of_a_port_for_its_information() {
    let program = program();
    assert_eq!(
        run(&program, "inspect_names", &AllowAll),
        atoms(&program, &["ok", "denied"])
    );
}
