use std::{
    env,
    fs::{self, File, FileTimes, Permissions},
    os::unix::{fs::PermissionsExt, fs::symlink, net::UnixListener},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, UNIX_EPOCH},
};

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/boot.rs"]
mod boot;
#[path = "../common/mod.rs"]
mod common;

use boot::{BOOT_MODULES, boot_beam};
use common::pinned_beam;

const FIND_EXECUTABLE_PROBE: &[u8] =
    include_bytes!("../../fixtures/find_executable/find_executable_probe.beam");

/// The environment variable that gives `find_executable_probe:main/0` and
/// the `isolated_find_executable` child the directory of the test files.
const DIR: &str = "PBEAM_FIND_EXECUTABLE_DIR";

/// The environment variable that gives the `isolated_find_executable`
/// child the file for its result.
const RESULT: &str = "PBEAM_FIND_EXECUTABLE_RESULT";

/// The pinned OTP modules that `find_executable_probe:main/0` calls after
/// the boot: `os:find_executable/1,2` calls `filename`, and
/// `file:read_file_info/1` calls `file_server_2`, which calls `prim_file`.
/// `BOOT_MODULES` has `prim_file` for `erl_prim_loader`.
const FILE_MODULES: [&str; 2] = ["file", "filename"];

/// An Erlang fun `T` for the `-eval` of an ERTS run: it gives the text of a
/// term as [`text`] does.
const ERTS_TEXT: &str = "T = fun T(X) when is_binary(X) -> \
                         [\"<<\", lists:join(\",\", [integer_to_list(B) || <<B>> <= X]), \">>\"]; \
                         T(X) when is_list(X) -> [\"[\", lists:join(\",\", [T(E) || E <- X]), \"]\"]; \
                         T(X) when is_tuple(X) -> \
                         [\"{\", lists:join(\",\", [T(E) || E <- tuple_to_list(X)]), \"}\"]; \
                         T(X) -> io_lib:write(X) end";

/// `find_executable_probe:names/1` on the pinned ERTS with a UTF-8 file name
/// encoding: `prim_file:internal_name2native/1` adds a NUL byte to the
/// native bytes (`erl_unicode.c:2629-2734`).
const NAMES_UTF8: &str = "[<<97,98,99,0>>,<<97,98,99,0>>,<<0>>,<<97,98,99,0>>,<<0>>,\
                          {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                          {error,badarg},<<195,169,0>>,<<196,128,0>>,<<244,143,191,191,0>>,\
                          {error,badarg},<<237,159,191,0>>,{error,badarg},{error,badarg},\
                          <<238,128,128,0>>,{error,badarg},{error,badarg},{error,badarg},\
                          {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                          {error,badarg}]";

/// `find_executable_probe:names/1` on the pinned ERTS with a Latin-1 file
/// name encoding: a character above 255 raises `badarg`.
const NAMES_LATIN1: &str = "[<<97,98,99,0>>,<<97,98,99,0>>,<<0>>,<<97,98,99,0>>,<<0>>,\
                            {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                            {error,badarg},<<233,0>>,{error,badarg},{error,badarg},\
                            {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                            {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                            {error,badarg},{error,badarg},{error,badarg},{error,badarg},\
                            {error,badarg}]";

/// Return a term as the `T` fun of [`ERTS_TEXT`] gives it: the `~w` text,
/// with each binary as its bytes, for the atoms, integers, binaries, lists,
/// and tuples of the probe results.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::binary(vec![97])])), "[<<97>>]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        Term::Binary(bytes) => {
            let bytes: Vec<String> = bytes.iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn finds_executables_and_reads_file_info_as_pinned_erts() {
    let directory = env::temp_dir().join(format!("pbeam-find-executable-{}", std::process::id()));
    make_files(&directory);
    for locale in ["C.UTF-8", "C"] {
        let utf8 = cfg!(target_os = "macos") || locale == "C.UTF-8";
        let erts = Command::new("erl")
            .args([
                "-noshell",
                "-pa",
                "fixtures/find_executable",
                "-eval",
                &format!(
                    "{ERTS_TEXT}, M = find_executable_probe, D = os:getenv(\"{DIR}\"), \
                     io:format(\"~s~n~s~n~s~n\", [T(M:main()), T(M:bifs(D)), T(M:names(1 bsl 60))]), \
                     halt()."
                ),
            ])
            .env(DIR, &directory)
            .env("LC_ALL", locale)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            erts.status.success() && erts.stderr.is_empty(),
            "{locale}: {erts:?}"
        );
        let erts = String::from_utf8(erts.stdout).unwrap();
        let names = if utf8 { NAMES_UTF8 } else { NAMES_LATIN1 };
        assert_eq!(erts.lines().nth(2), Some(names), "{locale}");
        // The executable of the second path directory, through the link
        // too, and not the file without an execute bit or the directory.
        let tool = text(&characters(&directory.join("bin2/tool")));
        let sub = text(&characters(&directory.join("bin2/sub")));
        let link = text(&characters(&directory.join("bin2/link")));
        assert!(
            erts.starts_with(&format!(
                "{{{sh},[{tool},{sub},{link},false,{tool},false,false],false,",
                sh = text(&characters(Path::new(&sh_path())))
            )),
            "{locale}: {erts}"
        );

        let result = directory.join("result");
        let child = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "isolated_find_executable",
                "--include-ignored",
                "--quiet",
            ])
            .env(DIR, &directory)
            .env(RESULT, &result)
            .env("LC_ALL", locale)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{locale}: child exited with {}\n{}",
            child.status,
            String::from_utf8_lossy(&child.stdout)
        );
        let pbeam = fs::read_to_string(&result).unwrap();
        fs::remove_file(&result).unwrap();
        assert_eq!(pbeam, erts, "{locale}");
    }
    fs::remove_dir_all(&directory).unwrap();
}

/// The child side of `finds_executables_and_reads_file_info_as_pinned_erts`:
/// run the boot and `find_executable_probe:main/0`, then `bifs/1` and
/// `names/1`, and write the text of each result to the result file. A
/// child process is necessary, because the boot sets the host signal
/// dispositions of `prim_tty` (see `run_isolated` in `tests/feature/boot.rs`),
/// and PBeam reads the file name encoding once for each process. The child
/// inherits `LLVM_PROFILE_FILE`, so `cargo llvm-cov` measures it.
#[test]
#[ignore = "finds_executables_and_reads_file_info_as_pinned_erts runs it in a child process"]
fn isolated_find_executable() {
    let directory = PathBuf::from(env::var_os(DIR).unwrap());
    let beams: Vec<Vec<u8>> = BOOT_MODULES
        .iter()
        .chain(&FILE_MODULES)
        .map(|module| pinned_beam(module))
        .collect();
    let boot = fs::read(boot_beam()).unwrap();
    let mut modules: Vec<&[u8]> = beams.iter().map(Vec::as_slice).collect();
    modules.extend([FIND_EXECUTABLE_PROBE, boot.as_slice()]);
    let program = BeamProgram::load(&modules).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let main = program
        .execute(
            "pbeam_boot",
            "start",
            &[atom("find_executable_probe"), atom("main")],
        )
        .unwrap();
    let bifs = program
        .execute("find_executable_probe", "bifs", &[characters(&directory)])
        .unwrap();
    let names = program
        .execute("find_executable_probe", "names", &[Term::Integer(1 << 60)])
        .unwrap();
    let results = [main, bifs, names].map(|term| text(&term) + "\n");
    fs::write(env::var_os(RESULT).unwrap(), results.concat()).unwrap();
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

/// Return the first `sh` of the directories of `PATH`, as
/// `os:find_executable("sh")` finds it.
fn sh_path() -> PathBuf {
    env::split_paths(&env::var_os("PATH").unwrap())
        .map(|directory| directory.join("sh"))
        .find(|path| path.is_file())
        .unwrap()
}

/// Make the files of `find_executable_probe:main/0` in `directory`.
fn make_files(directory: &Path) {
    fs::create_dir_all(directory.join("bin1/sub")).unwrap();
    fs::create_dir(directory.join("bin2")).unwrap();
    for (name, mode) in [
        ("bin1/tool", 0o644),
        ("bin2/tool", 0o755),
        ("bin2/sub", 0o755),
        ("write_only", 0o200),
        ("no_access", 0o000),
        ("old", 0o644),
        ("future", 0o644),
    ] {
        let path = directory.join(name);
        fs::write(&path, name).unwrap();
        fs::set_permissions(&path, Permissions::from_mode(mode)).unwrap();
    }
    symlink("tool", directory.join("bin2/link")).unwrap();
    UnixListener::bind(directory.join("socket")).unwrap();
    // Times outside 1902-01-01 to 9999-12-31, which `build_file_info`
    // limits (`prim_file_nif.h:97-105`). A file system can limit them first.
    for (name, time) in [
        ("old", UNIX_EPOCH - Duration::from_secs(3_000_000_000)),
        ("future", UNIX_EPOCH + Duration::from_secs(260_000_000_000)),
    ] {
        let times = FileTimes::new().set_accessed(time).set_modified(time);
        File::options()
            .write(true)
            .open(directory.join(name))
            .unwrap()
            .set_times(times)
            .unwrap();
    }
}
