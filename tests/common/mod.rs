//! Helpers that more than one integration test uses.

use std::{collections::HashMap, path::PathBuf, process::Command, sync::OnceLock};

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
pub(super) fn pinned_beam(module: &str) -> Vec<u8> {
    std::fs::read(pinned_beam_path(module)).unwrap()
}

/// Return the path of the pinned OTP 29.1.1 BEAM file of a module.
/// `code:which/1` gives the path of a kernel or stdlib module such as `io`.
/// For a preloaded ERTS module such as `erlang` or `erts_internal`, it
/// returns `preloaded`, so the path comes from the `ebin` directory of
/// `code:lib_dir(erts)`. Panic when the code path has no such module.
///
/// # Examples
///
/// ```rust,ignore
/// let path = pinned_beam_path("prim_tty");
/// assert!(path.ends_with("prim_tty.beam"));
/// ```
pub(super) fn pinned_beam_path(module: &str) -> PathBuf {
    pinned_beam_paths()
        .get(module)
        .unwrap_or_else(|| panic!("the pinned OTP code path has no module {module}"))
        .clone()
}

/// Return the path of each module of the pinned OTP code path, as
/// `code:which/1` gives it, from one `erl` run for each test process.
/// `code:all_available/0` gives the file of each module, as `code:which/1`
/// does: the loaded file, else the first file on the code path, else
/// `preloaded`. An `erl` run for each module would start about 45 runs for
/// each test that boots OTP.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(pinned_beam_paths()["lists"].ends_with("lists.beam"));
/// ```
fn pinned_beam_paths() -> &'static HashMap<String, PathBuf> {
    static PATHS: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    PATHS.get_or_init(|| {
        let output = Command::new("erl")
            .args([
                "-noshell",
                "-eval",
                "[io:format(\"~ts\\t~ts~n\", [Module, case File of \
                 preloaded -> filename:join([code:lib_dir(erts), \"ebin\", Module ++ \".beam\"]); \
                 _ -> File end]) || {Module, File, _} <- code:all_available()], halt().",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| {
                let (module, path) = line.split_once('\t').unwrap();
                (module.to_owned(), PathBuf::from(path))
            })
            .collect()
    })
}
