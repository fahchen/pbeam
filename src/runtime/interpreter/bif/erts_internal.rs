//! Bodies of the `erts_internal` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::erlang::signals_return;
use super::*;
use crate::beam::code::PrepareError;
use crate::runtime::{
    capability::{CapabilityKind, Operation},
    host::{
        errno::errno_id,
        filename_encoding,
        spawn::{self, SpawnError, Streams},
    },
    port::Ports,
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
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let result = group_leader(&mut context, &Term::Integer(1), &Term::Pid(target))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "badarg"));
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
    let result = match scheduler.process_mut(*target) {
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
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "ok"));
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
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "ok"));
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
/// of `port_command/2`, the exit of `port_close/1`, and the info of
/// `port_info/1`, each `CONTEXT_REDS/100` (`erl_port.h:369-370`, `380`;
/// `io.c:2071`, `2282`, `4906`).
const PORT_REDUCTIONS: i32 = 40;

/// The tuple options of `open_port/2` that ERTS accepts and PBeam does not
/// support (`erl_bif_port.c:849-976`).
const UNSUPPORTED_TUPLE_OPTIONS: [&str; 8] = [
    "packet",
    "line",
    "env",
    "arg0",
    "cd",
    "parallelism",
    "busy_limits_port",
    "busy_limits_msgq",
];

/// The atom options of `open_port/2` that ERTS accepts and PBeam does not
/// support (`erl_bif_port.c:979-1009`).
const UNSUPPORTED_ATOM_OPTIONS: [&str; 7] = [
    "stream",
    "line",
    "in",
    "out",
    "eof",
    "hide",
    "overlapped_io",
];

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
    /// The native bytes of the strings of `{args, Args}`, or no bytes
    /// without it.
    args: Vec<Vec<u8>>,
}

/// Run `erts_internal:open_port(PortName, PortSettings)`, which
/// `erlang:open_port/2` calls, as `erts_internal_open_port_2` and
/// `open_port` do (`erl_bif_port.c:60-130`, `795-1171`) for
/// `{spawn_executable, Name}`:
///
/// 1. Read the settings in order (see [`port_settings`]). A list that is
///    not proper, an element that ERTS does not accept, and
///    `stderr_to_stdout` with `nouse_stdio` give the atom `badopt`.
/// 2. Read the name. A term that is not `{spawn_executable, Name}`, or a
///    `Name` that is not a file name (see [`native_name`]), gives the atom
///    `badarg`.
/// 3. Check the capabilities of the caller: without a `port` capability
///    with `spawn` that permits the native bytes of `Name`, the BIF raises
///    `error:capability_denied` and starts no program. The constraint
///    compares the name as the caller gives it. It does not resolve a
///    relative name or a symbolic link.
/// 4. Start the program (see [`spawn::spawn`]). A pipe, the fd limit, and
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
/// An option that ERTS accepts and PBeam does not support, the port kinds
/// `spawn`, `spawn_driver`, and `fd`, a port without `binary`, and a failed
/// `posix_spawn` return a runtime error. A new atom in a full atom table
/// gives the error of `AtomTable::put`.
pub(super) fn open_port(
    context: &mut BifContext<'_>,
    name: &Term,
    settings: &Term,
) -> Result<BifResult, BeamError> {
    let Some(settings) = port_settings(&context.scheduler.atoms, settings)? else {
        return Ok(Value(Term::static_atom("badopt")));
    };
    let Some(path) = spawn_executable(&context.scheduler.atoms, name)? else {
        return Ok(Value(Term::static_atom("badarg")));
    };
    if !settings.binary {
        return Err(error(
            "unsupported open_port/2 without the binary option: see \"Ports\" in docs/erts-differences.md",
        ));
    }
    let scheduler = &mut *context.scheduler;
    let check = denial(
        scheduler,
        CapabilityKind::Port,
        Operation::Spawn,
        Some(&path),
    );
    if let Some(denied) = check? {
        return Ok(denied);
    }
    let child = match spawn::spawn(&path, &settings.args, settings.streams) {
        Ok(child) => child,
        Err(SpawnError::Open(errno)) => {
            return Ok(Value(Term::Atom(scheduler.atoms.put(&errno_id(errno))?)));
        }
        Err(SpawnError::Start(errno)) => {
            return Err(error(format!(
                "unsupported open_port/2 of a program that posix_spawn does not start: {}: see \"Ports\" in docs/erts-differences.md",
                errno_id(errno)
            )));
        }
    };
    let port = scheduler.open_port(path, settings.exit_status, child)?;
    let reference = scheduler.make_ref()?;
    let caller = scheduler.current_process_mut().id;
    let ack = Term::tuple(vec![Term::Reference(reference), Term::Port(port)]);
    scheduler.deliver(caller, ack);
    Ok(Value(Term::Reference(reference)))
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
        args: Vec::new(),
    };
    for element in elements {
        let option = match element {
            Term::Tuple(pair) if pair.len() == 2 => {
                let Term::Atom(option) = &pair[0] else {
                    return Ok(None);
                };
                match option.name() {
                    "args" => {
                        let Some(args) = port_args(&pair[1]) else {
                            return Ok(None);
                        };
                        result.args = args;
                        continue;
                    }
                    name if UNSUPPORTED_TUPLE_OPTIONS.contains(&name) => option,
                    _ => return Ok(None),
                }
            }
            Term::Atom(option) => {
                match option.name() {
                    "use_stdio" => result.streams.use_stdio = true,
                    "nouse_stdio" => result.streams.use_stdio = false,
                    "stderr_to_stdout" => result.streams.stderr_to_stdout = true,
                    "binary" => result.binary = true,
                    "exit_status" => result.exit_status = true,
                    name if UNSUPPORTED_ATOM_OPTIONS.contains(&name) => {
                        return Err(unsupported_option(name));
                    }
                    _ => return Ok(None),
                }
                continue;
            }
            _ => return Ok(None),
        };
        return Err(unsupported_option(atoms.name_of(option)));
    }
    if result.streams.stderr_to_stdout && !result.streams.use_stdio {
        return Ok(None);
    }
    Ok(Some(result))
}

/// Return the runtime error of an option of `open_port/2` that ERTS accepts
/// and PBeam does not support.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(unsupported_option("line").to_string().starts_with("unsupported open_port/2 option line"));
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
/// let args = Term::list_from(vec![Term::binary(b"-c".to_vec()), Term::nil()]);
/// assert_eq!(port_args(&args), Some(vec![b"-c".to_vec(), Vec::new()]));
/// ```
fn port_args(args: &Term) -> Option<Vec<Vec<u8>>> {
    let elements = args.proper_list()?;
    elements.map(|arg| native_name(arg, true)).collect()
}

/// Return the native bytes of the program of `{spawn_executable, Name}`,
/// or `None` for the `badarg` of ERTS: a term that is not a tuple with a
/// first element that ERTS accepts, a tuple of another size, and a name
/// that is not a file name without `[]` (`erl_bif_port.c:1033-1067`).
///
/// # Examples
///
/// ```rust,ignore
/// let name = Term::tuple(vec![Term::static_atom("spawn_executable"), Term::binary(b"/bin/sh".to_vec())]);
/// assert_eq!(spawn_executable(&name)?, Some(b"/bin/sh".to_vec()));
/// ```
///
/// # Errors
///
/// The port kinds `spawn`, `spawn_driver`, and `fd` return a runtime error.
fn spawn_executable(atoms: &AtomTable, name: &Term) -> Result<Option<Vec<u8>>, BeamError> {
    let Term::Tuple(elements) = name else {
        return Ok(None);
    };
    match elements.as_ref() {
        [Term::Atom(kind), name] if kind.name() == "spawn_executable" => {
            Ok(native_name(name, false))
        }
        [Term::Atom(kind), ..] if matches!(kind.name(), "spawn" | "spawn_driver" | "fd") => {
            Err(error(format!(
                "unsupported open_port/2 port kind {}: see \"Ports\" in docs/erts-differences.md",
                atoms.name_of(kind)
            )))
        }
        _ => Ok(None),
    }
}

/// Return the native bytes of a file name, as
/// `erts_convert_filename_to_encoding` with `allow_atom` does
/// (`erl_unicode.c:2057-2119`): a binary gives its bytes before its first
/// byte 0, because ERTS copies the bytes into a C string, and the port name
/// is that C string (`erl_unicode.c:2107-2108`, `io.c:323`, `366-367`). An
/// atom gives the bytes of its characters, and a list gives the bytes of
/// [`native_filename`]. `[]` is a name only with `allow_empty`. Return
/// `None` for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(native_name(&Term::binary(b"/bin/sh\0x".to_vec()), false), Some(b"/bin/sh".to_vec()));
/// assert_eq!(native_name(&Term::static_atom("ok"), false), Some(b"ok".to_vec()));
/// assert_eq!(native_name(&Term::nil(), false), None);
/// assert_eq!(native_name(&Term::nil(), true), Some(Vec::new()));
/// ```
fn native_name(name: &Term, allow_empty: bool) -> Option<Vec<u8>> {
    let characters;
    let name = match name {
        Term::Binary(bytes) => {
            let end = bytes.iter().position(|&byte| byte == 0);
            return Some(bytes[..end.unwrap_or(bytes.len())].to_vec());
        }
        Term::Atom(atom) => {
            let text = atom.name().chars();
            characters = Term::list_from(text.map(|c| Term::Integer(u32::from(c).into())));
            &characters
        }
        name if name.is_nil() && !allow_empty => return None,
        name => name,
    };
    native_filename(name, filename_encoding())
}

/// Return the open port of `port`, as `sig_lookup_port` finds it
/// (`erl_bif_port.c:133-147`), or `None` for a term that is not a port of
/// an open port. An atom names no port, because PBeam registers no port.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(open_port_id(scheduler, &Term::Port(port)), Some(port));
/// assert_eq!(open_port_id(scheduler, &Term::static_atom("foo")), None);
/// ```
fn open_port_id(scheduler: &mut Scheduler, port: &Term) -> Option<PortId> {
    let Term::Port(id) = port else {
        return None;
    };
    scheduler
        .resources
        .borrow_mut::<Ports>()
        .get(*id)
        .map(|_| *id)
}

/// Run `erts_internal:port_command(Port, Data, OptionList)`, which
/// `erlang:port_command/2` calls with `[]`, as
/// `erts_internal_port_command_3` and `erts_port_output` do
/// (`erl_bif_port.c:168-261`, `io.c:1715-2088`): write the bytes of the
/// iodata `Data` to the input of the program of `Port`, add them to the
/// output count, use 40 reductions, and return `true`. A failed write
/// closes the port with the `erl_errno_id` atom of the failure as its
/// reason, for example `epipe`, as `driver_failure_posix` does
/// (`sys_drivers.c:1180-1184`). A term that is not an open port, or `Data`
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
/// An `OptionList` other than `[]` returns a runtime error. A new atom in a
/// full atom table gives the error of `AtomTable::put`.
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
    let (Some(id), Some(bytes)) = (open_port_id(scheduler, port), iolist::iodata_bytes(data))
    else {
        return Ok(signals_return(scheduler, port, Term::static_atom("badarg")));
    };
    if let Some(denied) = port_denial(scheduler, id, Operation::Command)? {
        return Ok(denied);
    }
    scheduler
        .current_process_mut()
        .bump_reductions(PORT_REDUCTIONS);
    let ports = scheduler.resources.borrow_mut::<Ports>();
    let port_state = ports.get_mut(id).expect("the port is open");
    match port_state.child.write_input(&bytes) {
        Ok(()) => port_state.output += bytes.len() as u64,
        Err(errno) => {
            let reason = Term::Atom(scheduler.atoms.put(&errno_id(errno))?);
            scheduler.close_port(id, reason);
        }
    }
    Ok(signals_return(scheduler, port, Term::static_atom("true")))
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
    let Some(id) = open_port_id(scheduler, port) else {
        return Ok(signals_return(scheduler, port, Term::static_atom("badarg")));
    };
    if let Some(denied) = port_denial(scheduler, id, Operation::Close)? {
        return Ok(denied);
    }
    scheduler
        .current_process_mut()
        .bump_reductions(PORT_REDUCTIONS);
    scheduler.close_port(id, Term::static_atom("normal"));
    Ok(signals_return(scheduler, port, Term::static_atom("true")))
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
fn port_denial(
    scheduler: &mut Scheduler,
    id: PortId,
    operation: Operation,
) -> Result<Option<BifResult>, BeamError> {
    let ports = scheduler.resources.borrow_mut::<Ports>();
    let name = ports.get(id).expect("the port is open").name.clone();
    denial(scheduler, CapabilityKind::Port, operation, Some(&name))
}

/// Run `erts_internal:port_info(Port)`, which `erlang:port_info/1` calls, as
/// `erts_internal_port_info_1` and `erts_port_info` do
/// (`erl_bif_port.c:486-535`, `erl_bif_info.c:3654-3830`): for an open
/// port, use 40 reductions and return `[{name, Name}, {links, Links},
/// {id, Number}, {connected, Owner}, {input, Bytes}, {output, Bytes},
/// {os_pid, OsPid}]` (`ERTS_PORT_INFO_1_ITEMS`, `erl_port.h:100-108`). The
/// name is the list of the bytes of the program path, `links` holds the
/// owner, and `id` is the port number. A port
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
    let info = match (port, open_port_id(scheduler, port)) {
        (_, Some(id)) => {
            if let Some(denied) = port_denial(scheduler, id, Operation::Inspect)? {
                return Ok(denied);
            }
            scheduler
                .current_process_mut()
                .bump_reductions(PORT_REDUCTIONS);
            let port = scheduler
                .resources
                .borrow_mut::<Ports>()
                .get(id)
                .expect("the port is open");
            let item = |name, value| Term::tuple(vec![Term::static_atom(name), value]);
            let links = vec![Term::Pid(port.owner)];
            Term::list_from(vec![
                item(
                    "name",
                    Term::list_from(port.name.iter().map(|&byte| Term::Integer(byte.into()))),
                ),
                item("links", Term::list_from(links)),
                item(
                    "id",
                    Term::Integer(id.0.try_into().expect("a port number below 2^63")),
                ),
                item("connected", Term::Pid(port.owner)),
                item(
                    "input",
                    Term::Integer(port.input.try_into().expect("an input count below 2^63")),
                ),
                item(
                    "output",
                    Term::Integer(port.output.try_into().expect("an output count below 2^63")),
                ),
                item("os_pid", Term::Integer(port.child.os_pid.into())),
            ])
        }
        (Term::Port(_) | Term::Atom(_), None) => Term::static_atom("undefined"),
        (_, None) => Term::static_atom("badarg"),
    };
    Ok(signals_return(scheduler, port, info))
}

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
    let (Term::Atom(module), Term::Binary(bytes)) = (module, code) else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    let module_name = scheduler.atoms.name_of(module).to_owned();
    match code::prepare(bytes, &module_name, &mut scheduler.atoms) {
        Ok(prepared) => {
            let reference = scheduler.make_ref()?;
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
