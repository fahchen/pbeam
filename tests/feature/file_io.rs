use std::{
    fs,
    process::{Command, Output},
};

#[path = "../common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::pinned_beam_path;

const FILE_IO_PROBE: &str = "fixtures/boot/file_io_probe.beam";

/// The pinned OTP modules that the probe calls in addition to the boot
/// modules: `file` for the calls of the probe and `filename` for its paths.
const PROBE_MODULES: [&str; 2] = ["file", "filename"];

#[test]
fn opens_writes_positions_and_closes_a_file_as_pinned_erts() {
    // `file_io_probe:main/0` writes a scratch file with `file:write_file/2`
    // through `file_server_2`, and then opens it in write mode and in
    // append mode with `file:open/2` through a `file_io_server`. Each call
    // runs the `prim_file` NIFs `open_nif/2`, `write_nif/2`, `seek_nif/3`,
    // and `close_nif/1`, and `file:read_file/2` reads the file back. The
    // errors come from `open` (`eexist`, `enotdir`, `enoent`), from the
    // directory check of `efile_open` (`eisdir`), and from `lseek`
    // (`einval`), as unix_prim_file.c:173-226 and 508-540 give them.
    let expected = "[ok,\n {ok,<<\"ab\\ncd\\n\">>},\n ok,\n {ok,2},\n ok,\n {ok,13},\n \
        {error,einval},\n ok,ok,\n {ok,<<\"fiXYZ\\nsecond\\n\">>},\n ok,\n \
        {ok,<<\"fiXYZ\\nsecond\\ntail\\n\">>},\n {error,eexist},\n {error,eisdir},\n \
        {error,enotdir},\n {error,enoent}]\n";
    for (name, mut command) in [("ERTS", erts()), ("PBeam", pbeam())] {
        let directory =
            std::env::temp_dir().join(format!("pbeam-file-io-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let Output {
            status,
            stdout,
            stderr,
        } = command
            .env("FILE_IO_PROBE_PATH", directory.join("scratch.txt"))
            .env("TERM", "dumb")
            .env("LC_ALL", "C.UTF-8")
            .output()
            .unwrap();
        fs::remove_dir_all(&directory).unwrap();
        assert_eq!(
            (
                String::from_utf8_lossy(&stdout).as_ref(),
                String::from_utf8_lossy(&stderr).as_ref(),
                status.code()
            ),
            (expected, "", Some(0)),
            "{name}"
        );
    }
}

/// Return the `erl` command that runs `file_io_probe:main()` on the pinned
/// ERTS.
///
/// # Examples
///
/// ```rust,ignore
/// let output = erts().output()?;
/// ```
fn erts() -> Command {
    let mut command = Command::new("erl");
    command.args([
        "-noshell",
        "-pa",
        "fixtures/boot",
        "-eval",
        "file_io_probe:main(), halt().",
    ]);
    command
}

/// Return the `pbeam` command that boots with `pbeam_boot` and runs
/// `file_io_probe:main()`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = pbeam().output()?;
/// ```
fn pbeam() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args([
            "pbeam_boot",
            "start",
            "file_io_probe",
            "main",
            "--root",
            otp_root().to_str().unwrap(),
            "--",
            FILE_IO_PROBE,
        ])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .args(PROBE_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    command
}
