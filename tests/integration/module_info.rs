use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, Term};

const MODULE_INFO_PROBE: &[u8] =
    include_bytes!("../../fixtures/module_info/module_info_probe.beam");
const MODULE_INFO_KEYS: &[u8] = include_bytes!("../../fixtures/module_info/module_info_keys.beam");

/// `module_info_keys:main()` on the pinned ERTS, as `io:format("~w")`
/// prints it.
const ERTS_KEYS: &str = "[[{zeta,0},{alpha,1},{stub,1},{local,1},{main,0},{module_info,0},{module_info,1},\
    {'-main/0-lc$^0/1-0-',1},{'-alpha/1-fun-0-',1}],[],\
    <<1,165,140,141,80,2,65,251,244,250,234,50,182,67,75,34>>]";

/// `module_info_probe:main()` on the pinned ERTS, as `io:format("~w")`
/// prints it.
const ERTS_RESULT: &str = "[[persistent_term,counters,atomics,\
    erts_dirty_process_signal_handler,erts_trace_cleaner,erts_literal_area_collector,\
    erl_tracer,erts_internal,erlang,erl_prim_loader,prim_zip,prim_net,prim_socket,\
    socket_registry,zlib,prim_file,prim_inet,prim_eval,prim_buffer,init,erl_init,\
    erts_code_purger],module_info_probe,[{error,badarg},{error,badarg}],\
    module_info_probe,{error,badarg},{error,badarg},{error,badarg},{error,badarg}]";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// binaries, lists, and tuples of the probe results. An atom that is not a
/// lowercase letter followed by letters, digits, and `_` gets quotes.
///
/// The atom names come from `atoms`, the table that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::nil()]), &atoms), "[[]]");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom) => {
            let name = atoms.name_of(atom);
            let plain = name.starts_with(|c: char| c.is_ascii_lowercase())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if plain {
                name.to_owned()
            } else {
                format!("'{name}'")
            }
        }
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        Term::Integer(value) => value.to_string(),
        Term::Binary(bytes) => format!(
            "<<{}>>",
            bytes
                .bytes()
                .unwrap()
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
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
    // 5986-6030). `erlang:get_module_info/1` raises `badarg` for a module
    // that is not an atom or not loaded (erl_bif_info.c:5946-5984,
    // 6010-6019). `code:do_start/0` calls `M:module_info(module)` for each
    // module that `code_server` needs (code.erl:1699-1712).
    let program = BeamProgram::load(&[MODULE_INFO_PROBE]).unwrap();
    let result = program.execute("module_info_probe", "main", &[]).unwrap();
    assert_eq!(text(&result.term, result.atoms()), ERTS_RESULT);

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
fn gives_functions_nifs_and_md5_as_pinned_erts() {
    // `functions_in_module` gives the
    // functions in code order (erl_bif_info.c:5732-5767). A module without
    // a NIF library has no NIFs (erl_bif_info.c:5769-5805). `md5_of_module`
    // gives the MD5 of beam_file.c:1751-1830 (erl_bif_info.c:5911-5916).
    let program = BeamProgram::load(&[MODULE_INFO_KEYS]).unwrap();
    let result = program.execute("module_info_keys", "main", &[]).unwrap();
    assert_eq!(text(&result.term, result.atoms()), ERTS_KEYS);

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/module_info",
            "-eval",
            "io:format(\"~w~n\", [module_info_keys:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{ERTS_KEYS}\n")
    );
}

#[test]
fn stops_get_module_info_for_a_key_that_pbeam_does_not_implement() {
    // ERTS returns the attributes of the `Attr` chunk
    // (erl_bif_info.c:5864-5885). PBeam does not keep the chunk (row
    // "Module information" of docs/erts-differences.md).
    let program = BeamProgram::load(&[MODULE_INFO_PROBE]).unwrap();
    let failure = program
        .execute("module_info_probe", "attributes", &[])
        .unwrap_err()
        .to_string();
    assert!(
        failure.contains("erlang:get_module_info/2 key attributes is not implemented"),
        "{failure}"
    );
}

#[test]
fn stops_get_module_info_1_for_a_loaded_module() {
    // ERTS returns the pair list of the keys `module`, `exports`,
    // `attributes`, `compile`, and `md5` (erl_bif_info.c:5946-5984). PBeam
    // cannot give the key `exports`, so the call of a loaded module stops
    // the run at that key (row "Module information" of
    // docs/erts-differences.md).
    let program = BeamProgram::load(&[MODULE_INFO_PROBE]).unwrap();
    let failure = program
        .execute("module_info_probe", "info", &[])
        .unwrap_err()
        .to_string();
    assert!(
        failure.contains("erlang:get_module_info/1 key exports is not implemented"),
        "{failure}"
    );
}
