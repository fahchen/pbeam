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
            self, Driver, Port,
            inet::{Event, Inet, OK, Receiver},
        },
        timer::{PortTimer, Timer},
    },
    term::{Atom, PortId, Term},
};

impl Scheduler {
    /// Run `TCP_REQ_RECV` of the current process on the connected port
    /// `port` with its descriptor `tcp`, the timeout `timeout` in
    /// milliseconds, and the length `length`. Return the reply `[1, R1, R0]`
    /// (inet_drv.c:12306-12322) and the messages of the step. The caller
    /// holds the port lock of its control, as `erts_port_control` does
    /// (io.c:4268-4420), so a close of the port comes before the receive or
    /// after it, and then ends it with `{error, closed}` (see
    /// [`Self::tcp_stop`]). The port reads at once; when the receive waits,
    /// a timeout of 0 ends it with `{error, timeout}`, and another timeout
    /// other than `infinity` starts its timer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (reply, events) = scheduler.tcp_receive(port, tcp, u32::MAX, 0);
    /// ```
    pub(in crate::runtime) fn tcp_receive(
        &mut self,
        port: PortId,
        tcp: &mut Inet,
        timeout: u32,
        length: usize,
    ) -> (Vec<u8>, Vec<Event>) {
        let caller = self.current_process_mut().id;
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
                let timer =
                    self.start_port_timer(timeout, |key| Timer::SocketReceive { port, key });
                let receiver = tcp.stream.receiver.as_mut().expect("the receive waits");
                receiver.timer = Some(timer);
            }
        }
        ([&[OK][..], &id.to_be_bytes()].concat(), events)
    }

    /// Start the timer of the connect that waits on the port `port` with its
    /// descriptor `tcp`, for a timeout other than `infinity`
    /// (inet_drv.c:12111-12112), and return the reply `[1, R1, R0]` of its
    /// async reference (inet_drv.c:12113-12114). The caller holds the port
    /// lock of its control.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let reply = scheduler.tcp_connect(port, tcp, u32::MAX);
    /// ```
    pub(in crate::runtime) fn tcp_connect(
        &mut self,
        port: PortId,
        tcp: &mut Inet,
        timeout: u32,
    ) -> Vec<u8> {
        let timer = (timeout != INFINITY)
            .then(|| self.start_port_timer(timeout, |key| Timer::Connect { port, key }));
        let connector = tcp.stream.connector.as_mut().expect("the connect waits");
        connector.timer = timer;
        [&[OK][..], &connector.id.to_be_bytes()].concat()
    }

    /// End the connect of the timer on the port `port` with `{error,
    /// timeout}` (see [`Inet::connect_timeout`]). A port that closed and a
    /// connect that does not hold the timer change nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_connect_timeout(port, timer);
    /// ```
    pub(super) fn tcp_connect_timeout(&mut self, port: PortId, timer: PortTimer) {
        self.tcp_step(port, |tcp| {
            tcp.stream
                .connector
                .is_some_and(|c| c.timer == Some(timer))
                .then(|| tcp.connect_timeout())
        })
        .expect("a timeout makes no new atom");
    }

    /// End the receive of the timer on the port `port`, open or closing,
    /// with `{error, timeout}` (`tcp_inet_recv_timeout`,
    /// inet_drv.c:11941-11949). A port that terminated changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_receive_timeout(port, timer);
    /// ```
    pub(super) fn tcp_receive_timeout(&mut self, port: PortId, timer: PortTimer) {
        // A timer that a port task of another scheduler canceled first
        // finds no receive that holds it (`erl_hl_timer.c:708-720`).
        self.tcp_step(port, |tcp| {
            tcp.stream
                .receiver
                .is_some_and(|r| r.timer == Some(timer))
                .then(|| tcp.receive_timeout())
        })
        .expect("a timeout makes no new atom");
    }

    /// Read for the receive that waits on the port `port`, which the IO
    /// poll found ready for read (`tcp_inet_input`, inet_drv.c:13597-13600).
    /// A port that another scheduler closed after the poll drops the event.
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
        self.tcp_step(port, |tcp| Some(tcp.receive(0)))
    }

    /// Write the queue of the port `port`, which the IO poll found ready
    /// for write (`tcp_inet_output`, inet_drv.c:14234-14370). A port that
    /// another scheduler closed after the poll drops the event.
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
        self.tcp_step(port, |tcp| Some(tcp.send_ready()))
    }

    /// Run `step` on the descriptor of the `tcp_inet` port `port`, open or
    /// closing, and send its messages under the same port lock (see
    /// [`Self::send_events`]), then end the step (see [`Self::end_step`]).
    /// A port that is not in the table, as after a close on another
    /// scheduler, and a step that gives `None` change nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.tcp_step(port, |tcp| Some(tcp.send_ready()))?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`Self::send_events`].
    pub(in crate::runtime) fn tcp_step(
        &mut self,
        port: PortId,
        step: impl FnOnce(&mut Inet) -> Option<Vec<Event>>,
    ) -> Result<(), BeamError> {
        let Some(cell) = self.node.port(port) else {
            return Ok(());
        };
        let mut slot = port::lock(&cell);
        let Some(events) = slot.as_mut().and_then(Port::inet_mut).and_then(step) else {
            return Ok(());
        };
        let item = slot.as_mut().expect("the step ran on the port");
        let exit = self.send_events(port, item, events)?;
        drop(slot);
        self.end_step(port, exit);
        Ok(())
    }

    /// Send the messages `events` of a step of the port `port` with its
    /// state `item`, under the port lock of the step, as the driver sends
    /// them in its callback (`erl_drv_send_term` and `erl_drv_output_term`,
    /// io.c:6212-6227, 6277-6278), and cancel the timer of each receive
    /// that ends. The messages of active mode go to the owner of the port,
    /// and a port in the `CLOSING` state sends no message. Return the
    /// reason of [`Event::Exit`], for [`Self::end_step`] after the lock.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let exit = scheduler.send_events(port, item, events)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The atom of an error name in a full atom table.
    pub(in crate::runtime) fn send_events(
        &mut self,
        port: PortId,
        item: &mut Port,
        events: Vec<Event>,
    ) -> Result<Option<Term>, BeamError> {
        let (owner, closing) = (item.owner, item.closing);
        let tcp = item
            .inet_mut()
            .expect("the port of a step is a tcp_inet port");
        let mut exit = None;
        for event in events {
            let messages = match event {
                Event::Received { receiver, result } => {
                    if let Some(timer) = receiver.timer {
                        self.cancel_port_timer(timer);
                    }
                    let status = match result {
                        Ok(bytes) => {
                            Term::tuple(vec![Term::static_atom("ok"), tcp.data_term(bytes)])
                        }
                        Err(name) => error_status(Term::Atom(self.atoms.put(&name)?)),
                    };
                    let message = Self::async_message(port, receiver.id, status);
                    vec![(receiver.caller, message)]
                }
                Event::Datagram {
                    receiver,
                    address,
                    data,
                } => {
                    if let Some(timer) = receiver.timer {
                        self.cancel_port_timer(timer);
                    }
                    // `[H1, ..., H7 | Data]`, all bytes in `list` mode
                    // (inet_drv.c:4142-4152).
                    let address = address.into_iter().map(|byte| Term::Integer(byte.into()));
                    let packet = Term::list_with_tail(address.collect(), tcp.data_term(data));
                    let status = Term::tuple(vec![Term::static_atom("ok"), packet]);
                    vec![(
                        receiver.caller,
                        Self::async_message(port, receiver.id, status),
                    )]
                }
                Event::ActiveDatagram { address, data } => {
                    // `{udp, S, {A, B, C, D}, Port, Data}` (`load_address`,
                    // inet_drv.c:4124-4158).
                    // `address` is `[Family, P1, P0, A, B, C, D]`, or
                    // `[5, N, B1, ..., BN]` of a `local` address, which
                    // gives `{local, Name}` and the port 0
                    // (inet_drv.c:1945-1948).
                    let (ip, from) = match address.as_slice() {
                        [5, _, name @ ..] => (
                            vec![Term::static_atom("local"), Term::binary(name.to_vec())],
                            Term::Integer(0),
                        ),
                        // `[0]`, `AF_UNSPEC`, gives `{unspec, <<>>}` and the
                        // port 0 (inet_drv.c:1956-1963).
                        [0] => (
                            vec![
                                Term::Atom(self.atoms.put("unspec")?),
                                Term::binary(Vec::new()),
                            ],
                            Term::Integer(0),
                        ),
                        _ => (
                            address[3..]
                                .iter()
                                .map(|byte| Term::Integer((*byte).into()))
                                .collect(),
                            Term::Integer(u16::from_be_bytes([address[1], address[2]]).into()),
                        ),
                    };
                    let message = Term::tuple(vec![
                        Term::Atom(self.atoms.put("udp")?),
                        Term::Port(port),
                        Term::tuple(ip),
                        from,
                        tcp.data_term(data),
                    ]);
                    vec![(owner, message)]
                }
                Event::Passive => {
                    let tag = Term::Atom(self.atoms.put("udp_passive")?);
                    vec![(owner, Term::tuple(vec![tag, Term::Port(port)]))]
                }
                Event::DatagramError(name) => {
                    let reason = Term::Atom(self.atoms.put(&name)?);
                    let tag = Term::Atom(self.atoms.put("udp_error")?);
                    vec![(owner, Term::tuple(vec![tag, Term::Port(port), reason]))]
                }
                Event::Connected { connector, result } => {
                    if let Some(timer) = connector.timer {
                        self.cancel_port_timer(timer);
                    }
                    let status = match result {
                        Ok(()) => Term::static_atom("ok"),
                        Err(name) => error_status(Term::Atom(self.atoms.put(&name)?)),
                    };
                    vec![(
                        connector.caller,
                        Self::async_message(port, connector.id, status),
                    )]
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
                    vec![(sender.caller, message)]
                }
                Event::Data(bytes) => {
                    vec![(owner, active(port, Atom::TCP, Some(tcp.data_term(bytes))))]
                }
                Event::Closed => vec![(owner, active(port, Atom::TCP_CLOSED, None))],
                Event::Error(name) => {
                    let reason = Term::Atom(self.atoms.put(&name)?);
                    vec![(owner, active(port, Atom::TCP_ERROR, Some(reason)))]
                }
                Event::EmptyQueue(subscribers) => {
                    let message =
                        Term::tuple(vec![Term::Atom(Atom::EMPTY_OUT_Q), Term::Port(port)]);
                    subscribers
                        .into_iter()
                        .map(|subscriber| (subscriber, message.clone()))
                        .collect()
                }
                Event::Exit(reason) => {
                    exit = Some(Term::Atom(self.atoms.put(&reason)?));
                    Vec::new()
                }
            };
            for (to, message) in messages {
                if !closing {
                    self.port_send(to, message);
                }
            }
        }
        Ok(exit)
    }

    /// End a step of the port `port` after its port lock: close the port
    /// with the reason `exit` of [`Event::Exit`] (see
    /// [`Self::driver_exit`]), and terminate a port in the `CLOSING` state
    /// with an empty queue (see [`Self::tcp_flushed`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.end_step(port, None);
    /// ```
    pub(in crate::runtime) fn end_step(&mut self, port: PortId, exit: Option<Term>) {
        if let Some(reason) = exit {
            self.driver_exit(port, reason);
        }
        self.tcp_flushed(port);
    }

    /// Terminate the port `port` in the `CLOSING` state when its queue is
    /// empty after a step of its driver (`erts_port_driver_callback_epilogue`,
    /// erl_port.h:807-810). The stop of the driver sends no message, and
    /// dropping the driver closes the socket after the kernel sends the
    /// written bytes (`tcp_inet_stop`, inet_drv.c:11895-11920). An open
    /// port and a port that terminated change nothing.
    pub(super) fn tcp_flushed(&mut self, port: PortId) {
        // The port lock comes first; the table lock is taken after it is
        // released, as in `Scheduler::exit_port`.
        let cell = self.node.port(port);
        let mut slot = cell.as_ref().map(port::lock);
        let item = slot
            .as_mut()
            .filter(|slot| {
                slot.as_ref().is_some_and(|item| {
                    item.closing && matches!(&item.driver, Driver::Inet(tcp) if !tcp.has_output())
                })
            })
            .and_then(|slot| slot.take());
        drop(slot);
        if let Some(item) = item {
            self.node.ports().remove(port);
            self.stop_port(port, item);
        }
    }
}

/// Return the message `{Tag, S}`, or `{Tag, S, Value}` with `value`, of
/// the port `port` in active mode.
fn active(port: PortId, tag: Atom, value: Option<Term>) -> Term {
    let tag = Term::Atom(tag);
    Term::tuple([tag, Term::Port(port)].into_iter().chain(value).collect())
}

#[cfg(test)]
#[path = "../../../tests/runtime/scheduler/tcp_stream.rs"]
mod tests;
