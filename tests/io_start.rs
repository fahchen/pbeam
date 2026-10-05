use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

mod common;

use common::pinned_beam;

const IO_START_PROBE: &[u8] = include_bytes!("../fixtures/io_start/io_start_probe.beam");

/// Run `io_start_probe:Function()` on the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/io_start",
            "-eval",
            &format!("io:format(\"~w~n\", [io_start_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn loads_the_pinned_io_lib_modules() {
    // `user_drv:init_shell/2` (user_drv.erl:374) calls `io_lib:bformat/2`,
    // which runs `io_lib_format`, `lists`, and `unicode`.
    // `prim_tty:init_term/1` (prim_tty.erl:284) calls `os:type/0`. The pinned
    // `io_lib` has `bs_get_integer2/7` (opcode 117), `gc_bif3/7` (opcode 152),
    // and `is_any_native_record/2` (opcode 186), which the start path does
    // not run.
    let modules = ["io_lib", "io_lib_format", "lists", "unicode", "os"].map(pinned_beam);
    let beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    BeamProgram::load(&beams).unwrap();
}

#[test]
fn takes_the_tail_and_the_length_of_lists_as_pinned_erts() {
    // `io_start_probe:lists/0` runs `bif` `erlang:tl/1` with fail label 0,
    // as `group:init/1` (group.erl:136) does, `get_tl` and `gc_bif`
    // `erlang:length/1` with fail label 0, as `proc_lib:trans_init/3`
    // (proc_lib.erl:939-944) does, and both BIFs in a guard. A term that is
    // not a list, and the tail of `[]`, raise `badarg` (`bif.c:2931-2938`,
    // `erl_bif_guard.c:210-224`). In a guard, `badarg` jumps to the fail
    // label, and the next clause runs.
    let program = BeamProgram::load(&[IO_START_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let list = |names: &[&str]| Term::List(names.iter().map(|name| atom(name)).collect());
    let expected = vec![
        list(&["b", "c"]),
        list(&[]),
        atom("badarg"),
        atom("badarg"),
        Term::Integer(0),
        Term::Integer(3),
        atom("badarg"),
        atom("badarg"),
        list(&["y", "z"]),
        atom("two"),
        atom("other"),
        atom("other"),
        atom("long"),
        atom("other"),
    ];
    assert_eq!(
        program.execute("io_start_probe", "lists", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("lists"),
        b"[[b,c],[],badarg,badarg,0,3,badarg,badarg,[y,z],two,other,other,long,other]\n"
    );
}
