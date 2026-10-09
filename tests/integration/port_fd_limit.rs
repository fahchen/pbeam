use std::os::fd::{FromRawFd, OwnedFd};

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PORT_PROBE: &[u8] = include_bytes!("../../fixtures/port/port_probe.beam");

/// The first fd that `select` cannot wait for.
const FD_SETSIZE: i32 = libc::FD_SETSIZE as i32;

#[test]
fn gives_emfile_when_a_pipe_fd_is_not_below_fd_setsize() {
    // This test is the only test of its crate, because it fills the fd
    // table of the test process. The scheduler waits for the pipes of a
    // port with `select`, so the parent end of the output pipe must be
    // below `FD_SETSIZE`. ERTS fails with `EMFILE` for a pipe fd at or
    // above its maximum number of files (sys_drivers.c:486-495); PBeam
    // takes `FD_SETSIZE` as that maximum (row "Ports" in
    // docs/erts-differences.md).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[PORT_PROBE, &erlang]).unwrap();
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` and `setrlimit` read and write only `limit`.
    unsafe {
        assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit), 0);
        limit.rlim_cur = limit.rlim_max.min(2 * libc::FD_SETSIZE as libc::rlim_t);
        assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit), 0);
    }
    assert!(
        limit.rlim_cur > libc::FD_SETSIZE as libc::rlim_t + 16,
        "{limit:?}"
    );
    let mut fds = Vec::new();
    loop {
        // SAFETY: `F_DUPFD_CLOEXEC` makes a new fd, which the `OwnedFd`
        // owns.
        let fd = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_DUPFD_CLOEXEC, 0) };
        assert!(fd >= 0);
        // SAFETY: `fd` is a new fd that nothing else owns.
        fds.push(unsafe { OwnedFd::from_raw_fd(fd) });
        if fd >= FD_SETSIZE - 1 {
            break;
        }
    }
    let result = program.execute("port_probe", "fd_limit", &[]);
    drop(fds);
    // `emfile` is a new atom of the run, so the program has no atom for it.
    assert_eq!(
        format!("{:?}", result.unwrap()),
        "Tuple([Atom(\"error\"), Atom(\"emfile\")])"
    );
}
