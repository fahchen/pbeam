//! The accept queue of the `tcp_inet` driver on one scheduler: the
//! `INET_REQ_ACCEPT` request, the accept of a connection when the listen
//! socket is ready, the timer and the monitor of each caller, and the end of
//! each accept when the listen port closes (inet_drv.c:12144-12258,
//! 12547-12599, 12732-12771, 13461-13590, 11859-11890).
//!
//! A caller that waits gets one message from the listen port `L`:
//! `{inet_async, L, Ref, {ok, S}}` with the new port `S`, or `{inet_async,
//! L, Ref, {error, Reason}}` with `timeout`, `closed`, or the error name of
//! `accept` (`send_async_ok_port` and `send_async_error`,
//! inet_drv.c:2510-2560). `Ref` is the async reference of the `ACCEPT`
//! reply.
//!
//! ERTS keeps one caller in the state `INET_STATE_ACCEPTING` and more
//! callers in `INET_STATE_MULTI_ACCEPTING`. Both accept for the callers in
//! arrival order, so PBeam keeps one queue for both. The IO poll sees the
//! listen socket only when no process can run (row "Input select" of
//! `docs/erts-differences.md`), so an acceptor that exits leaves the queue
//! before the next accept.

use super::Scheduler;
use crate::{
    error::BeamError,
    runtime::{
        port::{
            Driver, Port, Ports,
            tcp_inet::{Acceptor, OK, TcpInet, errno_name, try_reply},
        },
        signal::Signal,
        timer::{Timer, Timers},
    },
    term::{PortId, ProcessId, Term},
};

/// The atom of the messages of an accept.
const INET_ASYNC: &str = "inet_async";

/// The timeout of an accept without end (`INET_INFINITY`).
const INFINITY: u32 = u32::MAX;

impl Scheduler {
    /// Run `INET_REQ_ACCEPT` of the current process on the listen port
    /// `listen` with the timeout `timeout` in milliseconds, and return the
    /// reply bytes (inet_drv.c:12144-12258).
    ///
    /// With an empty queue, the port accepts at once: a pending connection
    /// gives a new port and the message `{inet_async, L, Ref, {ok, S}}`
    /// before the reply, and an error of `accept` other than `EAGAIN` gives
    /// its error reply. Else the caller goes to the end of the queue with a
    /// monitor and, for a timeout other than `infinity`, a timer, and the
    /// listen socket joins the IO poll.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.tcp_accept(listen, u32::MAX)?, vec![1, 0, 0]);
    /// ```
    ///
    /// # Errors
    ///
    /// The port number or the reference number overflows.
    pub(in crate::runtime) fn tcp_accept(
        &mut self,
        listen: PortId,
        timeout: u32,
    ) -> Result<Vec<u8>, BeamError> {
        // `inet_init` makes the atom when the driver loads (inet_drv.c).
        self.atoms.put(INET_ASYNC)?;
        let caller = self.current_process_mut().id;
        let tcp = tcp_inet(self.resources.borrow_mut::<Ports>(), listen);
        if tcp.acceptors.is_empty()
            && let Some(accepted) = tcp.accept_now().transpose()
        {
            let accepted = accepted.map(|socket| (tcp.next_async_id(), tcp.copy(socket)));
            return try_reply(accepted, |(id, accepted)| {
                let port = self.open_accepted(caller, accepted)?;
                self.deliver(caller, self.async_message(listen, id, ok_port(port)));
                Ok(async_reply(id))
            });
        }
        let id = tcp.next_async_id();
        let monitor = self.make_ref()?;
        let signal = Signal::PortMonitor {
            port: listen,
            reference: monitor,
        };
        // The caller runs this request, so it exists.
        let _ = self.send_signal(None, caller, signal);
        let timer = (timeout != INFINITY).then(|| {
            let end = Timers::timeout_position(self.timers.now(), timeout.into())
                .expect("a timeout of 32 bits has an end");
            self.timers.start(
                end,
                Timer::Accept {
                    port: listen,
                    caller,
                },
            )
        });
        let acceptor = Acceptor {
            id,
            caller,
            monitor,
            timer,
        };
        let tcp = tcp_inet(self.resources.borrow_mut::<Ports>(), listen);
        tcp.acceptors.push_back(acceptor);
        Ok(async_reply(id))
    }

    /// Accept the pending connections of the listen port `listen`, which
    /// the IO poll found ready, for the callers in the queue, in order, as
    /// `tcp_inet_input` does (inet_drv.c:13461-13590): each accept ends the
    /// timer and the monitor of its caller, and sends the `inet_async`
    /// message. The accepts stop when the queue is empty, when no
    /// connection is pending, and after an error of `accept`, which goes to
    /// the first caller.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_accept_ready(listen)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The port number overflows.
    pub(in crate::runtime) fn tcp_accept_ready(&mut self, listen: PortId) -> Result<(), BeamError> {
        while self.accept_one(listen)? {}
        Ok(())
    }

    /// Accept one pending connection of the listen port `listen` for the
    /// first caller in the queue. Return `true` when the next caller can
    /// take the next connection.
    fn accept_one(&mut self, listen: PortId) -> Result<bool, BeamError> {
        let tcp = tcp_inet(self.resources.borrow_mut::<Ports>(), listen);
        if tcp.acceptors.is_empty() {
            return Ok(false);
        }
        let Some(accepted) = tcp.accept_now().transpose() else {
            return Ok(false);
        };
        let acceptor = tcp.acceptors.pop_front().expect("the queue has a caller");
        let failure = accepted.as_ref().err().map(errno_name);
        let accepted = accepted.ok().map(|socket| tcp.copy(socket));
        self.end_accept(listen, acceptor);
        let port = accepted
            .map(|accepted| self.open_accepted(acceptor.caller, accepted))
            .transpose()?;
        let reason = failure.map(|name| self.atoms.put(&name)).transpose()?;
        let error = reason.map(|reason| error_status(Term::Atom(reason)));
        let status = port.map(ok_port).or(error).expect("an accept or an error");
        self.deliver(
            acceptor.caller,
            self.async_message(listen, acceptor.id, status),
        );
        Ok(port.is_some())
    }

    /// End the accept of the timer of `caller` on the listen port `listen`,
    /// as `tcp_inet_timeout` and `tcp_inet_multi_timeout` do
    /// (inet_drv.c:12547-12599): the first accept of the caller leaves the
    /// queue, the port removes its monitor, and the caller gets
    /// `{inet_async, L, Ref, {error, timeout}}`. A port that closed and a
    /// caller that is not in the queue change nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_accept_timeout(listen, caller);
    /// ```
    pub(super) fn tcp_accept_timeout(&mut self, listen: PortId, caller: ProcessId) {
        if let Some(acceptor) = self.remove_acceptor(listen, caller) {
            self.demonitor_acceptor(listen, acceptor);
            let status = error_status(Term::static_atom("timeout"));
            self.deliver(caller, self.async_message(listen, acceptor.id, status));
        }
    }

    /// Run the monitor of the listen port `listen` for the process `pid`
    /// that exited, as `tcp_inet_process_exit` does
    /// (inet_drv.c:12732-12771): the first accept of the process leaves the
    /// queue, and its timer ends. A port that closed and a process that is
    /// not in the queue change nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_monitor_down(listen, pid);
    /// ```
    pub(in crate::runtime) fn tcp_monitor_down(&mut self, listen: PortId, pid: ProcessId) {
        if let Some(acceptor) = self.remove_acceptor(listen, pid)
            && let Some(timer) = acceptor.timer
        {
            self.timers.cancel(timer);
        }
    }

    /// End each accept of the listen port `listen` that closed, with its
    /// descriptor `tcp`, as `tcp_close_check` does (inet_drv.c:11859-11890):
    /// each caller in the queue, in order, loses its timer and its monitor
    /// and gets `{inet_async, L, Ref, {error, closed}}`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_stop(listen, tcp);
    /// ```
    pub(super) fn tcp_stop(&mut self, listen: PortId, tcp: TcpInet) {
        for acceptor in tcp.acceptors {
            self.end_accept(listen, acceptor);
            let status = error_status(Term::static_atom("closed"));
            self.deliver(
                acceptor.caller,
                self.async_message(listen, acceptor.id, status),
            );
        }
    }

    /// Remove the first accept of `caller` from the queue of the listen
    /// port `listen` (`remove_multi_op`, inet_drv.c:2379-2411), or return
    /// `None` when the port closed or the caller is not in the queue.
    fn remove_acceptor(&mut self, listen: PortId, caller: ProcessId) -> Option<Acceptor> {
        let ports = self.resources.borrow_mut::<Ports>();
        let tcp = ports.get_mut(listen).and_then(Port::tcp_inet_mut)?;
        let position = tcp
            .acceptors
            .iter()
            .position(|item| item.caller == caller)?;
        tcp.acceptors.remove(position)
    }

    /// End the timer and the monitor of `acceptor` of the listen port
    /// `listen`.
    fn end_accept(&mut self, listen: PortId, acceptor: Acceptor) {
        if let Some(timer) = acceptor.timer {
            self.timers.cancel(timer);
        }
        self.demonitor_acceptor(listen, acceptor);
    }

    /// Send the demonitor signal of the monitor of `acceptor` that the
    /// listen port `listen` holds (`driver_demonitor_process`). A caller
    /// that does not exist drops it.
    fn demonitor_acceptor(&mut self, listen: PortId, acceptor: Acceptor) {
        let signal = Signal::PortDemonitor {
            port: listen,
            reference: acceptor.monitor,
        };
        let _ = self.send_signal(None, acceptor.caller, signal);
    }

    /// Put the port of an accepted connection with the descriptor
    /// `accepted` into the table, owned by and linked to `owner`, as
    /// `driver_create_port` does in `tcp_inet_copy` (io.c:781-860,
    /// inet_drv.c:11795-11858), and return its number. The owner waits in
    /// the accept, so it exists: the IO poll runs only when no process can
    /// run, and an exiting process leaves the queue before it ends.
    fn open_accepted(&mut self, owner: ProcessId, accepted: TcpInet) -> Result<PortId, BeamError> {
        self.open_port_for(owner, b"tcp_inet".to_vec(), Driver::TcpInet(accepted))
    }
}

impl Scheduler {
    /// Return the message `{inet_async, L, Ref, Status}` of the listen port
    /// `listen`. The `ACCEPT` that the message ends made the atom.
    fn async_message(&self, listen: PortId, id: u16, status: Term) -> Term {
        let atom = self.atoms.get(INET_ASYNC).expect("an accept made the atom");
        Term::tuple(vec![
            Term::Atom(atom),
            Term::Port(listen),
            Term::Integer(id.into()),
            status,
        ])
    }
}

/// Return the descriptor of the open `tcp_inet` port `id`.
fn tcp_inet(ports: &mut Ports, id: PortId) -> &mut TcpInet {
    ports
        .get_mut(id)
        .and_then(Port::tcp_inet_mut)
        .expect("the port of an accept is an open tcp_inet port")
}

/// Return the `ACCEPT` reply `[1, R1, R0]` of the async reference `id`.
fn async_reply(id: u16) -> Vec<u8> {
    [&[OK][..], &id.to_be_bytes()].concat()
}

/// Return the status `{ok, S}` of the accepted port `port`.
fn ok_port(port: PortId) -> Term {
    Term::tuple(vec![Term::static_atom("ok"), Term::Port(port)])
}

/// Return the status `{error, Reason}`.
fn error_status(reason: Term) -> Term {
    Term::tuple(vec![Term::static_atom("error"), reason])
}
