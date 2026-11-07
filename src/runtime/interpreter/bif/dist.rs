//! Bodies of the BIFs that give the local node its name and its creation
//! (`dist.c`).

use std::sync::Arc;

use num_bigint::BigInt;

use super::{BifContext, BifResult, Term};
use crate::error::{BeamError, error};
use crate::runtime::{
    process::{SpawnLink, SpawnReply},
    scheduler::{Scheduler, dist_table::Controller, node::Named},
};
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
    let node = Arc::clone(scheduler.node());
    let _local_event = node.local_event();
    if !scheduler
        .node()
        .set_this_node(&scheduler.atoms, net_kernel, name, creation)
    {
        return Ok(BifResult::badarg());
    }
    let nonode = Atom::from_static("nonode@nohost").expect("nonode@nohost is a static atom");
    let (nonode_handle, handle) = (scheduler.make_magic_ref()?, scheduler.make_magic_ref()?);
    let start = connection_start(scheduler);
    let now = scheduler.timers.now();
    let mut dist = scheduler.node().dist();
    dist.expire_known(now);
    dist.set_local(&nonode, nonode_handle, name, handle, start);
    drop(dist);
    let watchers = scheduler.node().dist().nodes_monitors();
    scheduler.nodes_event(name, true, None, None, &watchers);
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

/// The default distribution flags of PBeam: the mandatory flags,
/// `DFLAG_DIST_MONITOR` (`dist.h:33`), `DFLAG_DIST_MONITOR_NAME`
/// (`dist.h:34`), `DFLAG_DIST_HDR_ATOM_CACHE` (`dist.h:43`), `DFLAG_EXIT_PAYLOAD` (`dist.h:52`), `DFLAG_FRAGMENTS` (`dist.h:53`), and `DFLAG_ALIAS`
/// (`dist.h:67`), which ERTS offers in `DFLAG_DIST_DEFAULT`
/// (`dist.h:113-123`).
///
/// ERTS sends a monitor packet only to a connection that offered both
/// monitor flags: `erts_dsig_send_monitor`, `erts_dsig_send_demonitor`, and
/// `erts_dsig_send_m_exit` each skip the packet when
/// `~dflags & (DFLAG_DIST_MONITOR | DFLAG_DIST_MONITOR_NAME)` (`dist.c:1507`,
/// `1545`, `1558`). Thus a node that offers `DFLAG_DIST_MONITOR` alone
/// gets no monitor packet at all, and PBeam offers both. A monitor of a name
/// stays out of scope (see row "Distribution" of
/// `docs/erts-differences.md`).
const DEFAULT_FLAGS: u64 =
    MANDATORY_FLAGS | 0x08 | 0x20 | 0x2000 | 0x40_0000 | 0x80_0000 | (0x8 << 32) | (1 << 32);

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
/// PBeam offers the mandatory flags, `DFLAG_DIST_MONITOR`,
/// `DFLAG_DIST_MONITOR_NAME`, `DFLAG_DIST_HDR_ATOM_CACHE`, `DFLAG_EXIT_PAYLOAD`, `DFLAG_FRAGMENTS`, and `DFLAG_ALIAS` as its
/// default flags, and a distribution module can add only those
/// (`DFLAG_DIST_ADDABLE` is the default, `dist.h:126`); ERTS also offers the
/// other flags of `dist.h:105-123`.
/// The rejectable and the strict order flags are those of ERTS. See row
/// "Distribution" of `docs/erts-differences.md`.
///
/// # Examples
///
/// ```rust,ignore
/// // {erts_dflags,55897960380,17230663572,55897960380,8396866,8192}
/// let Term::Tuple(record) = get_dflags() else { unreachable!() };
/// assert_eq!(record.len(), 6);
/// ```
/// The flag `DFLAG_UNICODE_IO` (`dist.h:42`).
const UNICODE_IO: u64 = 0x1000;

/// Run `net_kernel:dflag_unicode_io(Pid)` as `net_kernel_dflag_unicode_io_1`
/// does (`dist.c:6693-6711`): a pid of this node, also of an old
/// incarnation of this node, gives `true`; a pid of another node gives
/// whether its connection has `DFLAG_UNICODE_IO`, and a node with no
/// connection gives `false`. A term that is not a pid raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let value = dflag_unicode_io(&mut context, &Term::Pid(scheduler.current_process().id));
/// assert!(matches!(value, BifResult::Value(Term::Atom(atom)) if atom.is_static("true")));
/// ```
pub(super) fn dflag_unicode_io(context: &mut BifContext<'_>, pid: &Term) -> BifResult {
    let Term::External(external) = pid else {
        return match pid {
            Term::Pid(_) => BifResult::Value(Term::static_atom("true")),
            _ => BifResult::badarg(),
        };
    };
    if !matches!(
        external.data,
        crate::term::external_id::ExternalData::Pid { .. }
    ) {
        return BifResult::badarg();
    }
    let scheduler = &mut *context.scheduler;
    if external.node == scheduler.atoms.this_node().0 {
        return BifResult::Value(Term::static_atom("true"));
    }
    let unicode = scheduler
        .node()
        .dist()
        .connected(&external.node)
        .is_some_and(|(flags, _, _)| flags & UNICODE_IO != 0);
    BifResult::Value(Term::static_atom(if unicode { "true" } else { "false" }))
}

pub(super) fn get_dflags() -> Term {
    let flags = |flags: u64| Term::from_bigint(BigInt::from(flags));
    Term::tuple(vec![
        Term::static_atom("erts_dflags"),
        flags(DEFAULT_FLAGS),
        flags(MANDATORY_FLAGS),
        flags(DEFAULT_FLAGS),
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
    let now = scheduler.timers.now();
    let node = Arc::clone(scheduler.node());
    let event_lock = {
        let mut dist = node.dist();
        dist.expire_known(now);
        dist.insert(name, handle, start);
        dist.event_lock(name)
    };
    let _event = event_lock
        .lock()
        .expect("the node event lock is not poisoned");
    let mut dist = node.dist();
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
    let node = Arc::clone(context.scheduler.node());
    let event = {
        let dist = node.dist();
        match dist.lookup(handle) {
            Some((entry, connection_id)) if entry == *name => {
                Some((dist.event_lock(name), connection_id))
            }
            _ => None,
        }
    };
    let Some((event_lock, connection_id)) = event else {
        return BifResult::badarg();
    };
    let _event = event_lock
        .lock()
        .expect("the node event lock is not poisoned");
    let (connected, ended) = node.dist().abort_pending(name, connection_id);
    if let Some(ended) = ended {
        context.scheduler.dist_connection_down(&ended);
        context
            .scheduler
            .dist_monitor_down(&ended, &Term::static_atom("noconnection"));
    }
    BifResult::Value(Term::from(!connected))
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
    let now = scheduler.timers.now();
    let reference = scheduler.make_magic_ref()?;
    let event_lock = {
        let mut dist = node.dist();
        dist.expire_known(now);
        dist.insert(name, reference, start);
        dist.event_lock(name)
    };
    let _event = event_lock
        .lock()
        .expect("the node event lock is not poisoned");
    let handle = {
        let mut dist = node.dist();
        // ERTS cannot send the request to a controller that does not
        // exist (`dist.c:5250-5256`). The caller itself is live.
        if node.is_going_down()
            || node.live_owner(net_kernel).is_none()
            || node.live_owner(controller).is_none()
        {
            return Ok(BifResult::Value(Term::static_atom("badarg")));
        }
        let handle = dist.connect(name, Controller::Process(controller), flags);
        if handle.is_some() {
            scheduler.dist_flush_pending(&mut dist, name, flags)?;
        }
        handle.map(|handle| (handle, dist.connection_id(name), dist.nodes_monitors()))
    };
    let Some((handle, connection, watchers)) = handle else {
        return Ok(if caller == controller {
            BifResult::Value(Term::static_atom("badarg"))
        } else {
            message_reply(scheduler, caller, Term::static_atom("badarg"))?
        });
    };
    node.add_connection();
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
    scheduler.nodes_event(name, flags & 1 != 0, Some(connection), None, &watchers);
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
    let now = scheduler.timers.now();
    let reference = scheduler.make_magic_ref()?;
    let node = Arc::clone(scheduler.node());
    let event_lock = {
        let mut dist = node.dist();
        dist.expire_known(now);
        dist.insert(name, reference, start);
        dist.event_lock(name)
    };
    let _event = event_lock
        .lock()
        .expect("the node event lock is not poisoned");
    let mut setup = |item: Option<&mut crate::runtime::port::Port>| -> Result<_, BeamError> {
        let mut dist = node.dist();
        let handle = item
            .filter(|item| {
                !node.is_going_down()
                    && node.live_owner(net_kernel).is_some()
                    && !item.closing
                    && item.soft_busy()
            })
            .and_then(|_| dist.connect(name, Controller::Port(port), flags));
        if handle.is_some() {
            scheduler.dist_flush_pending(&mut dist, name, flags)?;
        }
        Ok(handle.map(|handle| (handle, dist.connection_id(name), dist.nodes_monitors())))
    };
    let handle = match node.with_port(port, |item| setup(Some(item))) {
        Some(handle) => handle?,
        None => setup(None)?,
    };
    let Some((handle, connection, watchers)) = handle else {
        return Ok(BifResult::Value(Term::static_atom("badarg")));
    };
    node.add_connection();
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
    scheduler.nodes_event(name, flags & 1 != 0, Some(connection), None, &watchers);
    scheduler.dist_port_send(port)?;
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
/// return `ok`. The controller gets `dist_data` now when the output queue
/// of the connection has a packet, and else when the next packet comes
/// (see `DistTable::enqueue`).
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
    let name = match controlled_entry(context, handle) {
        Ok(name) => name,
        Err(error) => return error,
    };
    let scheduler = &mut *context.scheduler;
    if scheduler.node().dist().notify(&name) {
        let caller = scheduler.current_process().id;
        scheduler.send_message(caller, caller, Term::static_atom("dist_data"));
    }
    BifResult::Value(Term::static_atom("ok"))
}

/// Run `erlang:dist_ctrl_get_data(DHandle)` as `dist_ctrl_get_data_1`
/// does (`dist.c:4558-4746`): take the first packet of the output queue
/// and return it as a list of binaries, or `{Size, Data}` with its bytes
/// with the option `get_size`, or return `none` for an empty queue.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(dist_ctrl_get_data(context, &handle), BifResult::Value(Term::static_atom("none")));
/// ```
pub(super) fn dist_ctrl_get_data(context: &mut BifContext<'_>, handle: &Term) -> BifResult {
    let name = match controlled_entry(context, handle) {
        Ok(name) => name,
        Err(error) => return error,
    };
    let mut dist = context.scheduler.node().dist();
    let Some(output) = dist.dequeue(&name) else {
        return BifResult::Value(Term::static_atom("none"));
    };
    let get_size = dist.get_size(&name);
    let connection = dist.connection_id(&name);
    drop(dist);
    if let Some((process, key)) = output.spawn_notsup {
        context
            .scheduler
            .dist_spawn_notsup(&name, connection, process, key);
    }
    let size: usize = output
        .elements
        .iter()
        .map(|element| element.bit_size() / 8)
        .sum();
    let data = Term::list_from(
        output
            .elements
            .into_iter()
            .map(Term::Binary)
            .collect::<Vec<_>>(),
    );
    if !get_size {
        return BifResult::Value(data);
    }
    let size = Term::from_bigint(BigInt::from(size));
    BifResult::Value(Term::tuple(vec![size, data]))
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
/// `erlang:dist_get_stat/1`. A packet with data goes to its target (see
/// `Scheduler::net_message`).
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
    let Some((name, connection)) = dist.lookup(handle) else {
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
        context
            .scheduler
            .net_message(&name, connection, flags, &bytes)?;
    }
    Ok(BifResult::Value(Term::static_atom("ok")))
}

/// Run `erlang:dist_get_stat(DHandle)` as `dist_get_stat_1` does
/// (`dist.c:4488-4530`): return `{ok, Read, Write, Pending}`, the count of
/// `erlang:dist_ctrl_put_data/2` calls, the count of the packets that the
/// controller took or wrote, and the queue size of the output queue (see
/// `DistTable::output`). `badarg` comes for a term that is not a dist
/// handle and for a handle of another connection.
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
            let count = |count: u64| Term::from_bigint(BigInt::from(count));
            let (out, queue_size) = dist.output(&name);
            BifResult::Value(Term::tuple(vec![
                Term::static_atom("ok"),
                count(dist.input(&name)),
                count(out),
                count(queue_size),
            ]))
        }
        _ => BifResult::badarg(),
    }
}

/// Run `erts_internal:dist_spawn_request(Node, MFA, Options, Wait)` as
/// `erts_internal_dist_spawn_request_4` does (`dist.c:5850-6235`): send a
/// spawn request to `Node` and return `{Ref, MonOpt}` for
/// `erlang:spawn_opt/5`, or `Ref` for `erlang:spawn_request/5`.
///
/// A request that ERTS rejects answers the atom `badarg` and raises nothing;
/// the `erlang` wrapper raises `badarg` (`dist.c:6213-6215`,
/// `erlang.erl:11133-11167`). A `spawn_request` with an option that ERTS
/// rejects returns the reference and sends `{Tag, Ref, error, badopt}`
/// (`dist.c:5955-5975`).
///
/// # Errors
///
/// The errors of the encode of the request.
#[expect(
    clippy::too_many_lines,
    reason = "the option table follows `erts_internal_dist_spawn_request_4`"
)]
pub(super) fn dist_spawn_request(
    context: &mut BifContext<'_>,
    node: &Term,
    mfa: &Term,
    options: &Term,
    wait: &Term,
) -> Result<BifResult, BeamError> {
    let badarg = || Ok(BifResult::Value(Term::static_atom("badarg")));
    let Term::Atom(node) = node else {
        return badarg();
    };
    // ERTS reads `spawn_request` and treats each other value as `spawn_opt`,
    // which its `ASSERT` checks in a debug build only (`dist.c:6106-6115`).
    let spawn_opt = !matches!(wait, Term::Atom(wait) if wait.is_static("spawn_request"));
    // The request cannot go to the node that the caller runs on
    // (`dist.c:5875-5878`).
    if *node == context.scheduler.atoms.this_node().0 {
        return badarg();
    }
    let Some(options) = options.proper_list() else {
        return badarg();
    };
    let Term::Tuple(pair) = mfa else {
        return badarg();
    };
    let [Term::Atom(module), Term::Atom(function), arguments] = pair.as_ref() else {
        return badarg();
    };
    let Some(arguments_list) = arguments.proper_list() else {
        return badarg();
    };
    let arguments_len = i64::try_from(arguments_list.count()).unwrap_or(i64::MAX);
    let arguments_list = arguments.clone();
    let mut link = false;
    let mut monitor = None;
    let mut monitor_priority = false;
    let mut reply_tag = Term::static_atom("spawn_reply");
    let mut reply = SpawnReply {
        no_smsg: false,
        no_emsg: false,
    };
    let mut badopt = false;
    let mut kept = Vec::new();
    for option in options {
        match option {
            Term::Atom(name) if name.is_static("link") => link = true,
            Term::Atom(name) if name.is_static("monitor") => {
                monitor = Some(Term::static_atom("DOWN"));
            }
            Term::Tuple(fields) => match fields.as_ref() {
                [Term::Atom(name), value] if name.is_static("reply_tag") => {
                    // The `reply_tag` option is for a spawn request only
                    // (`dist.c:5897-5928`).
                    if spawn_opt {
                        return badarg();
                    }
                    reply_tag = value.clone();
                }
                [Term::Atom(name), value] if name.is_static("reply") => {
                    // The `reply` option is for a spawn request only
                    // (`dist.c:5901-5928`).
                    if spawn_opt {
                        return badarg();
                    }
                    let Some(policy) = reply_option(value) else {
                        badopt = true;
                        break;
                    };
                    reply = policy;
                }
                [Term::Atom(name), value] if name.is_static("monitor") => {
                    let Some((tag, priority, alias)) = monitor_option(value) else {
                        // A bad `monitor` option is `badarg` for a spawn
                        // and a `badopt` reply for a spawn request.
                        if spawn_opt {
                            return badarg();
                        }
                        badopt = true;
                        break;
                    };
                    monitor_priority = priority;
                    if alias {
                        return Err(error(
                            "unsupported spawn monitor option alias: see \"Distribution\" in docs/erts-differences.md",
                        ));
                    }
                    monitor = Some(tag);
                }
                [Term::Atom(name), value] if name.is_static("link") => match link_option(value) {
                    LinkOption::Ok => link = true,
                    LinkOption::Priority => {
                        return Err(error(
                            "unsupported link option priority: see \"Distribution\" in docs/erts-differences.md",
                        ));
                    }
                    LinkOption::Invalid if spawn_opt => return badarg(),
                    LinkOption::Invalid => {
                        badopt = true;
                        break;
                    }
                },
                _ => kept.push(option.clone()),
            },
            _ => kept.push(option.clone()),
        }
    }
    let scheduler = &mut *context.scheduler;
    if badopt {
        let reference = scheduler.dist_spawn_badopt(&reply_tag, reply)?;
        return Ok(BifResult::Value(Term::Reference(reference)));
    }
    // The request names `link` and `monitor` so that the receiver sets the
    // flags of the reply (`dist.c:6034-6051`); a `{monitor, Opts}` becomes a
    // bare `monitor` because only the local monitor keeps the tag.
    let mut sent = Vec::new();
    if link {
        sent.push(Term::static_atom("link"));
    }
    if monitor.is_some() {
        sent.push(Term::static_atom("monitor"));
    }
    sent.extend(kept);
    let request = scheduler.dist_spawn_request(
        node,
        module,
        function,
        arguments_len,
        &arguments_list,
        Term::list_from(sent),
        monitor.as_ref(),
        monitor_priority,
        &reply_tag,
        reply,
        if link { SpawnLink::Yes } else { SpawnLink::No },
    );
    let (reference, monitored) = request?;
    let value = if spawn_opt {
        Term::tuple(vec![
            Term::Reference(reference),
            Term::static_atom(if monitored { "true" } else { "false" }),
        ])
    } else {
        Term::Reference(reference)
    };
    Ok(BifResult::Value(value))
}

/// The validity of a `{link, Options}` spawn option
/// (`erts_link_opts`, `bif.c:675-699`).
enum LinkOption {
    /// The options are a proper list of `priority`.
    Ok,
    /// The options name `priority`, which PBeam does not implement.
    Priority,
    /// The options are each other term, which ERTS rejects.
    Invalid,
}

/// Read the options of a `{link, Options}` spawn option as
/// `erts_link_opts` does (`bif.c:675-699`).
fn link_option(options: &Term) -> LinkOption {
    let Some(options) = options.proper_list() else {
        return LinkOption::Invalid;
    };
    let mut priority = false;
    for option in options {
        match option {
            Term::Atom(name) if name.is_static("priority") => priority = true,
            _ => return LinkOption::Invalid,
        }
    }
    if priority {
        LinkOption::Priority
    } else {
        LinkOption::Ok
    }
}

/// Read the options of a `{monitor, Options}` spawn option as
/// `erts_monitor_opts` does (`bif.c:702-780`): the tag, whether the
/// options name `priority`, and whether they name `alias`.
fn monitor_option(options: &Term) -> Option<(Term, bool, bool)> {
    let mut tag = Term::static_atom("DOWN");
    let mut priority = false;
    let mut alias = false;
    for option in options.proper_list()? {
        match option {
            Term::Atom(name) if name.is_static("priority") => priority = true,
            Term::Tuple(fields) => match fields.as_ref() {
                [Term::Atom(name), value] if name.is_static("tag") => tag = value.clone(),
                [Term::Atom(name), mode] if name.is_static("alias") => {
                    let known = matches!(mode, Term::Atom(mode)
                        if mode.is_static("explicit_unalias")
                            || mode.is_static("demonitor")
                            || mode.is_static("reply_demonitor"));
                    if !known {
                        return None;
                    }
                    alias = true;
                }
                _ => return None,
            },
            _ => return None,
        }
    }
    Some((tag, priority, alias))
}

/// Read the value of a `{reply, Value}` spawn option as
/// `erts_parse_spawn_opts` reads it (`erl_process.c:12370-12390`) and as
/// `erts_internal_dist_spawn_request_4` reads it (`dist.c:5901-5924`): the
/// message of a successful spawn, and the message of a failed spawn, are
/// each kept or suppressed.
pub(super) fn reply_option(value: &Term) -> Option<SpawnReply> {
    let Term::Atom(value) = value else {
        return None;
    };
    let (suppress_success, suppress_error) = if value.is_static("error_only") {
        (true, false)
    } else if value.is_static("success_only") {
        (false, true)
    } else if value.is_static("no") {
        (true, true)
    } else if value.is_static("yes") {
        (false, false)
    } else {
        return None;
    };
    Some(SpawnReply {
        no_smsg: suppress_success,
        no_emsg: suppress_error,
    })
}
