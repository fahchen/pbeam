#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const UNDEF_PROBE: &[u8] = include_bytes!("../../fixtures/undef/undef_probe.beam");
const UNDEF_TARGET: &[u8] = include_bytes!("../../fixtures/undef/undef_target.beam");
const UNDEF_HANDLER: &[u8] = include_bytes!("../../fixtures/undef/undef_handler.beam");

#[test]
fn raises_undef_for_a_function_that_a_loaded_module_does_not_export() {
    // Each call goes to the export entry of the function, which calls
    // `error_handler:undefined_function/3` (`beam_common.c:1294-1342`).
    // For `undef_target`, it raises `error:undef` with the first frame
    // `{undef_target, missing, Args, []}` (`error_handler.erl:165-178`).
    // `undef_handler` exports `'$handle_undefined_function'/2`, so the call
    // goes to it (`error_handler.erl:206-213`).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, UNDEF_PROBE, UNDEF_TARGET, UNDEF_HANDLER]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let list = |names: &[&str]| Term::List(names.iter().map(|name| atom(name)).collect());
    let frame = |arguments: &[&str]| {
        Term::Tuple(vec![
            atom("undef_target"),
            atom("missing"),
            list(arguments),
            Term::List(Vec::new()),
        ])
    };
    let caught =
        |arguments: &[&str]| Term::Tuple(vec![atom("error"), atom("undef"), frame(arguments)]);
    let handled = |function: &str, arguments: &[&str]| {
        Term::Tuple(vec![atom("handled"), atom(function), list(arguments)])
    };
    // `call_ext` (a), `call_ext_last` (b), `call_ext_only` (c), `apply`
    // (d), `apply_last` (e), `erlang:apply/3` (f, and g as
    // `call_ext_last`), `erlang:apply/2` (h), `call_fun` (i), `call_fun2`
    // (j, k), and `call_ext` of arity 0, each in a `try` that reads the
    // stack trace through `build_stacktrace`. Then `catch` (l), and the
    // handler through `call_ext` (m) and `apply` (n, o).
    assert_eq!(
        program.execute("undef_probe", "calls", &[]).unwrap(),
        Term::List(vec![
            caught(&["a"]),
            caught(&["b"]),
            caught(&["c"]),
            caught(&["d"]),
            caught(&["e"]),
            caught(&["f"]),
            caught(&["g"]),
            caught(&["h"]),
            caught(&["i"]),
            caught(&["j", "k"]),
            caught(&[]),
            Term::Tuple(vec![atom("EXIT"), atom("undef"), frame(&["l"])]),
            handled("missing", &["m"]),
            handled("other", &["n", "o"]),
        ])
    );

    // The reference uses the `erl` flags of the row "Undefined functions".
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/undef",
            "-eval",
            "io:format(\"~w~n\", [undef_probe:calls()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        "[{error,undef,{undef_target,missing,[a],[]}},{error,undef,{undef_target,missing,[b],[]}},{error,undef,{undef_target,missing,[c],[]}},{error,undef,{undef_target,missing,[d],[]}},{error,undef,{undef_target,missing,[e],[]}},{error,undef,{undef_target,missing,[f],[]}},{error,undef,{undef_target,missing,[g],[]}},{error,undef,{undef_target,missing,[h],[]}},{error,undef,{undef_target,missing,[i],[]}},{error,undef,{undef_target,missing,[j,k],[]}},{error,undef,{undef_target,missing,[],[]}},{'EXIT',undef,{undef_target,missing,[l],[]}},{handled,missing,[m]},{handled,other,[n,o]}]\n"
    );
}

#[test]
fn starts_a_spawned_process_at_a_function_that_its_module_does_not_export() {
    // ERTS starts each process with `i_apply_only` of its MFA
    // (`erl_process.c:12616`), so the child goes to the export entry of
    // `missing/2` or `stop/2` when it first runs. It exits with the `undef`
    // of `error_handler:crash/1`, or calls the handler of `undef_handler`,
    // which exits with its arguments.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, UNDEF_PROBE, UNDEF_TARGET, UNDEF_HANDLER]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let arguments = Term::List(vec![atom("a"), atom("b")]);
    let down = |module: &str, function: &str| {
        program
            .execute(
                "undef_probe",
                "down",
                &[atom(module), atom(function), arguments.clone()],
            )
            .unwrap()
    };
    assert_eq!(
        down("undef_target", "missing"),
        Term::Tuple(vec![
            atom("undef"),
            Term::List(vec![Term::Tuple(vec![
                atom("undef_target"),
                atom("missing"),
                arguments.clone(),
                Term::List(Vec::new()),
            ])]),
        ])
    );
    assert_eq!(
        down("undef_handler", "stop"),
        Term::Tuple(vec![atom("handled"), atom("stop"), arguments.clone()])
    );

    // The reference uses the `erl` flags of the row "Undefined functions":
    // without `-kernel logger_level none`, ERTS can write an error report of
    // the child to stderr.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/undef",
            "-eval",
            "io:format(\"~w~n\", [[undef_probe:down(undef_target, missing, [a, b]), undef_probe:down(undef_handler, stop, [a, b])]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        "[{undef,[{undef_target,missing,[a,b],[]}]},{handled,stop,[a,b]}]\n"
    );
}

#[test]
fn raises_system_limit_for_a_spawn_with_more_than_1023_arguments() {
    // ERTS starts each process with `i_apply_only` of its MFA
    // (`erl_process.c:12616`). Its `apply` copies at most `MAX_REG - 1`
    // arguments and raises `system_limit` for more, before it looks up the
    // export (`beam_common.c:1501-1513`). Thus 1023 arguments go to the
    // `undef` of the export entry, and 1024 arguments raise `system_limit`,
    // also for a module that is not loaded.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, UNDEF_PROBE, UNDEF_TARGET, UNDEF_HANDLER]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let arguments = |count: usize| Term::List(vec![atom("x"); count]);
    let down = |module: &str, count: usize| {
        program
            .execute(
                "undef_probe",
                "down",
                &[atom(module), atom("missing"), arguments(count)],
            )
            .unwrap()
    };
    assert_eq!(
        down("undef_target", 1023),
        Term::Tuple(vec![
            atom("undef"),
            Term::List(vec![Term::Tuple(vec![
                atom("undef_target"),
                atom("missing"),
                arguments(1023),
                Term::List(Vec::new()),
            ])]),
        ])
    );
    // PBeam gives the stack trace `[]` (row "Exception stack trace"). ERTS
    // gives `[{erlang, apply, [Module, missing, Args], ErrorInfo}]`, so the
    // reference compares only the reason.
    let system_limit = Term::Tuple(vec![atom("system_limit"), Term::List(Vec::new())]);
    assert_eq!(down("undef_target", 1024), system_limit);
    // `missing` is not a loaded module.
    assert_eq!(down("missing", 1024), system_limit);

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/undef",
            "-eval",
            "L1023 = lists:duplicate(1023, x), L1024 = lists:duplicate(1024, x), io:format(\"~w~n\", [[undef_probe:down(undef_target, missing, L1023) =:= {undef, [{undef_target, missing, L1023, []}]}, element(1, undef_probe:down(undef_target, missing, L1024)), element(1, undef_probe:down(missing, missing, L1024))]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        "[true,system_limit,system_limit]\n"
    );
}
