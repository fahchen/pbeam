use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const IO_FAIL_BIF_PROBE: &[u8] = include_bytes!("../fixtures/io_fail_bif/io_fail_bif_probe.beam");

/// Run `io_fail_bif_probe:Function()` on the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/io_fail_bif",
            "-eval",
            &format!("io:format(\"~w~n\", [io_fail_bif_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn checks_unicode_io_of_a_pid_as_pinned_erts() {
    // `io_fail_bif_probe:unicode_io/0` runs `net_kernel:dflag_unicode_io/1`,
    // as `io:bc_req/3` does for each `io:format/3` call to a pid. A pid of
    // the local node gives `true`, also after the process ends, and a term
    // that is not a pid raises `badarg` (`net_kernel_dflag_unicode_io_1`,
    // `dist.c:6670-6686`). PBeam has only local pids.
    let program = BeamProgram::load(&[IO_FAIL_BIF_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::Tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program
            .execute("io_fail_bif_probe", "unicode_io", &[])
            .unwrap(),
        Term::List(vec![atom("true"), atom("true"), badarg.clone(), badarg])
    );
    assert_eq!(
        reference("unicode_io"),
        b"[true,true,{error,badarg},{error,badarg}]\n"
    );
}
