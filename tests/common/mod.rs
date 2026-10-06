//! Helpers that more than one integration test uses.

use std::{path::PathBuf, process::Command};

/// Read the pinned OTP 29.1.1 BEAM file of a module from the mise-managed
/// Erlang installation that this repository pins.
///
/// # Examples
///
/// ```rust,ignore
/// let erlang = pinned_beam("erlang");
/// let io = pinned_beam("io");
/// let program = BeamProgram::load(&[&erlang, &io, PROBE])?;
/// ```
pub fn pinned_beam(module: &str) -> Vec<u8> {
    std::fs::read(pinned_beam_path(module)).unwrap()
}

/// Return the path of the pinned OTP 29.1.1 BEAM file of a module.
/// `code:which/1` gives the path of a kernel or stdlib module such as `io`.
/// For a preloaded ERTS module such as `erlang` or `erts_internal`, it
/// returns `preloaded`, so the path comes from the `ebin` directory of
/// `code:lib_dir(erts)`.
///
/// # Examples
///
/// ```rust,ignore
/// let path = pinned_beam_path("prim_tty");
/// assert!(path.ends_with("prim_tty.beam"));
/// ```
pub fn pinned_beam_path(module: &str) -> PathBuf {
    let path = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            &format!(
                "io:format(\"~ts\", [case code:which({module}) of \
                 preloaded -> filename:join([code:lib_dir(erts), \"ebin\", \"{module}.beam\"]); \
                 Path -> Path end]), halt()."
            ),
        ])
        .output()
        .unwrap();
    assert!(path.status.success());
    PathBuf::from(String::from_utf8(path.stdout).unwrap())
}
