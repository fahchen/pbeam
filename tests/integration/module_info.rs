use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const MODULE_INFO_PROBE: &[u8] =
    include_bytes!("../../fixtures/module_info/module_info_probe.beam");

/// `module_info_probe:main()` on the pinned ERTS, as `io:format("~w")`
/// prints it.
const ERTS_RESULT: &str = "[[persistent_term,counters,atomics,\
    erts_dirty_process_signal_handler,erts_trace_cleaner,erts_literal_area_collector,\
    erl_tracer,erts_internal,erlang,erl_prim_loader,prim_zip,prim_net,prim_socket,\
    socket_registry,zlib,prim_file,prim_inet,prim_eval,prim_buffer,init,erl_init,\
    erts_code_purger],module_info_probe,module_info_probe,{error,badarg},{error,badarg},\
    {error,badarg},{error,badarg}]";

/// Return a term as `io:format("~w")` prints it, for the atoms, lists, and
/// tuples of the probe result.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::nil()])), "[[]]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn gives_preloaded_modules_and_module_info_as_pinned_erts() {
    // `erlang:pre_loaded/0` gives the preloaded modules of the emulator,
    // with the last module of `PRELOAD_BEAM` first (erl_init.c:459-484,
    // erts/emulator/Makefile.in:804-828). `module_info/1` calls
    // `erlang:get_module_info/2`, which returns the module for the key
    // `module`, and raises `badarg` for a module that is not an atom or not
    // loaded, and for a term that is not a key (erl_bif_info.c:5918-5945,
    // 5986-6030). `code:do_start/0` calls `M:module_info(module)` for each
    // module that `code_server` needs (code.erl:1699-1712).
    let program = BeamProgram::load(&[MODULE_INFO_PROBE]).unwrap();
    let result = program.execute("module_info_probe", "main", &[]).unwrap();
    assert_eq!(text(&result), ERTS_RESULT);

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/module_info",
            "-eval",
            "io:format(\"~w~n\", [module_info_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{ERTS_RESULT}\n")
    );
}

#[test]
fn stops_get_module_info_for_a_key_that_pbeam_does_not_implement() {
    // ERTS returns the MD5 of the code chunks (erl_bif_info.c:5912-5917).
    // PBeam implements only the key `module` (row "Module information" of
    // docs/erts-differences.md).
    let program = BeamProgram::load(&[MODULE_INFO_PROBE]).unwrap();
    let failure = program
        .execute("module_info_probe", "md5", &[])
        .unwrap_err()
        .to_string();
    assert!(
        failure.contains("erlang:get_module_info/2 key md5 is not implemented"),
        "{failure}"
    );
}
