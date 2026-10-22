use std::os::unix::ffi::OsStrExt;

use super::{erlexec_path, put, set, split, unset, variable, variables};

/// A key that only this test writes. No other test of the binary reads it,
/// so the change of the copy of the environment is test-scoped.
const KEY: &[u8] = b"PBEAM_ENVIRONMENT_UNIT_TEST";

#[test]
fn splits_an_entry_at_its_first_equals_sign() {
    // `import_initial_env` splits each entry of `environ` at its first `=`
    // (`sys_env.c:139-151`).
    assert_eq!(split(b"A=1"), Some((b"A".to_vec(), b"1".to_vec())));
    assert_eq!(split(b"A="), Some((b"A".to_vec(), Vec::new())));
    assert_eq!(split(b"A=x=y"), Some((b"A".to_vec(), b"x=y".to_vec())));
    assert_eq!(split(b"=x"), Some((Vec::new(), b"x".to_vec())));
    assert_eq!(split(b"NOEQ"), None);
    assert_eq!(split(b""), None);
}

#[test]
fn gives_the_last_entry_of_a_key_as_the_erts_copy_does() {
    // Measured on ERTS 29.1.1: `erlexec` started with the raw environment
    // `PBEAM_DUP=first`, `PBEAM_DUP=second`, and `=lead=x` gives "second"
    // for os:getenv("PBEAM_DUP") and "lead=x" for os:getenv(""). A
    // `Command` cannot make a duplicate entry, and the `erl` script removes
    // these entries, so this test gives the raw entries to `set`, which
    // `import` runs over the entries of `environ`.
    let mut entries = Vec::new();
    for (key, value) in [&b"A=1"[..], b"A=2", b"=lead=x", b"NOEQ", b"=x"]
        .into_iter()
        .filter_map(split)
    {
        set(&mut entries, &key, value);
    }
    assert_eq!(
        entries,
        [
            (b"A".to_vec(), b"2".to_vec()),
            (b"".to_vec(), b"x".to_vec())
        ]
    );
}

#[test]
fn changes_its_own_copy_and_not_the_environment_of_the_process() {
    // The copy of the run holds the environment of the process first: the
    // `PATH` of each test process is not empty.
    assert!(variable(b"PATH").is_some_and(|path| !path.is_empty()));
    assert_eq!(variable(KEY), None);
    put(KEY, b"first");
    assert_eq!(variable(KEY), Some(b"first".to_vec()));
    // A later `put` of a known key replaces the value in its place, as
    // `erts_osenv_put_native` does (`erl_osenv.c:360-385`).
    put(KEY, b"second");
    assert_eq!(variable(KEY), Some(b"second".to_vec()));
    assert_eq!(
        variables()
            .into_iter()
            .filter(|(key, _)| key == KEY)
            .count(),
        1
    );
    assert_eq!(std::env::var_os(std::ffi::OsStr::from_bytes(KEY)), None);
    unset(KEY);
    assert_eq!(variable(KEY), None);
    unset(KEY);
}

#[test]
fn sets_the_path_of_erlexec_with_no_path() {
    assert_eq!(erlexec_path(None, b"/b", b"/r"), b"/b:/r/bin");
}

#[test]
fn puts_the_erlexec_directories_before_a_path_without_the_root() {
    assert_eq!(
        erlexec_path(Some(b"/usr/bin:/bin"), b"/b", b"/r"),
        b"/b:/r/bin:/usr/bin:/bin"
    );
}

#[test]
fn moves_the_bindir_first_in_a_path_with_the_root() {
    // `erlexec.c:579-619`: each element that is `bindir` is dropped, and
    // `bindir` is the first element.
    assert_eq!(
        erlexec_path(Some(b"/r/bin:/b:/x:/b"), b"/b", b"/r"),
        b"/b:/r/bin:/x"
    );
    assert_eq!(erlexec_path(Some(b"/x"), b"/b", b""), b"/b:/x");
}
