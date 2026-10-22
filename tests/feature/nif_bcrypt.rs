//! `bcrypt_elixir` 3.3.2 from Hex runs through the PBeam NIF ABI.
//!
//! The test fetches the dependencies of `fixtures/bcrypt/mix.lock` and
//! builds them with `mix compile`, so `elixir_make` builds `bcrypt_nif.so`
//! with the pinned ERTS headers and the host `cc`. Then the pinned ERTS and
//! PBeam run `nif_bcrypt:main/0` of `fixtures/nif` with the code path of
//! `bcrypt_elixir`, and the outputs are equal. The on_load function of
//! `Elixir.Bcrypt.Base` loads the library from `code:priv_dir/1`.
//!
//! The bundle test builds the release of the project, whose bundle ships
//! `lib/bcrypt_elixir-3.3.2/priv/bcrypt_nif.so`, and runs the one-file
//! executable with the entry `BcryptFixture.main/0`.

use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use std::{fs, thread};

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;
#[expect(
    dead_code,
    reason = "this test crate keeps the stdout of the run and its locale"
)]
#[path = "../common/run.rs"]
mod run;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;
use run::run_with_time_limit;

/// The Mix project that fetches and builds `bcrypt_elixir`.
const PROJECT: &str = "fixtures/bcrypt";

/// The code path of `bcrypt_elixir` that `mix compile` writes.
const EBIN: &str = "fixtures/bcrypt/_build/dev/lib/bcrypt_elixir/ebin";

/// The BEAM file of the module that calls the NIFs.
const PROBE: &str = "fixtures/nif/nif_bcrypt.beam";

/// The time that a run has to end.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(60);

#[test]
fn gives_the_erts_results_of_the_three_bcrypt_nifs() {
    build();
    let mut erts = Command::new("erl");
    erts.args(["-noshell", "-pa", "fixtures/nif", "-pa", EBIN, "-eval"])
        .arg("nif_bcrypt:main(), halt().");
    let expected = output(&mut erts);
    // The salt of 16 bytes 1..16 with the cost 4, and the result of
    // checkpass_nif/2 for the right and the wrong password.
    assert!(
        expected.contains("<<\"$2b$04$.OGB/.SE/ueHAeqKBO2NC.\">>"),
        "{expected}"
    );
    assert!(expected.contains(",0,-1,-1,"), "{expected}");

    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args(["pbeam_boot", "start", "nif_bcrypt", "main", "--root"])
        .arg(otp_root())
        .args(["--pa", "fixtures/nif", "--pa", EBIN, "--", PROBE])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    assert_eq!(output(&mut pbeam), expected);
}

/// Fetch the dependencies of the locked versions and build them.
fn build() {
    for arguments in [&["deps.get", "--check-locked"][..], &["compile"]] {
        // `EBIN` is the build path of the `dev` environment.
        mix(arguments, "dev");
    }
    assert!(Path::new(EBIN).join("../priv/bcrypt_nif.so").exists());
}

/// Run `command` and return its stdout. The run must exit with status 0
/// and write nothing to stderr.
#[test]
fn runs_a_bundle_that_ships_the_bcrypt_library() {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bcrypt-bundle");
    let _ = fs::remove_dir_all(&directory);
    let temporary = directory.join("tmp");
    fs::create_dir_all(&temporary).unwrap();
    let release = directory.join("release");
    mix(&["deps.get", "--check-locked"], "prod");
    let mut arguments = vec!["release", "--overwrite", "--path"];
    arguments.push(release.to_str().unwrap());
    mix(&arguments, "prod");
    let library = "lib/bcrypt_elixir-3.3.2/priv/bcrypt_nif.so";
    assert!(release.join(library).is_file());

    // The release on the pinned ERTS gives the expected line.
    let mut erts = Command::new(release.join("bin/bcrypt_fixture"));
    erts.args(["eval", "BcryptFixture.main()"]);
    let expected = output(&mut erts);
    assert!(
        expected.starts_with("{\"$2b$12$\", 60, true, false, \"$2b$04$.OGB/.SE/ueHAeqKBO2NC."),
        "{expected}"
    );

    // The gate opens the shipped library and passes Elixir.Bcrypt.Base,
    // whose path of erlang:load_nif/2 comes from code:priv_dir/1.
    let bundle = directory.join("bcrypt_fixture.pbeamb");
    let built = Command::new(env!("CARGO_BIN_EXE_pbeam_bundle"))
        .arg("bundle")
        .arg(&release)
        .arg(&bundle)
        .args(["Elixir.BcryptFixture", "main"])
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
        parsed.resource(library),
        Some(&fs::read(release.join(library)).unwrap()[..])
    );
    let executable = directory.join("bcrypt_fixture");
    let built = Command::new(env!("CARGO_BIN_EXE_pbeam_bundle"))
        .arg("executable")
        .arg(env!("CARGO_BIN_EXE_pbeam"))
        .arg(&bundle)
        .arg(&executable)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");

    // The run extracts the library to its path under the extracted root, so
    // the on_load of Elixir.Bcrypt.Base loads it. The entry returns and the
    // release keeps running, so SIGTERM stops it, and the run removes its
    // extraction at the exit.
    let mut command = Command::new(&executable);
    // `pbeam erl` rejects these erlexec variables, as `tests/feature/serve.rs`
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
        line.ok().flatten().map(|line| line + "\n").as_deref(),
        Some(expected.as_str()),
        "status: {status:?}\nstdout: {rest:?}\nstderr: {stderr}"
    );
    assert!(status.success(), "{status:?}\n{stderr}");
    assert!(
        fs::read_dir(&temporary).unwrap().next().is_none(),
        "the run keeps its extracted directory"
    );
    fs::remove_dir_all(&directory).unwrap();
}

/// Run `mix` with `arguments` in the project with `MIX_ENV` `environment`,
/// and panic with its output when it fails.
fn mix(arguments: &[&str], environment: &str) {
    let built = Command::new("mix")
        .args(arguments)
        .env("MIX_ENV", environment)
        .current_dir(PROJECT)
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

fn output(command: &mut Command) -> String {
    let output: Output = run_with_time_limit(command, Stdio::piped(), RUN_TIME_LIMIT);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}
