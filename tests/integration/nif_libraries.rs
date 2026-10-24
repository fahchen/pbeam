use pbeam_runtime::{BeamProgram, Term};

/// The stand-ins of the modules of the OTP NIF libraries without bindings.
/// Each defines the functions of the `nif_funcs` table of its library.
const STAND_INS: [&[u8]; 4] = [
    include_bytes!("../../fixtures/nif_libraries/zstd.beam"),
    include_bytes!("../../fixtures/nif_libraries/prim_net.beam"),
    include_bytes!("../../fixtures/nif_libraries/erl_tracer.beam"),
    include_bytes!("../../fixtures/nif_libraries/prim_socket.beam"),
];

#[test]
fn stops_the_run_at_a_function_of_a_library_without_bindings() {
    // PBeam has no binding of these functions, so a call of a function of
    // a table stops the run with the runtime error of the static library
    // (`Library::unbound_fails`).
    let program = BeamProgram::load(&STAND_INS).unwrap();
    for (module, function, arity, expected) in [
        (
            "zstd",
            "get_frame_header_nif",
            1,
            "zstd:get_frame_header_nif/1",
        ),
        ("prim_net", "nif_if_names", 0, "prim_net:nif_if_names/0"),
        ("erl_tracer", "enabled", 3, "erl_tracer:enabled/3"),
        ("prim_socket", "nif_listen", 2, "prim_socket:nif_listen/2"),
    ] {
        let arguments = vec![program.host_term(Term::Integer(0)); arity];
        let error = program
            .execute(module, function, &arguments)
            .unwrap_err()
            .to_string();
        assert_eq!(error, format!("{expected} NIF is not supported"));
    }
}
