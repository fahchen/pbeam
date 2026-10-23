//! NIF shared libraries: `erlang:load_nif/2` of a module without a static
//! or an extension library opens `Path.so` with the PBeam NIF ABI loader.
//!
//! The test compiles `fixtures/nif/nif_add.c` with the vendored
//! `vendor/erts/include/erl_nif.h` of OTP 29.1.1 at test time, one time for
//! each variant, so the library matches the host. The expected values are
//! the values of the pinned ERTS with the same libraries, built on macOS
//! with `cc -bundle -undefined dynamic_lookup -Ivendor/erts/include
//! [-DNIF_ADD_...] -o /tmp/pbeam865/V.so fixtures/nif/nif_add.c`:
//!
//! ```text
//! mise run erts-eval --pa $PWD/fixtures/nif 'nif_add:calls("/tmp/pbeam865/base")'
//! {{error,undef},ok,3,-3,{error,badarg},{error,badarg},{error,undef},
//!  {error,{reload,"NIF library already loaded (reload disallowed since OTP 20)."}}}
//! mise run erts-eval --pa $PWD/fixtures/nif 'nif_add:trace("/tmp/pbeam865/base")'
//! {badarg,{nif_add,add,[1,a],[]}}
//! mise run erts-eval --pa $PWD/fixtures/nif 'nif_add:vetoed("/tmp/pbeam865/load")'
//! {{error,{load,"Library load-call unsuccessful (7)."}},ok,3}
//! mise run erts-eval --pa $PWD/fixtures/nif 'nif_add:load("/tmp/pbeam865/newer", 0)'
//! {error,{bad_lib,"That 'nif_add' NIF library needs erts-14.0 or newer. Either try to recompile the NIF lib or use a newer erts runtime."}}
//! ```
//!
//! The other `nif_add:load/2` results of the ERTS run are in
//! [`gives_the_erts_error_of_each_bad_entry`]. ERTS loads a 2.17 library,
//! an upgrade callback, and a module without `-nifs`, and
//! PBeam does not support them yet.

#[path = "../common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamError, BeamProgram, HostTerm, Term};

const NIF_ADD: &[u8] = include_bytes!("../../fixtures/nif/nif_add.beam");
const NIF_BARE: &[u8] = include_bytes!("../../fixtures/nif/nif_bare.beam");

/// Return the directory of the compiled libraries of the test binary.
fn directory() -> PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nif");
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// Run `cc` with `arguments` and panic with its output when it fails.
fn cc(arguments: &[&str]) {
    let output = Command::new("cc").args(arguments).output().unwrap();
    assert!(
        output.status.success(),
        "cc {arguments:?}: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Return the arguments of `cc` that build a shared library on the host.
fn shared() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["-bundle", "-undefined", "dynamic_lookup"]
    } else {
        &["-shared", "-fPIC"]
    }
}

/// Compile the variant `name` of `nif_add.c` with `defines` and the extra
/// `cc` arguments, and return its path without `.so`, the path that
/// `erlang:load_nif/2` gets. Each test builds the variants it loads under
/// its own names, so two tests do not write one file.
fn build(name: &str, defines: &[&str], extra: &[&str]) -> String {
    let path = directory().join(name);
    let output = format!("{}.so", path.display());
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/nif/nif_add.c");
    let include = concat!("-I", env!("CARGO_MANIFEST_DIR"), "/vendor/erts/include");
    let mut arguments = shared().to_vec();
    arguments.extend([include, "-o", &output, source]);
    arguments.extend(defines);
    arguments.extend(extra);
    cc(&arguments);
    path.display().to_string()
}

/// Return the program of the fixtures.
fn program() -> BeamProgram {
    BeamProgram::load(&[
        &pinned_beam("erlang"),
        &pinned_beam("erts_internal"),
        NIF_ADD,
        NIF_BARE,
    ])
    .unwrap()
}

/// Return `text` as a list of its bytes.
fn string(text: &str) -> Term {
    Term::list_from(text.bytes().map(|byte| Term::Integer(byte.into())))
}

/// Run `Module:Function(Arguments)` and return its result.
fn execute(
    program: &BeamProgram,
    module: &str,
    function: &str,
    arguments: Vec<Term>,
) -> Result<HostTerm, BeamError> {
    let arguments: Vec<_> = arguments
        .into_iter()
        .map(|term| program.host_term(term))
        .collect();
    program.execute(module, function, &arguments)
}

/// Return the result of `nif_add:load(Path, 0)`.
fn load(program: &BeamProgram, path: &str) -> Result<HostTerm, BeamError> {
    execute(
        program,
        "nif_add",
        "load",
        vec![string(path), Term::Integer(0)],
    )
}

/// Return the atom `name` of the table of `result`.
fn atom(result: &HostTerm, name: &str) -> Term {
    Term::Atom(result.atom(name).unwrap())
}

/// Return `{error, {Kind, Text}}` with the atoms of `result`.
fn load_error(result: &HostTerm, kind: &str, text: &str) -> Term {
    Term::tuple(vec![
        atom(result, "error"),
        Term::tuple(vec![atom(result, kind), string(text)]),
    ])
}

/// Return `{error, Reason}` with the atoms of `result`.
fn caught(result: &HostTerm, reason: &str) -> Term {
    Term::tuple(vec![atom(result, "error"), atom(result, reason)])
}

/// Return the text of the `{error, {Kind, Text}}` result of `result`, and
/// check `Kind`.
fn error_text(result: &HostTerm, kind: &str) -> String {
    let Term::Tuple(outer) = &result.term else {
        panic!("not an error tuple: {:?}", result.term);
    };
    let Term::Tuple(inner) = &outer[1] else {
        panic!("not an error tuple: {:?}", result.term);
    };
    assert_eq!(inner[0], atom(result, kind), "{:?}", result.term);
    let bytes: Vec<u8> = inner[1]
        .proper_list()
        .unwrap()
        .map(|byte| match byte {
            Term::Integer(byte) => u8::try_from(*byte).unwrap(),
            other => panic!("not a byte: {other:?}"),
        })
        .collect();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn runs_the_functions_of_the_library_only_after_its_load_as_erts() {
    let program = program();
    let path = build("calls", &[], &[]);
    let result = execute(&program, "nif_add", "calls", vec![string(&path)]).unwrap();
    let badarg = caught(&result, "badarg");
    let undef = caught(&result, "undef");
    let reload = "NIF library already loaded (reload disallowed since OTP 20).";
    assert_eq!(
        result.term,
        Term::tuple(vec![
            undef.clone(),
            atom(&result, "ok"),
            Term::Integer(3),
            Term::Integer(-3),
            badarg.clone(),
            badarg,
            undef,
            load_error(&result, "reload", reload),
        ])
    );
}

#[test]
fn gives_the_functions_of_the_library_as_the_nifs_of_the_module_as_erts() {
    // `nifs_in_module` gives `[]` for a module without a NIF library, and
    // the functions of its library in library order after the load
    // (erl_bif_info.c:5769-5805).
    let program = program();
    let path = build("nifs", &[], &[]);
    let result = execute(&program, "nif_add", "nifs", vec![string(&path)]).unwrap();
    let add = Term::tuple(vec![atom(&result, "add"), Term::Integer(2)]);
    assert_eq!(
        result.term,
        Term::tuple(vec![
            Term::nil(),
            atom(&result, "ok"),
            Term::list_from(vec![add])
        ])
    );
}

#[test]
fn raises_badarg_with_the_frame_of_the_nif_as_erts() {
    let program = program();
    let path = build("trace", &[], &[]);
    let binary = Term::binary(path.into_bytes());
    let result = execute(&program, "nif_add", "trace", vec![binary]).unwrap();
    let arguments = Term::list_from(vec![Term::Integer(1), atom(&result, "a")]);
    let frame = Term::tuple(vec![
        atom(&result, "nif_add"),
        atom(&result, "add"),
        arguments,
        Term::nil(),
    ]);
    assert_eq!(
        result.term,
        Term::tuple(vec![atom(&result, "badarg"), frame])
    );
}

#[test]
fn raises_badarg_for_a_term_returned_after_enif_make_badarg_as_erts() {
    // mise run erts-eval --pa $PWD/fixtures/nif 'nif_add:trace("/tmp/pbeam865/late")'
    // {badarg,{nif_add,add,[1,a],[]}}
    let program = program();
    let path = build("late_badarg", &["-DNIF_ADD_LATE_BADARG"], &[]);
    let result = execute(&program, "nif_add", "trace", vec![string(&path)]).unwrap();
    let arguments = Term::list_from(vec![Term::Integer(1), atom(&result, "a")]);
    let frame = Term::tuple(vec![
        atom(&result, "nif_add"),
        atom(&result, "add"),
        arguments,
        Term::nil(),
    ]);
    assert_eq!(
        result.term,
        Term::tuple(vec![atom(&result, "badarg"), frame])
    );
}

#[test]
fn gives_the_veto_of_the_load_callback_and_closes_the_library_as_erts() {
    let program = program();
    let path = build("vetoed", &["-DNIF_ADD_LOAD"], &[]);
    let result = execute(&program, "nif_add", "vetoed", vec![string(&path)]).unwrap();
    let veto = load_error(&result, "load", "Library load-call unsuccessful (7).");
    assert_eq!(
        result.term,
        Term::tuple(vec![veto, atom(&result, "ok"), Term::Integer(3)])
    );
}

#[test]
fn gives_the_erts_error_of_each_bad_entry() {
    let program = program();
    let cases = [
        (
            "newer",
            "-DNIF_ADD_MINOR=19",
            "That 'nif_add' NIF library needs erts-14.0 or newer. Either try to recompile the NIF lib or use a newer erts runtime.",
        ),
        (
            "major1",
            "-DNIF_ADD_MAJOR=1",
            "That old NIF library (1.18) is not compatible with this erts runtime (2.18). Try recompile the NIF lib.",
        ),
        (
            "minor5",
            "-DNIF_ADD_MINOR=5",
            "That old NIF library (2.5) is not compatible with this erts runtime (2.18). Try recompile the NIF lib.",
        ),
        // ERTS loads a 2.17 library; PBeam supports only 2.18.
        (
            "minor17",
            "-DNIF_ADD_MINOR=17",
            "That NIF library (2.17) is not supported by PBeam, which supports only NIF version 2.18. Try recompile the NIF lib.",
        ),
        (
            "variant",
            "-DNIF_ADD_VM_VARIANT=\"beam.other\"",
            "Library (beam.other) not compiled for this vm variant (beam.vanilla).",
        ),
        (
            "name",
            "-DNIF_ADD_NAME=nif_other",
            "Library module name 'nif_other' does not match calling module 'nif_add'",
        ),
        (
            "function",
            "-DNIF_ADD_FUNCTION",
            "Function not found nif_add:sub/2",
        ),
        (
            "flags",
            "-DNIF_ADD_FLAGS=3",
            "Illegal flags field value 3 for NIF nif_add:add/2",
        ),
        (
            "null_entry",
            "-DNIF_ADD_NULL_ENTRY",
            "Library init-call unsuccessful",
        ),
    ];
    for (name, define, text) in cases {
        let path = build(name, &[define], &[]);
        let result = load(&program, &path).unwrap();
        assert_eq!(result.term, load_error(&result, "bad_lib", text), "{name}");
    }
}

#[test]
fn gives_the_dlerror_text_of_a_failed_open_as_erts() {
    let program = program();
    let missing = directory().join("missing").display().to_string();
    let result = load(&program, &missing).unwrap();
    let text = error_text(&result, "load_failed");
    assert!(
        text.starts_with("Failed to load NIF library: '") && text.contains(&missing),
        "{text}"
    );

    let path = build("no_init", &["-DNIF_ADD_NO_INIT"], &[]);
    let result = load(&program, &path).unwrap();
    let text = error_text(&result, "bad_lib");
    assert!(
        text.starts_with("Failed to find library init function: '") && text.contains("_nif_init"),
        "{text}"
    );
}

#[test]
fn names_the_library_when_the_dlerror_text_does_not_as_erts() {
    let program = program();
    let dependency = directory().join("libnif_add_dep.so");
    let dependency = dependency.display().to_string();
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/nif/nif_add_dep.c");
    let library = if cfg!(target_os = "macos") {
        "-dynamiclib"
    } else {
        "-shared"
    };
    cc(&[library, "-fPIC", "-o", &dependency, source]);
    let search = format!("-L{}", directory().display());
    let path = build(
        "dependency",
        &["-DNIF_ADD_DEP"],
        &[&search, "-lnif_add_dep"],
    );
    std::fs::remove_file(&dependency).unwrap();
    let result = load(&program, &path).unwrap();
    let text = error_text(&result, "load_failed");
    // The glibc text names only the missing dependency; the macOS text
    // names the library too.
    let expected = if cfg!(target_os = "macos") {
        "Failed to load NIF library: 'dlopen(".to_owned()
    } else {
        format!("Failed to load NIF library {path}: 'libnif_add_dep.so: cannot open")
    };
    assert!(text.starts_with(&expected), "{text}");
}

#[test]
fn gives_the_upgrade_error_of_a_library_without_an_upgrade_callback_as_erts() {
    let program = program();
    let binary = Term::binary(NIF_ADD.to_vec());
    let path = build("upgrade_none", &[], &[]);
    let result = execute(
        &program,
        "nif_add",
        "reload",
        vec![string(&path), binary.clone()],
    )
    .unwrap();
    let text = "Upgrade not supported by this NIF library.";
    assert_eq!(result.term, load_error(&result, "upgrade", text));

    // ERTS runs the upgrade callback and gives `ok`.
    let path = build("upgrade", &["-DNIF_ADD_UPGRADE"], &[]);
    let failure = execute(&program, "nif_add", "reload", vec![string(&path), binary]).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "unsupported: the upgrade callback of the NIF library of nif_add"
    );
}

#[test]
fn stops_the_run_for_a_part_that_pbeam_does_not_support() {
    let program = program();
    // ERTS loads the library and gives `{ok, 3}`.
    let path = build("bare", &["-DNIF_ADD_NAME=nif_bare"], &[]);
    let failure = execute(&program, "nif_bare", "calls", vec![string(&path)]).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "unsupported: erlang:load_nif/2 of the NIF library of nif_bare: the module has no nif_start instruction"
    );
}

#[test]
fn calls_a_library_that_another_scheduler_loaded() {
    // The node keeps the library of the load, so a call on the other
    // scheduler runs its function (`beam/erl_nif.c:4990-4997`). The fixture
    // names the path relative to the directory of the crate.
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/nif_cross");
    std::fs::create_dir_all(&directory).unwrap();
    let output = format!("{}/nif_add.so", directory.display());
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/nif/nif_add.c");
    let include = concat!("-I", env!("CARGO_MANIFEST_DIR"), "/vendor/erts/include");
    let mut arguments = shared().to_vec();
    arguments.extend([include, "-o", &output, source]);
    cc(&arguments);
    let program = program();
    let ok = Term::Atom(program.atom("ok").unwrap());
    for _ in 0..10 {
        let results = program
            .execute_on_schedulers(&[("nif_add", "cross_load"), ("nif_add", "cross_call")])
            .unwrap();
        assert_eq!(results[0].term, ok);
        assert_eq!(results[1].term, Term::Integer(3));
    }
}

#[test]
fn stops_the_run_for_a_result_that_no_enif_call_made() {
    let program = program();
    let path = build("bad_result", &["-DNIF_ADD_BAD_RESULT"], &[]);
    let failure = execute(&program, "nif_add", "trace", vec![string(&path)]).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "the NIF returned the term 0x3039, which no enif call of the call made"
    );
}

/// Return the results of `nif_add:probes(Path)`, with `None` for each
/// exception `{error, badarg, {nif_add, add, [Kind, Arg], []}}`.
fn probes(program: &BeamProgram, path: &str) -> Vec<Option<i64>> {
    let result = execute(program, "nif_add", "probes", vec![string(path)]).unwrap();
    let error = atom(&result, "error");
    let badarg = atom(&result, "badarg");
    let add = Term::tuple(vec![atom(&result, "nif_add"), atom(&result, "add")]);
    result
        .term
        .proper_list()
        .unwrap()
        .map(|item| match item {
            Term::Integer(value) => Some(*value),
            Term::Tuple(caught) => {
                let Term::Tuple(frame) = &caught[2] else {
                    panic!("not a frame: {item:?}");
                };
                assert_eq!((&caught[0], &caught[1]), (&error, &badarg));
                assert_eq!(Term::tuple(frame[..2].to_vec()), add);
                assert_eq!(frame[2].proper_list().unwrap().count(), 2);
                None
            }
            other => panic!("not a probe result: {other:?}"),
        })
        .collect()
}

/// The results of `nif_add:probes(Path)` on the pinned ERTS, for each
/// library of `-DNIF_ADD_PROBE`, also with `-DNIF_ADD_FLAGS=1`. PBeam forms
/// no bitstring that is not a whole number of bytes, so the probes have
/// none; ERTS gives badarg for `<<1:3>>`.
///
/// ```text
/// mise run erts-eval --pa $PWD/fixtures/nif 'nif_add:probes("/tmp/pbeam866/probe")'
/// [7,{error,badarg,{nif_add,add,[0,-1],[]}},-1,{error,badarg,...},{error,badarg,...},
///  {error,badarg,...},0,1000,4097,-3903,-3903,97,97,97,0,0,0,3,2,2,
///  {error,badarg,{nif_add,add,[3,"ab"],[]}},2111,{error,badarg,{nif_add,add,[a,0],[]}}]
/// ```
const PROBES: [Option<i64>; 23] = [
    Some(7),
    None,
    Some(-1),
    None,
    None,
    None,
    Some(0),
    Some(1000),
    Some(4097),
    Some(-3903),
    Some(-3903),
    Some(97),
    Some(97),
    Some(97),
    Some(0),
    Some(0),
    Some(0),
    Some(3),
    Some(2),
    Some(2),
    None,
    Some(2111),
    None,
];

/// Return the result of `nif_add:probe(Path, Kind, 0)`.
fn probe(program: &BeamProgram, path: &str, kind: i64) -> Result<HostTerm, BeamError> {
    let arguments = vec![string(path), Term::Integer(kind), Term::Integer(0)];
    execute(program, "nif_add", "probe", arguments)
}

/// Return the bytes of the binary `term`.
fn bytes(term: &Term) -> Vec<u8> {
    let Term::Binary(binary) = term else {
        panic!("not a binary: {term:?}");
    };
    binary
        .bytes()
        .expect("the term is a binary of whole bytes")
        .to_vec()
}

#[test]
fn gives_the_erts_results_of_the_enif_calls_of_bcrypt() {
    let program = program();
    let path = build("probe", &["-DNIF_ADD_PROBE"], &[]);
    assert_eq!(probes(&program, &path), PROBES);
}

#[test]
fn runs_a_dirty_nif_on_the_dirty_pool_of_its_flags_as_erts() {
    // ERTS gives the same results on a dirty CPU scheduler, and the thread
    // names `erts_dcpus_N` and `erts_dios_N` (`erl_process.c:8956-8971`).
    let program = program();
    let cpu = build("probe_cpu", &["-DNIF_ADD_PROBE", "-DNIF_ADD_FLAGS=1"], &[]);
    assert_eq!(probes(&program, &cpu), PROBES);
    let name = bytes(&probe(&program, &cpu, 6).unwrap().term);
    assert!(name.starts_with(b"erts_dcpus_"), "{name:?}");
    let io = build("probe_io", &["-DNIF_ADD_PROBE", "-DNIF_ADD_FLAGS=2"], &[]);
    let name = bytes(&probe(&program, &io, 6).unwrap().term);
    assert!(name.starts_with(b"erts_dios_"), "{name:?}");
}

#[test]
fn stops_the_run_for_the_utf8_encoding_of_enif_get_string() {
    // ERTS gives 3 for `probe(Path, 5, "ab")`.
    let program = program();
    let path = build("probe_utf8", &["-DNIF_ADD_PROBE"], &[]);
    let arguments = vec![string(&path), Term::Integer(5), string("ab")];
    let failure = execute(&program, "nif_add", "probe", arguments).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "unsupported: enif_get_string with the encoding 2"
    );
}
