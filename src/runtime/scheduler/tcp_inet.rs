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

use std::io;

use super::Scheduler;
use crate::{
    error::BeamError,
    runtime::{
        port::{
            self, Driver, Port,
            inet::{Acceptor, Inet, OK, errno_name, try_reply},
        },
        signal::Signal,
        timer::{PortTimer, Timer},
    },
    term::{Atom, PortId, ProcessId, Term},
};

/// The result of [`Scheduler::tcp_accept`] under the port lock.
pub(in crate::runtime) enum Accept {
    /// The caller waits in the queue; the reply bytes.
    Queued(Vec<u8>),
    /// The accept at once: the async reference and the descriptor of the
    /// connection, or the error of `accept`.
    Now(io::Result<(u16, Inet)>),
}

/// The timeout of an accept without end (`INET_INFINITY`).
pub(super) const INFINITY: u32 = u32::MAX;

/// Whether the stop of a port tells the receive that waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StopNotice {
    /// The receive that waits gets an error message.
    Notify,
    /// The receive gets no message: the port terminates in the `CLOSING`
    /// state.
    Silent,
}

impl Scheduler {
    /// Run `INET_REQ_ACCEPT` of the current process on the listen port
    /// `listen` with its descriptor `tcp` and the timeout `timeout` in
    /// milliseconds (inet_drv.c:12144-12258). The caller holds the port lock
    /// of its control, as `erts_port_control` does (io.c:4268-4420), so a
    /// close of the port comes before the accept or after it, and then ends
    /// it with `{error, closed}` (see [`Self::tcp_stop`]).
    ///
    /// With an empty queue, the port accepts at once: the result is the
    /// accepted connection or the error of `accept` other than `EAGAIN`,
    /// for [`Self::tcp_accepted`] after the port lock. Else the caller goes
    /// to the end of the queue with a monitor and, for a timeout other than
    /// `infinity`, a timer, and the listen socket joins the IO poll.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let accept = scheduler.tcp_accept(listen, tcp, u32::MAX)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The reference number overflows.
    pub(in crate::runtime) fn tcp_accept(
        &mut self,
        listen: PortId,
        tcp: &mut Inet,
        timeout: u32,
    ) -> Result<Accept, BeamError> {
        if tcp.acceptors.is_empty()
            && let Some(accepted) = tcp.accept_now().transpose()
        {
            let accepted = accepted.map(|socket| (tcp.next_async_id(), tcp.copy(socket)));
            return Ok(Accept::Now(accepted));
        }
        let caller = self.current_process_mut().id;
        let id = tcp.next_async_id();
        let monitor = self.make_ref()?;
        let signal = Signal::PortMonitor {
            port: listen,
            reference: monitor,
        };
        // The caller runs this request, so it exists.
        let _ = self.send_signal(None, caller, signal);
        let timer = (timeout != INFINITY).then(|| {
            self.start_port_timer(timeout, |key| Timer::Accept {
                port: listen,
                caller,
                key,
            })
        });
        tcp.acceptors.push_back(Acceptor {
            id,
            caller,
            monitor,
            timer,
        });
        Ok(Accept::Queued(async_reply(id)))
    }

    /// End the accept at once of [`Self::tcp_accept`] on the listen port
    /// `listen` after the port lock, and return the reply bytes: a new port
    /// and the message `{inet_async, L, Ref, {ok, S}}` before the reply, or
    /// the error reply of `accept`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let reply = scheduler.tcp_accepted(listen, accepted)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The port number overflows.
    pub(in crate::runtime) fn tcp_accepted(
        &mut self,
        listen: PortId,
        accepted: io::Result<(u16, Inet)>,
    ) -> Result<Vec<u8>, BeamError> {
        let caller = self.current_process_mut().id;
        try_reply(accepted, |(id, accepted)| {
            let port = self.open_accepted(caller, accepted)?;
            self.port_send(caller, Self::async_message(listen, id, ok_port(port)));
            Ok(async_reply(id))
        })
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
        // A listen port that another scheduler closed after the IO poll
        // drops the event.
        let Some(cell) = self.node.port(listen) else {
            return Ok(false);
        };
        let mut ports = port::lock(&cell);
        let Some(tcp) = ports.as_mut().and_then(Port::inet_mut) else {
            return Ok(false);
        };
        if tcp.acceptors.is_empty() {
            return Ok(false);
        }
        let Some(accepted) = tcp.accept_now().transpose() else {
            return Ok(false);
        };
        let acceptor = tcp.acceptors.pop_front().expect("the queue has a caller");
        let failure = accepted.as_ref().err().map(errno_name);
        let accepted = accepted.ok().map(|socket| tcp.copy(socket));
        drop(ports);
        self.end_accept(listen, acceptor);
        let port = accepted
            .map(|accepted| self.open_accepted(acceptor.caller, accepted))
            .transpose()?;
        let reason = failure.map(|name| self.atoms.put(&name)).transpose()?;
        let error = reason.map(|reason| error_status(Term::Atom(reason)));
        let status = port.map(ok_port).or(error).expect("an accept or an error");
        self.port_send(
            acceptor.caller,
            Self::async_message(listen, acceptor.id, status),
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
    /// scheduler.tcp_accept_timeout(listen, caller, timer);
    /// ```
    pub(super) fn tcp_accept_timeout(
        &mut self,
        listen: PortId,
        caller: ProcessId,
        timer: PortTimer,
    ) {
        // A timer that a port task of another scheduler canceled first
        // finds no accept that holds it (`erl_hl_timer.c:708-720`).
        if let Some(acceptor) = self.remove_acceptor(listen, caller, Some(timer)) {
            self.demonitor_acceptor(listen, acceptor);
            let status = error_status(Term::static_atom("timeout"));
            self.port_send(caller, Self::async_message(listen, acceptor.id, status));
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
        if let Some(acceptor) = self.remove_acceptor(listen, pid, None)
            && let Some(timer) = acceptor.timer
        {
            self.cancel_port_timer(timer);
        }
    }

    /// End each accept of the listen port `listen` that closed, with its
    /// descriptor `tcp`, as `tcp_close_check` does (inet_drv.c:11859-11890):
    /// each caller in the queue, in order, loses its timer and its monitor
    /// and gets `{inet_async, L, Ref, {error, closed}}`, and so does the
    /// receive or the connect that waits (inet_drv.c:11883-11885). With [`StopNotice::Silent`], for a port that terminates in
    /// the `CLOSING` state, the receive gets no message (io.c:6212-6227); a
    /// connected port has no accept.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_stop(listen, tcp, StopNotice::Notify);
    /// ```
    pub(super) fn tcp_stop(&mut self, listen: PortId, tcp: Inet, notice: StopNotice) {
        let waiting = tcp.stream.receiver.or(tcp.stream.connector);
        if let Some(receiver) = waiting {
            if let Some(timer) = receiver.timer {
                self.cancel_port_timer(timer);
            }
            let status = error_status(Term::static_atom("closed"));
            let message = Self::async_message(listen, receiver.id, status);
            if notice == StopNotice::Notify {
                self.port_send(receiver.caller, message);
            }
        }
        for acceptor in tcp.acceptors {
            self.end_accept(listen, acceptor);
            let status = error_status(Term::static_atom("closed"));
            self.port_send(
                acceptor.caller,
                Self::async_message(listen, acceptor.id, status),
            );
        }
    }

    /// Remove the first accept of `caller` from the queue of the listen
    /// port `listen` (`remove_multi_op`, inet_drv.c:2379-2411), or return
    /// `None` when the port closed or the caller is not in the queue. With
    /// `timer`, the accept must hold that timer.
    fn remove_acceptor(
        &mut self,
        listen: PortId,
        caller: ProcessId,
        timer: Option<PortTimer>,
    ) -> Option<Acceptor> {
        let cell = self.node.port(listen)?;
        let mut ports = port::lock(&cell);
        let tcp = ports.as_mut().and_then(Port::inet_mut)?;
        let position = tcp.acceptors.iter().position(|item| {
            item.caller == caller && timer.is_none_or(|t| item.timer == Some(t))
        })?;
        tcp.acceptors.remove(position)
    }

    /// End the timer and the monitor of `acceptor` of the listen port
    /// `listen`.
    fn end_accept(&mut self, listen: PortId, acceptor: Acceptor) {
        if let Some(timer) = acceptor.timer {
            self.cancel_port_timer(timer);
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
    fn open_accepted(&mut self, owner: ProcessId, accepted: Inet) -> Result<PortId, BeamError> {
        self.open_port_for(owner, b"tcp_inet".to_vec(), Driver::Inet(accepted))
    }
}

impl Scheduler {
    /// Return the message `{inet_async, L, Ref, Status}` of the listen port
    /// `listen`.
    pub(super) fn async_message(listen: PortId, id: u16, status: Term) -> Term {
        Term::tuple(vec![
            Term::Atom(Atom::INET_ASYNC),
            Term::Port(listen),
            Term::Integer(id.into()),
            status,
        ])
    }
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
pub(super) fn error_status(reason: Term) -> Term {
    Term::tuple(vec![Term::static_atom("error"), reason])
}
