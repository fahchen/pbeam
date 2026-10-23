//! The `bin` directory of `erts` of the pinned OTP. A test that gives
//! `--bindir` to the `pbeam_boot` form includes this file with
//! `#[path = "../common/bindir.rs"] mod bindir;`, because a function in
//! `common/boot.rs` would be dead code in each other test crate.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

/// Return the `bin` directory of `erts` of the pinned OTP, as
/// `init:get_argument(bindir)` gives it, from one `erl` run for each test
/// process. `pbeam_boot` gives it to `init` for the `--bindir` option of
/// the `pbeam` binary, as `erlexec` gives `-bindir` (erlexec.c:1154-1155).
/// On Linux, `inet_gethost_native` starts `inet_gethost` from it during the
/// kernel start (inet_gethost_native.erl:412-418).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(otp_bindir().join("inet_gethost").is_file());
/// ```
pub fn otp_bindir() -> &'static Path {
    static BINDIR: OnceLock<PathBuf> = OnceLock::new();
    BINDIR.get_or_init(|| {
        let output = Command::new("erl")
            .args([
                "-noshell",
                "-eval",
                "{ok, [[Dir]]} = init:get_argument(bindir), io:put_chars(Dir), halt().",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        PathBuf::from(String::from_utf8(output.stdout).unwrap())
    })
}
