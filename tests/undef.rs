mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const UNDEF_PROBE: &[u8] = include_bytes!("../fixtures/undef/undef_probe.beam");
const UNDEF_TARGET: &[u8] = include_bytes!("../fixtures/undef/undef_target.beam");
const UNDEF_HANDLER: &[u8] = include_bytes!("../fixtures/undef/undef_handler.beam");

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
