use std::process::Command;

use pbeam_runtime::BeamProgram;

const NIF_ERROR_PROBE: &[u8] = include_bytes!("../fixtures/nif_error/nif_error_probe.beam");

#[test]
fn raises_undef_for_a_local_function_that_calls_nif_error() {
    // ERTS raises `error:undef`. The stack trace starts with the
    // `erlang:nif_error/1` frame and the local `stub/0` frame. Frames below
    // these two depend on the caller, so the check compares only these two.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/nif_error",
            "-eval",
            "try nif_error_probe:main() catch Class:Reason:Stack -> \
             io:format(\"~p ~p ~p~n\", [Class, Reason, \
             lists:sublist([{M, F, A} || {M, F, A, _} <- Stack], 2)]) end, \
             halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"error undef [{erlang,nif_error,[undef]},{nif_error_probe,stub,0}]\n"
    );

    // PBeam exits with the same reason but an empty stack trace. This is the
    // "Exception stack trace" row in `docs/erts-differences.md`.
    let program = BeamProgram::load(&[NIF_ERROR_PROBE]).unwrap();
    let error = program.execute("nif_error_probe", "main", &[]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("root process exited with Tuple([Atom(\"undef\"), List([])])"),
        "expected an undef exit reason with an empty stack trace, got {error}"
    );
}
