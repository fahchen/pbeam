//! `bcrypt_elixir` 3.3.2 from Hex runs through the PBeam NIF ABI.
//!
//! The test fetches the dependencies of `fixtures/bcrypt/mix.lock` and
//! builds them with `mix compile`, so `elixir_make` builds `bcrypt_nif.so`
//! with the pinned ERTS headers and the host `cc`. Then the pinned ERTS and
//! PBeam run `nif_bcrypt:main/0` of `fixtures/nif` with the code path of
//! `bcrypt_elixir`, and the outputs are equal. The on_load function of
//! `Elixir.Bcrypt.Base` loads the library from `code:priv_dir/1`.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

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
        let built = Command::new("mix")
            .args(arguments)
            // `EBIN` is the build path of the `dev` environment.
            .env("MIX_ENV", "dev")
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
    assert!(Path::new(EBIN).join("../priv/bcrypt_nif.so").exists());
}

/// Run `command` and return its stdout. The run must exit with status 0
/// and write nothing to stderr.
fn output(command: &mut Command) -> String {
    let output: Output = run_with_time_limit(command, Stdio::piped(), RUN_TIME_LIMIT);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}
