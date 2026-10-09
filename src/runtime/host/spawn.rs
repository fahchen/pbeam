//! Start an external program with pipes to its standard streams, wait for
//! its exit, read its output, and write its input, as the ERTS spawn driver
//! (`erts/emulator/sys/unix/sys_drivers.c`) and its child setup program
//! (`erts/emulator/sys/unix/erl_child_setup.c`) do for
//! `open_port({spawn_executable, Name}, Settings)`.
//!
//! ERTS forks each child in `erl_child_setup`, a separate program that
//! ERTS starts at boot, and gets the exit status of each child from it.
//! PBeam starts the child with `posix_spawn` and waits for its exit status
//! in one waiter thread for each child.

use std::{
    ffi::{CString, c_char, c_int, c_short},
    fs::File,
    io::{IoSlice, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    thread,
};

use super::{environment, errno::errno, input, output};

/// The size of one read of the output pipe, as `ERTS_SYS_READ_BUF_SZ`
/// (`sys_drivers.c:165`, `1336-1347`).
const READ_SIZE: usize = 64 * 1024;

/// The lowest fd of the child ends of the pipes in the parent. The child
/// gets its pipes as fds 0 and 1, or 3 and 4 (`erl_child_setup.c:315-336`),
/// and a `dup2` to one of these fds must not replace the source of a later
/// `dup2`.
const FIRST_PIPE_FD: c_int = 5;

/// The `posix_spawn` flag that makes the child the leader of a new session,
/// as the `setsid` of `erl_child_setup.c:347-352`. macOS defines it in
/// `<sys/spawn.h>` (`POSIX_SPAWN_SETSID` is `0x0400`), and the `libc` crate
/// has no constant for it.
#[cfg(target_os = "macos")]
const POSIX_SPAWN_SETSID: c_short = 0x0400;
/// The `posix_spawn` flag that makes the child the leader of a new session,
/// as the `setsid` of `erl_child_setup.c:347-352`.
#[cfg(not(target_os = "macos"))]
const POSIX_SPAWN_SETSID: c_short = libc::POSIX_SPAWN_SETSID;

/// How the child gets its pipes: the `use_stdio`, `nouse_stdio`, and
/// `stderr_to_stdout` options of `open_port/2`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Streams {
    /// `use_stdio`: the input pipe is fd 0 of the child, and the output
    /// pipe is fd 1. Else (`nouse_stdio`) they are fds 3 and 4
    /// (`erl_child_setup.c:315-336`).
    pub(crate) use_stdio: bool,
    /// `stderr_to_stdout`: fd 2 of the child is the output pipe. Else the
    /// child keeps fd 2 of the parent (`sys_drivers.c:500`,
    /// `erl_child_setup.c:340-345`).
    pub(crate) stderr_to_stdout: bool,
}

/// Why [`spawn`] did not start the program.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SpawnError {
    /// A pipe, the access check, or the fd limit failed with this `errno`
    /// before the start, as `spawn_start` returns `ERL_DRV_ERROR_ERRNO`
    /// (`sys_drivers.c:484-526`). `open_port/2` raises the `erl_errno_id`
    /// atom of the value.
    Open(c_int),
    /// `posix_spawn` failed with this `errno`: the fork or the `execve` of
    /// the program failed. ERTS starts the port first, and the child exits
    /// with the `errno` of the failed `execve` as its status
    /// (`erl_child_setup.c:361-369`).
    Start(c_int),
}

/// A program that [`spawn`] started, with the parent ends of its pipes.
/// The port owns it. Dropping it closes the pipes, as the `stop` callback
/// of the spawn driver closes them (`sys_drivers.c:1039-1056`); the child
/// then reads the end of file of its input. The waiter thread keeps the
/// write end of the status pipe until the child exits.
#[derive(Debug)]
pub(crate) struct Child {
    /// The OS pid of the child, as `port_info/1` gives it in `os_pid`.
    pub(crate) os_pid: libc::pid_t,
    /// The write end of the input pipe, which is fd 0 or 3 of the child.
    input: OwnedFd,
    /// The read end of the output pipe, which is fd 1 or 4 of the child,
    /// and fd 2 with `stderr_to_stdout`.
    output: OwnedFd,
    /// The read end of the status pipe. The waiter thread writes the wait
    /// status of the child to it, as `erl_child_setup` reports a
    /// `SIGCHLD` to ERTS (`erl_child_setup.c:396-420`).
    status: OwnedFd,
}

/// Start the program at the native path `path` with the native arguments
/// `args`, as `spawn_start` (`sys_drivers.c:471-729`) and
/// `erl_child_setup` do for `spawn_executable`:
///
/// 1. Make the output pipe and the input pipe (`sys_drivers.c:484-495`).
///    The parent end of the output pipe must be below `FD_SETSIZE`, because
///    the scheduler waits for it with `select`; else the start fails with
///    `EMFILE`, as ERTS fails for an fd at or above its maximum number of
///    files (`sys_drivers.c:486-495`).
/// 2. Check that the path is executable with `access(X_OK)`
///    (`sys_drivers.c:518-526`).
/// 3. Start the child with `posix_spawn`: the pipes as its fds 0 and 1, or
///    3 and 4, the output pipe also as fd 2 with `stderr_to_stdout`, a new
///    session (`setsid`), an empty signal mask, `argv[0]` the path, and the
///    environment of the process (`erl_child_setup.c:310-369`).
/// 4. Start the waiter thread, which reaps the child.
///
/// A path or an argument ends at its first byte 0, as the C string that
/// ERTS makes from a binary does (`erl_unicode.c:2087-2113`).
///
/// # Examples
///
/// ```rust,ignore
/// let streams = Streams { use_stdio: true, stderr_to_stdout: false };
/// let child = spawn(b"/bin/sh", &[b"-c".to_vec(), b"echo hi".to_vec()], streams)?;
/// assert_eq!(spawn(b"/nonexistent", &[], streams).unwrap_err(), SpawnError::Open(libc::ENOENT));
/// ```
pub(crate) fn spawn(path: &[u8], args: &[Vec<u8>], streams: Streams) -> Result<Child, SpawnError> {
    let (output, child_output) = pipe().map_err(SpawnError::Open)?;
    let (child_input, input) = pipe().map_err(SpawnError::Open)?;
    let (status, status_writer) = pipe().map_err(SpawnError::Open)?;
    let path = c_string(path);
    // SAFETY: `path` is a NUL-terminated string.
    os_result(unsafe { libc::access(path.as_ptr(), libc::X_OK) }).map_err(SpawnError::Open)?;
    let args: Vec<CString> = args.iter().map(|arg| c_string(arg)).collect();
    let mut argv: Vec<*const c_char> = [path.as_ptr()]
        .into_iter()
        .chain(args.iter().map(|arg| arg.as_ptr()))
        .collect();
    argv.push(std::ptr::null());
    let (stdin, stdout) = if streams.use_stdio { (0, 1) } else { (3, 4) };
    let mut targets = vec![
        (child_output.as_raw_fd(), stdout),
        (child_input.as_raw_fd(), stdin),
    ];
    if streams.stderr_to_stdout {
        targets.push((child_output.as_raw_fd(), libc::STDERR_FILENO));
    }
    let os_pid = posix_spawn(&path, &argv, &targets).map_err(SpawnError::Start)?;
    let status_writer = File::from(status_writer);
    thread::spawn(move || wait(os_pid, status_writer));
    Ok(Child {
        os_pid,
        input,
        output,
        status,
    })
}

impl Child {
    /// Return the fd that has input when the child writes output or closes
    /// its output, for the `select` of the scheduler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(child.output_fd() < libc::FD_SETSIZE as c_int);
    /// ```
    pub(crate) fn output_fd(&self) -> RawFd {
        self.output.as_raw_fd()
    }

    /// Return the fd that has input when the child exits, for the `select`
    /// of the scheduler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(child.status_fd() < libc::FD_SETSIZE as c_int);
    /// ```
    pub(crate) fn status_fd(&self) -> RawFd {
        self.status.as_raw_fd()
    }

    /// Read the output of the child with one `read` of at most 64 KiB, as
    /// `ready_input` does (`sys_drivers.c:1336-1347`). Empty bytes are the
    /// end of file. Return the `errno` of a failed `read`, for example
    /// `EINTR`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The child ran `printf hi`.
    /// assert_eq!(child.read_output(), Ok(b"hi".to_vec()));
    /// assert_eq!(child.read_output(), Ok(Vec::new()));
    /// ```
    pub(crate) fn read_output(&self) -> Result<Vec<u8>, c_int> {
        input::read(self.output_fd(), READ_SIZE)
    }

    /// Read the exit status of the child after the status pipe has input,
    /// as `port_inp_failure` makes it from the wait status
    /// (`sys_drivers.c:1242-1250`): `128 + Signal` when a signal ended the
    /// child, else its exit code. Return `None` when the waiter thread got
    /// no wait status: the host reaped the child, because the disposition
    /// of `SIGCHLD` is `SIG_IGN` after `os:set_signal(sigchld, ignore)`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The child ran `exit 3`.
    /// assert_eq!(child.read_status(), Some(3));
    /// // The child ran `kill -9 $$`.
    /// assert_eq!(child.read_status(), Some(137));
    /// ```
    ///
    /// # Panics
    ///
    /// Panic when the status pipe holds a part of a wait status. The waiter
    /// thread writes the 4 bytes of the status with one `write` before it
    /// closes the pipe.
    pub(crate) fn read_status(&self) -> Option<i32> {
        let bytes = input::read(self.status_fd(), 4).expect("the status pipe is open");
        if bytes.is_empty() {
            return None;
        }
        let status = c_int::from_ne_bytes(bytes.try_into().expect("the waiter writes 4 bytes"));
        Some(if libc::WIFSIGNALED(status) {
            128 + libc::WTERMSIG(status)
        } else {
            libc::WEXITSTATUS(status)
        })
    }

    /// Write all bytes of `bytes` to the input of the child, as `output`
    /// of the spawn driver writes them with `writev` (`sys_drivers.c:1140-1203`).
    /// The pipe is blocking, so the write returns when the child has read
    /// the bytes that do not fit in the pipe. Return the `errno` of a
    /// failed write, for example `EPIPE` when the child closed its input.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(child.write_input(b"hello"), Ok(()));
    /// ```
    pub(crate) fn write_input(&self, bytes: &[u8]) -> Result<(), c_int> {
        output::write_all(self.input.as_raw_fd(), &mut [IoSlice::new(bytes)])
    }
}

/// Make a pipe and return its read end and its write end. Each end is
/// close-on-exec and at least `FIRST_PIPE_FD`. The read end must be below
/// `FD_SETSIZE`, else the result is `EMFILE`. The scheduler waits with
/// `select` for the read ends of the output pipe and the status pipe; the
/// check also covers the read end of the input pipe, which only the child
/// reads.
///
/// # Examples
///
/// ```rust,ignore
/// let (reader, writer) = pipe()?;
/// assert!(reader.as_raw_fd() >= FIRST_PIPE_FD);
/// ```
fn pipe() -> Result<(OwnedFd, OwnedFd), c_int> {
    let (reader, writer) = std::io::pipe().map_err(|error| error.raw_os_error().unwrap_or(0))?;
    let reader = above_child_fds(&OwnedFd::from(reader))?;
    let writer = above_child_fds(&OwnedFd::from(writer))?;
    let limit = c_int::try_from(libc::FD_SETSIZE).expect("FD_SETSIZE fits in an fd");
    if reader.as_raw_fd() >= limit {
        return Err(libc::EMFILE);
    }
    Ok((reader, writer))
}

/// Return a close-on-exec copy of `fd` at `FIRST_PIPE_FD` or above.
///
/// # Examples
///
/// ```rust,ignore
/// let copy = above_child_fds(&fd)?;
/// assert!(copy.as_raw_fd() >= FIRST_PIPE_FD);
/// ```
fn above_child_fds(fd: &OwnedFd) -> Result<OwnedFd, c_int> {
    // SAFETY: `F_DUPFD_CLOEXEC` makes a new fd and changes no other fd.
    let copy =
        os_result(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, FIRST_PIPE_FD) })?;
    // SAFETY: `fcntl` returned a new fd, which nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
}

/// Return `result`, or the `errno` of a host call that returned -1.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(os_result(3), Ok(3));
/// ```
fn os_result(result: c_int) -> Result<c_int, c_int> {
    if result < 0 { Err(errno()) } else { Ok(result) }
}

/// Return the bytes of `bytes` before the first byte 0 as a C string.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(c_string(b"/bin/sh\0x"), CString::new("/bin/sh")?);
/// ```
fn c_string(bytes: &[u8]) -> CString {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    CString::new(&bytes[..end]).expect("the bytes before the first 0 have no 0")
}

/// Start `path` with `argv` and the environment of the process in a new
/// session with an empty signal mask, after each `dup2` of `targets`
/// (source fd, child fd) in order. Return the pid of the child, or the
/// `errno` that `posix_spawn` returns.
///
/// # Examples
///
/// ```rust,ignore
/// let pid = posix_spawn(&path, &argv, &[(child_output, 1), (child_input, 0)])?;
/// ```
fn posix_spawn(
    path: &CString,
    argv: &[*const c_char],
    targets: &[(RawFd, RawFd)],
) -> Result<libc::pid_t, c_int> {
    // SAFETY: `init` initializes the zeroed action list and attributes,
    // which `destroy` frees after the call. Each `dup2` source is an open
    // fd of this process. `argv` and the environment are null-terminated
    // arrays of NUL-terminated strings that live during the call, and
    // `posix_spawn` writes the pid only to `pid`.
    unsafe {
        let mut actions: libc::posix_spawn_file_actions_t = std::mem::zeroed();
        let mut attributes: libc::posix_spawnattr_t = std::mem::zeroed();
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::posix_spawn_file_actions_init(&raw mut actions);
        libc::posix_spawnattr_init(&raw mut attributes);
        for &(source, target) in targets {
            libc::posix_spawn_file_actions_adddup2(&raw mut actions, source, target);
        }
        libc::sigemptyset(&raw mut mask);
        libc::posix_spawnattr_setsigmask(&raw mut attributes, &raw const mask);
        let flags = POSIX_SPAWN_SETSID
            | c_short::try_from(libc::POSIX_SPAWN_SETSIGMASK).expect("a flag fits in c_short");
        libc::posix_spawnattr_setflags(&raw mut attributes, flags);
        let mut pid = 0;
        let result = libc::posix_spawn(
            &raw mut pid,
            path.as_ptr(),
            &raw const actions,
            &raw const attributes,
            argv.as_ptr().cast(),
            environment::environ().cast(),
        );
        libc::posix_spawn_file_actions_destroy(&raw mut actions);
        libc::posix_spawnattr_destroy(&raw mut attributes);
        if result == 0 { Ok(pid) } else { Err(result) }
    }
}

/// Wait until the child `pid` exits, and write its wait status to
/// `status`, as `erl_child_setup` reports the exit of a child
/// (`erl_child_setup.c:396-420`). The thread reaps the child, so it does
/// not stay a zombie. When `waitpid` fails, the thread writes nothing, and
/// the status pipe gives its end of file: with the disposition `SIG_IGN`
/// for `SIGCHLD`, the host reaps the child and `waitpid` fails with
/// `ECHILD`. ERTS reaps its children in `erl_child_setup`, a separate
/// program that the disposition of the BEAM process does not change.
///
/// # Examples
///
/// ```rust,ignore
/// thread::spawn(move || wait(pid, status_writer));
/// ```
fn wait(pid: libc::pid_t, mut status: File) {
    let mut wait_status = 0;
    // SAFETY: `waitpid` writes only to `wait_status`.
    let waited = std::iter::repeat_with(|| unsafe { libc::waitpid(pid, &raw mut wait_status, 0) })
        .find(|&result| result >= 0 || errno() != libc::EINTR)
        .is_some_and(|result| result >= 0);
    if waited {
        // The port can close the read end before the child exits.
        let _ = status.write_all(&wait_status.to_ne_bytes());
    }
}
