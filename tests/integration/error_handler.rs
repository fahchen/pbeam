//! The export entry of a function that a module does not export calls
//! `error_handler:undefined_function/3` (`beam_common.c:1294-1342`), and
//! `erlang:process_flag(error_handler, Module)` sets the module of that call
//! (`process_flag_2`, `bif.c:2045-2051`).

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const PROBE: &[u8] = include_bytes!("../../fixtures/error_handler/error_handler_probe.beam");
const TARGET: &[u8] = include_bytes!("../../fixtures/error_handler/error_handler_target.beam");
const CUSTOM: &[u8] = include_bytes!("../../fixtures/error_handler/error_handler_custom.beam");
const UNDEF_INIT: &[u8] = include_bytes!("../../fixtures/undef/undef_init.beam");

/// The pinned `error_handler` module that the export entry runs, and the
/// `init` module that its `ensure_loaded/1` calls
/// (`error_handler.erl:182-198`). The run loads `error_handler_target` and
/// `error_handler_custom`, and `reference` loads them too, so the two runs
/// compare the same code. The probe starts the init responder of
/// `fixtures/undef/undef_init` only when the run has no init process, so the
/// ERTS run of the reference keeps its own init process.
fn program() -> BeamProgram {
    let (erlang, error_handler, init) = (
        pinned_beam("erlang"),
        pinned_beam("error_handler"),
        pinned_beam("init"),
    );
    BeamProgram::load(&[
        &erlang,
        &error_handler,
        &init,
        UNDEF_INIT,
        PROBE,
        TARGET,
        CUSTOM,
    ])
    .unwrap()
}

/// Run `error_handler_probe:Function()` on the pinned ERTS and return its
/// `~w` output. The run loads the two modules that `program()` loads and that
/// ERTS does not load at its own start, because `error_handler_custom` is the
/// handler of the measured calls and `error_handler_target` is the module of
/// the failed calls. Without the load, the ERTS run takes the code path of
/// `error_handler:ensure_loaded/1` on the first `undef` and the module of the
/// handler is not in the export table, so its result and its reductions are
/// not those of PBeam (`error_handler.erl:182-198`).
fn reference(function: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/error_handler",
            "-pa",
            "fixtures/undef",
            "-eval",
            &format!(
                "{{module,error_handler_target}} = code:ensure_loaded(error_handler_target), \
                 {{module,error_handler_custom}} = code:ensure_loaded(error_handler_custom), \
                 io:format(\"~w~n\", [error_handler_probe:{function}()]), halt()."
            ),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

#[test]
fn raises_undef_through_error_handler_beam_as_pinned_erts() {
    // `error_handler:undefined_function/3` raises `error:undef` with the
    // frame of the failed call and the frames of its callers
    // (`error_handler.erl:165-178`). PBeam runs the pinned module, so the
    // first frame is the tuple that `crash/3` builds. The failed call is the
    // last call of `caller/0`, so the handler replaces the frame of
    // `caller/0` and the frame of `frames/0` follows the first frame. ERTS
    // puts the frames of `erl_eval` and `init` after its own caller frames.
    let program = program();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let frame = Term::tuple(vec![
        atom("error_handler_target"),
        atom("missing"),
        Term::list_from(vec![atom("a")]),
        Term::nil(),
    ]);
    assert_eq!(
        program
            .execute("error_handler_probe", "first", &[])
            .unwrap()
            .term,
        Term::tuple(vec![atom("error"), atom("undef"), frame])
    );
    assert_eq!(
        reference("first"),
        "{error,undef,{error_handler_target,missing,[a],[]}}\n"
    );
    // The first frame of the whole stack trace is the frame of the failed
    // call, and `error_handler:crash/1` keeps the frames of the callers after
    // it (`error_handler.erl:172-178`).
    let frames = program
        .execute("error_handler_probe", "frames", &[])
        .unwrap()
        .term
        .proper_list()
        .unwrap()
        .map(text)
        .collect::<Vec<_>>();
    assert_eq!(frames[0], "{error_handler_target,missing,[a],[]}");
    assert!(frames.len() >= 2, "{frames:?}");
    assert!(
        reference("frames").starts_with("[{error_handler_target,missing,[a],[]},"),
        "{}",
        reference("frames")
    );
}

#[test]
fn sets_the_error_handler_of_the_process_as_pinned_erts() {
    // `process_flag(error_handler, Module)` sets the module and returns the
    // old handler (`erts_proc_set_error_handler`, `erl_process.h:2432-2446`;
    // `process_flag_2`, `bif.c:2045-2051`). The handler module exports
    // `undefined_function/3`, which ERTS then runs in place of
    // `error_handler` (`call_error_handler`, `beam_common.c:1310-1316`).
    // The custom handler of the fixture continues the call of the default
    // handler and raises `undef` for a function that the loaded module does
    // not export (`error_handler.erl:85-98`).
    let program = program();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    let undef = Term::tuple(vec![
        atom("EXIT"),
        Term::tuple(vec![
            atom("undef"),
            Term::list_from(vec![Term::tuple(vec![
                atom("error_handler_target"),
                atom("missing"),
                Term::list_from(vec![atom("b")]),
                Term::nil(),
            ])]),
        ]),
    ]);
    assert_eq!(
        program
            .execute("error_handler_probe", "flag", &[])
            .unwrap()
            .term,
        Term::tuple(vec![
            atom("error_handler"),
            atom("error_handler_custom"),
            undef,
        ])
    );
    assert_eq!(
        program
            .execute("error_handler_probe", "flag_errors", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("error_handler"),
            badarg.clone(),
            badarg.clone(),
            badarg,
        ])
    );
    // The two runs load the same modules, so the whole result matches: the
    // custom handler raises `undef` through `erlang:raise/3` with the frame of
    // the failed call, and `erlang:raise/3` keeps the frames of its argument
    // (`bif.c:1626-1687`).
    assert_eq!(
        reference("flag"),
        "{error_handler,error_handler_custom,\
         {'EXIT',{undef,[{error_handler_target,missing,[b],[]}]}}}\n"
    );
}

#[test]
fn sets_the_error_handler_for_an_unloaded_module_as_pinned_erts() {
    // A custom handler answers a call of a module that the run does not load:
    // `error_handler_custom:undefined_function/3` reads the export of that
    // module and raises `undef`, so the call needs no code server and no init
    // process (`error_handler.erl:85-98`). The run of the library API has
    // neither, and the probe starts no init process, so only a custom handler
    // can answer this call. A process with the default handler keeps the
    // runtime error `unresolved external MFA` (row "Undefined functions" of
    // `docs/erts-differences.md`).
    let program = program();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let undef = Term::tuple(vec![
        atom("EXIT"),
        Term::tuple(vec![
            atom("undef"),
            Term::list_from(vec![Term::tuple(vec![
                atom("error_handler_unloaded"),
                atom("value"),
                Term::list_from(vec![atom("a")]),
                Term::nil(),
            ])]),
        ]),
    ]);
    assert_eq!(
        program
            .execute("error_handler_probe", "flag_unloaded", &[])
            .unwrap()
            .term,
        Term::tuple(vec![
            atom("error_handler"),
            atom("error_handler_custom"),
            undef,
        ])
    );
    assert_eq!(
        reference("flag_unloaded"),
        "{error_handler,error_handler_custom,\
         {'EXIT',{undef,[{error_handler_unloaded,value,[a],[]}]}}}\n"
    );
}

#[test]
fn charges_the_reductions_of_error_handler_beam_as_pinned_erts() {
    // The error handler runs the pinned `error_handler.beam`, so the
    // reductions of its code come from that module. The fixture measures
    // each call in a new process after a switch out, as
    // `preempt_probe:measure/3` does. The `plain` case raises `undef` through
    // `error_handler:crash/3` (`error_handler.erl:165-178`), and the
    // `handled` case sets the custom handler, which runs its own
    // `undefined_function/3` in place of `error_handler`
    // (`error_handler.erl:206-213`).
    let program = program();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let reductions = |name: &str| match name {
        "plain" => 25,
        "handled" => 8,
        other => panic!("unexpected case {other}"),
    };
    let expected = ["plain", "handled"]
        .into_iter()
        .map(|name| Term::tuple(vec![atom(name), Term::Integer(reductions(name).into())]))
        .collect::<Vec<_>>();
    assert_eq!(
        program
            .execute("error_handler_probe", "reductions", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    // The two runs load the same modules, so the `handled` case matches
    // exactly. The `plain` case costs 9 reductions more on PBeam: this run of
    // the library API has no code server, so
    // `error_handler:ensure_loaded/1` calls `init:ensure_loaded/1` and waits
    // for the reply of the init process of the fixture, where the ERTS run
    // calls `code:ensure_loaded/1` (`error_handler.erl:182-198`). The boot run
    // of `tests/feature/error_handler.rs` has the code server of ERTS and
    // takes the same path, and there the two runs give the same counts.
    assert_eq!(reference("reductions"), "[{plain,16},{handled,8}]\n");
}

/// Return the `~w` text of a term of atoms, integers, tuples, and lists.
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        _ if let Some(elements) = term.proper_list() => {
            format!("[{}]", join(&elements.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}
