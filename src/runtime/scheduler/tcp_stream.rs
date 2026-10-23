//! The receive and the send of a connected `tcp_inet` port on one
//! scheduler: `TCP_REQ_RECV` with its timer, the output of the port, and
//! the steps of the IO poll when the socket is ready for read or write
//! (see `port/inet/stream.rs`; inet_drv.c:12268-12325, 12657-12731,
//! 13461-13612, 14234-14370).
//!
//! A receive ends with `{inet_async, S, Ref, {ok, Data}}` or `{inet_async,
//! S, Ref, {error, Reason}}`, and a send with `{inet_reply, S, ok, Mref}`
//! or `{inet_reply, S, {error, Reason}, Mref}` (prim_inet.erl:575-624,
//! 808-824). The IO poll sees the socket only when no process can run (row
//! "Input select" of `docs/erts-differences.md`).

use super::{
    Scheduler,
    tcp_inet::{INFINITY, error_status},
};
use crate::{
    error::BeamError,
    runtime::{
        port::{
            Driver, Port, Ports,
            inet::{Event, Inet, OK, Output, Receiver},
        },
        timer::{Timer, Timers},
    },
    term::{Atom, PortId, ProcessId, Term},
};

impl Scheduler {
    /// Run `TCP_REQ_RECV` of the current process on the connected port
    /// `port` with the timeout `timeout` in milliseconds and the length
    /// `length`, and return the reply `[1, R1, R0]` (inet_drv.c:12306-12322).
    /// The port reads at once; when the receive waits, a timeout of 0 ends
    /// it with `{error, timeout}`, and another timeout other than
    /// `infinity` starts its timer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.tcp_receive(port, u32::MAX, 0)?, vec![1, 0, 0]);
    /// ```
    ///
    /// # Errors
    ///
    /// The atom of an error name in a full atom table.
    pub(in crate::runtime) fn tcp_receive(
        &mut self,
        port: PortId,
        timeout: u32,
        length: usize,
    ) -> Result<Vec<u8>, BeamError> {
        let caller = self.current_process_mut().id;
        let mut ports = self.node.ports();
        let tcp = stream(&mut ports, port);
        let id = tcp.next_async_id();
        tcp.stream.receiver = Some(Receiver {
            id,
            caller,
            timer: None,
        });
        let mut events = tcp.receive(length);
        if tcp.stream.receiver.is_some() {
            if timeout == 0 {
                events.extend(tcp.receive_now_timeout());
            } else if timeout != INFINITY {
                let end = Timers::timeout_position(self.timers.now(), timeout.into())
                    .expect("a timeout of 32 bits has an end");
                let timer = self.timers.start(end, Timer::SocketReceive { port });
                let receiver = tcp.stream.receiver.as_mut().expect("the receive waits");
                receiver.timer = Some(timer);
            }
        }
        drop(ports);
        self.deliver_events(port, events)?;
        Ok([&[OK][..], &id.to_be_bytes()].concat())
    }

    /// Deliver the data of the port `port` that a `SETOPTS` of the current
    /// process made active (see [`Inet::activate`]), and return the reply
    /// `[1]` (inet_drv.c:11131-11148). The messages and the end of the
    /// port come before the reply, as the driver runs them in its control.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.tcp_activate(port, true)?, vec![1]);
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`Self::deliver_events`].
    pub(in crate::runtime) fn tcp_activate(
        &mut self,
        port: PortId,
        read: bool,
    ) -> Result<Vec<u8>, BeamError> {
        let events = stream(&mut self.node.ports(), port).activate(read);
        self.deliver_events(port, events)?;
        Ok(vec![OK])
    }

    /// End the receive of the timer on the port `port`, open or closing,
    /// with `{error, timeout}` (`tcp_inet_recv_timeout`,
    /// inet_drv.c:11941-11949). A port that terminated changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_receive_timeout(port);
    /// ```
    pub(super) fn tcp_receive_timeout(&mut self, port: PortId) {
        let events = self
            .node
            .ports()
            .get_mut(port)
            .and_then(Port::inet_mut)
            .map(Inet::receive_timeout);
        if let Some(events) = events {
            self.deliver_events(port, events)
                .expect("a timeout makes no new atom");
        }
    }

    /// Read for the receive that waits on the port `port`, which the IO
    /// poll found ready for read (`tcp_inet_input`, inet_drv.c:13597-13600).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_receive_ready(port)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The atom of an error name in a full atom table.
    pub(super) fn tcp_receive_ready(&mut self, port: PortId) -> Result<(), BeamError> {
        let events = stream(&mut self.node.ports(), port).receive(0);
        self.deliver_events(port, events)
    }

    /// Write the queue of the port `port`, which the IO poll found ready
    /// for write (`tcp_inet_output`, inet_drv.c:14234-14370).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_send_ready(port)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The atom of an error name in a full atom table.
    pub(super) fn tcp_send_ready(&mut self, port: PortId) -> Result<(), BeamError> {
        let events = stream(&mut self.node.ports(), port).send_ready();
        self.deliver_events(port, events)
    }

    /// Run the output `bytes` of the current process on the `tcp_inet`
    /// port `port` (`tcp_inet_commandv`, inet_drv.c:12657-12731). A header
    /// that is too short fails the port with the reason `einval`
    /// (`driver_failure_posix`, io.c:7267-7270).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_send(port, &[0, 0])?;
    /// ```
    ///
    /// # Errors
    ///
    /// An output that PBeam does not run (see [`Inet::send`]), and the atom
    /// of an error name in a full atom table.
    pub(in crate::runtime) fn tcp_send(
        &mut self,
        port: PortId,
        bytes: &[u8],
    ) -> Result<(), BeamError> {
        let caller = self.current_process_mut().id;
        let seed = self.local_node_hash();
        let sent = stream(&mut self.node.ports(), port).send(bytes, caller, seed)?;
        match sent {
            Output::Sent(events) => self.deliver_events(port, events),
            Output::Failed => {
                self.fail_port(port, Term::static_atom("einval"));
                Ok(())
            }
        }
    }

    /// Send the messages `events` of the port `port`, and cancel the timer
    /// of each receive that ends. The messages of active mode go to the
    /// owner of the port (`erl_drv_output_term`), and [`Event::Exit`]
    /// closes the port (see [`Self::driver_exit`]). A port in the
    /// `CLOSING` state sends no message (see [`Self::port_deliver`]), its
    /// end terminates it (io.c:7206-7207), and so does an empty queue after
    /// the events (see [`Self::tcp_flushed`]).
    pub(super) fn deliver_events(
        &mut self,
        port: PortId,
        events: Vec<Event>,
    ) -> Result<(), BeamError> {
        for event in events {
            match event {
                Event::Received { receiver, result } => {
                    if let Some(timer) = receiver.timer {
                        self.timers.cancel(timer);
                    }
                    let status = match result {
                        Ok(bytes) => {
                            let data = stream(&mut self.node.ports(), port).data_term(bytes);
                            Term::tuple(vec![Term::static_atom("ok"), data])
                        }
                        Err(name) => error_status(Term::Atom(self.atoms.put(&name)?)),
                    };
                    let message = self.async_message(port, receiver.id, status);
                    self.port_deliver(port, receiver.caller, message);
                }
                Event::Replied { sender, result } => {
                    let status = match result {
                        Ok(()) => Term::static_atom("ok"),
                        Err(name) => error_status(Term::Atom(self.atoms.put(&name)?)),
                    };
                    let message = Term::tuple(vec![
                        Term::Atom(Atom::INET_REPLY),
                        Term::Port(port),
                        status,
                        Term::Reference(sender.tag),
                    ]);
                    self.port_deliver(port, sender.caller, message);
                }
                Event::Data(bytes) => {
                    let data = stream(&mut self.node.ports(), port).data_term(bytes);
                    self.deliver_active(port, Atom::TCP, Some(data));
                }
                Event::Closed => self.deliver_active(port, Atom::TCP_CLOSED, None),
                Event::Error(name) => {
                    let reason = Term::Atom(self.atoms.put(&name)?);
                    self.deliver_active(port, Atom::TCP_ERROR, Some(reason));
                }
                Event::EmptyQueue(subscribers) => {
                    let message =
                        Term::tuple(vec![Term::Atom(Atom::EMPTY_OUT_Q), Term::Port(port)]);
                    for subscriber in subscribers {
                        self.port_deliver(port, subscriber, message.clone());
                    }
                }
                Event::Exit(reason) => {
                    let reason = Term::Atom(self.atoms.put(&reason)?);
                    self.driver_exit(port, reason);
                }
            }
        }
        self.tcp_flushed(port);
        Ok(())
    }

    /// Terminate the port `port` in the `CLOSING` state when its queue is
    /// empty after a step of its driver (`erts_port_driver_callback_epilogue`,
    /// erl_port.h:807-810). The stop of the driver sends no message, and
    /// dropping the driver closes the socket after the kernel sends the
    /// written bytes (`tcp_inet_stop`, inet_drv.c:11895-11920). An open
    /// port and a port that terminated change nothing.
    pub(super) fn tcp_flushed(&mut self, port: PortId) {
        let mut ports = self.node.ports();
        let flushed = ports.get(port).is_some_and(|item| {
            item.closing && matches!(&item.driver, Driver::Inet(tcp) if !tcp.has_output())
        });
        if flushed {
            let item = ports.remove(port).expect("the flushed port is closing");
            drop(ports);
            self.stop_port(port, item);
        }
    }

    /// Send `message` of the port `port` to `to`, or drop it when the port
    /// is in the `CLOSING` state, as `erl_drv_send_term` and
    /// `erl_drv_output_term` do (io.c:6212-6227, 6277-6278).
    fn port_deliver(&mut self, port: PortId, to: ProcessId, message: Term) {
        let closing = self.node.ports().closing(port);
        if !closing {
            self.deliver(to, message);
        }
    }

    /// Send `{Tag, S}`, or `{Tag, S, Value}` with `value`, of the port
    /// `port` to its owner.
    fn deliver_active(&mut self, port: PortId, tag: Atom, value: Option<Term>) {
        let tag = Term::Atom(tag);
        let message = Term::tuple([tag, Term::Port(port)].into_iter().chain(value).collect());
        let owner = self
            .node
            .ports()
            .get(port)
            .expect("the port of a message is open")
            .owner;
        self.port_deliver(port, owner, message);
    }
}

/// Return the descriptor of the `tcp_inet` port `id`, open or closing.
fn stream(ports: &mut Ports, id: PortId) -> &mut Inet {
    ports
        .get_mut(id)
        .and_then(Port::inet_mut)
        .expect("the port of a receive or a send is an open or closing tcp_inet port")
}
