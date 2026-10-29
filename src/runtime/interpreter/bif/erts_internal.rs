//! Bodies of the `erts_internal` BIFs that do not fit on one table line.

use std::cmp::Ordering;

use super::BifResult::Value;
use super::erlang::signals_return;
use super::{
    AtomTable, BeamError, BifContext, BifResult, ExceptionClass, ProcessId, Scheduler,
    SpawnOptions, Target, Term, boot_denial, code, create_process, denial, error, iolist, maps,
    other_process_denial, print_term, resolve,
};
use crate::beam::code::PrepareError;
use crate::runtime::process::LinkId;
use crate::runtime::scheduler::{Accept, PTAB_SIZE, check_process_code_reply};
use crate::runtime::{
    capability::{CapabilityKind, Operation},
    host::{
        environment,
        errno::errno_id,
        filename_encoding, gethost,
        spawn::{self, Fds, ReadWrite, Setting, SpawnError, Streams},
    },
    port::{
        self, Driver, Input, Lines, Packets, Port, PortCell, Program, Spawn,
        inet::{Control, Event, Inet, OK, Output, SocketType},
    },
    signal::Signal,
    unicode::native_filename,
};
use crate::term::PortId;

/// Run `erts_internal:group_leader(Leader, Target)`, which
/// `erlang:group_leader/2` in `erlang.beam` calls, as
/// `erts_internal_group_leader_2` (`bif.c:4914-4937`) does: set the group
/// leader of the local process `Target` to `Leader` at once and return
/// `true`. Return `false` when `Target` is another process with signals or
/// messages that it did not handle (`erts_try_lock_sig_free_proc`,
/// `erl_process.c:14835-14883`); `erlang:group_leader/2` then sends the
/// change as a signal through `erts_internal:group_leader/3`. Return the
/// atom `badarg` without an exception when `Leader` is not a pid, when
/// `Target` is not a pid or no process has it, and when `Target` exits
/// (`erts_set_group_leader`, `bif.c:4860-4870`). For another process, the
/// caller needs the `process` capability with `group_leader` after the pid
/// checks, else the BIF raises `error:capability_denied` before it looks up
/// the process.
///
/// # Examples
///
/// ```rust,ignore
/// let result = group_leader(&mut context, &Term::Pid(leader), &Term::Pid(target))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("true"));
/// let result = group_leader(&mut context, &Term::Integer(1), &Term::Pid(target))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("badarg"));
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for a full atom table.
pub(super) fn group_leader(
    context: &mut BifContext<'_>,
    leader: &Term,
    target: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Pid(leader), Term::Pid(target)) = (leader, target) else {
        return Ok(Value(Term::static_atom("badarg")));
    };
    let scheduler = &mut *context.scheduler;
    if let Some(denied) = other_process_denial(scheduler, *target, Operation::GroupLeader)? {
        return Ok(denied);
    }
    let caller = scheduler.current_process_mut().id;
    // A live process of another scheduler takes the routed request.
    let remote = scheduler.remote_owner(*target).is_some();
    let result = match scheduler.process_mut(*target) {
        None if remote => "false",
        None => "badarg",
        Some(process) if *target != caller && process.mailbox.has_incoming() => "false",
        Some(process) if process.exiting.is_some() => "badarg",
        Some(process) => {
            process.group_leader = *leader;
            "true"
        }
    };
    Ok(Value(Term::static_atom(result)))
}

/// Run `erts_internal:group_leader(Leader, Target, Ref)` as
/// `erts_internal_group_leader_3` (`bif.c:4898-4912`) does: send a
/// group-leader signal to the local process `Target` and return `ok`. The
/// target replies `{Ref, true}` when it handles the signal, or
/// `{Ref, badarg}` when it exits first. A target that no process has
/// gives the caller `{Ref, badarg}` at once
/// (`erts_proc_sig_send_group_leader`, `erl_proc_sig_queue.c:2970-3025`).
/// `Leader` that is not a pid, `Target` that is not a pid, and `Ref` that is
/// not a reference raise `badarg`. For another process, the caller then
/// needs the `process` capability with `group_leader`, else the BIF raises
/// `error:capability_denied` and sends no signal and no reply.
///
/// # Examples
///
/// ```rust,ignore
/// let result = group_leader_request(&mut context, &Term::Pid(leader), &Term::Pid(target), &Term::Reference(reference))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("ok"));
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for a full atom table.
pub(super) fn group_leader_request(
    context: &mut BifContext<'_>,
    leader: &Term,
    target: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Pid(leader), Term::Pid(target), Term::Reference(reference)) =
        (leader, target, reference)
    else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    if let Some(denied) = other_process_denial(scheduler, *target, Operation::GroupLeader)? {
        return Ok(denied);
    }
    let caller = scheduler.current_process_mut().id;
    let request = Signal::GroupLeader {
        leader: *leader,
        requester: caller,
        reference: *reference,
    };
    if scheduler
        .send_signal(Some(caller), *target, request)
        .is_err()
    {
        let reply = Term::tuple(vec![
            Term::Reference(*reference),
            Term::static_atom("badarg"),
        ]);
        scheduler.deliver(caller, reply);
    }
    Ok(Value(Term::static_atom("ok")))
}

/// Run `erts_internal:is_process_alive(Pid, Ref)`, which
/// `erts_internal:is_process_alive/1` calls, as
/// `erts_internal_is_process_alive_2` (`erl_bif_info.c:4049-4058`) does:
/// send an is-alive request to `Pid` and return `ok`. The process replies
/// `{Ref, true}` when it handles the request, or `{Ref, false}` when it
/// exits first. A pid that no process has gives the caller `{Ref, false}`
/// at once (`erts_proc_sig_send_is_alive_request`,
/// `erl_proc_sig_queue.c:3027-3073`), and then the result traps to
/// `erlang:bif_handle_signals_return/2` when the caller has incoming
/// signals. A term that is not a pid, and a term that is not a reference
/// or is an alias (a pid reference), also of an owner that exited, raise
/// `badarg`. For another process, the caller then needs the `process`
/// capability with `inspect`, else the BIF raises `error:capability_denied`
/// and sends no request and no reply.
///
/// # Examples
///
/// ```rust,ignore
/// let result = is_process_alive(&mut context, &Term::Pid(child), &Term::Reference(reference))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("ok"));
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for a full atom table.
pub(super) fn is_process_alive(
    context: &mut BifContext<'_>,
    pid: &Term,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Pid(target), Term::Reference(reference)) = (pid, reference) else {
        return Ok(BifResult::badarg());
    };
    if reference.is_pid_reference() {
        return Ok(BifResult::badarg());
    }
    let scheduler = &mut *context.scheduler;
    if let Some(denied) = other_process_denial(scheduler, *target, Operation::Inspect)? {
        return Ok(denied);
    }
    let caller = scheduler.current_process_mut().id;
    let request = Signal::IsAlive {
        requester: caller,
        reference: *reference,
    };
    if scheduler
        .send_signal(Some(caller), *target, request)
        .is_ok()
    {
        return Ok(Value(Term::static_atom("ok")));
    }
    let reply = Term::tuple(vec![
        Term::Reference(*reference),
        Term::static_atom("false"),
    ]);
    scheduler.deliver(caller, reply);
    Ok(signals_return(scheduler, pid, Term::static_atom("ok")))
}

/// The reductions of a port operation that ERTS runs at once: the output
/// of `port_command/2`, the exit of `port_close/1`, the control of
/// `port_control/3`, and the info of `port_info/1`, each `CONTEXT_REDS/100`
/// (`erl_port.h:369-370`, `378`, `380`; `io.c:2071`, `2282`, `4408`,
/// `4906`).
const PORT_REDUCTIONS: i32 = 40;

/// The tuple options of `open_port/2` that ERTS accepts and PBeam does not
/// support (`erl_bif_port.c:849-976`).
const UNSUPPORTED_TUPLE_OPTIONS: [&str; 2] = ["busy_limits_port", "busy_limits_msgq"];

/// The options of `open_port/2` that PBeam supports, as the fields of the
/// ERTS `SysDriverOpts` and the port flags that `open_port` sets
/// (`erl_bif_port.c:812-834`).
#[derive(Debug)]
struct PortSettings {
    /// `use_stdio` (the default) or `nouse_stdio`, and `stderr_to_stdout`.
    streams: Streams,
    /// `binary`: the data of a message is a binary.
    binary: bool,
    /// `exit_status`.
    exit_status: bool,
    /// `eof`: the end of file sends `{Port, eof}`, and the port stays open
    /// (`ERTS_PORT_SFLG_SOFT_EOF`, `erl_bif_port.c:995-996`).
    eof: bool,
    /// The header size of `{packet, N}`: 1, 2, or 4, or 0 without it
    /// (`packet_bytes`, `erl_bif_port.c:849-861`). A small integer N whose
    /// low 32 bits are 1, 2, or 4 also sets it.
    packet: usize,
    /// The low 32 bits of the maximum line size of `{line, L}`, 512 for
    /// `line` (`erl_bif_port.c:862-869`, `985-986`), or `None` without
    /// them.
    line: Option<i32>,
    /// The native bytes of the strings of `{args, Args}`, or `None`
    /// without it.
    args: Option<Vec<Vec<u8>>>,
    /// The native bytes of `{arg0, A}`, or `None` without it
    /// (`erl_bif_port.c:894-910`).
    arg0: Option<Vec<u8>>,
    /// The changes of the last `{env, Env}` in order: a key and its value,
    /// or a key without a value to remove (`erl_bif_port.c:870-880`,
    /// `1170-1211`).
    environment: Vec<environment::Change>,
    /// The `Dir` of the last `{cd, Dir}`, which `open_port` converts after
    /// the name (`erl_bif_port.c:911-912`, `1094-1098`).
    directory: Option<Term>,
    /// `{parallelism, Boolean}`, false without it (`erl_bif_port.c:913-919`).
    parallelism: bool,
    /// `DO_READ` of the `in` option and `DO_WRITE` of the `out` option
    /// (`erl_bif_port.c:991-994`); both without them (`erl_bif_port.c:1015-1016`).
    read_write: ReadWrite,
}

/// Run `erts_internal:open_port(PortName, PortSettings)`, which
/// `erlang:open_port/2` calls, as `erts_internal_open_port_2` and
/// `open_port` do (`erl_bif_port.c:60-130`, `795-1171`) for
/// `{spawn_executable, Name}` and `{spawn, Command}` (see [`open_driver_port`] for
/// `{spawn_driver, Name}`):
///
/// 1. Read the settings in order (see [`port_settings`]). A list that is
///    not proper, an element that ERTS does not accept, and
///    `stderr_to_stdout` with `nouse_stdio` give the atom `badopt`.
/// 2. Read the name. A term that is not `{spawn_executable, Name}` or
///    `{spawn, Command}`, a `Name` that is not a file name (see
///    [`native_name`]), and `{args, Args}` with `spawn` give the atom
///    `badarg`. A first word of `Command` that names a driver opens the
///    driver (see [`open_driver_port`]).
/// 3. Check the capabilities of the caller: without a `port` capability
///    with `spawn` that permits the native bytes of `Name` or `Command`,
///    the BIF raises
///    `error:capability_denied` and starts no program. The constraint
///    compares the name as the caller gives it. It does not resolve a
///    relative name or a symbolic link.
/// 4. Start the program (see [`spawn::spawn`] and [`spawn::spawn_shell`]). A program named
///    `inet_gethost` runs in a thread of the node (see [`is_gethost`]). A pipe, the fd limit, and
///    the access check give the `erl_errno_id` atom of their failure, for
///    example `enoent`.
/// 5. Open the port, owned by and linked to the caller, and return a new
///    reference. The caller gets `{Reference, Port}`, as the init ack of a
///    driver with `ERL_DRV_FLAG_USE_INIT_ACK` sends it
///    (`sys_drivers.c:251`, `1332`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = open_port(&mut context, &name, &settings)?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// An option that ERTS accepts and PBeam does not support, an fd that is
/// not open, and `{line, L}` whose low 32 bits are not positive return a
/// runtime error. A new atom in a full atom table gives the error
/// of `AtomTable::put`.
pub(super) fn open_port(
    context: &mut BifContext<'_>,
    name: &Term,
    settings: &Term,
) -> Result<BifResult, BeamError> {
    let Some(settings) = port_settings(&context.scheduler.atoms, settings)? else {
        return Ok(Value(Term::static_atom("badopt")));
    };
    let atoms = &context.scheduler.atoms;
    let has_argv = settings.args.is_some() || settings.arg0.is_some();
    let name = match port_name(atoms, name) {
        Some(name) if !has_argv || matches!(name, PortName::Executable(_)) => name,
        // An argument vector only with `spawn_executable`
        // (`erl_bif_port.c:1086-1092`).
        _ => return Ok(Value(Term::static_atom("badarg"))),
    };
    // `open_port` converts no `Dir` for `{cd, []}` (`erl_bif_port.c:1094-1098`).
    let directory = match settings
        .directory
        .as_ref()
        .filter(|directory| !directory.is_nil())
    {
        Some(directory) => match native_name(atoms, directory, false) {
            Some(directory) => Some(directory),
            None => return Ok(Value(Term::static_atom("badarg"))),
        },
        None => None,
    };
    let (path, args, is_command) = match name {
        PortName::Executable(path) => (path, settings.args.clone().unwrap_or_default(), false),
        PortName::Driver(driver) => {
            return open_driver_port(context, driver, &settings);
        }
        PortName::Fd(input, output) => {
            return open_fd_port(context, input, output, &settings);
        }
        PortName::Command(command) => {
            // The first word of the command names a driver, or the spawn
            // driver runs the command (`io.c:599-622`).
            if matches!(
                first_word(&command),
                b"tcp_inet" | b"udp_inet" | b"ram_file_drv" | b"fd"
            ) {
                return open_driver_port(context, command, &settings);
            }
            (command, Vec::new(), true)
        }
    };
    let scheduler = &mut *context.scheduler;
    // The check uses the port name, which the later port operations check.
    let check = denial(
        scheduler,
        CapabilityKind::Port,
        Operation::Spawn,
        Some(&path),
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let lines = settings.line.map(line_size).transpose()?;
    let setting = Setting {
        environment: environment::merged(&settings.environment),
        directory,
    };
    let started = if is_command {
        spawn::spawn_shell(&path, settings.streams, settings.read_write, &setting)
    } else if is_gethost(&path) {
        spawn::serve(move |input, output| gethost::serve(&args, input, output))
    } else {
        let arg0 = settings.arg0.as_deref().unwrap_or(&path);
        spawn::spawn(
            &path,
            arg0,
            &args,
            settings.streams,
            settings.read_write,
            &setting,
        )
    };
    let child = match started {
        Ok(child) => child,
        Err(SpawnError::Open(errno)) => {
            return Ok(Value(Term::Atom(scheduler.atoms.put(&errno_id(errno))?)));
        }
    };
    let spawn = Spawn {
        exit_status: settings.exit_status,
        soft_eof: settings.eof,
        packets: Packets::new(settings.packet),
        binary: settings.binary,
        lines,
        parallelism: settings.parallelism,
        program: Program::Child(child),
        // The `out` option keeps no read side, and the port reads nothing
        // (`sys_drivers.c:1307-1311`).
        input: if settings.read_write.read {
            Input::Open
        } else {
            Input::Write
        },
    };
    let port = scheduler.open_port(path, Driver::Spawn(spawn))?;
    let reference = scheduler.make_ref()?;
    let caller = scheduler.current_process_mut().id;
    let ack = Term::tuple(vec![Term::Reference(reference), Term::Port(port)]);
    scheduler.deliver(caller, ack);
    Ok(Value(Term::Reference(reference)))
}

/// Return true when the last component of the native path `path` is
/// `inet_gethost`, the port program that `inet_gethost_native` opens as
/// `filename:join(BinDir, "inet_gethost")` (inet_gethost_native.erl:412-421).
/// PBeam serves that program itself (see [`gethost::serve`]), without the
/// `access` check of the file and without the arguments: a bundle has no
/// `erts/bin` directory.
fn is_gethost(path: &[u8]) -> bool {
    path.rsplit(|&byte| byte == b'/').next() == Some(b"inet_gethost".as_slice())
}

/// Open the port of `{spawn_driver, Name}` with the driver name `driver`
/// and the settings `settings`, as `open_port` and `erts_open_driver` do
/// (`erl_bif_port.c:1043-1062`, `io.c:323-560`): open a `tcp_inet` or
/// `udp_inet` port, owned by and linked to the caller, and return it. These
/// drivers have no init ack, so the BIF returns the port, not a reference
/// (erlang.erl:7985-7993).
///
/// # Examples
///
/// ```rust,ignore
/// let result = open_driver_port(&mut context, b"tcp_inet".to_vec(), &settings)?;
/// assert!(matches!(result, BifResult::Value(Term::Port(_))));
/// ```
///
/// # Errors
///
/// A driver other than `tcp_inet` and `udp_inet`, a setting other than
/// `binary`, a port without `binary`, and an open by a runtime process in
/// a run with a policy return a runtime error: no capability kind permits a
/// network effect yet (rows "tcp_inet ports" and "udp_inet ports" of
/// `docs/erts-differences.md`). A boot process opens the port, because the
/// boot authority passes each check (row "Boot authority"), as the kernel
/// start opens a `udp_inet` port in `inet_config:set_hostname/0`.
fn open_driver_port(
    context: &mut BifContext<'_>,
    driver: Vec<u8>,
    settings: &PortSettings,
) -> Result<BifResult, BeamError> {
    // `exit_status` only with the spawn driver (`io.c:625-628`).
    if settings.exit_status {
        return Ok(Value(Term::static_atom("badarg")));
    }
    let socket_type = match first_word(&driver) {
        b"tcp_inet" => SocketType::Stream,
        b"udp_inet" => SocketType::Datagram,
        _ => {
            return Err(error(format!(
                "unsupported open_port/2 driver {}: see \"tcp_inet ports\" in docs/erts-differences.md",
                String::from_utf8_lossy(&driver)
            )));
        }
    };
    let only_binary = matches!(
        settings,
        PortSettings {
            streams: Streams {
                use_stdio: true,
                stderr_to_stdout: false,
            },
            binary: true,
            exit_status: false,
            eof: false,
            packet: 0,
            line: None,
            args: None,
            arg0: None,
            directory: None,
            parallelism: false,
            ..
        }
    ) && settings.environment.is_empty();
    if !only_binary {
        return Err(error(
            "unsupported open_port/2 settings of an inet driver port other than [binary]: see \"tcp_inet ports\" in docs/erts-differences.md",
        ));
    }
    let scheduler = &mut *context.scheduler;
    if !scheduler.skips_checks() && !scheduler.current_process().security.is_boot() {
        return Err(error(
            "unsupported inet driver port in a run with a policy: no capability permits a network effect: see \"tcp_inet ports\" in docs/erts-differences.md",
        ));
    }
    let port = scheduler.open_port(driver, Driver::Inet(Inet::new(socket_type)))?;
    Ok(Value(Term::Port(port)))
}

/// Return the bytes of `name` before its first space, the driver name that
/// `erts_open_driver` looks up for `spawn` and `spawn_driver` (`io.c:599-622`).
fn first_word(name: &[u8]) -> &[u8] {
    name.split(|&byte| byte == b' ').next().unwrap_or_default()
}

/// Open the port of `{fd, In, Out}` with the settings `settings`, as
/// `open_port` and `fd_start` do (`erl_bif_port.c:1067-1081`, `1100-1102`;
/// `sys_drivers.c:817-980`): the port reads `In` and writes `Out`, its name
/// is `"In/Out"`, and it is owned by and linked to the caller. The fd
/// driver has no init ack (`sys_drivers.c:277`), so the BIF returns the
/// port. `exit_status` gives the atom `badarg`. ERTS keeps each fd as a
/// `Uint` (`erl_bif_port.c:1074-1075`), and `fd_start` fails with `einval`
/// for an fd at or above the `int` maximum number of files
/// (`sys_drivers.c:822-824`, `sys.c:119`), so a negative fd or an fd above
/// `i32::MAX` gives the atom `einval`. The port does not close its fds
/// (`sys_drivers.c:1002-1027`).
///
/// # Errors
///
/// A port without `binary` and an fd that is not open return a runtime
/// error: ERTS opens a port on an fd that is not open and is below its
/// maximum number of files. A new atom in a full atom table gives the error
/// of `AtomTable::put`.
fn open_fd_port(
    context: &mut BifContext<'_>,
    input: i64,
    output: i64,
    settings: &PortSettings,
) -> Result<BifResult, BeamError> {
    if settings.exit_status {
        return Ok(Value(Term::static_atom("badarg")));
    }
    if !settings.binary {
        return Err(error(
            "unsupported open_port/2 without the binary option: see \"Ports\" in docs/erts-differences.md",
        ));
    }
    let (Ok(input), Ok(output)) = (i32::try_from(input as u64), i32::try_from(output as u64))
    else {
        return Ok(Value(Term::static_atom("einval")));
    };
    // `fd_start` refuses an fd at or above `sys_max_files()`, the soft
    // `RLIMIT_NOFILE` (sys_drivers.c:822-824), for each direction that
    // `in` or `out` keeps, which `open_port` gives as `einval`.
    let max = nix::sys::resource::getrlimit(nix::sys::resource::Resource::RLIMIT_NOFILE)
        .map_or(u64::MAX, |(soft, _)| soft);
    if (settings.read_write.read && input as u64 >= max)
        || (settings.read_write.write && output as u64 >= max)
    {
        return Ok(Value(Term::static_atom("einval")));
    }
    // ERTS opens a port on an fd that is not open: the select of the input
    // fails without a message, and a write fails with `ebadf`. An fd that
    // is not open has no entry in the fd directory of the process
    // (`/dev/fd`, which is `/proc/self/fd` on Linux).
    let closed = |fd| std::fs::symlink_metadata(format!("/dev/fd/{fd}")).is_err();
    let port_input = if !settings.read_write.read {
        // The `out` option keeps no read side (`sys_drivers.c:385-400`).
        Input::Write
    } else if closed(input) {
        Input::Ended
    } else {
        Input::Open
    };
    let name = format!("{input}/{output}").into_bytes();
    let scheduler = &mut *context.scheduler;
    let check = denial(
        scheduler,
        CapabilityKind::Port,
        Operation::Spawn,
        Some(&name),
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let spawn = Spawn {
        exit_status: false,
        soft_eof: settings.eof,
        packets: Packets::new(settings.packet),
        binary: true,
        lines: settings.line.map(line_size).transpose()?,
        parallelism: settings.parallelism,
        // The `in` option keeps no write side: a write fails with `ebadf`
        // (`sys_drivers.c:1145`, `1177`).
        program: Program::Fds(Fds {
            input: if settings.read_write.read { input } else { -1 },
            output: if settings.read_write.write {
                output
            } else {
                -1
            },
        }),
        input: port_input,
    };
    let port = scheduler.open_port(name, Driver::Spawn(spawn))?;
    Ok(Value(Term::Port(port)))
}

/// Read the settings of `open_port/2` in order, as `open_port` does
/// (`erl_bif_port.c:834-1022`). Return `None` for the `badopt` of ERTS: a
/// term that is not a proper list, an element that is not an option of
/// ERTS, an `{args, Args}` whose `Args` is not a proper list of file names
/// (`convert_args`, `erl_bif_port.c:1214-1246`), and `stderr_to_stdout`
/// with `nouse_stdio`. A later option replaces an earlier one.
///
/// # Examples
///
/// ```rust,ignore
/// let settings = port_settings(&list)?.unwrap();
/// assert!(settings.binary);
/// assert!(port_settings(&Term::static_atom("foo"))?.is_none());
/// ```
///
/// # Errors
///
/// An option that ERTS accepts and PBeam does not support returns a runtime
/// error, also when a later element is not valid.
fn port_settings(atoms: &AtomTable, settings: &Term) -> Result<Option<PortSettings>, BeamError> {
    let Some(elements) = settings.proper_list() else {
        return Ok(None);
    };
    let mut result = PortSettings {
        streams: Streams {
            use_stdio: true,
            stderr_to_stdout: false,
        },
        binary: false,
        exit_status: false,
        eof: false,
        packet: 0,
        line: None,
        args: None,
        arg0: None,
        environment: Vec::new(),
        directory: None,
        parallelism: false,
        read_write: ReadWrite {
            read: false,
            write: false,
        },
    };
    for element in elements {
        let option = match element {
            Term::Tuple(pair) if pair.len() == 2 => {
                let Term::Atom(option) = &pair[0] else {
                    return Ok(None);
                };
                if option.is_static("args") {
                    let Some(args) = port_args(atoms, &pair[1]) else {
                        return Ok(None);
                    };
                    result.args = Some(args);
                    continue;
                } else if option.is_static("packet") {
                    // ERTS keeps the small integer in the C `int`
                    // `packet_bytes`, which holds its low 32 bits
                    // (`erl_bif_port.c:850-861`).
                    match pair[1].small_integer().map(|size| size as i32) {
                        Some(size @ (1 | 2 | 4)) => result.packet = size as usize,
                        _ => return Ok(None),
                    }
                    continue;
                } else if option.is_static("line") {
                    // ERTS checks the small integer as an `Sint` and then
                    // keeps its low 32 bits in the C `int` `bufsiz`
                    // (`erl_bif_port.c:862-869`, `1139-1140`;
                    // `io.c:3149-3155`).
                    match pair[1].small_integer() {
                        Some(size) if size > 0 => result.line = Some(size as i32),
                        _ => return Ok(None),
                    }
                    continue;
                } else if option.is_static("env") {
                    // A later `env` replaces an earlier one
                    // (`erl_bif_port.c:870-880`).
                    let Some(changes) = environment_changes(&pair[1]) else {
                        return Ok(None);
                    };
                    result.environment = changes;
                    continue;
                } else if option.is_static("arg0") {
                    let Some(arg0) = native_name(atoms, &pair[1], true) else {
                        return Ok(None);
                    };
                    result.arg0 = Some(arg0);
                    continue;
                } else if option.is_static("cd") {
                    result.directory = Some(pair[1].clone());
                    continue;
                } else if option.is_static("parallelism") {
                    result.parallelism = match &pair[1] {
                        Term::Atom(value) if value.is_static("true") => true,
                        Term::Atom(value) if value.is_static("false") => false,
                        _ => return Ok(None),
                    };
                    continue;
                } else if option.is_one_of(&UNSUPPORTED_TUPLE_OPTIONS) {
                    option
                } else {
                    return Ok(None);
                }
            }
            Term::Atom(option) => {
                if option.is_static("use_stdio") {
                    result.streams.use_stdio = true;
                } else if option.is_static("nouse_stdio") {
                    result.streams.use_stdio = false;
                } else if option.is_static("stderr_to_stdout") {
                    result.streams.stderr_to_stdout = true;
                } else if option.is_static("binary") {
                    result.binary = true;
                } else if option.is_static("stream") {
                    result.packet = 0;
                } else if option.is_static("line") {
                    result.line = Some(512);
                } else if option.is_static("exit_status") {
                    result.exit_status = true;
                } else if option.is_static("in") {
                    result.read_write.read = true;
                } else if option.is_static("out") {
                    result.read_write.write = true;
                } else if option.is_static("eof") {
                    result.eof = true;
                } else if option.is_static("overlapped_io") {
                    // Only a Windows port uses it (`erl_sys_driver.h:56`).
                } else if option.is_static("hide") {
                    // Only a Windows port hides its window
                    // (`erl_bif_port.c:997-998`, `hide_window`).
                } else {
                    return Ok(None);
                }
                continue;
            }
            _ => return Ok(None),
        };
        return Err(unsupported_option(atoms.name_of(option)));
    }
    // Mutually exclusive settings (`erl_bif_port.c:1018-1022`).
    if (result.streams.stderr_to_stdout && !result.streams.use_stdio)
        || (result.line.is_some() && result.packet != 0)
    {
        return Ok(None);
    }
    // Without `in` and `out` the port reads and writes
    // (`erl_bif_port.c:1015-1016`).
    if !result.read_write.read && !result.read_write.write {
        result.read_write = ReadWrite::both();
    }
    Ok(Some(result))
}

/// Return the changes of the `Env` of `{env, Env}` in order, as
/// `merge_global_environment` reads them (`erl_bif_port.c:1183-1210`), or
/// `None` for the `badopt` of ERTS: `Env` is not a proper list, an element
/// is not a pair, or a key or a value is not valid. A key is a flat or
/// nested list of characters without `=` and without 0, as for
/// `os:putenv/2`. The value `false` or `[]` removes the key; another value
/// is a list of characters without 0 (`erl_osenv.c:269-313`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(environment_changes(&env), Some(vec![(b"A".to_vec(), Some(b"1".to_vec()))]));
/// ```
fn environment_changes(env: &Term) -> Option<Vec<environment::Change>> {
    let encoding = filename_encoding();
    let key = |term: &Term| native_filename(term, encoding).filter(|key| !key.contains(&b'='));
    env.proper_list()?
        .map(|pair| match pair {
            Term::Tuple(pair) if pair.len() == 2 => {
                let key = key(&pair[0])?;
                let value = &pair[1];
                if value.is_nil() || matches!(value, Term::Atom(atom) if atom.is_static("false")) {
                    Some((key, None))
                } else {
                    Some((key, Some(native_filename(value, encoding)?)))
                }
            }
            _ => None,
        })
        .collect()
}

/// Return the line buffer of a `{line, L}` whose low 32 bits are `size`.
///
/// # Errors
///
/// A `size` that is not positive returns a runtime error. ERTS allocates
/// `sizeof(LineBuf)` plus a negative `int` as a size and fails
/// (`io.c:3149-3153`). With 0, each byte that is not a newline sends an
/// empty `noeol` part and stays in the input (`io.c:3260-3283`), so ERTS
/// sends messages without end.
fn line_size(size: i32) -> Result<Lines, BeamError> {
    match usize::try_from(size) {
        Ok(size) if size > 0 => Ok(Lines::new(size)),
        _ => Err(error(
            "unsupported open_port/2 option {line, L} whose low 32 bits are not positive: see \"Ports\" in docs/erts-differences.md",
        )),
    }
}

/// Return the runtime error of an option of `open_port/2` that ERTS accepts
/// and PBeam does not support.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(unsupported_option("hide").to_string().starts_with("unsupported open_port/2 option hide"));
/// ```
fn unsupported_option(name: &str) -> BeamError {
    error(format!(
        "unsupported open_port/2 option {name}: see \"Ports\" in docs/erts-differences.md"
    ))
}

/// Return the native bytes of each file name of the proper list `args`, as
/// `convert_args` does (`erl_bif_port.c:1214-1246`), or `None` when `args`
/// is not a proper list or an element is not a file name (see
/// [`native_name`]). `[]` is an empty name here.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let args = Term::list_from(vec![Term::binary(b"-c".to_vec()), Term::nil()]);
/// assert_eq!(port_args(&atoms, &args), Some(vec![b"-c".to_vec(), Vec::new()]));
/// ```
fn port_args(atoms: &AtomTable, args: &Term) -> Option<Vec<Vec<u8>>> {
    let elements = args.proper_list()?;
    elements.map(|arg| native_name(atoms, arg, true)).collect()
}

/// The name of a port of `open_port/2`.
#[derive(Debug, PartialEq, Eq)]
enum PortName {
    /// The native bytes of the program of `{spawn_executable, Name}`.
    Executable(Vec<u8>),
    /// The native bytes of the driver name of `{spawn_driver, Name}`.
    Driver(Vec<u8>),
    /// The native bytes of the command of `{spawn, Command}`.
    Command(Vec<u8>),
    /// The small integers `In` and `Out` of `{fd, In, Out}`.
    Fd(i64, i64),
}

/// Return the name of `{spawn_executable, Name}`, `{spawn_driver, Name}`,
/// `{spawn, Command}`, or `{fd, In, Out}`, or `None` for the `badarg` of
/// ERTS: a term that is not a tuple with a first element that ERTS
/// accepts, a tuple of another size, a name that is not a file name without
/// `[]`, and an fd that is not a small integer (`erl_bif_port.c:1033-1084`).
///
/// # Examples
///
/// ```rust,ignore
/// let name = Term::tuple(vec![Term::static_atom("spawn_executable"), Term::binary(b"/bin/sh".to_vec())]);
/// assert_eq!(port_name(&atoms, &name), Some(PortName::Executable(b"/bin/sh".to_vec())));
/// ```
fn port_name(atoms: &AtomTable, name: &Term) -> Option<PortName> {
    let Term::Tuple(elements) = name else {
        return None;
    };
    match elements.as_ref() {
        [Term::Atom(kind), name] if kind.is_static("spawn_executable") => {
            native_name(atoms, name, false).map(PortName::Executable)
        }
        [Term::Atom(kind), name] if kind.is_static("spawn_driver") => {
            native_name(atoms, name, false).map(PortName::Driver)
        }
        [Term::Atom(kind), name] if kind.is_static("spawn") => {
            native_name(atoms, name, false).map(PortName::Command)
        }
        [Term::Atom(kind), input, output] if kind.is_static("fd") => input
            .small_integer()
            .zip(output.small_integer())
            .map(|(input, output)| PortName::Fd(input, output)),
        _ => None,
    }
}

/// Return the native bytes of a file name, as
/// `erts_convert_filename_to_encoding` with `allow_atom` does
/// (`erl_unicode.c:2057-2119`): a binary gives its bytes before its first
/// byte 0, because ERTS copies the bytes into a C string, and the port name
/// is that C string (`erl_unicode.c:2107-2108`, `io.c:323`, `366-367`). An
/// atom gives the bytes of its characters, and a list gives the bytes of
/// [`native_filename`]. `[]` is a name only with `allow_empty`. Return
/// `None` for each other term. `atoms` is the table of the run that owns
/// `name`.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// assert_eq!(native_name(&atoms, &Term::binary(b"/bin/sh\0x".to_vec()), false), Some(b"/bin/sh".to_vec()));
/// assert_eq!(native_name(&atoms, &Term::static_atom("ok"), false), Some(b"ok".to_vec()));
/// assert_eq!(native_name(&atoms, &Term::nil(), false), None);
/// assert_eq!(native_name(&atoms, &Term::nil(), true), Some(Vec::new()));
/// ```
fn native_name(atoms: &AtomTable, name: &Term, allow_empty: bool) -> Option<Vec<u8>> {
    let characters;
    let name = match name {
        Term::Binary(bytes) => {
            let bytes = bytes.bytes()?;
            let end = bytes.iter().position(|&byte| byte == 0);
            return Some(bytes[..end.unwrap_or(bytes.len())].to_vec());
        }
        Term::Atom(atom) => {
            let text = atoms.name_of(atom).chars();
            characters = Term::list_from(text.map(|c| Term::Integer(u32::from(c).into())));
            &characters
        }
        name if name.is_nil() && !allow_empty => return None,
        name => name,
    };
    native_filename(name, filename_encoding())
}

/// The port table entry of a port term: its identifier and the handle of
/// its lock.
pub(super) type Found = Option<(PortId, PortCell)>;

/// The guard of the lock of a found port.
pub(super) type Slot<'a> = Option<port::PortGuard<'a>>;

/// Return the port table entry of `port`, as `sig_lookup_port` and
/// `data_lookup_port` find the port (erl_bif_port.c:133-160), or `None`
/// for a term that is not a port in the table. An atom names no port,
/// because PBeam registers no port. The caller locks the port once with
/// [`lock_found`] and checks its state under that lock, as ERTS checks the
/// state of a port under the port lock (`erts_port_lookup`, erl_port.h).
///
/// # Examples
///
/// ```rust,ignore
/// let found = found_port(scheduler, &Term::Port(port))?;
/// let mut slot = lock_found(&found);
/// assert!(open(&mut slot).is_some());
/// ```
pub(super) fn found_port(scheduler: &mut Scheduler, port: &Term) -> Result<Found, BeamError> {
    let Term::Port(id) = port else {
        return Ok(None);
    };
    Ok(scheduler.node.port(*id).map(|cell| (*id, cell)))
}

/// Lock the port of `found`.
pub(super) fn lock_found(found: &Found) -> Slot<'_> {
    found.as_ref().map(|(_, cell)| port::lock(cell))
}

/// Return the port of `slot` when it is open or in the `CLOSING` state, as
/// `sig_lookup_port` finds it for `erlang:port_info/1,2` and
/// `erlang:port_control/3` (`erl_bif_port.c:133-148`), or `None` when it
/// closed.
pub(super) fn located<'a>(slot: &'a mut Slot<'_>) -> Option<&'a mut Port> {
    slot.as_mut().and_then(|slot| slot.as_mut())
}

/// Return the port of `slot` when it is open. A port in the `CLOSING`
/// state is not open: the BIFs that take this port give `badarg` for it,
/// as `data_lookup_port` does (erl_bif_port.c:157-160).
pub(super) fn open<'a>(slot: &'a mut Slot<'_>) -> Option<&'a mut Port> {
    located(slot).filter(|port| !port.closing)
}

/// Run `erts_internal:port_command(Port, Data, OptionList)`, which
/// `erlang:port_command/2` calls with `[]`, as
/// `erts_internal_port_command_3` and `erts_port_output` do
/// (`erl_bif_port.c:168-261`, `io.c:1715-2088`): write the bytes of the
/// iodata `Data` to the input of the program of `Port`, add them to the
/// output count, use 40 reductions, and return `true`. A failed write
/// closes the port with the `erl_errno_id` atom of the failure as its
/// reason, for example `epipe`, as `driver_failure_posix` does
/// (`sys_drivers.c:1180-1184`). With `{packet, N}`, the program gets a
/// header with the size before the bytes, and data that does not fit in a
/// header of 1 or 2 bytes closes the port with `einval`
/// (`sys_drivers.c:1140-1203`). On a `tcp_inet` port, the driver runs the
/// output with its `inet_reply` message (see `Scheduler::tcp_send`). A term
/// that is not an open port, or `Data`
/// that is not iodata, gives the atom `badarg`. The result traps to
/// `erlang:bif_handle_signals_return/2` when the caller has incoming
/// signals. After these checks, a caller without a `port` capability with
/// `command` that permits the program path of `Port` gets
/// `error:capability_denied`, and the BIF writes nothing.
///
/// # Examples
///
/// ```rust,ignore
/// let result = port_command(&mut context, &Term::Port(port), &Term::binary(b"hi".to_vec()), &Term::nil())?;
/// ```
///
/// # Errors
///
/// An `OptionList` other than `[]` and an output of a `tcp_inet` port that
/// PBeam does not run return a runtime error. A new atom in a full atom
/// table gives the error of `AtomTable::put`.
pub(super) fn port_command(
    context: &mut BifContext<'_>,
    port: &Term,
    data: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    if *options != Term::nil() {
        return Err(error(
            "unsupported erlang:port_command/3 options: see \"Ports\" in docs/erts-differences.md",
        ));
    }
    let scheduler = &mut *context.scheduler;
    // One lock of the port covers the check, the capability, and the
    // write or the send of the `tcp_inet` driver with its messages, as the
    // port lock does in `erts_port_output` (io.c:1715-2088).
    let found = found_port(scheduler, port)?;
    let mut slot = lock_found(&found);
    let (Some(port_state), Some(bytes), Some(&(id, _))) =
        (open(&mut slot), iolist::iodata_bytes(data), found.as_ref())
    else {
        drop(slot);
        return Ok(signals_return(scheduler, port, Term::static_atom("badarg")));
    };
    if let Some(denied) = port_denial(scheduler, &port_state.name, Operation::Command)? {
        return Ok(denied);
    }
    let written = output(scheduler, id, port_state, bytes)?;
    drop(slot);
    written.finish(scheduler, id)?;
    Ok(signals_return(scheduler, port, Term::static_atom("true")))
}

/// The end of an output to a port that runs after the release of the port
/// lock (see [`output`]).
enum Written {
    /// The output needs nothing more.
    Done,
    /// The `tcp_inet` driver sent the bytes and ends its step with the
    /// exit reason, if any (see `Scheduler::end_step`).
    Step(Option<Term>),
    /// A header that is too short fails the port with the reason `einval`
    /// (`driver_failure_posix`, io.c:7267-7270).
    Failed,
    /// The write to the program failed with this `errno`.
    Errno(i32),
}

impl Written {
    /// Run the end of the output to the port `id`: close or fail the port
    /// for a failed output.
    ///
    /// # Errors
    ///
    /// A new atom in a full atom table gives the error of `AtomTable::put`.
    fn finish(self, scheduler: &mut Scheduler, id: PortId) -> Result<(), BeamError> {
        match self {
            Written::Done => {}
            Written::Step(exit) => scheduler.end_step(id, exit),
            Written::Failed => scheduler.fail_port(id, Term::static_atom("einval")),
            Written::Errno(errno) => {
                let reason = Term::Atom(scheduler.atoms.put(&errno_id(errno))?);
                scheduler.close_port(id, reason);
            }
        }
        Ok(())
    }
}

/// Write `bytes` to the open port `id` with the state `port_state`, as
/// `erts_port_output` does (`io.c:1715-2088`): use 40 reductions, write
/// the bytes to the program or send them on the socket of a `tcp_inet`
/// port, and add them to the output count. The caller releases the port
/// lock and then runs [`Written::finish`].
///
/// # Errors
///
/// An output of a `tcp_inet` port that PBeam does not run returns a
/// runtime error.
fn output(
    scheduler: &mut Scheduler,
    id: PortId,
    port_state: &mut Port,
    bytes: Vec<u8>,
) -> Result<Written, BeamError> {
    scheduler
        .current_process_mut()
        .bump_reductions(PORT_REDUCTIONS);
    let spawn = match &mut port_state.driver {
        Driver::Spawn(spawn) => spawn,
        Driver::Inet(tcp) => {
            let caller = scheduler.current_process_mut().id;
            let sent = tcp.send(&bytes, caller, scheduler.local_node_hash())?;
            port_state.output += bytes.len() as u64;
            return Ok(match sent {
                Output::Sent(events) => {
                    Written::Step(scheduler.send_events(id, port_state, events)?)
                }
                Output::Failed => Written::Failed,
            });
        }
    };
    // A packet that does not fit in its header fails with `EINVAL`
    // (`sys_drivers.c:1153-1158`).
    let size = bytes.len() as u64;
    let written = spawn.packets.frame(bytes).ok_or(libc::EINVAL);
    let written = written.and_then(|framed| spawn.program.write_input(&framed));
    Ok(match written {
        Ok(()) => {
            port_state.output += size;
            Written::Done
        }
        Err(errno) => Written::Errno(errno),
    })
}

/// Run `erts_internal:port_close(Port)`, which `erlang:port_close/1` calls,
/// as `erts_internal_port_close_1` and `erts_port_exit` do
/// (`erl_bif_port.c:384-432`, `io.c:2241-2300`): close the port with the
/// reason `normal` from any process, use 40 reductions, and return `true`.
/// The owner then gets the link exit signal `normal` of the port. A term
/// that is not an open port gives the atom `badarg`. The result traps to
/// `erlang:bif_handle_signals_return/2` when the caller has incoming
/// signals, as the `EXIT` signal of the port to its owner. After the port
/// check, a caller without a `port` capability with `close` that permits
/// the program path of `Port` gets `error:capability_denied`, and the port
/// stays open.
///
/// # Examples
///
/// ```rust,ignore
/// let result = port_close(&mut context, &Term::Port(port))?;
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn port_close(
    context: &mut BifContext<'_>,
    port: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let found = found_port(scheduler, port)?;
    let mut slot = lock_found(&found);
    let (Some(port_state), Some(&(id, _))) = (open(&mut slot), found.as_ref()) else {
        drop(slot);
        return Ok(signals_return(scheduler, port, Term::static_atom("badarg")));
    };
    if let Some(denied) = port_denial(scheduler, &port_state.name, Operation::Close)? {
        return Ok(denied);
    }
    drop(slot);
    scheduler
        .current_process_mut()
        .bump_reductions(PORT_REDUCTIONS);
    // Of two closes on two schedulers, only the first closes the port; the
    // other gives `badarg`, as in ERTS (io.c:2241-2290).
    let closed = scheduler.close_port(id, Term::static_atom("normal"));
    let result = if closed { "true" } else { "badarg" };
    Ok(signals_return(scheduler, port, Term::static_atom(result)))
}

/// Run `erlang:exit(Port, Reason)` and `erlang:exit/3` for a port, as
/// `exit_2` and `erts_port_exit` do (`bif.c:1774-1797`,
/// `io.c:2241-2300`): close the open port `port` with the reason `reason`
/// from each process, also with `normal` and from a process that is not
/// its owner (`erts_deliver_port_exit` with `drop_normal` 0,
/// io.c:3876-3879), and return `true`. The reason `kill` gives the link
/// exit signal `killed` (io.c:3854). A port that is not open gives `true`.
/// A caller without a `port` capability with `close` that permits the
/// program path of the port gets `error:capability_denied`, and the port
/// stays open.
///
/// # Examples
///
/// ```rust,ignore
/// let result = port_exit(scheduler, &Term::Port(port), &Term::static_atom("kill"))?;
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn port_exit(
    scheduler: &mut Scheduler,
    port: &Term,
    reason: &Term,
) -> Result<BifResult, BeamError> {
    let found = found_port(scheduler, port)?;
    let mut slot = lock_found(&found);
    if let (Some(port_state), Some(&(id, _))) = (open(&mut slot), found.as_ref()) {
        if let Some(denied) = port_denial(scheduler, &port_state.name, Operation::Close)? {
            return Ok(denied);
        }
        drop(slot);
        if scheduler.close_port(id, reason.clone()) {
            scheduler
                .current_process_mut()
                .bump_reductions(PORT_REDUCTIONS);
        }
    }
    Ok(Value(Term::static_atom("true")))
}

/// Run `erts_internal:port_call(Port, Operation, Data)`, which
/// `erlang:port_call/2,3` calls, as `erts_internal_port_call_3` does
/// (`erl_bif_port.c:263-320`): give the atom `badarg`, which
/// `erlang:port_call/3` raises. The spawn driver and the `tcp_inet` driver,
/// the only drivers of PBeam, have no `call` callback
/// (`sys_drivers.c:246`, `inet_drv.c:1369`), so `erts_port_call` gives
/// `ERTS_PORT_OP_BADARG` for each port (`call_driver_call`,
/// io.c:4473-4474). The result traps to
/// `erlang:bif_handle_signals_return/2` when the caller has incoming
/// signals. For a port that is open or in the `CLOSING` state and an
/// `Operation` from 0 to 2^32 - 1 (erl_bif_port.c:270-276), a caller
/// without a `port` capability with `command` that permits the program
/// path of the port gets `error:capability_denied`, as for
/// `erlang:port_control/3`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = port_call(scheduler, &Term::Port(port), &Term::Integer(1))?;
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn port_call(
    scheduler: &mut Scheduler,
    port: &Term,
    operation: &Term,
) -> Result<BifResult, BeamError> {
    let valid = operation
        .small_integer()
        .is_some_and(|value| u32::try_from(value).is_ok());
    let found = found_port(scheduler, port)?;
    let mut slot = lock_found(&found);
    if let (Some(port_state), true) = (located(&mut slot), valid)
        && let Some(denied) = port_denial(scheduler, &port_state.name, Operation::Command)?
    {
        return Ok(denied);
    }
    drop(slot);
    Ok(signals_return(scheduler, port, Term::static_atom("badarg")))
}

/// The operation of a message to a port (see [`port_bang`]).
enum Bang {
    /// `{Owner, close}`.
    Close,
    /// `{Owner, {command, Data}}` with iodata `Data`.
    Command(Vec<u8>),
    /// `{Owner, {connect, NewOwner}}`.
    Connect(ProcessId),
}

/// Run the message `message` to the open port `id`, which the `send`
/// instruction, `erlang:send/2`, and `erlang:send/3` give to a port, as
/// `erts_port_command` does (`io.c:3974-4015`). The pid in the message
/// must be the owner of the port, also when another process sends it:
///
/// - `{Owner, {command, Data}}` writes the bytes of the iodata `Data` as
///   `erlang:port_command/2` does (`call_driver_output`, io.c:1368-1369).
/// - `{Owner, close}` closes the port with the reason `normal`, and then
///   the owner gets `{Port, closed}` after the link exit signal `normal`
///   (`call_deliver_port_exit`, io.c:2182-2198, `erts_deliver_port_exit`,
///   io.c:3876-3882).
/// - `{Owner, {connect, Pid}}` with a local pid makes `Pid` the owner of
///   the port without a link, and the old owner gets `{Port, connected}`
///   (`set_port_connected`, io.c:2341-2351).
///
/// Each other message, a pid that is not the owner, and `Data` that is not
/// iodata send the exit signal `badsig` from the port to its owner
/// (`send_badsig`, io.c:1246-1254). A port that is not open drops the
/// message. The caller needs a `port` capability with `close` for a close,
/// and with `command` for the other two operations, that permits the
/// program path of the port; else the send returns
/// `error:capability_denied` and the port does nothing. A runtime process
/// that connects a port to a boot process gets the same denial.
///
/// # Examples
///
/// ```rust,ignore
/// let close = Term::tuple(vec![Term::Pid(owner), Term::static_atom("close")]);
/// assert!(port_bang(scheduler, port, &close)?.is_none());
/// ```
///
/// # Errors
///
/// An output of a `tcp_inet` port that PBeam does not run returns a
/// runtime error. A new atom in a full atom table gives the error of
/// `AtomTable::put`.
pub(super) fn port_bang(
    scheduler: &mut Scheduler,
    id: PortId,
    message: &Term,
) -> Result<Option<BifResult>, BeamError> {
    let found = found_port(scheduler, &Term::Port(id))?;
    let mut slot = lock_found(&found);
    let Some(port_state) = open(&mut slot) else {
        return Ok(None);
    };
    let owner = port_state.owner;
    let bang = match message.tuple_elements() {
        Some([Term::Pid(from), operation]) if *from == owner => match operation {
            Term::Atom(atom) if atom.is_static("close") => Some(Bang::Close),
            operation => match operation.tuple_elements() {
                Some([Term::Atom(atom), data]) if atom.is_static("command") => {
                    iolist::iodata_bytes(data).map(Bang::Command)
                }
                Some([Term::Atom(atom), Term::Pid(pid)]) if atom.is_static("connect") => {
                    Some(Bang::Connect(*pid))
                }
                _ => None,
            },
        },
        _ => None,
    };
    let Some(bang) = bang else {
        drop(slot);
        let badsig = Signal::Exit {
            from: LinkId::Port(id),
            reason: Term::static_atom("badsig"),
            linked: false,
            normal_kills: false,
            alias: None,
        };
        // An owner that does not exist drops the signal.
        let _ = scheduler.send_signal(None, owner, badsig);
        return Ok(None);
    };
    let denied = match bang {
        Bang::Close => port_denial(scheduler, &port_state.name, Operation::Close)?,
        Bang::Command(_) => port_denial(scheduler, &port_state.name, Operation::Command)?,
        Bang::Connect(pid) => connect_denial(scheduler, &port_state.name, pid)?,
    };
    if denied.is_some() {
        return Ok(denied);
    }
    let port_term = Term::Port(id);
    match bang {
        Bang::Command(bytes) => {
            let written = output(scheduler, id, port_state, bytes)?;
            drop(slot);
            written.finish(scheduler, id)?;
        }
        Bang::Close => {
            drop(slot);
            // Of two closes on two schedulers, only the first closes the
            // port and replies, when the port terminates (io.c:2179-2180,
            // 3713-3719, 3870-3874).
            if scheduler.close_port_replying(id, owner) {
                scheduler
                    .current_process_mut()
                    .bump_reductions(PORT_REDUCTIONS);
            }
        }
        Bang::Connect(pid) => {
            port_state.owner = pid;
            drop(slot);
            scheduler
                .current_process_mut()
                .bump_reductions(CONNECT_REDUCTIONS);
            let connected = Term::tuple(vec![port_term, Term::static_atom("connected")]);
            scheduler.port_send(owner, connected);
        }
    }
    Ok(None)
}

/// The only command that the control of the spawn driver runs
/// (`ERTS_SPAWN_DRV_CONTROL_MAGIC_NUMBER`, sys.h:1467).
const SPAWN_CONTROL: u32 = 0x04c7_6a00;

/// Run `erts_internal:port_control(Port, Operation, Data)`, which
/// `erlang:port_control/3` calls, as `erts_internal_port_control_3` and
/// `erts_port_control` do (`erl_bif_port.c:322-376`, `io.c:4268-4420`):
/// run the request `Operation` of the driver of an open port, or of a
/// port in the `CLOSING` state, with the bytes of the iodata `Data`, use
/// 40 reductions, and return the reply as a list of bytes. A term that is
/// not such a port, an `Operation` that is
/// not an integer from 0 to 2^32 - 1, data that is not iodata, and a
/// request that the driver rejects give the atom `badarg` and use no
/// reductions. The control of the spawn driver, `spawn_control`
/// (sys_drivers.c:241, 731-756), rejects each command other than
/// `ERTS_SPAWN_DRV_CONTROL_MAGIC_NUMBER` (`0x04c76a00`, sys.h:1467). That
/// command, which `erl_child_setup` sends when the program exits, sets the
/// exit state of the port in ERTS; PBeam does not run it.
/// The result traps to `erlang:bif_handle_signals_return/2` when the
/// caller has incoming signals. After the argument checks, a caller
/// without a `port` capability with `command` that permits the name of the
/// port gets `error:capability_denied`, and the request does not run.
///
/// # Examples
///
/// ```rust,ignore
/// let open = Term::list_from(vec![Term::Integer(1), Term::Integer(1), Term::Integer(1)]);
/// let result = port_control(&mut context, &Term::Port(port), &Term::Integer(0x03f1_a301), &open)?;
/// ```
///
/// # Errors
///
/// A request of the `tcp_inet` driver that PBeam does not run (see
/// [`Inet::control`]), the command `0x04c76a00` on a port of the spawn
/// driver, and the atom `capability_denied` in a full atom table.
pub(super) fn port_control(
    context: &mut BifContext<'_>,
    port: &Term,
    operation: &Term,
    data: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let command = operation
        .small_integer()
        .and_then(|value| u32::try_from(value).ok());
    let found = found_port(scheduler, port)?;
    let mut slot = lock_found(&found);
    let (Some(port_state), Some(command), Some(bytes), Some(&(id, _))) = (
        located(&mut slot),
        command,
        iolist::iodata_bytes(data),
        found.as_ref(),
    ) else {
        drop(slot);
        return Ok(signals_return(scheduler, port, Term::static_atom("badarg")));
    };
    if let Some(denied) = port_denial(scheduler, &port_state.name, Operation::Command)? {
        return Ok(denied);
    }
    let caller = scheduler.current_process_mut().id;
    let Some(tcp) = port_state.inet_mut() else {
        drop(slot);
        if command == SPAWN_CONTROL {
            return Err(error(
                "unsupported erlang:port_control/3 command 16#04c76a00 on a spawn port: see \"Ports\" in docs/erts-differences.md",
            ));
        }
        return Ok(signals_return(scheduler, port, Term::static_atom("badarg")));
    };
    // The step of the request runs under the port lock of the control, as
    // in `erts_port_control` (io.c:4268-4420), so a close on another
    // scheduler comes before it or after it. The messages of the step go
    // out under the same lock; the end of the port comes after it.
    let mut events = None;
    let reply = match tcp.control(command, &bytes, caller)? {
        Control::Reply(reply) => Accept::Queued(reply),
        Control::Accept(timeout) => scheduler.tcp_accept(id, tcp, timeout)?,
        Control::Connect(timeout) => Accept::Queued(scheduler.tcp_connect(id, tcp, timeout)),
        Control::Receive { timeout, length } => {
            let (reply, received) = scheduler.tcp_receive(id, tcp, timeout, length);
            events = Some(received);
            Accept::Queued(reply)
        }
        Control::Passive { reply, count } => {
            events = Some((0..count).map(|_| Event::Passive).collect());
            Accept::Queued(reply)
        }
        Control::Active { read } => {
            events = Some(tcp.activate(read));
            Accept::Queued(vec![OK])
        }
        Control::Badarg => {
            drop(slot);
            return Ok(signals_return(scheduler, port, Term::static_atom("badarg")));
        }
    };
    let exit = events
        .map(|events| scheduler.send_events(id, port_state, events))
        .transpose()?;
    drop(slot);
    if let Some(exit) = exit {
        scheduler.end_step(id, exit);
    }
    let reply = match reply {
        Accept::Queued(reply) => reply,
        Accept::Now(accepted) => scheduler.tcp_accepted(id, accepted)?,
    };
    scheduler
        .current_process_mut()
        .bump_reductions(PORT_REDUCTIONS);
    let result = Term::list_from(reply.into_iter().map(|byte| Term::Integer(byte.into())));
    Ok(signals_return(scheduler, port, result))
}

/// Return the `error:capability_denied` exception when no `port`
/// capability of the caller permits `operation` for the program path of the
/// open port `id`, else `None` (see `denial`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(port_denial(&mut scheduler, port, Operation::Close)?.is_none());
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn port_denial(
    scheduler: &mut Scheduler,
    name: &[u8],
    operation: Operation,
) -> Result<Option<BifResult>, BeamError> {
    denial(scheduler, CapabilityKind::Port, operation, Some(name))
}

/// Run `erts_internal:port_info(Port)`, which `erlang:port_info/1` calls, as
/// `erts_internal_port_info_1` and `erts_port_info` do
/// (`erl_bif_port.c:486-535`, `erl_bif_info.c:3654-3830`): for an open
/// port, also in the `CLOSING` state, use 40 reductions and return `[{name, Name}, {links, Links},
/// {id, Number}, {connected, Owner}, {input, Bytes}, {output, Bytes},
/// {os_pid, OsPid}]` (`ERTS_PORT_INFO_1_ITEMS`, `erl_port.h:100-108`). The
/// name is the list of the bytes of the program path, `links` holds each
/// linked process, and `id` is the port number (see [`port_item`]). A port
/// that closed and an atom give `undefined`; each other term gives the atom
/// `badarg`. The result traps to `erlang:bif_handle_signals_return/2` when
/// the caller has incoming signals. For an open port, a caller without a
/// `port` capability with `inspect` that permits the program path of the
/// port gets `error:capability_denied`, and the BIF uses no reduction and
/// reads no information.
///
/// # Examples
///
/// ```rust,ignore
/// let result = port_info(&mut context, &Term::Port(port))?;
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn port_info(context: &mut BifContext<'_>, port: &Term) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let found = found_port(scheduler, port)?;
    let mut slot = lock_found(&found);
    let info = match (port, located(&mut slot)) {
        (&Term::Port(id), Some(port_state)) => {
            if let Some(denied) = port_denial(scheduler, &port_state.name, Operation::Inspect)? {
                return Ok(denied);
            }
            scheduler
                .current_process_mut()
                .bump_reductions(PORT_REDUCTIONS);
            let port = &*port_state;
            Term::list_from(
                PORT_INFO_ITEMS
                    .iter()
                    .map(|&name| port_item(id, port, name))
                    .collect::<Result<Vec<_>, _>>()?,
            )
        }
        (Term::Port(_) | Term::Atom(_), _) => Term::static_atom("undefined"),
        _ => Term::static_atom("badarg"),
    };
    drop(slot);
    Ok(signals_return(scheduler, port, info))
}

/// The items of `port_info/1`, in order (`ERTS_PORT_INFO_1_ITEMS`,
/// `erl_port.h:100-108`).
const PORT_INFO_ITEMS: [&str; 7] = [
    "name",
    "links",
    "id",
    "connected",
    "input",
    "output",
    "os_pid",
];

/// The items of `port_info/2` that PBeam does not give yet
/// (`erts_bld_port_info`, `erl_bif_info.c:3654-3830`).
const OTHER_PORT_ITEMS: [&str; 6] = [
    "registered_name",
    "memory",
    "monitors",
    "monitored_by",
    "queue_size",
    "locking",
];

/// Return the item `{Name, Value}` of the open port `port` with the number
/// `id`, as `erts_bld_port_info` builds it (`erl_bif_info.c:3654-3830`),
/// for a name of [`PORT_INFO_ITEMS`] or `parallelism`. The
/// name is the list of the bytes of the program path or driver name,
/// `links` holds each linked process in the ERTS link tree order, and `id`
/// is the port number.
///
/// # Examples
///
/// ```rust,ignore
/// let item = port_item(id, port, "links")?;
/// ```
fn port_item(id: PortId, port: &Port, name: &str) -> Result<Term, BeamError> {
    let value = match name {
        "name" => Term::list_from(port.name.iter().map(|&byte| Term::Integer(byte.into()))),
        // `erts_bld_port_info` conses each entry of the post-order walk to
        // the front, so the last entry of the walk is the head of the list
        // (`erl_bif_info.c:3676-3696`, `erl_rbtree.h:1343-1458`).
        "links" => {
            let mut links: Vec<Term> = port
                .links
                .post_order()
                .map(|(pid, ())| Term::Pid(*pid))
                .collect();
            links.reverse();
            Term::list_from(links)
        }
        "id" => Term::Integer(id.0.try_into().expect("a port number below 2^63")),
        "connected" => Term::Pid(port.owner),
        "input" => Term::Integer(port.input.try_into().expect("an input count below 2^63")),
        "output" => Term::Integer(port.output.try_into().expect("an output count below 2^63")),
        // `ERTS_PORT_SFLG_PARALLELISM` (`erl_bif_info.c:3884-3895`). An
        // inet port opens without the option.
        "parallelism" => Term::static_atom(match &port.driver {
            Driver::Spawn(spawn) if spawn.parallelism => "true",
            _ => "false",
        }),
        // The last item, `os_pid`.
        _ => match &port.driver {
            Driver::Spawn(spawn) => match spawn.program.child() {
                Some(child) => Term::Integer(child.os_pid.into()),
                None => Term::static_atom("undefined"),
            },
            Driver::Inet(_) => Term::static_atom("undefined"),
        },
    };
    Ok(Term::tuple(vec![Term::static_atom(name), value]))
}

/// Run `erts_internal:port_info(Port, Item)`, which `erlang:port_info/2`
/// calls, as `erts_internal_port_info_2` and `erts_port_info` do
/// (`erl_bif_port.c:543-595`, `erl_bif_info.c:3654-3830`): for an open
/// port, also in the `CLOSING` state, use 40 reductions (`ERTS_PORT_REDS_INFO`) and return the item of
/// [`port_item`]; an item that is not an item name gives the atom
/// `badarg` before the capability check. A port that closed and an atom give `undefined`, and each
/// other term gives the atom `badtype`. The result traps to
/// `erlang:bif_handle_signals_return/2` when the caller has incoming
/// signals. For an open port, a caller without a `port` capability with
/// `inspect` that permits the program path of the port gets
/// `error:capability_denied`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = port_info_item(&mut context, &Term::Port(port), &Term::static_atom("connected"))?;
/// ```
///
/// # Errors
///
/// An item of [`OTHER_PORT_ITEMS`], and the atom `capability_denied` in a
/// full atom table.
pub(super) fn port_info_item(
    context: &mut BifContext<'_>,
    port: &Term,
    item: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let found = found_port(scheduler, port)?;
    let mut slot = lock_found(&found);
    let info = match (port, located(&mut slot)) {
        (&Term::Port(id), Some(port_state)) => {
            let name = match item {
                Term::Atom(name) => scheduler.atoms.name_of(name).to_owned(),
                _ => String::new(),
            };
            let name = name.as_str();
            if !PORT_INFO_ITEMS.contains(&name)
                && !OTHER_PORT_ITEMS.contains(&name)
                && name != "parallelism"
            {
                return Ok(signals_return(scheduler, port, Term::static_atom("badarg")));
            }
            if let Some(denied) = port_denial(scheduler, &port_state.name, Operation::Inspect)? {
                return Ok(denied);
            }
            if OTHER_PORT_ITEMS.contains(&name) {
                return Err(error(format!(
                    "unsupported erlang:port_info/2 item {name}: see \"Ports\" in docs/erts-differences.md"
                )));
            }
            scheduler
                .current_process_mut()
                .bump_reductions(PORT_REDUCTIONS);
            port_item(id, port_state, name)?
        }
        (Term::Port(_) | Term::Atom(_), _) => Term::static_atom("undefined"),
        _ => Term::static_atom("badtype"),
    };
    drop(slot);
    Ok(signals_return(scheduler, port, info))
}

/// Run `erts_internal:port_connect(Port, Pid)`, which
/// `erlang:port_connect/2` calls, as `erts_internal_port_connect_2` and
/// `erts_port_connect` do (`erl_bif_port.c:438-484`, io.c:2355-2470):
/// make the local pid `pid` the owner of the open port `port` and link
/// them (see [`Scheduler::connect_port`]), use 20 reductions
/// (`ERTS_PORT_REDS_CONNECT`), and return `true`. Each process may connect
/// a port, not only its owner. A port that is not open, an atom, a `pid`
/// that is not a pid, and a pid of a process that does not exist give the
/// atom `badarg`, which `erlang:port_connect/2` raises. The result traps to
/// `erlang:bif_handle_signals_return/2` when the caller has incoming
/// signals. For an open port and a pid, a caller without a `port`
/// capability with `command` that permits the program path of the port
/// gets `error:capability_denied`, and so does a runtime process that
/// connects the port to a boot process.
///
/// # Examples
///
/// ```rust,ignore
/// let result = port_connect(&mut context, &Term::Port(port), &Term::Pid(handler))?;
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn port_connect(
    context: &mut BifContext<'_>,
    port: &Term,
    pid: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let found = found_port(scheduler, port)?;
    let mut slot = lock_found(&found);
    let name = open(&mut slot).map(|port_state| port_state.name.clone());
    drop(slot);
    let result = match (found.as_ref().zip(name), pid) {
        (Some(((id, _), name)), &Term::Pid(pid)) => {
            let id = *id;
            if let Some(denied) = connect_denial(scheduler, &name, pid)? {
                return Ok(denied);
            }
            let connected = scheduler.connect_port(id, pid);
            if connected {
                scheduler
                    .current_process_mut()
                    .bump_reductions(CONNECT_REDUCTIONS);
            }
            Term::static_atom(if connected { "true" } else { "badarg" })
        }
        _ => Term::static_atom("badarg"),
    };
    Ok(signals_return(scheduler, port, result))
}

/// Return the denial of a connect of the port with the program path
/// `name` to `pid`: a caller without a `port` capability with `command`
/// that permits `name`, and a runtime process that makes the boot process
/// `pid` the owner, as for `link/1` and `send/2` (see [`boot_denial`]).
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
fn connect_denial(
    scheduler: &mut Scheduler,
    name: &[u8],
    pid: ProcessId,
) -> Result<Option<BifResult>, BeamError> {
    let boot = boot_denial(scheduler, pid)?;
    Ok(port_denial(scheduler, name, Operation::Command)?.or(boot))
}

/// The reductions of a port connect (`ERTS_PORT_REDS_CONNECT`,
/// `CONTEXT_REDS / 200`, erl_port.h:371).
const CONNECT_REDUCTIONS: i32 = 20;

/// Run `erts_internal:prepare_loading(Module, Code)`, which
/// `erlang:prepare_loading/2` calls, as `erts_internal_prepare_loading_2`
/// (`beam_bif_load.c:231-266`) does. A `Module` that is not an atom, and
/// then a `Code` that is not a binary, raise `badarg`. Bytes that ERTS
/// rejects as a bad file, and a module name in the bytes that is not
/// `Module`, return `{error, badfile}` (see [`code::prepare`]). Else the
/// BIF returns a new reference, and the code table of the run keeps the
/// prepared module under it until `erlang:finish_loading/1` loads it. ERTS
/// returns a magic reference, for which `erlang:is_reference/1` is also
/// `true`. The atoms of the bytes go into the atom table of the run. ERTS
/// also sends an error report to the system logger for bad bytes
/// (`beam_load_report_error`, `beam_load.c:372`); PBeam sends none (row
/// "Code loading" of `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let prepared = prepare_loading(&mut context, &load_target, &Term::Binary(code))?;
/// assert!(matches!(prepared, BifResult::Value(Term::Reference(_))));
/// let bad = prepare_loading(&mut context, &load_target, &Term::binary(b"bad".to_vec()))?;
/// assert!(matches!(bad, BifResult::Value(Term::Tuple(_))));
/// ```
///
/// # Errors
///
/// Bytes that the PBeam loader cannot decode, such as a module with an
/// `-on_load` function (opcode 149), give the runtime error
/// `erts_internal:prepare_loading/2 cannot load module M: <cause>`.
pub(super) fn prepare_loading(
    context: &mut BifContext<'_>,
    module: &Term,
    code: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Atom(module), Some(bytes)) = (module, code.binary_bytes()) else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    let module_name = scheduler.atoms.name_of(module).to_owned();
    match code::prepare(bytes, &module_name, &scheduler.atoms) {
        Ok(prepared) => {
            let reference = scheduler.make_magic_ref()?;
            scheduler.code.prepare(reference, prepared);
            Ok(Value(Term::Reference(reference)))
        }
        Err(PrepareError::BadFile) => Ok(Value(Term::tuple(vec![
            Term::static_atom("error"),
            Term::static_atom("badfile"),
        ]))),
        Err(PrepareError::Unsupported(cause)) => Err(error(format!(
            "erts_internal:prepare_loading/2 cannot load module {module_name}: {cause}"
        ))),
    }
}

/// The most digits of each base 2 to 36 that fit in a small integer with
/// its sign, less one, on a 64-bit emulator (`digits_per_small_lookup`,
/// `big.c:4114-4127`).
const DIGITS_PER_SMALL: [u32; 35] = [
    59, 37, 29, 25, 22, 21, 19, 18, 17, 17, 16, 15, 15, 15, 14, 14, 14, 13, 13, 13, 13, 13, 12, 12,
    12, 12, 12, 12, 12, 11, 11, 11, 11, 11, 11,
];

/// Run `erts_internal:list_to_integer(String, Base)`, which
/// `erlang:list_to_integer/1,2` calls, as
/// `erts_internal_list_to_integer_2` (`big.c:4258-4354`) does. It reads an
/// optional `-` or `+` and the longest prefix of digits of `Base`, and
/// returns `{Integer, Rest}` with the rest of the list. It returns the atom
/// `no_integer` for `[]` and for a list with no digit after the sign,
/// `not_a_list` for a term that is not a list, `badarg` for a `Base` that is
/// not a small integer from 2 to 36, and `big` when the digits after the
/// leading zeros do not fit in a small integer; `erlang:list_to_integer/1,2`
/// then reads the string as a binary. ERTS reads each element that is a
/// small integer as its low 8 bits (`byte c = unsigned_val(...)`), so
/// `[$1 + 256]` gives `{1, []}`. An element that is not a small integer ends
/// the digits.
///
/// # Examples
///
/// ```rust,ignore
/// // "-12a" with base 10 gives {-12, "a"}.
/// let value = list_to_integer(&string("-12a"), &Term::Integer(10));
/// // [] gives no_integer.
/// let value = list_to_integer(&Term::Nil, &Term::Integer(10));
/// ```
pub(super) fn list_to_integer(list: &Term, base: &Term) -> BifResult {
    let atom = |name| Value(Term::static_atom(name));
    if list.is_nil() {
        return atom("no_integer");
    }
    let Some((head, tail)) = list.cons_cell() else {
        return atom("not_a_list");
    };
    let base = match base.small_integer() {
        Some(base @ 2..=36) => base as u32,
        _ => return atom("badarg"),
    };
    let byte = |term: &Term| term.small_integer().map(|value| value.to_le_bytes()[0]);
    let digit = |byte: u8| char::from(byte).to_digit(base);
    // ERTS compares the first element with the small integers `$-` and
    // `$+`, not with its low 8 bits.
    let (negative, mut rest) = match head.small_integer() {
        Some(0x2d) => (true, tail),
        Some(0x2b) => (false, tail),
        _ => (false, list),
    };
    let mut zeros = 0;
    while let Some((head, tail)) = rest.cons_cell() {
        match byte(head) {
            Some(b'0') => {
                zeros += 1;
                rest = tail;
            }
            Some(other) if digit(other).is_none() => {
                return if zeros == 0 {
                    atom("no_integer")
                } else {
                    Value(Term::tuple(vec![Term::Integer(0), rest.clone()]))
                };
            }
            _ => break,
        }
    }
    if !rest.is_nonempty_list() {
        return if zeros == 0 {
            atom("no_integer")
        } else {
            Value(Term::tuple(vec![Term::Integer(0), rest.clone()]))
        };
    }
    let digits_per_small = DIGITS_PER_SMALL[base as usize - 2];
    let (mut value, mut count) = (0_i64, 0);
    while count <= digits_per_small {
        let Some((head, tail)) = rest.cons_cell() else {
            break;
        };
        let Some(next) = byte(head).and_then(digit) else {
            break;
        };
        value = value * i64::from(base) + i64::from(next);
        count += 1;
        rest = tail;
    }
    if count == 0 {
        atom("no_integer")
    } else if count > digits_per_small {
        atom("big")
    } else {
        let value = if negative { -value } else { value };
        Value(Term::tuple(vec![Term::Integer(value), rest.clone()]))
    }
}

/// Run `erts_internal:binary_to_integer(Binary, Base)`, which
/// `erlang:binary_to_integer/1,2` call, as
/// `erts_internal_binary_to_integer_2` with `chars_to_integer`
/// (`big.c:4162-4256`) do. `Base` must be a small integer from 2 to 36.
/// `Binary` must hold a whole number of bytes; ERTS copies a bit string
/// with a bit offset into an aligned buffer (`erl_bits.h:606-633`). The
/// text reads one optional `-` or `+`, then the digits of the base. An
/// empty binary, a sign without a digit, a term that is not a binary, a
/// base outside 2 to 36, and a digit outside the base give the atom
/// `badarg`; the BIF raises nothing. Leading zeros do not count toward the
/// digit limit, and a text above `DIGITS_PER_SMALL` digits gives the atom
/// `big` before ERTS reads a digit, so a digit outside the base of such a
/// text gives `big`, not `badarg`. `erlang:binary_to_integer/1,2` raise
/// that `badarg` and read `big` with `big_binary_to_int`
/// (`erlang.erl:1056-1129`).
///
/// # Examples
///
/// ```rust,ignore
/// let binary = Term::binary(b"3FF".to_vec());
/// assert!(matches!(binary_to_integer(&binary, &Term::Integer(16)), BifResult::Value(Term::Integer(1023))));
/// let bad = Term::binary(b"1a".to_vec());
/// assert!(matches!(binary_to_integer(&bad, &Term::Integer(10)), BifResult::Value(Term::Atom(atom)) if atom.is_static("badarg")));
/// let many = Term::binary(b"123456789012345678".to_vec());
/// assert!(matches!(binary_to_integer(&many, &Term::Integer(10)), BifResult::Value(Term::Atom(_))));
/// ```
pub(super) fn binary_to_integer(binary: &Term, base: &Term) -> BifResult {
    let badarg = || Value(Term::static_atom("badarg"));
    let Some(base) = base.small_integer().filter(|base| (2..=36).contains(base)) else {
        return badarg();
    };
    let Some(bytes) = binary.binary_bytes() else {
        return badarg();
    };
    chars_to_integer(bytes, base as u32)
}

/// Return the integer that the byte text `text` reads as in `base`, as
/// `chars_to_integer` (`big.c:4162-4229`) gives it. A byte of the base
/// counts one digit, and `DIGITS_PER_SMALL` gives the count above which
/// the result is the atom `big`. A text that is empty, that holds no digit
/// after the sign, or that holds a byte outside the base gives the atom
/// `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(chars_to_integer(b"-12", 10), BifResult::Value(Term::Integer(-12))));
/// assert!(matches!(chars_to_integer(b"", 10), BifResult::Value(Term::Atom(_))));
/// assert!(matches!(chars_to_integer(b"01234567890123456789", 10), BifResult::Value(Term::Atom(_))));
/// ```
fn chars_to_integer(text: &[u8], base: u32) -> BifResult {
    let badarg = || Value(Term::static_atom("badarg"));
    let Some(first) = text.first() else {
        return badarg();
    };
    let (negative, text) = match first {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    if text.is_empty() {
        return badarg();
    }
    let zeros = text.iter().take_while(|&&byte| byte == b'0').count();
    let digits = &text[zeros..];
    if digits.is_empty() {
        return Value(Term::Integer(0));
    }
    if digits.len() > DIGITS_PER_SMALL[base as usize - 2] as usize {
        return Value(Term::static_atom("big"));
    }
    let mut value = 0_i64;
    for &byte in digits {
        let Some(digit) = char::from(byte).to_digit(base) else {
            return badarg();
        };
        value = value * i64::from(base) + i64::from(digit);
    }
    Value(Term::Integer(if negative { -value } else { value }))
}

/// Run `erts_internal:scheduler_wall_time(Enable)`, which
/// `kernel_refc:init/1` calls with `false` (`kernel_refc.erl:52-54`,
/// `141-142`), as `erts_internal_scheduler_wall_time_1` (`bif.c:5306-5315`)
/// does: each term other than `true` disables the scheduler wall time, and
/// the BIF gives the old state. PBeam never enables it, so the old state
/// is `false`. `erts_sched_wall_time_request` makes a reference, and each
/// scheduler sends it to the caller; the scheduler of the caller sends it
/// before the BIF returns (`erl_process.c:1382-1421`, `1209-1340`). PBeam
/// sends the reference one time for each scheduler of the node before the
/// BIF returns. The BIF then traps to
/// `erts_internal:await_sched_wall_time_modifications(Ref, Result)`, which
/// receives `Ref` one time for each scheduler and returns `Result`
/// (`erts_internal.erl:756-774`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = scheduler_wall_time(context, &Term::static_atom("false"))?;
/// assert!(matches!(result, BifResult::Trap { .. }));
/// assert!(scheduler_wall_time(context, &Term::static_atom("true")).is_err());
/// ```
///
/// # Errors
///
/// Return a runtime error for `true`: PBeam has no scheduler wall time. A
/// reference after the last one gives the error of `make_ref`.
pub(super) fn scheduler_wall_time(
    context: &mut BifContext<'_>,
    enable: &Term,
) -> Result<BifResult, BeamError> {
    if matches!(enable, Term::Atom(atom) if atom.is_static("true")) {
        return Err(error(
            "unsupported erts_internal:scheduler_wall_time(true): see \"Scheduling\" in docs/erts-differences.md",
        ));
    }
    let scheduler = &mut *context.scheduler;
    let reference = Term::Reference(scheduler.make_ref()?);
    let caller = scheduler.current_process_mut().id;
    for _ in 0..scheduler.scheduler_count() {
        scheduler.deliver(caller, reference.clone());
    }
    Ok(BifResult::Trap {
        module: "erts_internal",
        function: "await_sched_wall_time_modifications",
        arguments: vec![reference, Term::static_atom("false")],
    })
}

/// Run `erts_internal:is_system_process(Pid)`, which `init` calls for each
/// process when it kills the processes at a stop (`init.erl:1082-1093`), as
/// `erts_internal_is_system_process_1` (`erl_bif_info.c:4932-4947`) does:
/// `true` for a live process with the system flag, `false` for each other
/// pid, and `badarg` for a term that is not a pid. The exit of a system
/// process stops the run, so each process of [`Scheduler::is_system`] is
/// live. The set of system processes is per scheduler, so a system process
/// of another scheduler thread gives `false` (row "Node boot" of
/// `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(is_system_process(&mut scheduler, &Term::Pid(init)), BifResult::Value(Term::Atom(atom)) if atom.is_static("true")));
/// assert!(matches!(is_system_process(&mut scheduler, &Term::Integer(1)), BifResult::Raise { .. }));
/// ```
pub(super) fn is_system_process(scheduler: &mut Scheduler, pid: &Term) -> BifResult {
    match pid {
        Term::Pid(id) => Value(Term::static_atom(if scheduler.is_system(*id) {
            "true"
        } else {
            "false"
        })),
        _ => BifResult::badarg(),
    }
}

/// The indices of the ERTS process table that `erts_internal:processes_next/1`
/// reads for each pid that it can give, and for one reduction
/// (`ERTS_PTAB_REDS_MULTIPLIER`, `erl_ptab.c:1470`).
const PTAB_REDS_MULTIPLIER: u64 = 25;

/// Run `erts_internal:processes_next(Index)`, which `erlang:processes_next/1`
/// of `erlang.beam` calls, as `erts_ptab_processes_next`
/// (`erl_ptab.c:1472-1534`, `bif.c:4107-4118`) does. The call gives at most
/// one pid for each reduction left, and at least one. It reads 25 indices
/// of the table for each such pid, from `Index`, and returns
/// `{Next, Pids}`: the pids of the processes at these indices in the index
/// order of the ERTS table, also an exiting process, and the index after
/// the read. It stops after the last pid that it can give. It charges 1
/// reduction for each 25 indices that it read, and 1 more. `Index` equal
/// to the size of the table gives `none`. An `Index` that is not a small
/// integer, or that is more than the size of the table, raises `badarg`. A
/// process that is not a boot process does not get the pid of the policy
/// process, as for `erlang:processes/0`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = processes_next(&mut scheduler, &Term::Integer(0))?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// ```
///
/// # Errors
///
/// A budget of the current process that is less than 0.
pub(super) fn processes_next(
    scheduler: &mut Scheduler,
    first: &Term,
) -> Result<BifResult, BeamError> {
    let Some(first) = (match first {
        Term::Integer(first) => u64::try_from(*first).ok(),
        _ => None,
    }) else {
        return Ok(BifResult::badarg());
    };
    // `MAX(ERTS_BIF_REDS_LEFT(c_p), 1)` (`erl_ptab.c:1486`).
    let max_pids = scheduler.current_process().bif_reductions_or_zero().max(1) as u64;
    let limit = PTAB_SIZE.min(first.saturating_add(max_pids * PTAB_REDS_MULTIPLIER));
    if first > limit {
        return Ok(BifResult::badarg());
    }
    if first == limit {
        return Ok(Value(Term::static_atom("none")));
    }
    let hidden = super::erlang::hidden_policy(scheduler);
    let found = scheduler.ptab_indexed(first, limit, max_pids as usize, |id| Some(id) != hidden);
    let next = match found.last() {
        Some((index, _)) if found.len() as u64 == max_pids => index + 1,
        _ => limit,
    };
    let pids: Vec<Term> = found.into_iter().map(|(_, id)| Term::Pid(id)).collect();
    let scanned = (next - first) / PTAB_REDS_MULTIPLIER + 1;
    scheduler
        .current_process_mut()
        .bump_reductions(scanned as i32);
    Ok(Value(Term::tuple(vec![
        Term::Integer(next as i64),
        Term::list_from(pids),
    ])))
}

/// Run `erts_internal:request_system_task(Pid, Priority, Request)` for the
/// request `{check_process_code, RequestId, Module}`, which
/// `erts_code_purger` sends to each process (`erts_code_purger.erl:342-344`),
/// as `request_system_task` (`erl_process.c:11309-11492`) checks its
/// arguments. The BIF returns `ok`, and the caller gets the message
/// `{check_process_code, RequestId, Result}`. `Result` is `true` when the
/// process `Pid` has a frame in the old code of `Module`, as
/// `check_process_code` (`beam_bif_load.c:1149-1212`) looks at the
/// instruction pointer and the continuation pointers of the stack, and
/// else `false`. A pid of no process, or of an exiting process, gives
/// `false` (`erl_process.c:11425-11433`, `11478-11483`).
///
/// A `Pid` that is not a pid, a `Priority` that is not `max`, `high`,
/// `normal`, `low`, or `inherit`, and a `Request` that is not a tuple of 2
/// to 4 elements with a known request type raise `badarg`, and so does a
/// `check_process_code` request with a `Module` that is not an atom. ERTS
/// runs the check when the process `Pid` next runs. PBeam runs it at once
/// for a process of the scheduler of the caller (row "Code loading" of
/// `docs/erts-differences.md`). A process of another scheduler of a run
/// with scheduler threads gets the request through the inbox of its
/// scheduler, which runs the check at once when it drains the inbox.
///
/// # Examples
///
/// ```rust,ignore
/// let request = Term::tuple(vec![Term::static_atom("check_process_code"), tag, module]);
/// let result = request_system_task(&mut scheduler, &pid, &Term::static_atom("normal"), &request)?;
/// assert!(matches!(result, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// The request types `garbage_collect` and `system_task_test` give the
/// runtime error `unsupported erts_internal:request_system_task/3 request
/// R`.
pub(super) fn request_system_task(
    scheduler: &mut Scheduler,
    target: &Term,
    priority: &Term,
    request: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Pid(target) = target else {
        return Ok(BifResult::badarg());
    };
    let Term::Atom(priority) = priority else {
        return Ok(BifResult::badarg());
    };
    if !priority.is_one_of(&["max", "high", "normal", "low", "inherit"]) {
        return Ok(BifResult::badarg());
    }
    let Term::Tuple(elements) = request else {
        return Ok(BifResult::badarg());
    };
    let [Term::Atom(kind), id, arguments @ ..] = elements.as_ref() else {
        return Ok(BifResult::badarg());
    };
    if arguments.len() > 2 {
        return Ok(BifResult::badarg());
    }
    // A `garbage_collect` request with an argument that is not `minor` or
    // `major` raises `badarg`, before ERTS schedules the task
    // (`erl_process.c:11412-11423`).
    if kind.is_static("garbage_collect") {
        let [Term::Atom(mode), ..] = arguments else {
            return Ok(BifResult::badarg());
        };
        if !mode.is_one_of(&["minor", "major"]) {
            return Ok(BifResult::badarg());
        }
    }
    // `system_task_test` is not a static atom: ERTS compares its name
    // (`erl_process.c:11447`).
    if kind.is_static("garbage_collect") || scheduler.atoms.name_of(kind) == "system_task_test" {
        return Err(error(format!(
            "unsupported erts_internal:request_system_task/3 request {}",
            scheduler.atoms.name_of(kind)
        )));
    }
    let (true, [Term::Atom(module), ..]) = (kind.is_static("check_process_code"), arguments) else {
        return Ok(BifResult::badarg());
    };
    let caller = scheduler.current_process().id;
    // A process of another scheduler runs the check when it handles the
    // signal, as an ERTS process runs its system task
    // (`erl_process.c:11309-11492`).
    if let Some(index) = scheduler.remote_owner(*target) {
        let node = scheduler.node();
        node.check_process_code(index, *target, caller, id.clone(), module.clone());
        return Ok(Value(Term::static_atom("ok")));
    }
    let refers = scheduler.check_process_code(*target, module);
    let reply = check_process_code_reply(id.clone(), refers);
    scheduler.send_message(caller, caller, reply);
    Ok(Value(Term::static_atom("ok")))
}

/// Run `erts_internal:purge_module(Module, Operation)` for the operations
/// that `erts_code_purger` runs after a failed `-on_load` function
/// (`erts_code_purger.erl:166-182`), as `erts_internal_purge_module_2`
/// (`beam_bif_load.c:2088-2337`) does. A caller that is not the process
/// registered as `erts_code_purger` raises `notsup`. A `Module` that is not
/// an atom, and an `Operation` that is not `prepare`, `prepare_on_load`,
/// `abort`, or `complete`, raise `badarg`.
///
/// - `prepare` returns `false` for a module with no old code, which
///   `code:purge/1` and `code:soft_purge/1` then report, and raises
///   `badarg` while a purge runs.
/// - `prepare_on_load` makes the code that waits for the `-on_load`
///   function of `Module` the old code of the module and returns `true`
///   after the caller switches out. It returns `false` for a module that
///   the run does not have, and raises `badarg` while a purge runs.
/// - `complete` removes the old code and returns `true`. It raises
///   `badarg` when no purge of `Module` runs.
///
/// # Examples
///
/// ```rust,ignore
/// let prepared = purge_module(&mut scheduler, &module, &Term::static_atom("prepare_on_load"))?;
/// assert!(matches!(prepared, BifResult::YieldReturn(_)));
/// ```
///
/// # Errors
///
/// `prepare` of a module with old code after `erlang:delete_module/1`, and
/// `abort`, give the runtime error `unsupported
/// erts_internal:purge_module/2 operation O`: PBeam does not purge such
/// old code yet. `abort` follows only a `prepare` that returns `true`. `prepare_on_load`
/// gives the error of [`CodeTable::prepare_on_load_purge`].
pub(super) fn purge_module(
    scheduler: &mut Scheduler,
    module: &Term,
    operation: &Term,
) -> Result<BifResult, BeamError> {
    let purger = scheduler.whereis("erts_code_purger");
    if purger != Some(scheduler.current_process().id) {
        return Ok(BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::static_atom("notsup"),
        });
    }
    let (Term::Atom(module), Term::Atom(operation)) = (module, operation) else {
        return Ok(BifResult::badarg());
    };
    let name = scheduler.atoms.name_of(module).to_owned();
    // `prepare` raises `badarg` while a purge runs, and gives `false` for
    // a module with no old code (`beam_bif_load.c:2108-2110`, `2143-2144`).
    if operation.is_static("prepare") && scheduler.code.old_code_pending() {
        return Ok(BifResult::badarg());
    }
    if operation.is_static("prepare") && !scheduler.code.has_old(&name) {
        return Ok(Value(Term::static_atom("false")));
    }
    if operation.is_one_of(&["prepare", "abort"]) {
        return Err(error(format!(
            "unsupported erts_internal:purge_module/2 operation {}",
            scheduler.atoms.name_of(operation)
        )));
    }
    if operation.is_static("prepare_on_load") {
        return Ok(match scheduler.code.prepare_on_load_purge(&name)? {
            // ERTS suspends the caller until all schedulers see the purge
            // (`beam_bif_load.c:2169-2185`).
            Some(true) => BifResult::YieldReturn(Term::static_atom("true")),
            Some(false) => Value(Term::static_atom("false")),
            None => BifResult::badarg(),
        });
    }
    if operation.is_static("complete") && scheduler.code.complete_purge(&name) {
        return Ok(Value(Term::static_atom("true")));
    }
    Ok(BifResult::badarg())
}

/// Run `erts_internal:time_unit/0` as `erts_internal_time_unit_0`
/// (`erl_time_sup.c:2537-2540`) does: return the native time unit, the
/// number of parts of a second, `ERTS_MONOTONIC_TIME_UNIT`
/// (`erl_time.h:282-283`). The native unit of PBeam is one nanosecond, so
/// the value is 1000000000.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(time_unit(), BifResult::Value(Term::Integer(1_000_000_000))));
/// ```
pub(super) fn time_unit() -> BifResult {
    Value(Term::Integer(crate::runtime::time::TIME_UNIT))
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/erts_internal.rs"]
mod tests;

/// Run `erts_internal:spawn_system_process(Module, Function, Arguments)`,
/// which `prim_file:on_load/0` calls (`prim_file.erl:117-121`), as
/// `erts_internal_spawn_system_process_3` (`erl_init.c:436-456`) does: start
/// a process that runs `Module:Function` with the elements of `Arguments`
/// and the default spawn options, as `erlang:spawn/3` does
/// (`erl_init.c:333-353`). The process inherits the group leader of the
/// caller. It is a system process (`SPO_SYSTEM_PROC`): its exit stops the
/// run (see `Scheduler::finish_current`).
///
/// ERTS first looks up the export in the active code. When `Module` does
/// not export `Function` with the length of `Arguments`, also for a
/// `Module` or `Function` that is not an atom and for an improper list,
/// it stops the emulator with the text `No function M:F/A` and a crash dump
/// (`erl_init.c:344-346`). PBeam stops the run with the runtime error `No
/// function M:F/A`, with the terms as ERTS prints them, and writes no crash
/// dump.
///
/// # Examples
///
/// ```rust,ignore
/// let start = Term::static_atom("start");
/// let result = spawn_system_process(context, &Term::static_atom("prim_file"), &start, &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// ```
///
/// # Errors
///
/// The runtime error `No function M:F/A` for a function that is not
/// exported, and the errors of `create_process`.
pub(super) fn spawn_system_process(
    context: &mut BifContext<'_>,
    module: &Term,
    function: &Term,
    arguments: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let arity = arguments.proper_list().map(Iterator::count);
    let exported = match (module, function, arity.and_then(|n| u32::try_from(n).ok())) {
        (Term::Atom(module), Term::Atom(function), Some(arity)) => {
            let (module, function) = (
                scheduler.atoms.name_of(module),
                scheduler.atoms.name_of(function),
            );
            resolve(module, function, arity).is_some()
                || scheduler
                    .code
                    .export_frame(module, function, arity)?
                    .is_some()
        }
        _ => false,
    };
    if !exported {
        // `erts_list_length` gives -1 for an improper list.
        let arity = arity.map_or(-1, |n| i64::try_from(n).unwrap_or(i64::MAX));
        let text = |term: &Term| -> String {
            print_term(term, &scheduler.atoms)
                .into_iter()
                .map(char::from)
                .collect()
        };
        return Err(error(format!(
            "No function {}:{}/{arity}",
            text(module),
            text(function)
        )));
    }
    let spawned = create_process(
        scheduler,
        module,
        function,
        arguments,
        SpawnOptions::default(),
    );
    if let Ok(BifResult::Value(Term::Pid(pid))) = &spawned {
        scheduler.mark_system(*pid);
    }
    spawned
}

/// Run `erts_internal:term_to_string(Term, Limit)` as
/// `erts_internal_term_to_string_2` (`bif.c:4156-4190`) does: return the
/// bytes of the `%.*T` text of `Term` with the precision `Limit` as a list
/// of integers (see [`crate::term::print_term_limited`]). `Limit` is
/// a small integer in `2..INT_MAX`, or `undefined` for `INT_MAX`; each other
/// `Limit` raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let text = term_to_string(&atoms, &Term::static_atom("ab"), &Term::Integer(2));
/// // The list of the bytes of "ab... ".
/// ```
pub(super) fn term_to_string(atoms: &AtomTable, term: &Term, limit: &Term) -> BifResult {
    const INT_MAX: i64 = i32::MAX as i64;
    let limit = match limit {
        Term::Integer(limit) if (2..INT_MAX).contains(limit) => *limit,
        Term::Atom(atom) if atom.is_static("undefined") => INT_MAX,
        _ => return BifResult::badarg(),
    };
    let text = crate::term::print_term_limited(term, atoms, limit);
    Value(Term::list_from(
        text.into_iter().map(|byte| Term::Integer(i64::from(byte))),
    ))
}

/// Return the ERTS name of the kind of `term`, as
/// `erts_internal:term_type/1` does (`erl_map.c:3336-3417`). PBeam does
/// not keep the ERTS form of a binary (heap, reference-counted, or
/// sub-binary), so a binary and a native record raise an unsupported error
/// (`docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(term_type(&atoms, &Term::Nil)?, Value(Term::static_atom("nil")));
/// ```
pub(super) fn term_type(atoms: &AtomTable, term: &Term) -> Result<BifResult, BeamError> {
    let name = match term {
        Term::Cons(_) => "list",
        Term::Tuple(_) => "tuple",
        Term::Fun(fun) => match fun.target() {
            Target::Local(_) => "fun",
            Target::External { .. } => "export",
        },
        Term::Map(map) if map.is_hashmap() => "hashmap",
        Term::Map(_) => "flatmap",
        Term::Integer(_) if term.small_integer().is_some() => "fixnum",
        Term::Integer(_) | Term::BigInt(_) => "bignum",
        Term::Reference(_) | Term::Resource(_) => "reference",
        Term::Float(_) => "hfloat",
        Term::Pid(_) => "pid",
        Term::Port(_) => "port",
        Term::Atom(_) => "atom",
        Term::Nil => "nil",
        Term::Binary(_) | Term::Record(_) | Term::MatchContext(_) => {
            return Err(error(
                "erts_internal:term_type/1 cannot name the ERTS form of a binary or a native record"
                    .to_owned(),
            ));
        }
    };
    Ok(Value(Term::Atom(atoms.put(name)?)))
}

/// Return the ERTS term order of `left` against `right`, as
/// `erts_internal:cmp_term/2` does (`bif.c:5417-5427`): `-1` when `left` is
/// less, `0` when the terms are equal, and `1` when `left` is greater. The
/// comparison is exact, because `CMP_TERM` calls `erts_cmp` with
/// `exact = 1` (`erl_utils.h:122`), so each integer is less than each float
/// and `-0.0` is less than `0.0`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(
///     cmp_term(&atoms, &Term::Integer(1), &Term::Float(1.0))?,
///     Value(Term::Integer(-1))
/// );
/// ```
///
/// # Errors
///
/// Never returns an error: each term has a place in the term order.
pub(super) fn cmp_term(
    atoms: &AtomTable,
    left: &Term,
    right: &Term,
) -> Result<BifResult, BeamError> {
    let order = left.compare_exact(right, atoms)?;
    Ok(Value(Term::Integer(match order {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    })))
}

/// Return the key tuple of a flatmap, as
/// `erts_internal:map_to_tuple_keys/1` does (`erl_map.c:3318-3328`): a
/// hashmap raises `error:badarg`, and another term raises
/// `error:{badmap, Term}`. The empty map gives the empty tuple. PBeam keeps
/// no key tuple, and ERTS shares one key tuple between flatmaps, so a new
/// tuple would give a wrong `erts_debug:size/1`: a flatmap with keys stops
/// the run with an unsupported error (`docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(map_to_tuple_keys(&Term::Map(Map::new()))?, Value(Term::tuple(vec![])));
/// ```
pub(super) fn map_to_tuple_keys(term: &Term) -> Result<BifResult, BeamError> {
    let Term::Map(map) = term else {
        return Ok(maps::error_with("badmap", term));
    };
    match map.flat_pairs() {
        Some([]) => Ok(Value(Term::tuple(vec![]))),
        Some(_) => Err(error(
            "erts_internal:map_to_tuple_keys/1 cannot give the shared key tuple of a flatmap"
                .to_owned(),
        )),
        None => Ok(BifResult::badarg()),
    }
}

/// Return whether `left` and `right` are one ERTS term word, as
/// `erts_debug:same/2` does (`beam_debug.c:65-68`). Two equal copies of a
/// term that ERTS boxes and PBeam does not, such as a float, stop the run
/// with an unsupported error, because PBeam cannot tell them apart
/// (`docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(erts_debug_same(&Term::Nil, &Term::Nil)?, Value(Term::static_atom("true")));
/// ```
pub(super) fn erts_debug_same(left: &Term, right: &Term) -> Result<BifResult, BeamError> {
    match left.same_object(right) {
        Some(same) => Ok(Value(Term::static_atom(if same { "true" } else { "false" }))),
        None => Err(error(
            "erts_debug:same/2 cannot tell two equal copies of a float, a reference, or a 64-bit bignum apart"
                .to_owned(),
        )),
    }
}
