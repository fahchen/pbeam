use super::{ReadWrite, enter_error};

/// A directory of this test whose permission bits have no execute bit, so
/// only this test reads it.
const NO_EXECUTE: &str = "/tmp/pbeam_spawn_no_execute";

#[test]
fn reads_the_directions_of_the_in_and_out_options() {
    // `read_write` starts with no direction, and `open_port/2` keeps both
    // without the `in` and `out` options (`erl_bif_port.c:991-994`,
    // `1015-1016`).
    let both = ReadWrite::both();
    assert!(both.read && both.write);
    let mut set = ReadWrite {
        read: false,
        write: false,
    };
    set.read = true;
    assert!(set.read && !set.write);
    set.write = true;
    assert!(set.read && set.write);
}

#[test]
fn gives_the_errno_of_a_directory_that_chdir_refuses() {
    // The `chdir` of the child setup program exits with its `errno`
    // (`erl_child_setup.c:302-306`): a missing path gives `enoent`, a file
    // gives `enotdir`, and a directory without an execute bit gives
    // `eacces`.
    assert_eq!(enter_error(b"/pbeam_no_such_directory"), Some(libc::ENOENT));
    assert_eq!(enter_error(b"/etc/hosts"), Some(libc::ENOTDIR));
    std::fs::create_dir(NO_EXECUTE).ok();
    let metadata = std::fs::metadata(NO_EXECUTE).unwrap();
    let mut permissions = metadata.permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o600);
    std::fs::set_permissions(NO_EXECUTE, permissions).unwrap();
    assert_eq!(enter_error(NO_EXECUTE.as_bytes()), Some(libc::EACCES));
    assert_eq!(enter_error(b"/tmp"), None);
    std::fs::remove_dir(NO_EXECUTE).unwrap();
}
