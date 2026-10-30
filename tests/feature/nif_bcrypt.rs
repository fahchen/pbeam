//! `bcrypt_elixir` 3.3.2 from Hex runs through the PBeam NIF ABI.
//!
//! The test fetches the dependencies of `fixtures/bcrypt/mix.lock` and
//! builds them with `mix compile`, so `elixir_make` builds `bcrypt_nif.so`
//! with the pinned ERTS headers and the host `cc`. Then the pinned ERTS and
//! PBeam run `nif_bcrypt:main/0` of `fixtures/nif` with the code path of
//! `bcrypt_elixir`, and the outputs are equal. The `on_load` function of
//! `Elixir.Bcrypt.Base` loads the library from `code:priv_dir/1`.
//!
//! The bundle test builds the release of the project, whose bundle ships
//! `lib/bcrypt_elixir-3.3.2/priv/bcrypt_nif.so`, and runs the one-file
//! executable with the entry `BcryptFixture.main/0`.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::path::Path;
use std::process::Command;

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;
#[path = "../common/mix_bundle.rs"]
mod mix_bundle;
#[expect(
    dead_code,
    reason = "this test crate keeps the stdout of the run and its locale"
)]
#[path = "../common/run.rs"]
mod run;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;
use mix_bundle::{Release, mix, output, run_release_bundle};

/// The Mix project that fetches and builds `bcrypt_elixir`.
const PROJECT: &str = "fixtures/bcrypt";

/// The code path of `bcrypt_elixir` that `mix compile` writes.
const EBIN: &str = "fixtures/bcrypt/_build/dev/lib/bcrypt_elixir/ebin";

/// The BEAM file of the module that calls the NIFs.
const PROBE: &str = "fixtures/nif/nif_bcrypt.beam";

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
        mix(PROJECT, arguments, "dev");
    }
    assert!(Path::new(EBIN).join("../priv/bcrypt_nif.so").exists());
}

#[test]
fn runs_a_bundle_that_ships_the_bcrypt_library() {
    let expected = run_release_bundle(&Release {
        project: PROJECT,
        app: "bcrypt_fixture",
        entry: "BcryptFixture",
        library: "lib/bcrypt_elixir-3.3.2/priv/bcrypt_nif.so",
    });
    assert!(
        expected.starts_with("{\"$2b$12$\", 60, true, false, \"$2b$04$.OGB/.SE/ueHAeqKBO2NC."),
        "{expected}"
    );
}
