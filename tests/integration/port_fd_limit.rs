use std::os::fd::{FromRawFd, OwnedFd};

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PORT_PROBE: &[u8] = include_bytes!("../../fixtures/port/port_probe.beam");

/// The first fd that one `fd_set` cannot hold.
const FD_SETSIZE: i32 = libc::FD_SETSIZE as i32;

#[test]
fn runs_a_port_whose_pipes_are_at_or_above_fd_setsize() {
    // This test is the only test of its crate, because it fills the fd
    // table of the test process. ERTS fails with `EMFILE` only for a pipe
    // fd at or above its maximum number of files (sys_drivers.c:486-495),
    // and its poll has no `FD_SETSIZE` limit (erl_poll.c:148-196). The
    // scheduler waits for the pipes with `select` sets that grow with the
    // largest fd, so a port with such pipes runs.
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
    let result = program.execute("port_probe", "high_fd", &[]);
    drop(fds);
    assert_eq!(
        result.unwrap().term,
        Term::tuple(vec![Term::binary(b"hi".to_vec()), Term::Integer(5)])
    );
}
