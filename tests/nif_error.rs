use pbeam_runtime::BeamProgram;

const NIF_ERROR_PROBE: &[u8] = include_bytes!("../fixtures/nif_error/nif_error_probe.beam");

#[test]
fn raises_undef_for_a_local_function_that_calls_nif_error() {
    let program = BeamProgram::load(&[NIF_ERROR_PROBE]).unwrap();
    let error = program.execute("nif_error_probe", "main", &[]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("root process exited with Tuple([Atom(\"undef\"), List([])])"),
        "expected an undef exit reason, got {error}"
    );
}
