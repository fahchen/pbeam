use std::{
    env,
    ffi::OsStr,
    fs,
    os::unix::{ffi::OsStrExt, fs::symlink},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
};

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PRIM_FILE_READ_PROBE: &[u8] =
    include_bytes!("../../fixtures/prim_file_read/prim_file_read_probe.beam");

/// The environment variable that gives the ERTS run and the
/// `isolated_prim_file_read` child the directory of the test files.
const DIR: &str = "PBEAM_PRIM_FILE_READ_DIR";

/// The environment variable that gives the `isolated_prim_file_read` child
/// the file for its result.
const RESULT: &str = "PBEAM_PRIM_FILE_READ_RESULT";

/// The name of the current directory of each run, `café` in UTF-8, in the
/// test directory.
const CWD: &str = "caf\u{e9}";

/// An Erlang fun `T` for the `-eval` of an ERTS run: it gives the text of a
/// term as [`text`] does.
const ERTS_TEXT: &str = "T = fun T(X) when is_binary(X) -> \
                         [\"<<\", lists:join(\",\", [integer_to_list(B) || <<B>> <= X]), \">>\"]; \
                         T(X) when is_list(X) -> [\"[\", lists:join(\",\", [T(E) || E <- X]), \"]\"]; \
                         T(X) when is_tuple(X) -> \
                         [\"{\", lists:join(\",\", [T(E) || E <- tuple_to_list(X)]), \"}\"]; \
                         T(X) -> io_lib:write(X) end";

/// The `prim_file:internal_native2name/1` results of
/// `prim_file_read_probe:main/1` on the pinned ERTS with a UTF-8 file name
/// encoding: bytes that are not valid UTF-8 give `{error,warning}`
/// (`erl_unicode.c:2735-2824`).
const NAMES_UTF8: &str = "[[],[97,98,99],[99,97,102,233],{error,warning},[97,0,98],[128512],\
                          {error,warning},{error,warning},{error,warning},{error,warning},\
                          {error,badarg},{error,badarg},{error,badarg}]";

/// The `prim_file:internal_native2name/1` results of
/// `prim_file_read_probe:main/1` on the pinned ERTS with a Latin-1 file name
/// encoding: each byte is one character.
const NAMES_LATIN1: &str = "[[],[97,98,99],[99,97,102,195,169],[97,255],[97,0,98],\
                            [240,159,152,128],[192,128],[237,160,128],[244,144,128,128],\
                            [97,195],{error,badarg},{error,badarg},{error,badarg}]";

/// Return a term as the `T` fun of [`ERTS_TEXT`] gives it: the `~w` text,
/// with each binary as its bytes, for the atoms, integers, binaries, lists,
/// and tuples of the probe results.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::Binary(vec![97])])), "[<<97>>]");
/// ```
fn text(term: &Term) -> String {
    let join =
        |terms: &mut dyn Iterator<Item = &Term>| terms.map(text).collect::<Vec<_>>().join(",");
    if let Some(mut terms) = term.proper_list() {
        return format!("[{}]", join(&mut terms));
    }
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::Tuple(terms) => format!("{{{}}}", join(&mut terms.iter())),
        Term::Binary(bytes) => {
            let bytes: Vec<String> = bytes.iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn reads_files_directories_links_and_the_current_directory_as_pinned_erts() {
    let directory = env::temp_dir().join(format!("pbeam-prim-file-read-{}", process::id()));
    make_files(&directory);
    let cwd = directory.join(CWD);
    let fixtures = env::current_dir().unwrap().join("fixtures/prim_file_read");
    // The result file is outside the test directory, so the directory
    // listings of the two runs are the same.
    let result = directory.with_extension("result");
    for locale in ["C.UTF-8", "C"] {
        let utf8 = cfg!(target_os = "macos") || locale == "C.UTF-8";
        let erts = Command::new("erl")
            .args([
                OsStr::new("-noshell"),
                OsStr::new("-pa"),
                fixtures.as_os_str(),
                OsStr::new("-eval"),
                OsStr::new(&format!(
                    "{ERTS_TEXT}, D = os:getenv(\"{DIR}\"), \
                     io:format(\"~s~n\", [T(prim_file_read_probe:main(D))]), halt()."
                )),
            ])
            .env(DIR, &directory)
            .env("LC_ALL", locale)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            erts.status.success() && erts.stderr.is_empty(),
            "{locale}: {erts:?}"
        );
        let erts = String::from_utf8(erts.stdout).unwrap();
        // Values that do not depend on the host: the contents of file, the
        // size and the ends of big, /dev/null, and the names.
        let names = if utf8 { NAMES_UTF8 } else { NAMES_LATIN1 };
        for value in [
            "[{error,eisdir},{ok,<<104,101,108,108,111>>},{error,enoent},{error,eisdir},",
            "{70000,<<0,1,2,3>>,<<218,219,220,221>>},{ok,<<>>},",
            &format!(",{names}}}\n"),
        ] {
            assert!(erts.contains(value), "{locale}: {value} not in {erts}");
        }

        let child = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "isolated_prim_file_read",
                "--include-ignored",
                "--quiet",
            ])
            .env(DIR, &directory)
            .env(RESULT, &result)
            .env("LC_ALL", locale)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{locale}: child exited with {}\n{}\n{}",
            child.status,
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        let pbeam = fs::read_to_string(&result).unwrap();
        fs::remove_file(&result).unwrap();
        assert_eq!(pbeam, erts, "{locale}");
    }
    fs::remove_dir_all(&directory).unwrap();
}

/// The child side of
/// `reads_files_directories_links_and_the_current_directory_as_pinned_erts`:
/// run `prim_file_read_probe:main/1` and write the text of its result to the
/// result file. A child process is necessary, because the probe reads the
/// current directory, and PBeam reads the file name encoding once for each
/// process. The child inherits `LLVM_PROFILE_FILE`, so `cargo llvm-cov`
/// measures it.
#[test]
#[ignore = "reads_files_directories_links_and_the_current_directory_as_pinned_erts runs it in a child process"]
fn isolated_prim_file_read() {
    let directory = PathBuf::from(env::var_os(DIR).unwrap());
    let prim_file = pinned_beam("prim_file");
    let program = BeamProgram::load(&[&prim_file, PRIM_FILE_READ_PROBE]).unwrap();
    let result = program
        .execute("prim_file_read_probe", "main", &[characters(&directory)])
        .unwrap();
    fs::write(env::var_os(RESULT).unwrap(), text(&result) + "\n").unwrap();
}

/// Return the path as a list of its bytes, which is the character list of
/// an ASCII path.
fn characters(path: &Path) -> Term {
    Term::list_from(
        path.as_os_str()
            .as_encoded_bytes()
            .iter()
            .map(|&byte| Term::Integer(byte.into())),
    )
}

/// Make the files of `prim_file_read_probe:main/1` in `directory`. The
/// names that are not UTF-8 are made only when the file system accepts
/// them: APFS on macOS rejects them with `EILSEQ`.
fn make_files(directory: &Path) {
    for name in ["dir", "empty", CWD, "raw"] {
        fs::create_dir_all(directory.join(name)).unwrap();
    }
    fs::write(directory.join("file"), "hello").unwrap();
    let big: Vec<u8> = (0..70_000).map(|index| (index % 251) as u8).collect();
    fs::write(directory.join("big"), big).unwrap();
    for name in ["a", "b"] {
        fs::write(directory.join("dir").join(name), name).unwrap();
    }
    symlink("file", directory.join("link")).unwrap();
    symlink("missing", directory.join("dangling")).unwrap();
    symlink("x".repeat(300), directory.join("long")).unwrap();
    symlink(OsStr::from_bytes(b"target\xff"), directory.join("raw_link")).unwrap();
    let raw = directory.join("raw");
    match fs::write(raw.join(OsStr::from_bytes(b"name\xff")), "raw") {
        Ok(()) => {
            let raw_dir = raw.join(OsStr::from_bytes(b"dir\xff"));
            fs::create_dir(&raw_dir).unwrap();
            fs::write(raw_dir.join("inner"), "inner").unwrap();
        }
        Err(error) => assert_eq!(error.raw_os_error(), Some(libc::EILSEQ)),
    }
}
