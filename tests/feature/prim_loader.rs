use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{self, Command, Output, Stdio},
};

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;

use boot::{BOOT_MODULES, boot_beam};
use common::pinned_beam_path;

/// The probe that writes the `erl_prim_loader` results.
const PRIM_LOADER_PROBE: &str = "fixtures/prim_loader/prim_loader_probe";

/// The pinned OTP modules that `io:format/2` with `~p` calls after the
/// boot, in addition to `BOOT_MODULES`.
const FORMAT_MODULES: [&str; 4] = ["io_lib_pretty", "erl_scan", "erl_features", "epp"];

/// The second line of `prim_loader_probe:main/0` for the flags `-pa d1 -pa
/// d2`, measured on the pinned ERTS: `get_file/1` of a base name searches
/// the path in order, so `d2/a.txt` hides `d1/a.txt`
/// (erl_prim_loader.erl:595-627). `get_file/1` of a name with a `/` and
/// `read_file/1` read the name from the current directory
/// (erl_prim_loader.erl:298-299, 595-612). A file that does not exist gives
/// `error` (erl_prim_loader.erl:424-425). `init:get_argument(loader)` gives
/// `error`, and `init:get_argument(pa)` gives the value of each `-pa` flag in
/// the command line order (init.erl:1808-1819).
const RESULTS_WITH_PA: &str = "[{ok,{binary,\"two\"},\"d2/a.txt\"},\n \
                               {ok,{binary,\"c\"},\"d1/c.txt\"},\n \
                               error,\n \
                               {ok,{binary,\"one\"},\"d1/a.txt\"},\n \
                               {ok,{binary,\"one\"}},\n \
                               error,error,error,\n \
                               {ok,[[\"d1\"],[\"d2\"]]}]\n";

/// The second line of `prim_loader_probe:main/0` without `-pa`, measured on
/// the pinned ERTS: no directory of the path has `a.txt` or `c.txt`.
const RESULTS_WITHOUT_PA: &str = "[error,error,error,\n \
                                  {ok,{binary,\"one\"},\"d1/a.txt\"},\n \
                                  {ok,{binary,\"one\"}},\n \
                                  error,error,error,error]\n";

#[test]
fn starts_erl_prim_loader_with_the_code_path_of_pa_as_pinned_erts() {
    // `pbeam --pa d1 --pa d2` gives `pbeam_boot:start/3` the boot flags
    // `[{pa, [<<"d1">>]}, {pa, [<<"d2">>]}]`, as `init:parse_boot_args/1`
    // gives them for `erl -pa d1 -pa d2` (init.erl:1668-1709). `pbeam_boot`
    // starts the pinned `erl_prim_loader` with the path `Pa ++ Pz ++ ["."]`,
    // with the last `-pa` first (init.erl:1164-1176, 1259-1262). Without
    // `--pa`, `pbeam_boot:start/2` gives no flags, so the path is `["."]`.
    // The ERTS boot script then replaces the path with `Pa ++ BootPath ++
    // Pz`, where `BootPath` is the `lib` directories of `start_clean.boot`
    // (init.erl:1323-1332). PBeam runs no boot script (row "Boot" of
    // `docs/erts-differences.md`), so the paths differ after the `-pa`
    // directories.
    let directory = env::temp_dir().join(format!("pbeam-prim-loader-{}", process::id()));
    for (file, contents) in [("d1/a.txt", "one"), ("d1/c.txt", "c"), ("d2/a.txt", "two")] {
        let path = directory.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    let with_pa = ["--pa", "d1", "--pa", "d2"];
    let cases = [
        (
            &with_pa[..],
            RESULTS_WITH_PA,
            "{ok,[\"d2\",\"d1\",",
            "{ok,[\"d2\",\"d1\",\".\"]}\n",
        ),
        (&[][..], RESULTS_WITHOUT_PA, "{ok,[\"/", "{ok,[\".\"]}\n"),
    ];
    for (flags, results, erts_path, pbeam_path) in cases {
        let erts = run(&mut erts(&directory, flags));
        let (path, rest) = erts.split_once('\n').unwrap();
        assert!(path.starts_with(erts_path), "ERTS {flags:?}: {erts}");
        assert!(rest.ends_with(results), "ERTS {flags:?}: {erts}");
        let pbeam = run(&mut pbeam(&directory, flags));
        assert_eq!(pbeam, format!("{pbeam_path}{results}"), "PBeam {flags:?}");
    }
    fs::remove_dir_all(&directory).unwrap();
}

/// Return the command of the probe on the pinned ERTS, with the `erl` flag
/// of each `--pa` flag, in `directory`. The probe loads with
/// `code:load_abs/1`, so the code path flags are only those of `flags`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut erts(&directory, &["--pa", "d1"]));
/// ```
fn erts(directory: &Path, flags: &[&str]) -> Command {
    let probe = absolute(PRIM_LOADER_PROBE);
    let mut command = Command::new("erl");
    command.args(["+S", "1", "-noshell"]);
    command.args(
        flags
            .iter()
            .map(|flag| flag.strip_prefix('-').unwrap_or(flag)),
    );
    command.args([
        "-eval",
        &format!(
            "{{module, _}} = code:load_abs(\"{}\"), prim_loader_probe:main(), halt().",
            probe.display()
        ),
    ]);
    command.current_dir(directory);
    command
}

/// Return the command of the probe on the `pbeam` binary with `flags`, in
/// `directory`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run(&mut pbeam(&directory, &["--pa", "d1"]));
/// ```
fn pbeam(directory: &Path, flags: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["pbeam_boot", "start", "prim_loader_probe", "main"])
        .args(flags)
        .arg("--")
        .arg(absolute(format!("{PRIM_LOADER_PROBE}.beam")))
        .arg(absolute(boot_beam()))
        .args(
            BOOT_MODULES
                .iter()
                .chain(&FORMAT_MODULES)
                .map(|module| pinned_beam_path(module)),
        )
        .current_dir(directory);
    command
}

/// Run a command and return its stdout. The run must exit with status 0 and
/// write nothing to stderr.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(run(Command::new("echo").arg("a")), "a\n");
/// ```
fn run(command: &mut Command) -> String {
    let output: Output = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{command:?}: {output:?}"
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Return the absolute path of a path in the repository.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(absolute("fixtures").is_absolute());
/// ```
fn absolute(path: impl AsRef<Path>) -> PathBuf {
    env::current_dir().unwrap().join(path)
}
