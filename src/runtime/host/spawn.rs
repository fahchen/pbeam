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
    ffi::{CString, OsStr, c_char, c_int, c_short},
    fs::{self, File},
    io::{IoSlice, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::ffi::OsStrExt,
    },
    thread,
};

use super::{
    errno::{errno, errno_code},
    input, output,
};

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

/// Which pipe directions a port keeps: the `in` and `out` options of
/// `open_port/2` as `read_write` holds them (`erl_bif_port.c:991-994`,
/// `1015-1016`). `in` sets `DO_READ`, `out` sets `DO_WRITE`, and without
/// both options the port keeps the two directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReadWrite {
    /// `DO_READ`: the port reads the output of the child (`in`).
    pub(crate) read: bool,
    /// `DO_WRITE`: the port writes the input of the child (`out`).
    pub(crate) write: bool,
}

impl ReadWrite {
    /// The two directions, which `open_port/2` keeps without the `in` and
    /// `out` options (`erl_bif_port.c:1015-1016`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(ReadWrite::both().read && ReadWrite::both().write);
    /// ```
    pub(crate) fn both() -> Self {
        Self {
            read: true,
            write: true,
        }
    }
}

/// The environment and the working directory of a child: the `{env, Env}`
/// and `{cd, Dir}` options of `open_port/2`.
#[derive(Debug, Default)]
pub(crate) struct Setting {
    /// The key and the value of each variable of the environment of the
    /// child (see [`super::environment::merged`]).
    pub(crate) environment: Vec<(Vec<u8>, Vec<u8>)>,
    /// The native path of `{cd, Dir}`, or `None` without it.
    pub(crate) directory: Option<Vec<u8>>,
}

/// Why [`spawn`] did not start the program.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SpawnError {
    /// A pipe or the access check failed with this `errno` before the
    /// start, as `spawn_start` returns `ERL_DRV_ERROR_ERRNO`
    /// (`sys_drivers.c:484-526`). `open_port/2` raises the `erl_errno_id`
    /// atom of the value.
    Open(c_int),
}

/// A program that [`spawn`] started, with the parent ends of its pipes.
/// The port owns it. Dropping it closes the pipes, as the `stop` callback
/// of the spawn driver closes them (`sys_drivers.c:1039-1056`); the child
/// then reads the end of file of its input. The waiter thread keeps the
/// write end of the status pipe until the child exits.
#[derive(Debug)]
pub(crate) struct Child {
    /// The OS pid of the child, as `port_info/1` gives it in `os_pid`. It
    /// is 0 when the start failed before the program ran, where ERTS gives
    /// the pid of the child that failed (`sys_drivers.c:1331`); row "Ports"
    /// of `docs/erts-differences.md` records it.
    pub(crate) os_pid: libc::pid_t,
    /// The write end of the input pipe, which is fd 0 or 3 of the child,
    /// or `None` with the `in` option: ERTS closes the parent end after
    /// the handshake (`sys_drivers.c:1320-1329`), and the child keeps the
    /// standard input of the node (`erl_child_setup.c:317-321`,
    /// `334-337`). A write returns `EBADF`, as the `writev` of the driver
    /// on its closed fd does (`sys_drivers.c:1145`, `1177`).
    input: Option<OwnedFd>,
    /// The read end of the output pipe, which is fd 1 or 4 of the child
    /// and fd 2 with `stderr_to_stdout`, or `None` with the `out` option:
    /// ERTS closes the parent end after the handshake
    /// (`sys_drivers.c:1307-1311`), and the child keeps the standard
    /// output of the node (`erl_child_setup.c:323-327`, `330-333`).
    output: Option<OwnedFd>,
    /// The read end of the status pipe. The waiter thread writes the wait
    /// status of the child to it, as `erl_child_setup` reports a
    /// `SIGCHLD` to ERTS (`erl_child_setup.c:396-420`).
    status: OwnedFd,
}

/// Start the program at the native path `path` with the native arguments
/// `arguments`, as `spawn_start` (`sys_drivers.c:471-729`) and
/// `erl_child_setup` do for `spawn_executable`:
///
/// 1. Make the output pipe and the input pipe (`sys_drivers.c:484-495`).
///    ERTS fails with `EMFILE` for a pipe fd at or above its maximum number
///    of files (`sys_drivers.c:486-495`), the fd limit of the process, which
///    `pipe` also gives.
/// 2. Check that the path is executable with `access(X_OK)`
///    (`sys_drivers.c:518-526`).
/// 3. Start the child with `posix_spawn`: the pipes as its fds 0 and 1, or
///    3 and 4, the output pipe also as fd 2 with `stderr_to_stdout`, a new
///    session (`setsid`), an empty signal mask, `argv[0]` `program_name`, the
///    environment and the working directory of `setting`
///    (`erl_child_setup.c:290-369`). A directory that the child cannot
///    enter fails `posix_spawn` with the `errno` of `chdir`.
/// 4. Start the waiter thread, which reaps the child.
///
/// A path or an argument ends at its first byte 0, as the C string that
/// ERTS makes from a binary does (`erl_unicode.c:2087-2113`).
///
/// # Examples
///
/// ```rust,ignore
/// let streams = Streams { use_stdio: true, stderr_to_stdout: false };
/// let child = spawn(b"/bin/sh", b"sh", &[b"-c".to_vec(), b"echo hi".to_vec()], streams, &setting)?;
/// assert_eq!(spawn(b"/nonexistent", b"x", &[], streams, &setting).unwrap_err(), SpawnError::Open(libc::ENOENT));
/// ```
pub(crate) fn spawn(
    path: &[u8],
    program_name: &[u8],
    arguments: &[Vec<u8>],
    streams: Streams,
    read_write: ReadWrite,
    setting: &Setting,
) -> Result<Child, SpawnError> {
    start(path, program_name, arguments, streams, read_write, setting)
}

/// Start the command `command` of `open_port({spawn, Command}, Settings)`
/// as `spawn_start` and `erl_child_setup` do: run `/bin/sh` with the
/// arguments `sh`, `-c`, and `exec Command` (`sys_drivers.c:474-475`,
/// `528-544`; `erl_child_setup.c:364-366`). The other steps are the steps
/// of [`spawn`]. ERTS has no access check for `spawn`; the access check of
/// `/bin/sh` passes, because `/bin/sh` is executable.
///
/// # Examples
///
/// ```rust,ignore
/// let child = spawn_shell(b"echo hi", streams, &setting)?;
/// ```
pub(crate) fn spawn_shell(
    command: &[u8],
    streams: Streams,
    read_write: ReadWrite,
    setting: &Setting,
) -> Result<Child, SpawnError> {
    let line = [b"exec ".as_slice(), command].concat();
    start(
        b"/bin/sh",
        b"sh",
        &[b"-c".to_vec(), line],
        streams,
        read_write,
        setting,
    )
}

/// Start the program at the native path `path` with `argv[0]` `program_name` and
/// the other arguments `arguments`, as [`spawn`] describes.
fn start(
    path: &[u8],
    program_name: &[u8],
    arguments: &[Vec<u8>],
    streams: Streams,
    read_write: ReadWrite,
    setting: &Setting,
) -> Result<Child, SpawnError> {
    let (output, child_output) = pipe().map_err(SpawnError::Open)?;
    let (child_input, input) = pipe().map_err(SpawnError::Open)?;
    let (status, status_writer) = pipe().map_err(SpawnError::Open)?;
    let path = c_string(path);
    // SAFETY: `path` is a NUL-terminated string.
    os_result(unsafe { libc::access(path.as_ptr(), libc::X_OK) }).map_err(SpawnError::Open)?;
    // The child setup program enters the working directory before it
    // moves the pipes: it prints the `spawn` line and exits with the
    // `errno` of the failed `chdir` (`erl_child_setup.c:300-306`), so the
    // port opens, its pipes close, and `exit_status` gives the `errno`.
    if let Some((directory, errno)) = setting
        .directory
        .as_deref()
        .and_then(|directory| enter_error(directory).map(|errno| (directory, errno)))
    {
        let mut stderr = std::io::stderr();
        let _ = stderr.write_all(b"spawn: Could not cd to ");
        let _ = stderr.write_all(directory);
        let _ = stderr.write_all(b"\r\n");
        let _ = stderr.flush();
        drop(child_output);
        drop(child_input);
        return Ok(failed(
            errno,
            read_write,
            input,
            output,
            status,
            status_writer,
        ));
    }
    let directory = setting.directory.as_deref().map(c_string);
    let environment: Vec<CString> = setting
        .environment
        .iter()
        .map(|(key, value)| c_string(&[key.as_slice(), b"=", value].concat()))
        .collect();
    let mut envp: Vec<*const c_char> = environment.iter().map(|entry| entry.as_ptr()).collect();
    envp.push(std::ptr::null());
    let program_name = c_string(program_name);
    let arguments: Vec<CString> = arguments.iter().map(|arg| c_string(arg)).collect();
    let mut argv: Vec<*const c_char> = [program_name.as_ptr()]
        .into_iter()
        .chain(arguments.iter().map(|arg| arg.as_ptr()))
        .collect();
    argv.push(std::ptr::null());
    let (stdin, stdout) = if streams.use_stdio { (0, 1) } else { (3, 4) };
    // The child setup program moves the input pipe to the standard input
    // or fd 3 only with `out`, and the output pipe to the standard output
    // or fd 4 only with `in`; `stderr_to_stdout` moves the output pipe to
    // fd 2 in each direction (`erl_child_setup.c:315-345`).
    let mut targets = Vec::new();
    if read_write.read {
        targets.push((child_output.as_raw_fd(), stdout));
    }
    if read_write.write {
        targets.push((child_input.as_raw_fd(), stdin));
    }
    if streams.stderr_to_stdout {
        targets.push((child_output.as_raw_fd(), libc::STDERR_FILENO));
    }
    // A failed `execve` makes the child exit with its `errno`
    // (`erl_child_setup.c:361-369`), so a `posix_spawn` that fails in the
    // child gives the pipes and the status of a child that died at once.
    let os_pid = match posix_spawn(&path, &argv, &envp, directory.as_ref(), &targets) {
        Ok(os_pid) => os_pid,
        Err(errno) => {
            drop(child_output);
            drop(child_input);
            return Ok(failed(
                errno,
                read_write,
                input,
                output,
                status,
                status_writer,
            ));
        }
    };
    drop(child_output);
    drop(child_input);
    let status_writer = File::from(status_writer);
    thread::spawn(move || wait(os_pid, status_writer));
    Ok(Child {
        os_pid,
        // ERTS closes the parent end that its direction does not use
        // after the handshake (`sys_drivers.c:1320-1329`).
        input: read_write.write.then(|| nonblocking(input)),
        output: read_write.read.then(|| nonblocking(output)),
        status,
    })
}

/// Return the child of a start that ran no program: the child setup
/// program closed both pipes and exited with `errno` before the `execve`
/// (`erl_child_setup.c:302-306`, `361-369`). The port reads the end of
/// file of the output pipe with `in`, `exit_status` reads `errno` from
/// the status pipe, and a write fails with `EPIPE`, as a write to the
/// input pipe of a dead child does.
fn failed(
    errno: c_int,
    read_write: ReadWrite,
    input: OwnedFd,
    output: OwnedFd,
    status: OwnedFd,
    status_writer: OwnedFd,
) -> Child {
    // The wait status of `exit(errno & 0xff)`, which `WEXITSTATUS` reads.
    let wait_status = (errno & 0xff) << 8;
    let _ = File::from(status_writer).write_all(&wait_status.to_ne_bytes());
    Child {
        os_pid: 0,
        input: read_write.write.then(|| nonblocking(input)),
        output: read_write.read.then(|| nonblocking(output)),
        status,
    }
}

/// Return the `errno` with which `chdir` refuses the directory `dir`, or
/// `None` when the child can enter it, as the `chdir` of the child setup
/// program does (`erl_child_setup.c:302-306`): `metadata` resolves the
/// path as `chdir` does, a path that is not a directory gives `ENOTDIR`,
/// and `access` with `X_OK` gives the errno of a directory that the user
/// cannot search, with the rights of the user (root, ACLs).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(enter_error(b"/no/such/dir"), Some(libc::ENOENT));
/// assert_eq!(enter_error(b"/tmp"), None);
/// ```
fn enter_error(dir: &[u8]) -> Option<c_int> {
    match fs::metadata(OsStr::from_bytes(dir)) {
        Err(error) => Some(error.raw_os_error().unwrap_or(libc::ENOENT)),
        Ok(metadata) if !metadata.is_dir() => Some(libc::ENOTDIR),
        Ok(_) => nix::unistd::access(OsStr::from_bytes(dir), nix::unistd::AccessFlags::X_OK)
            .err()
            .map(errno_code),
    }
}

/// Start a port program of ERTS that PBeam runs in a thread, not as an
/// external program: make the pipes as [`spawn`] does, and run
/// `program(input, output)` in a new thread with the child ends of the
/// input and output pipes. `program` owns the input pipe. When `program`
/// returns its exit status, the thread closes the output pipe and writes the wait status of that exit
/// status to the status pipe. The `os_pid` of the port is the pid of the
/// node process.
///
/// # Examples
///
/// ```rust,ignore
/// let child = serve(move |input, output| gethost::serve(&args, input, output))?;
/// ```
pub(crate) fn serve(
    program: impl FnOnce(File, &mut File) -> i32 + Send + 'static,
) -> Result<Child, SpawnError> {
    let (output, child_output) = pipe().map_err(SpawnError::Open)?;
    let (child_input, input) = pipe().map_err(SpawnError::Open)?;
    let (status, status_writer) = pipe().map_err(SpawnError::Open)?;
    thread::spawn(move || {
        let mut output = File::from(child_output);
        let code = program(File::from(child_input), &mut output);
        drop(output);
        // The wait status of `exit(code)`, which `WEXITSTATUS` reads.
        let _ = File::from(status_writer).write_all(&((code & 0xff) << 8).to_ne_bytes());
    });
    Ok(Child {
        os_pid: std::process::id().try_into().unwrap_or(0),
        input: Some(nonblocking(input)),
        output: Some(nonblocking(output)),
        status,
    })
}

/// Return the parent end `fd` of a pipe of a port with `O_NONBLOCK`, as
/// `spawn_start` sets the read end of the output pipe and the write end of
/// the input pipe (`SET_NONBLOCKING`, `sys_drivers.c:497-498`). A write
/// takes what the pipe holds and returns at once. A read of an empty pipe
/// fails with `EAGAIN` and does not block the scheduler: a ready report
/// can name an fd that a port closed and a new port got again.
fn nonblocking(fd: OwnedFd) -> OwnedFd {
    // The flag change of a valid pipe fd does not fail.
    let _ = nix::fcntl::fcntl(
        &fd,
        nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
    );
    fd
}

impl Child {
    /// Return the fd that has input when the child writes output or closes
    /// its output, for the `select` of the scheduler, or `None` with the
    /// `out` option.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(child.output_fd() >= FIRST_PIPE_FD);
    /// ```
    pub(crate) fn output_fd(&self) -> Option<RawFd> {
        self.output.as_ref().map(std::os::fd::AsRawFd::as_raw_fd)
    }

    /// Return the fd that has input when the child exits, for the `select`
    /// of the scheduler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(child.status_fd() >= FIRST_PIPE_FD);
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
        // The port reads only with the `in` option, which keeps this end.
        let output = self
            .output_fd()
            .expect("a port reads a child with the in option");
        input::read(output, READ_SIZE)
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

    /// Write what the input pipe takes of `bytes` without a wait, and return
    /// the count: 0 when the pipe is full or a signal stops the write, as
    /// the driver writes its non-blocking fd (`sys_drivers.c:1102-1120`,
    /// `1473-1490`). A port with the `in` option fails with `EBADF`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(child.write_some(b"hi"), Ok(2));
    /// ```
    pub(crate) fn write_some(&self, bytes: &[u8]) -> Result<usize, c_int> {
        let Some(input) = &self.input else {
            return Err(libc::EBADF);
        };
        match nix::unistd::write(input, bytes) {
            Ok(count) => Ok(count),
            Err(nix::errno::Errno::EAGAIN | nix::errno::Errno::EINTR) => Ok(0),
            Err(errno) => Err(errno as c_int),
        }
    }

    /// Return the write end of the input pipe, or `None` with `in`.
    pub(crate) fn input_fd(&self) -> Option<c_int> {
        self.input.as_ref().map(AsRawFd::as_raw_fd)
    }
}

/// The fds of a port of `open_port({fd, In, Out}, Settings)`, as the fd
/// driver keeps them (`fd_start`, `sys_drivers.c:817-980`). The port does
/// not own them: they stay open after the port closes (`fd_stop`,
/// `sys_drivers.c:1002-1027`).
#[derive(Debug)]
pub(crate) struct Fds {
    /// The fd that the port reads.
    pub(crate) input: RawFd,
    /// The fd that the port writes.
    pub(crate) output: RawFd,
}

impl Fds {
    /// Read `input` with one `read` of at most 64 KiB, as `ready_input`
    /// does (`sys_drivers.c:1336-1347`). Empty bytes are the end of file.
    pub(crate) fn read(&self) -> Result<Vec<u8>, c_int> {
        input::read(self.input, READ_SIZE)
    }

    /// Write all bytes of `bytes` to `output`. ERTS writes an fd that is
    /// not a tty from an async thread, which blocks as this write does
    /// (`sys_drivers.c:894-899`, `976-979`).
    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), c_int> {
        output::write_all(self.output, &mut [IoSlice::new(bytes)])
    }
}

/// Make a pipe and return its read end and its write end. Each end is
/// close-on-exec and at least `FIRST_PIPE_FD`.
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

/// Start `path` with `argv` and the environment `envp` in a new
/// session with an empty signal mask, in the working directory
/// `directory`, after each `dup2` of `targets`
/// (source fd, child fd) in order. Return the pid of the child, or the
/// `errno` that `posix_spawn` returns.
///
/// # Examples
///
/// ```rust,ignore
/// let pid = posix_spawn(&path, &argv, &envp, None, &[(child_output, 1), (child_input, 0)])?;
/// ```
fn posix_spawn(
    path: &CString,
    argv: &[*const c_char],
    envp: &[*const c_char],
    directory: Option<&CString>,
    targets: &[(RawFd, RawFd)],
) -> Result<libc::pid_t, c_int> {
    // SAFETY: `init` initializes the zeroed action list and attributes,
    // which `destroy` frees after the call. Each `dup2` source is an open
    // fd of this process. `directory` is a NUL-terminated string that
    // lives during the call. `argv` and `envp` are null-terminated
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
        if let Some(directory) = directory {
            libc::posix_spawn_file_actions_addchdir_np(&raw mut actions, directory.as_ptr());
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
            envp.as_ptr().cast(),
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

#[cfg(test)]
#[path = "../../../tests/runtime/host/spawn.rs"]
mod tests;
