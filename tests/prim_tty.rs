use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

mod common;

use common::pinned_beam;

const PRIM_TTY_PROBE: &[u8] = include_bytes!("../fixtures/prim_tty/prim_tty_probe.beam");

#[test]
fn raises_badarg_from_the_write_nif_binding_for_a_term_that_is_not_a_tty() {
    // `main/0` starts three pinned `prim_tty:writer_loop/2` writers and
    // returns their exit reasons. The first gets the sync `{write, From,
    // Chars}` message (prim_tty.erl:560), the second gets the async
    // `{write, Chars}` message (prim_tty.erl:554), and the third gets the
    // atom `stdout` as its TTY and `not_iodata` as its iovec. Each writer
    // calls the local `write_nif/2`. The ERTS NIF raises `badarg` because
    // its first argument is not a TTY resource (prim_tty_nif.c:363-364),
    // before it reads the iovec. The logger level `none` stops the
    // asynchronous "Error in process" reports; it does not change the
    // result.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/prim_tty",
            "-eval",
            "ok = logger:set_primary_config(level, none), \
             io:format(\"~w~n\", [[{R, M, F} || {R, [{M, F, _, _} | _]} <- prim_tty_probe:main()]]), \
             halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[{badarg,prim_tty,write_nif},{badarg,prim_tty,write_nif},{badarg,prim_tty,write_nif}]\n"
    );

    // The `nif_start` instruction of `write_nif/2` runs the PBeam binding,
    // which raises `badarg` for each term. Without the binding, the stub
    // body gives `{undef, []}`. PBeam gives the stack trace `[]`; this is
    // the "Exception stack trace" row in `docs/erts-differences.md`.
    let modules = ["erlang", "prim_tty"].map(pinned_beam);
    let mut beams = vec![PRIM_TTY_PROBE];
    beams.extend(modules.iter().map(Vec::as_slice));
    let program = BeamProgram::load(&beams).unwrap();
    let badarg = Term::Tuple(vec![
        Term::Atom(program.atom("badarg").unwrap()),
        Term::List(vec![]),
    ]);
    assert_eq!(
        program.execute("prim_tty_probe", "main", &[]).unwrap(),
        Term::List(vec![badarg.clone(), badarg.clone(), badarg])
    );
}
