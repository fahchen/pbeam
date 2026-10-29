//! Runs `module_info` of preloaded and loaded modules on PBeam and compares the results with the
//! pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, Term};

const MODULE_INFO_PROBE: &[u8] =
    include_bytes!("../../fixtures/module_info/module_info_probe.beam");
const MODULE_INFO_KEYS: &[u8] = include_bytes!("../../fixtures/module_info/module_info_keys.beam");
const MODULE_INFO_CHUNKS: &[u8] =
    include_bytes!("../../fixtures/module_info/module_info_chunks.beam");

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
        Term::BigInt(value) => value.to_string(),
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

/// Return `beam` with each chunk that `drop` names removed, and with the
/// size of the `FOR1` form updated. Each chunk has a 4-byte name, a 4-byte
/// size, and data padded to 4 bytes.
///
/// # Examples
///
/// ```rust,ignore
/// let stripped = without_chunks(beam, &[b"Attr"]);
/// ```
fn without_chunks(beam: &[u8], drop: &[&[u8; 4]]) -> Vec<u8> {
    let mut body = b"BEAM".to_vec();
    let mut pos = 12;
    while pos < beam.len() {
        let size = usize::try_from(u32::from_be_bytes(
            beam[pos + 4..pos + 8].try_into().unwrap(),
        ))
        .unwrap();
        let end = pos + 8 + size.next_multiple_of(4);
        if !drop.iter().any(|name| beam[pos..pos + 4] == name[..]) {
            body.extend_from_slice(&beam[pos..end]);
        }
        pos = end;
    }
    let mut out = b"FOR1".to_vec();
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
    out.extend(body);
    out
}

/// Return `beam` with the chunk `CInf` replaced by `data`, which is 4 bytes
/// long.
fn with_compile_info(beam: &[u8], data: [u8; 4]) -> Vec<u8> {
    let mut out = without_chunks(beam, &[b"CInf"]);
    out.extend_from_slice(b"CInf\0\0\0\x04");
    out.extend_from_slice(&data);
    let size = u32::try_from(out.len() - 8).unwrap();
    out[4..8].copy_from_slice(&size.to_be_bytes());
    out
}

/// Return the output of `module_info_chunks:main()` on the pinned ERTS, as
/// `io:format("~w")` prints it, after `prepare` loads the module.
fn erts_chunks(prepare: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/module_info",
            "-eval",
            &format!("{prepare} io:format(\"~w~n\", [module_info_chunks:main()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

#[test]
fn gives_attributes_compile_native_addresses_and_native_as_pinned_erts() {
    // `attributes_for_module` and `compilation_info_for_module` decode the
    // `Attr` and `CInf` chunks, `native_addresses` gives `[]`, and
    // `has_native` gives `false` (erl_bif_info.c:5807-5909). The `CInf`
    // term holds the source path of the host that compiled the fixture, so
    // the ERTS output is the reference for that part.
    let program = BeamProgram::load(&[MODULE_INFO_CHUNKS]).unwrap();
    let result = program.execute("module_info_chunks", "main", &[]).unwrap();
    let text = text(&result.term, result.atoms());
    assert!(
        text.starts_with(
            "[[{vsn,[1]},{probe_attribute,[{key,<<118,97,108,117,101>>},-3]}],\
             [{version,"
        ),
        "{text}"
    );
    assert!(text.ends_with("]}],[],false]"), "{text}");
    assert_eq!(erts_chunks(""), format!("{text}\n"));
}

#[test]
fn gives_empty_attributes_and_compile_for_a_module_without_the_chunks() {
    // A module without the `Attr` or `CInf` chunk has no attribute or
    // compile pointer, so both keys give `[]` (erl_bif_info.c:5871-5872,
    // 5894-5895, jit/asm_load.c:1080, 1096). `beam_lib:strip/1` removes
    // both chunks for the ERTS reference.
    let stripped = without_chunks(MODULE_INFO_CHUNKS, &[b"Attr", b"CInf"]);
    let program = BeamProgram::load(&[&stripped]).unwrap();
    let result = program.execute("module_info_chunks", "main", &[]).unwrap();
    assert_eq!(text(&result.term, result.atoms()), "[[],[],[],false]");
    assert_eq!(
        erts_chunks(
            "{ok, F} = file:read_file(\"fixtures/module_info/module_info_chunks.beam\"), {ok, {M, B}} = beam_lib:strip(F), \
             {module, M} = code:load_binary(M, \"stripped\", B),"
        ),
        "[[],[],[],false]\n"
    );
}

#[test]
fn refuses_a_compile_chunk_that_is_not_an_external_term() {
    // The ERTS loader refuses the module with `badfile` when the `CInf`
    // chunk is not one valid external term (jit/asm_load.c:1096-1111): a
    // first byte that is not 131, or a term that ends early.
    let failure = BeamProgram::load(&[&with_compile_info(MODULE_INFO_CHUNKS, *b"\0\0\0\0")])
        .unwrap_err()
        .to_string();
    assert!(
        failure.contains("the BEAM compile information chunk is not an external term"),
        "{failure}"
    );
    assert!(
        BeamProgram::load(&[&with_compile_info(MODULE_INFO_CHUNKS, *b"\x83\x68\x02\x6a")]).is_err()
    );
}

#[test]
fn gives_the_exports_and_the_module_info_list_as_pinned_erts() {
    // `exported_from_module` prepends each export of the module in the
    // order of the export table (erl_bif_info.c:5822-5862), and the loader
    // puts the exports there in `ExpT` order (jit/asm_load.c:1180-1186).
    // `module_info_0` builds the pairs from `md5` to `module`
    // (erl_bif_info.c:5946-5984). The ERTS run loads the probe before the
    // calls, so no apply makes an export entry of it first.
    for function in ["exports", "info"] {
        let program = BeamProgram::load(&[MODULE_INFO_PROBE]).unwrap();
        let result = program.execute("module_info_probe", function, &[]).unwrap();
        let reference = Command::new("erl")
            .args([
                "-noshell",
                "-pa",
                "fixtures/module_info",
                "-eval",
                &format!(
                    "{{module, _}} = code:ensure_loaded(module_info_probe), \
                     io:format(\"~w~n\", [module_info_probe:{function}()]), halt()."
                ),
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success() && reference.stderr.is_empty(),
            "{reference:?}"
        );
        assert_eq!(
            format!("{}\n", text(&result.term, result.atoms())),
            String::from_utf8(reference.stdout).unwrap(),
            "{function}"
        );
    }
}
