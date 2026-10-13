use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const GROUP_PROBE: &[u8] = include_bytes!("../../fixtures/group/group_probe.beam");

#[test]
fn takes_the_fail_labels_of_guard_bifs_and_is_ge_as_pinned_erts() {
    // `kind/1` runs `tuple_size/1` and `element/2` through `bif` instructions
    // with a fail label that is not 0, and `is_ge` on two integers. `atom`
    // makes `tuple_size/1` jump to the fail label, and `{x}` makes `is_ge`
    // jump.
    let program = BeamProgram::load(&[GROUP_PROBE]).unwrap();
    let output = Term::Atom(program.atom("output").unwrap());
    let other = Term::Atom(program.atom("other").unwrap());
    assert_eq!(
        program.execute("group_probe", "guards", &[]).unwrap().term,
        Term::list_from(vec![output, other.clone(), other])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/group",
            "-eval",
            "io:format(\"~p~n\", [group_probe:guards()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"[output,other,other]\n");
}

#[test]
fn registers_a_process_name_as_pinned_erts() {
    // `registry/0` registers `undefined`, a first name, a second name for the
    // same process, and a name that another process holds, then reads the
    // `registered_name` of the caller.
    let program = BeamProgram::load(&[GROUP_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("group_probe", "registry", &[])
            .unwrap()
            .term,
        Term::tuple(vec![
            atom("badarg"),
            atom("true"),
            atom("badarg"),
            atom("badarg"),
            Term::tuple(vec![atom("registered_name"), atom("pbeam_probe_reg")]),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/group",
            "-eval",
            "io:format(\"~p~n\", [group_probe:registry()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{badarg,true,badarg,badarg,{registered_name,pbeam_probe_reg}}\n"
    );
}

#[test]
fn executes_the_pinned_group_server_for_one_put_chars_request() {
    // `main/0` makes a group process the group leader and sends one
    // `put_chars` request through `io:put_chars/2`. The pinned `group` runs
    // in the `gen_statem` loop. It converts the chardata with
    // `unicode:characters_to_binary(Chars, utf8)` and sends
    // `{put_chars_sync, unicode, Binary, From}` to the fixture driver
    // (`group.erl` lines 619-621). `lists`, `unicode`,
    // `proc_lib`, and `logger` are not loaded: each `lists` and `unicode`
    // call on the path is a BIF, and nothing calls `proc_lib` or `logger`.
    // The group process waits in `receive` after `main/0` returns, and PBeam
    // still returns the result of the root process.
    let modules = [
        "group",
        "gen_statem",
        "gen",
        "io",
        "erlang",
        "erts_internal",
    ]
    .map(pinned_beam);
    let mut beams = vec![GROUP_PROBE];
    beams.extend(modules.iter().map(Vec::as_slice));
    let program = BeamProgram::load(&beams).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("group_probe", "main", &[]).unwrap().term,
        Term::tuple(vec![
            atom("ok"),
            Term::tuple(vec![
                atom("unicode"),
                Term::binary(b"Hello, world!\n".to_vec()),
            ]),
            atom("noise_ok"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/group",
            "-eval",
            "io:format(\"~p~n~p~n~p~n\", [group_probe:main(), group_probe:registry(), group_probe:guards()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{ok,{unicode,<<\"Hello, world!\\n\">>},noise_ok}\n\
          {badarg,true,badarg,badarg,{registered_name,pbeam_probe_reg}}\n\
          [output,other,other]\n"
    );
}
