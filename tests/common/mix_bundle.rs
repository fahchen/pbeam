//! Build the release of a fixture Mix project that ships a NIF library, and
//! run its one-file executable on PBeam with the result of the pinned ERTS.
//! `tests/feature/nif_bcrypt.rs` and `tests/feature/nif_exqlite.rs` use it.

use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use std::{fs, thread};

use crate::run::run_with_time_limit;

/// The time that a run has to end.
pub(crate) const RUN_TIME_LIMIT: Duration = Duration::from_secs(60);

/// A fixture Mix project whose release ships one NIF library.
pub(crate) struct Release<'a> {
    /// The directory of the project, for example `fixtures/bcrypt`.
    pub project: &'a str,
    /// The name of the application and of its release.
    pub app: &'a str,
    /// The Elixir module whose `main/0` writes one line and returns.
    pub entry: &'a str,
    /// The path of the NIF library in the release.
    pub library: &'a str,
}

/// Build the release of `release`, run `main/0` on the pinned ERTS, bundle
/// the release with `pbeam_bundle`, and run the one-file executable. Return
/// the line of the ERTS run after the PBeam run gives the same line.
pub(crate) fn run_release_bundle(release: &Release<'_>) -> String {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{}-bundle", release.app));
    let _ = fs::remove_dir_all(&directory);
    let temporary = directory.join("tmp");
    fs::create_dir_all(&temporary).unwrap();
    let path = directory.join("release");
    mix(release.project, &["deps.get", "--check-locked"], "prod");
    let mut arguments = vec!["release", "--overwrite", "--path"];
    arguments.push(path.to_str().unwrap());
    mix(release.project, &arguments, "prod");
    assert!(path.join(release.library).is_file());

    // The release on the pinned ERTS gives the expected line.
    let mut erts = Command::new(path.join("bin").join(release.app));
    erts.args(["eval", &format!("{}.main()", release.entry)]);
    let expected = output(&mut erts);

    // The gate opens the shipped library and passes the module whose path
    // of erlang:load_nif/2 comes from code:priv_dir/1.
    let bundle = directory.join(format!("{}.pbeamb", release.app));
    let built = Command::new(env!("CARGO_BIN_EXE_pbeam_bundle"))
        .arg("bundle")
        .arg(&path)
        .arg(&bundle)
        .args([&format!("Elixir.{}", release.entry), "main"])
        .arg(start_list(&directory))
        .output()
        .unwrap();
    assert!(
        built.status.success() && built.stderr.is_empty(),
        "{built:?}"
    );
    let bytes = fs::read(&bundle).unwrap();
    let parsed = pbeam_runtime::Bundle::parse(&bytes).unwrap();
    assert_eq!(
        parsed.resource(release.library),
        Some(&fs::read(path.join(release.library)).unwrap()[..])
    );
    let executable = directory.join(release.app);
    let built = Command::new(env!("CARGO_BIN_EXE_pbeam_bundle"))
        .arg("executable")
        .arg(env!("CARGO_BIN_EXE_pbeam"))
        .arg(&bundle)
        .arg(&executable)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");

    // The run extracts the library to its path under the extracted root, so
    // the on_load of the NIF module loads it. The entry returns and the
    // release keeps running, so SIGTERM stops it, and the run removes its
    // extraction at the exit.
    let mut command = Command::new(&executable);
    // The run removes these erlexec variables, as `tests/feature/serve.rs`
    // records, so a shell setting does not change the run.
    for name in [
        "ERL_OTP29_FLAGS",
        "ERL_AFLAGS",
        "ERL_FLAGS",
        "ERL_ZFLAGS",
        "ERL_EPMD_PORT",
    ] {
        command.env_remove(name);
    }
    let mut child = command
        .env("TMPDIR", &temporary)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let reader = thread::spawn(move || {
        let mut lines = stdout.lines().map_while(Result::ok);
        let _ = sender.send(lines.next());
        lines.collect::<Vec<_>>()
    });
    let line = receiver.recv_timeout(RUN_TIME_LIMIT);
    let group = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
    let status = child.wait().unwrap();
    let rest = reader.join().unwrap();
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    assert_eq!(
        line.ok()
            .flatten()
            .map(|line| format!("{line}\n"))
            .as_deref(),
        Some(expected.as_str()),
        "status: {status:?}\nstdout: {rest:?}\nstderr: {stderr}"
    );
    assert!(status.success(), "{status:?}\n{stderr}");
    assert!(
        fs::read_dir(&temporary).unwrap().next().is_none(),
        "the run keeps its extracted directory"
    );
    fs::remove_dir_all(&directory).unwrap();
    expected
}

/// Run `mix` with `arguments` in `project` with `MIX_ENV` `environment`,
/// and panic with its output when it fails.
pub(crate) fn mix(project: &str, arguments: &[&str], environment: &str) {
    let built = Command::new("mix")
        .args(arguments)
        .env("MIX_ENV", environment)
        .current_dir(project)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "mix {arguments:?}: {}\n{}",
        String::from_utf8_lossy(&built.stdout),
        String::from_utf8_lossy(&built.stderr)
    );
}

/// Write the start list of a release bundle, the files of the preloaded
/// modules of the pinned OTP, as `scripts/example.sh --write-start` does,
/// and return its path.
fn start_list(directory: &Path) -> PathBuf {
    let path = directory.join("start.list");
    let mut erl = Command::new("erl");
    erl.args(["-noshell", "-eval"]).arg(
        "Ebin = filename:join(code:lib_dir(erts), \"ebin\"), \
         [io:put_chars([filename:join(Ebin, atom_to_list(M) ++ \".beam\"), $\\n]) || M <- erlang:pre_loaded()], \
         halt().",
    );
    fs::write(&path, output(&mut erl)).unwrap();
    path
}

/// Run `command` and return its stdout. The run must exit with status 0
/// and write nothing to stderr.
pub(crate) fn output(command: &mut Command) -> String {
    let output: Output = run_with_time_limit(command, Stdio::piped(), RUN_TIME_LIMIT);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}
