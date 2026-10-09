use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const REGISTRY_PROBE: &[u8] = include_bytes!("../../fixtures/registry/registry_probe.beam");

#[test]
fn looks_up_registered_names_as_pinned_erts() {
    // `registry_probe:names/0` registers the root process and finds it with
    // `whereis/1`. A name that no process has and the atom `undefined` give
    // `undefined`. A term that is not an atom raises `error:badarg`, as
    // ERTS `whereis_1` (`bif.c:2342-2351`) does. A name that an exit freed
    // gives `undefined`.
    let program = BeamProgram::load(&[REGISTRY_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("registry_probe", "names", &[]).unwrap(),
        Term::List(vec![
            atom("named"),
            atom("undefined"),
            atom("undefined"),
            atom("badarg"),
            atom("badarg"),
            atom("undefined"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/registry",
            "-eval",
            "io:format(\"~w~n\", [registry_probe:names()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[named,undefined,undefined,badarg,badarg,undefined]\n"
    );
}

#[test]
fn reads_the_registered_name_of_another_process_as_pinned_erts() {
    // `registry_probe:info/0` reads `registered_name` of a named and of an
    // unnamed live process, as the `prim_tty` writer reads it of its parent
    // (`prim_tty.erl:569-571`). A process that is not alive gives
    // `undefined` for each valid item, also a list of items, and an item
    // that is not valid raises `error:badarg` first, as ERTS
    // `process_info_bif` (`erl_bif_info.c:1293-1386`) does.
    let program = BeamProgram::load(&[REGISTRY_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("registry_probe", "info", &[]).unwrap(),
        Term::List(vec![
            Term::Tuple(vec![atom("registered_name"), atom("registry_probe_info")]),
            Term::List(vec![]),
            atom("undefined"),
            atom("undefined"),
            atom("undefined"),
            atom("badarg"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/registry",
            "-eval",
            "io:format(\"~w~n\", [registry_probe:info()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[{registered_name,registry_probe_info},[],undefined,undefined,undefined,badarg]\n"
    );
}

#[test]
fn sends_to_registered_names_as_pinned_erts() {
    // `registry_probe:send/0` sends to its own name, to the name of an echo
    // process, and to `{Name, node()}`; the echo process sends back to a
    // pid. A name that no process has, a tuple that is not two atoms, and
    // each other term raise `error:badarg`. `{Name, node()}` without a
    // process, another node, and a reference drop the message and return
    // it, as ERTS `do_send` (`bif.c:2458-2695`) and `erl_send`
    // (`bif.c:2845-2908`) do. `empty` shows that no dropped message came
    // back.
    let program = BeamProgram::load(&[REGISTRY_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let errors = [
        "badarg", "badarg", "badarg", "badarg", "dropped", "dropped", "badarg", "badarg", "badarg",
        "dropped",
    ];
    assert_eq!(
        program.execute("registry_probe", "send", &[]).unwrap(),
        Term::List(vec![
            atom("to_self"),
            atom("received"),
            atom("to_echo"),
            atom("received"),
            atom("to_tuple"),
            atom("received"),
            Term::List(errors.into_iter().map(atom).collect()),
            atom("empty"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/registry",
            "-eval",
            "io:format(\"~w~n\", [registry_probe:send()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[to_self,received,to_echo,received,to_tuple,received,[badarg,badarg,badarg,badarg,dropped,dropped,badarg,badarg,badarg,dropped],empty]\n"
    );
}

#[test]
fn exits_with_badarg_for_an_uncaught_send_to_a_missing_name() {
    // `registry_probe:send_to/1` sends to a name that no process has, with
    // no `try` around the send. ERTS raises `error:badarg`.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/registry",
            "-eval",
            "try registry_probe:send_to(registry_probe_missing) \
             catch Class:Reason -> io:format(\"~w ~w~n\", [Class, Reason]) end, \
             halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"error badarg\n");

    // PBeam exits with the same reason but an empty stack trace. This is the
    // "Exception stack trace" row in `docs/erts-differences.md`.
    let program = BeamProgram::load(&[REGISTRY_PROBE]).unwrap();
    let missing = Term::Atom(program.atom("registry_probe_missing").unwrap());
    let error = program
        .execute("registry_probe", "send_to", &[missing])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("root process exited with {badarg,[]}"),
        "expected a badarg exit reason with an empty stack trace, got {error}"
    );
}
