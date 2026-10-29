//! Opens, writes, positions and closes a file through the `pbeam` binary and compares the result
//! with the pinned ERTS.

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
/// modules: `file` for the calls of the probe, `filename` for its paths,
/// and `prim_buffer` for the read buffer of a file with a read mode.
const PROBE_MODULES: [&str; 3] = ["file", "filename", "prim_buffer"];

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
    //
    // A file with a read mode then holds a read buffer of
    // `prim_buffer:new/0` (`prim_file.erl:516-517`), and `file:read_line/1`,
    // `file:read/2`, and `file:position/2` run the `prim_buffer` NIFs
    // `find_byte_index/2`, `write/2`, `size/1`, `read/2` (`copying_read/2`
    // and `peek_head/1`), `skip/2`, and `wipe/1`. `io:get_line/2` reads a
    // line of the file `utf16.txt` with the encoding `{utf16, little}`, and
    // an append-only open keeps a read buffer too.
    //
    // `file:consult/1` then reads the terms of
    // `fixtures/boot/file_io_probe_terms.config` through the same read
    // buffer, and the file `fixtures/boot/file_io_probe_bad.config` gives
    // the parse error of `erl_parse` for its second term. The float term
    // of the third term runs `erlang:list_to_float/1`, which `erl_scan`
    // calls for a float token (`erl_scan.erl:1969`).
    let expected = "[ok,\n {ok,<<\"ab\\ncd\\n\">>},\n ok,\n {ok,2},\n ok,\n {ok,13},\n \
        {error,einval},\n ok,ok,\n {ok,<<\"fiXYZ\\nsecond\\n\">>},\n ok,\n \
        {ok,<<\"fiXYZ\\nsecond\\ntail\\n\">>},\n {error,eexist},\n {error,eisdir},\n \
        {error,enotdir},\n {error,enoent},\n {ok,\"fiXYZ\\n\"},\n {ok,\"se\"},\n \
        {ok,\"cond\\ntail\\n\"},\n {ok,18},\n eof,ok,ok,\"abc\\n\",eof,ok,ok,\n \
        {ok,25},\n ok,\n \
        {ok,[{plug_hello,[{port,4004},{enabled,true}]},\n      \
        [atom,<<\"binary\">>,15],\n      {timeout,1.5,-2.5}]},\n \
        {error,{1,erl_parse,[\"syntax error before: \",\"'.'\"]}}]\n";
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
            .env(
                "FILE_IO_PROBE_TERMS",
                "fixtures/boot/file_io_probe_terms.config",
            )
            .env(
                "FILE_IO_PROBE_BAD_TERMS",
                "fixtures/boot/file_io_probe_bad.config",
            )
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
