//! Bodies of the BIFs that give the local node its name and its creation
//! (`dist.c`).

use std::sync::Arc;

use num_bigint::BigInt;

use super::{BifContext, BifResult, Term};
use crate::error::{BeamError, error};
use crate::runtime::scheduler::{Scheduler, dist_table::Controller, node::Named};
use crate::term::{Atom, PortId, ProcessId};

/// The lowest creation of an alive node. `setnode/2` raises a lower
/// creation to it (`dist.c:5029-5030`).
const MIN_CREATION: u32 = 4;

/// Run `erlang:setnode(Name, Creation)` as ERTS `setnode_2` does
/// (`dist.c:4978-5071`): give the local node the name `Name` and the
/// creation `Creation`, mark the process registered as `net_kernel` as the
/// owner of the distribution, and return `true`. Its exit gives the node
/// `nonode@nohost` and the creation 0 again (see `Node::net_exits`).
///
/// `Creation` is a small integer that is not negative, and only its low 32
/// bits count (`term_to_Uint32`, `big.c:3648-3663`); a creation below 4
/// becomes 4. `badarg` comes for another creation, for a name that is not a
/// node name atom or is `nonode@nohost`, for a node that is alive, and when
/// no live process has the name `net_kernel`.
///
/// ERTS also skips each creation that a live identifier of another
/// incarnation of the same name has (`dist.c:5027-5035`). PBeam keeps no
/// node table, so it does not skip a creation; the row "Distribution" of
/// `docs/erts-differences.md` records this.
///
/// # Examples
///
/// ```rust,ignore
/// // net_kernel is registered and the node is not alive.
/// assert!(matches!(setnode(context, &name, &Term::Integer(1))?, BifResult::Value(_)));
/// assert_eq!(context.scheduler.atoms.this_node(), (name, 4));
/// ```
pub(super) fn setnode(
    context: &mut BifContext<'_>,
    name: &Term,
    creation: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    // A small integer that is not negative gives its low 32 bits, as the
    // C cast to `Uint32` does.
    let Some(creation) = creation
        .small_integer()
        .filter(|creation| *creation >= 0)
        .map(|creation| {
            u32::try_from(creation & i64::from(u32::MAX)).expect("the low 32 bits fit in u32")
        })
    else {
        return Ok(BifResult::badarg());
    };
    let Term::Atom(name) = name else {
        return Ok(BifResult::badarg());
    };
    if !is_node_name(scheduler.atoms.name_of(name).as_bytes()) || name.is_static("nonode@nohost") {
        return Ok(BifResult::badarg());
    }
    let Some(Named::Process(net_kernel)) = scheduler.name_owner("net_kernel") else {
        return Ok(BifResult::badarg());
    };
    let creation = creation.max(MIN_CREATION);
    if !scheduler
        .node()
        .set_this_node(&scheduler.atoms, net_kernel, name, creation)
    {
        return Ok(BifResult::badarg());
    }
    let nonode = Atom::from_static("nonode@nohost").expect("nonode@nohost is a static atom");
    let (nonode_handle, handle) = (scheduler.make_magic_ref()?, scheduler.make_magic_ref()?);
    let start = connection_start(scheduler);
    scheduler
        .node()
        .dist()
        .set_local(&nonode, nonode_handle, name, handle, start);
    Ok(BifResult::Value(Term::static_atom("true")))
}

/// Return the first connection id of a new dist entry before the mask: the
/// low 32 bits of the monotonic time, as ERTS takes it
/// (`erl_node_tables.c:157-162`).
fn connection_start(scheduler: &Scheduler) -> u32 {
    u32::try_from(scheduler.timers.now() & i64::from(u32::MAX)).expect("the low 32 bits fit in u32")
}

/// Run `erts_internal:get_creation/0` as ERTS does (`dist.c:5571-5586`):
/// return the creation of the local node, or `undefined` for the creation 0
/// of a node that is not alive.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(get_creation(0), Term::static_atom("undefined"));
/// assert_eq!(get_creation(4), Term::Integer(4));
/// ```
pub(super) fn get_creation(creation: u32) -> Term {
    if creation == 0 {
        Term::static_atom("undefined")
    } else {
        Term::Integer(i64::from(creation))
    }
}

/// Return whether `name` is a full node name, as `is_node_name` checks it
/// (`dist.c:815-841`): one or more of `[a-zA-Z0-9_-]`, then `@`, then one or
/// more bytes with no other `@`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_node_name(b"a@host"));
/// assert!(!is_node_name(b"a.b@host"));
/// assert!(!is_node_name(b"a@b@c"));
/// assert!(!is_node_name(b"a@"));
/// ```
fn is_node_name(name: &[u8]) -> bool {
    let Some(at) = name
        .iter()
        .position(|&byte| !(byte == b'-' || byte == b'_' || byte.is_ascii_alphanumeric()))
    else {
        return false;
    };
    name[at] == b'@' && at > 0 && at + 1 < name.len() && !name[at + 1..].contains(&b'@')
}

/// The mandatory distribution flags (`DFLAG_DIST_MANDATORY`,
/// `dist.h:80-98`): the flags of OTP 25 and of OTP 26.
const MANDATORY_FLAGS: u64 = 0x0004 // DFLAG_EXTENDED_REFERENCES
    | 0x0010 // DFLAG_FUN_TAGS
    | 0x0100 // DFLAG_EXTENDED_PIDS_PORTS
    | 0x10000 // DFLAG_UTF8_ATOMS
    | 0x0080 // DFLAG_NEW_FUN_TAGS
    | 0x40000 // DFLAG_BIG_CREATION
    | 0x0800 // DFLAG_NEW_FLOATS
    | 0x20000 // DFLAG_MAP_TAG
    | 0x0200 // DFLAG_EXPORT_PTR_TAG
    | 0x0400 // DFLAG_BIT_BINARIES
    | 0x0100_0000 // DFLAG_HANDSHAKE_23
    | (0x4 << 32) // DFLAG_V4_NC
    | 0x0200_0000; // DFLAG_UNLINK_ID

/// The flags that a local distribution module can reject
/// (`DFLAG_DIST_REJECTABLE`, `dist.h:129-132`): `DFLAG_DIST_HDR_ATOM_CACHE`,
/// `DFLAG_HIDDEN_ATOM_CACHE`, `DFLAG_FRAGMENTS`, and `DFLAG_ATOM_CACHE`.
const REJECTABLE_FLAGS: u64 = 0x2000 | 0x40 | 0x0080_0000 | 0x02;

/// The flags of the features that need delivery in strict order
/// (`DFLAG_DIST_STRICT_ORDER`, `dist.h:135`): `DFLAG_DIST_HDR_ATOM_CACHE`.
const STRICT_ORDER_FLAGS: u64 = 0x2000;

/// Return whether `name` is a node name atom (`is_node_name_atom`,
/// `dist.c:843-851`).
fn node_name_atom<'a>(context: &BifContext<'_>, name: &'a Term) -> Option<&'a Atom> {
    match name {
        Term::Atom(atom) if is_node_name(context.scheduler.atoms.name_of(atom).as_bytes()) => {
            Some(atom)
        }
        _ => None,
    }
}

/// Run `erts_internal:get_dflags/0` (`dist.c:5543-5569`): return
/// `{erts_dflags, Default, Mandatory, Addable, Rejectable, StrictOrder}`.
///
/// PBeam offers only the mandatory flags as its default flags, and a
/// distribution module can add only those (`DFLAG_DIST_ADDABLE` is the
/// default, `dist.h:126`); ERTS also offers the flags of `dist.h:105-123`.
/// The rejectable and the strict order flags are those of ERTS. See row
/// "Distribution" of `docs/erts-differences.md`.
///
/// # Examples
///
/// ```rust,ignore
/// // {erts_dflags,17230663572,17230663572,17230663572,8396866,8192}
/// let Term::Tuple(record) = get_dflags() else { unreachable!() };
/// assert_eq!(record.len(), 6);
/// ```
pub(super) fn get_dflags() -> Term {
    let flags = |flags: u64| Term::from_bigint(BigInt::from(flags));
    Term::tuple(vec![
        Term::static_atom("erts_dflags"),
        flags(MANDATORY_FLAGS),
        flags(MANDATORY_FLAGS),
        flags(MANDATORY_FLAGS),
        flags(REJECTABLE_FLAGS),
        flags(STRICT_ORDER_FLAGS),
    ])
}

/// Run `erts_internal:new_connection(Node)` as
/// `erts_internal_new_connection_1` does (`dist.c:5588-5625`): find or
/// make the dist entry of `Node`, make an idle entry pending with the next
/// connection id, and return the dist handle `{ConnId, MagicRef}` of the
/// current connection of the entry. `badarg` comes for a term that is not
/// a node name atom and for the name of the local node.
///
/// # Examples
///
/// ```rust,ignore
/// // {9574870, #Ref<0.290125712.2216820737.68546>}
/// assert!(matches!(new_connection(context, &node)?, BifResult::Value(Term::Tuple(_))));
/// ```
pub(super) fn new_connection(
    context: &mut BifContext<'_>,
    node: &Term,
) -> Result<BifResult, BeamError> {
    let Some(name) = node_name_atom(context, node) else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    if *name == scheduler.atoms.this_node().0 {
        return Ok(BifResult::badarg());
    }
    let handle = scheduler.make_magic_ref()?;
    let start = connection_start(scheduler);
    let mut dist = scheduler.node().dist();
    dist.insert(name, handle, start);
    dist.new_connection(name);
    Ok(BifResult::Value(dist.current_dhandle(name)))
}

/// Run `erts_internal:abort_pending_connection(Node, DHandle)` as
/// `erts_internal_abort_pending_connection_2` does (`dist.c:5693-5711`):
/// make the pending connection of `DHandle` idle, and return `false` when
/// the connection of `DHandle` is up, else `true`. `badarg` comes for a
/// `Node` that is not an atom and for a dist handle that is not of the
/// entry of `Node`.
///
/// # Examples
///
/// ```rust,ignore
/// let handle = new_connection(context, &node);
/// assert_eq!(abort_pending_connection(context, &node, &handle), BifResult::Value(Term::from(true)));
/// ```
pub(super) fn abort_pending_connection(
    context: &mut BifContext<'_>,
    node: &Term,
    handle: &Term,
) -> BifResult {
    let Term::Atom(name) = node else {
        return BifResult::badarg();
    };
    let mut dist = context.scheduler.node().dist();
    match dist.lookup(handle) {
        Some((entry, connection_id)) if entry == *name => {
            let connected = dist.abort_pending(name, connection_id);
            BifResult::Value(Term::from(!connected))
        }
        _ => BifResult::badarg(),
    }
}

/// Run `erts_internal:create_dist_channel(Node, Controller, {Flags,
/// Creation})` for a process controller as
/// `erts_internal_create_dist_channel_3` does (`dist.c:5127-5383`): make
/// the pending connection of `Node` connected with `Controller` as its
/// distribution controller, send `{dist_ctrlr, Controller, Node, Caller}`
/// to `net_kernel` (`dist.c:5394-5408`), and return `{ok, DHandle}` when
/// the caller is the controller, or `{message, Ref}` and the message
/// `{Ref, DHandle}` to the caller.
///
/// ERTS runs the setup in the controller when the caller is another
/// process (`erts_proc_sig_send_rpc_request`, `dist.c:5234-5266`), and
/// the controller replies. PBeam runs it in the caller under the lock of
/// the dist table, which orders it with the exit of the controller: a
/// controller that left the process table gives `badarg`, as a request to
/// a process that does not exist does (`dist.c:5255-5256`), and one that
/// exits after the setup makes the entry idle. ERTS can also reply
/// `{Ref, badarg}` to a controller that exits between the two steps
/// (`dist.c:5474-5475`); PBeam has no such window.
///
/// The result is the atom `badarg` (`dist.c:5375-5377`), which
/// `erlang:setnode/3` raises (`erlang.erl:6077-6087`), when no process with
/// the distribution flag is registered as `net_kernel`, for a `Node` that is not a node name atom or
/// that is the local node, for a `Controller` that is not a local pid, for
/// flags that are not an integer from 0 to 2^64 - 1 or a creation that is
/// not a small integer that is not negative, and when the entry is not pending, has a controller, or the
/// controller controls another entry. Flags without each mandatory flag
/// stop the run with a runtime error, where ERTS sends an error report and
/// raises `badarg` (`dist.c:5176-5187`). A port controller takes
/// [`port_channel`].
///
/// # Examples
///
/// ```rust,ignore
/// // The caller is the controller.
/// let result = create_dist_channel(context, &node, &Term::Pid(caller), &options)?;
/// assert!(matches!(result, BifResult::Value(Term::Tuple(_))));
/// ```
pub(super) fn create_dist_channel(
    context: &mut BifContext<'_>,
    node: &Term,
    controller: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let scheduler = &mut *context.scheduler;
    let net_kernel = match scheduler.name_owner("net_kernel") {
        Some(Named::Process(net_kernel)) if scheduler.node().net_kernel() == Some(net_kernel) => {
            net_kernel
        }
        _ => return Ok(BifResult::Value(Term::static_atom("badarg"))),
    };
    let Some(name) = node_name_atom(context, node) else {
        return Ok(BifResult::Value(Term::static_atom("badarg")));
    };
    let scheduler = &mut *context.scheduler;
    let controller = match controller {
        Term::Pid(pid) => Controller::Process(*pid),
        Term::Port(port) => Controller::Port(*port),
        _ => return Ok(BifResult::Value(Term::static_atom("badarg"))),
    };
    let Term::Tuple(options) = options else {
        return Ok(BifResult::Value(Term::static_atom("badarg")));
    };
    let [flags, creation] = options.as_ref() else {
        return Ok(BifResult::Value(Term::static_atom("badarg")));
    };
    // `term_to_Uint32` takes a small integer that is not negative and keeps
    // its low 32 bits (`big.c:3647-3663`); `term_to_Uint64` takes an
    // integer from 0 to 2^64 - 1 (`big.c:3607-3645`).
    let (Some(flags), Some(_)) = (
        flags.bigint().and_then(|flags| u64::try_from(&*flags).ok()),
        creation.small_integer().filter(|creation| *creation >= 0),
    ) else {
        return Ok(BifResult::Value(Term::static_atom("badarg")));
    };
    if !flags & MANDATORY_FLAGS != 0 {
        return Err(error(
            "unsupported erts_internal:create_dist_channel/3 with flags that miss a mandatory flag: see \"Distribution\" in docs/erts-differences.md",
        ));
    }
    if *name == scheduler.atoms.this_node().0 {
        return Ok(BifResult::Value(Term::static_atom("badarg")));
    }
    let caller = scheduler.current_process().id;
    let controller = match controller {
        Controller::Process(process) => process,
        Controller::Port(port) => {
            return port_channel(scheduler, name, port, flags, net_kernel);
        }
    };
    let start = connection_start(scheduler);
    let node = Arc::clone(scheduler.node());
    let handle = {
        let mut dist = node.dist();
        // ERTS makes the entry first (`dist.c:5199`).
        let reference = scheduler.make_magic_ref()?;
        dist.insert(name, reference, start);
        // ERTS cannot send the request to a controller that does not
        // exist (`dist.c:5250-5256`). The caller itself is live.
        if node.live_owner(controller).is_none() {
            return Ok(BifResult::Value(Term::static_atom("badarg")));
        }
        dist.connect(name, Controller::Process(controller), flags)
    };
    let Some(handle) = handle else {
        return Ok(if caller == controller {
            BifResult::Value(Term::static_atom("badarg"))
        } else {
            message_reply(scheduler, caller, Term::static_atom("badarg"))?
        });
    };
    scheduler.send_message(
        controller,
        net_kernel,
        Term::tuple(vec![
            Term::static_atom("dist_ctrlr"),
            Term::Pid(controller),
            Term::Atom(name.clone()),
            Term::Pid(caller),
        ]),
    );
    Ok(if caller == controller {
        BifResult::Value(Term::tuple(vec![Term::static_atom("ok"), handle]))
    } else {
        message_reply(scheduler, caller, handle)?
    })
}

/// Make the pending connection of `name` connected with the port
/// controller `port`, as the port branch of
/// `erts_internal_create_dist_channel_3` does (`dist.c:5269-5336`), send
/// `{dist_ctrlr, Port, Node, Caller}` to `net_kernel`
/// (`dist.c:5394-5408`), and return `{ok, DHandle}`. The result is the atom
/// `badarg` when the entry is not pending, the port is not open, its
/// driver has no `ERL_DRV_FLAG_SOFT_BUSY` (only `tcp_inet` has it), or the
/// port already controls a connection. The port lock comes before the lock
/// of the dist table, as in the exit of a port, so a port that exits after
/// the setup makes the entry idle.
fn port_channel(
    scheduler: &mut Scheduler,
    name: &Atom,
    port: PortId,
    flags: u64,
    net_kernel: ProcessId,
) -> Result<BifResult, BeamError> {
    let start = connection_start(scheduler);
    let reference = scheduler.make_magic_ref()?;
    let node = Arc::clone(scheduler.node());
    let setup = |item: Option<&mut crate::runtime::port::Port>| {
        let mut dist = node.dist();
        // ERTS makes the entry first (`dist.c:5199`).
        dist.insert(name, reference, start);
        item.filter(|item| !item.closing && item.soft_busy())
            .and_then(|_| dist.connect(name, Controller::Port(port), flags))
    };
    let handle = match node.with_port(port, |item| setup(Some(item))) {
        Some(handle) => handle,
        None => setup(None),
    };
    let Some(handle) = handle else {
        return Ok(BifResult::Value(Term::static_atom("badarg")));
    };
    let caller = scheduler.current_process().id;
    scheduler.send_message(
        caller,
        net_kernel,
        Term::tuple(vec![
            Term::static_atom("dist_ctrlr"),
            Term::Port(port),
            Term::Atom(name.clone()),
            Term::Pid(caller),
        ]),
    );
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("ok"),
        handle,
    ])))
}

/// Give the caller the reply `{Ref, Result}` of the setup that ERTS runs
/// in the controller, and return `{message, Ref}` (`dist.c:5265`).
fn message_reply(
    scheduler: &mut Scheduler,
    caller: ProcessId,
    result: Term,
) -> Result<BifResult, BeamError> {
    let reference = Term::Reference(scheduler.make_ref()?);
    scheduler.deliver(caller, Term::tuple(vec![reference.clone(), result]));
    Ok(BifResult::Value(Term::tuple(vec![
        Term::static_atom("message"),
        reference,
    ])))
}

/// Return the `error:notsup` exception (`EXC_NOTSUP`).
fn notsup() -> BifResult {
    BifResult::Raise {
        class: super::ExceptionClass::Error,
        reason: Term::static_atom("notsup"),
    }
}

/// Return the entry that the caller controls when `handle` is a dist
/// handle of that entry and of its current connection, as the start of the
/// `dist_ctrl_*` BIFs checks it (`dist.c:4275-4297`): `notsup` when the
/// caller controls no entry, and `badarg` for another handle.
fn controlled_entry(context: &BifContext<'_>, handle: &Term) -> Result<Atom, BifResult> {
    let scheduler = &context.scheduler;
    let dist = scheduler.node().dist();
    let Some(entry) = dist.controlled_by(Controller::Process(scheduler.current_process().id))
    else {
        return Err(notsup());
    };
    match dist.lookup(handle) {
        Some((name, id)) if name == entry && id == dist.connection_id(&name) => Ok(name),
        _ => Err(BifResult::badarg()),
    }
}

/// Run `erlang:dist_ctrl_get_data_notification(DHandle)` as
/// `dist_ctrl_get_data_notification_1` does (`dist.c:4272-4330`), and
/// return `ok`. ERTS sends `dist_data` to the controller when the output
/// queue of the connection has data; PBeam sends no data to another node
/// yet (#1407), so the queue stays empty and no notification comes.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(dist_ctrl_get_data_notification(context, &handle), BifResult::Value(Term::static_atom("ok")));
/// ```
pub(super) fn dist_ctrl_get_data_notification(
    context: &mut BifContext<'_>,
    handle: &Term,
) -> BifResult {
    match controlled_entry(context, handle) {
        Ok(_) => BifResult::Value(Term::static_atom("ok")),
        Err(error) => error,
    }
}

/// Run `erlang:dist_ctrl_get_data(DHandle)` as `dist_ctrl_get_data_1`
/// does (`dist.c:4558-4630`): return `none` for an empty output queue.
/// PBeam sends no data to another node yet (#1407), so the queue stays
/// empty.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(dist_ctrl_get_data(context, &handle), BifResult::Value(Term::static_atom("none")));
/// ```
pub(super) fn dist_ctrl_get_data(context: &mut BifContext<'_>, handle: &Term) -> BifResult {
    match controlled_entry(context, handle) {
        Ok(_) => BifResult::Value(Term::static_atom("none")),
        Err(error) => error,
    }
}

/// Run `erlang:dist_ctrl_get_opt(DHandle, get_size)` as
/// `dist_ctrl_get_opt_2` does (`dist.c:4451-4486`): return the option
/// `get_size`. Another option raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(dist_ctrl_get_opt(context, &handle, &get_size), BifResult::Value(Term::from(false)));
/// ```
pub(super) fn dist_ctrl_get_opt(
    context: &mut BifContext<'_>,
    handle: &Term,
    option: &Term,
) -> BifResult {
    let name = match controlled_entry(context, handle) {
        Ok(name) => name,
        Err(error) => return error,
    };
    match option {
        Term::Atom(option) if option.is_static("get_size") => {
            BifResult::Value(Term::from(context.scheduler.node().dist().get_size(&name)))
        }
        _ => BifResult::badarg(),
    }
}

/// Run `erlang:dist_ctrl_set_opt(DHandle, get_size, Value)` as
/// `dist_ctrl_set_opt_3` does (`dist.c:4408-4449`): set the option
/// `get_size` to `true` or `false`, and return the old value. Another
/// option or value raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(dist_ctrl_set_opt(context, &handle, &get_size, &Term::from(true)), BifResult::Value(Term::from(false)));
/// ```
pub(super) fn dist_ctrl_set_opt(
    context: &mut BifContext<'_>,
    handle: &Term,
    option: &Term,
    value: &Term,
) -> BifResult {
    let name = match controlled_entry(context, handle) {
        Ok(name) => name,
        Err(error) => return error,
    };
    let (Term::Atom(option), Term::Atom(value)) = (option, value) else {
        return BifResult::badarg();
    };
    let on = value.is_static("true");
    if !option.is_static("get_size") || !(on || value.is_static("false")) {
        return BifResult::badarg();
    }
    let old = std::mem::replace(context.scheduler.node().dist().get_size_mut(&name), on);
    BifResult::Value(Term::from(old))
}

/// Run `erlang:dist_ctrl_input_handler(DHandle, Pid)` as
/// `dist_ctrl_input_handler_2` does (`dist.c:4532-4556`): make the local
/// process `Pid` the process that gives the data of the connection to
/// `erlang:dist_ctrl_put_data/2`, and return `ok`. `notsup` comes for a
/// term that is not a dist handle, and `badarg` when the caller is not the
/// controller of the entry, for a `Pid` that is not a pid, and for a
/// handle of another connection.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(dist_ctrl_input_handler(context, &handle, &Term::Pid(handler)), BifResult::Value(Term::static_atom("ok")));
/// ```
pub(super) fn dist_ctrl_input_handler(
    context: &mut BifContext<'_>,
    handle: &Term,
    handler: &Term,
) -> BifResult {
    let scheduler = &context.scheduler;
    let mut dist = scheduler.node().dist();
    let Some((name, id)) = dist.lookup(handle) else {
        return notsup();
    };
    let caller = Controller::Process(scheduler.current_process().id);
    if dist.controlled_by(caller).as_ref() != Some(&name) && !dist.port_controlled(&name) {
        return BifResult::badarg();
    }
    let Term::Pid(handler) = handler else {
        return BifResult::badarg();
    };
    if dist.connection_id(&name) != id {
        return BifResult::badarg();
    }
    dist.set_input_handler(&name, *handler);
    BifResult::Value(Term::static_atom("ok"))
}

/// Run `erlang:dist_ctrl_put_data(DHandle, Data)` as `dist_ctrl_put_data_2`
/// does (`dist.c:4332-4406`): give the data of one packet of the
/// connection to the node, and return `ok`. A list goes to
/// `erts_internal:dist_ctrl_put_data/2`, which makes a binary of it and
/// calls again (`erts_internal.erl:687-716`). `badarg` comes for a term
/// that is not a dist handle, for a bitstring that is not a whole number of
/// bytes, and for a term that is not a bitstring or `[]`; `notsup` comes
/// when the caller is not the input handler of the entry. ERTS does not
/// check the connection id here.
///
/// An empty packet is a tick, which only counts in the input count of
/// `erlang:dist_get_stat/1`. A packet with data stops the run with a
/// runtime error: PBeam does not take distribution messages yet (#1407).
///
/// # Examples
///
/// ```rust,ignore
/// let tick = Term::binary(Vec::new());
/// assert_eq!(dist_ctrl_put_data(context, &handle, &tick)?, BifResult::Value(Term::static_atom("ok")));
/// ```
pub(super) fn dist_ctrl_put_data(
    context: &mut BifContext<'_>,
    handle: &Term,
    data: &Term,
) -> Result<BifResult, BeamError> {
    if matches!(data, Term::Cons(_)) {
        return Ok(BifResult::Trap {
            module: "erts_internal",
            function: "dist_ctrl_put_data",
            arguments: vec![handle.clone(), data.clone()],
        });
    }
    let scheduler = &context.scheduler;
    let mut dist = scheduler.node().dist();
    let Some((name, _)) = dist.lookup(handle) else {
        return Ok(BifResult::badarg());
    };
    if dist.input_handler(&name) != Some(scheduler.current_process().id) {
        return Ok(notsup());
    }
    dist.count_input(&name);
    let flags = dist.flags(&name);
    if matches!(data, Term::Nil) {
        return Ok(BifResult::Value(Term::static_atom("ok")));
    }
    let Term::Binary(data) = data else {
        return Ok(BifResult::badarg());
    };
    if !data.bit_size().is_multiple_of(8) {
        return Ok(BifResult::badarg());
    }
    drop(dist);
    // A packet with data goes to its target (`erts_net_message`,
    // dist.c:4672-4680).
    if data.bit_size() != 0 {
        let bytes = data.to_bytes();
        context.scheduler.net_message(flags, &bytes)?;
    }
    Ok(BifResult::Value(Term::static_atom("ok")))
}

/// Run `erlang:dist_get_stat(DHandle)` as `dist_get_stat_1` does
/// (`dist.c:4488-4530`): return `{ok, Read, Write, Pending}`, the count of
/// `erlang:dist_ctrl_put_data/2` calls, the count of the packets that
/// `erlang:dist_ctrl_get_data/1` gave, and the bytes of the output queue.
/// PBeam sends no data yet (#1407), so the two last are 0. `badarg` comes
/// for a term that is not a dist handle and for a handle of another
/// connection.
///
/// # Examples
///
/// ```rust,ignore
/// // {ok, 3, 0, 0}
/// assert!(matches!(dist_get_stat(context, &handle), BifResult::Value(Term::Tuple(_))));
/// ```
pub(super) fn dist_get_stat(context: &mut BifContext<'_>, handle: &Term) -> BifResult {
    let dist = context.scheduler.node().dist();
    match dist.lookup(handle) {
        Some((name, id)) if id == dist.connection_id(&name) => {
            let input = Term::from_bigint(BigInt::from(dist.input(&name)));
            BifResult::Value(Term::tuple(vec![
                Term::static_atom("ok"),
                input,
                Term::Integer(0),
                Term::Integer(0),
            ]))
        }
        _ => BifResult::badarg(),
    }
}
