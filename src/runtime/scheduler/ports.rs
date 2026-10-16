//! Open, poll, and close the ports of the node, as the port operations of
//! `io.c` and the spawn driver (`sys/unix/sys_drivers.c`) do on one
//! scheduler.
//!
//! ERTS runs a port operation at once when no other thread holds the port
//! (`try_imm_drv_call`, `io.c:923-1002`), else it schedules a port task.
//! PBeam has one thread, so each operation runs at once. The IO poll reads
//! the pipes of the ports only when no process can run (see
//! [`Scheduler::wait_input`]).

use super::{LinkId, Scheduler};
use crate::{
    error::{BeamError, error},
    runtime::{
        port::{Driver, Port, PortEvent, Ports, inet},
        signal::Signal,
    },
    term::{PortId, ProcessId, Reference, Term},
};

impl Scheduler {
    /// Open a port with the name `name` and the driver `driver`, owned by
    /// and linked to the current process, as `erts_internal_open_port_2` does
    /// (`erl_bif_port.c:60-130`). The link goes into both the port and the
    /// link tree of the process at once, as `erts_link_tree_insert` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let spawn = Spawn { exit_status: true, child, end_of_file: false };
    /// let port = scheduler.open_port(b"/bin/sh".to_vec(), Driver::Spawn(spawn))?;
    /// assert_eq!(port, PortId(0));
    /// ```
    ///
    /// # Errors
    ///
    /// The port number overflows.
    pub(in crate::runtime) fn open_port(
        &mut self,
        name: Vec<u8>,
        driver: Driver,
    ) -> Result<PortId, BeamError> {
        let owner = self.current_process_mut().id;
        self.open_port_for(owner, name, driver)
    }

    /// Open a port with the name `name` and the driver `driver`, owned by
    /// and linked to the process `owner`, which exists (see
    /// [`Self::open_port`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let port = scheduler.open_port_for(owner, b"tcp_inet".to_vec(), driver)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The port number overflows.
    pub(super) fn open_port_for(
        &mut self,
        owner: ProcessId,
        name: Vec<u8>,
        driver: Driver,
    ) -> Result<PortId, BeamError> {
        let port = Port {
            owner,
            name,
            data: Term::static_atom("undefined"),
            input: 0,
            output: 0,
            driver,
            monitors: Vec::new(),
        };
        let id = self.resources.borrow_mut::<Ports>().insert(port)?;
        self.process_mut(owner)
            .expect("the owner of a port exists")
            .links
            .insert(LinkId::Port(id), 0);
        Ok(id)
    }

    /// Handle the fd `fd` of a port that the IO poll found ready, as
    /// `ready_input` and `port_inp_failure` do (`sys_drivers.c:1205-1347`):
    ///
    /// - Output of the child goes to the owner as `{Port, {data, Bytes}}`
    ///   (`driver_output` in binary mode), and adds to the input count.
    /// - The end of file closes the port with the reason `normal`
    ///   (`driver_failure_eof`). With `exit_status`, the port first waits
    ///   for the exit of the child (`dd->alive == 1`), and then sends
    ///   `{Port, {exit_status, Status}}` to the owner before it closes
    ///   (`driver_report_exit`).
    /// - A datagram on an active `udp_inet` socket stops the run: PBeam does
    ///   not send `{udp, Socket, Address, Port, Data}` (inet_drv.c:14619-14621).
    ///
    /// An fd that no port waits for changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.port_ready(child.output_fd())?;
    /// ```
    ///
    /// # Errors
    ///
    /// A port with `exit_status` whose child the host reaped returns a
    /// runtime error: PBeam has no exit status for it after
    /// `os:set_signal(sigchld, ignore)`. A ready active `udp_inet` socket
    /// returns a runtime error.
    pub(in crate::runtime) fn port_ready(&mut self, fd: libc::c_int) -> Result<(), BeamError> {
        let ports = self.resources.borrow_mut::<Ports>();
        if let Some(listen) = ports.accepting(fd) {
            return self.tcp_accept_ready(listen);
        }
        if ports.receiving(fd) {
            return Err(error(inet::DATAGRAM_RECEIVE));
        }
        if let Some(port) = ports.stream_receiving(fd) {
            return self.tcp_receive_ready(port);
        }
        let Some((id, event)) = ports.read(fd) else {
            return Ok(());
        };
        let port = ports.get_mut(id).expect("the port of a read is open");
        let owner = port.owner;
        match event {
            PortEvent::Data(bytes) => {
                port.input += bytes.len() as u64;
                self.deliver(owner, port_message(id, "data", Term::binary(bytes)));
            }
            PortEvent::EndOfFile => self.close_port(id, Term::static_atom("normal"))?,
            PortEvent::ExitStatus(Some(status)) => {
                let status = Term::Integer(status.into());
                self.deliver(owner, port_message(id, "exit_status", status));
                self.close_port(id, Term::static_atom("normal"))?;
            }
            PortEvent::ExitStatus(None) => {
                return Err(error(
                    "unsupported exit status of a port program after os:set_signal(sigchld, ignore): see \"Ports\" in docs/erts-differences.md",
                ));
            }
        }
        Ok(())
    }

    /// Write the queued output of the port of the socket `fd`, which the
    /// IO poll found ready for write. An fd of no port with queued output,
    /// for example of a socket that a read of the same wait closed,
    /// changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.port_writable(fd)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The error of [`Self::tcp_send_ready`].
    pub(in crate::runtime) fn port_writable(&mut self, fd: libc::c_int) -> Result<(), BeamError> {
        match self.resources.borrow_mut::<Ports>().sending(fd) {
            Some(port) => self.tcp_send_ready(port),
            None => Ok(()),
        }
    }

    /// Close the open port `id` with the exit reason `reason`, as
    /// `erts_deliver_port_exit` and `terminate_port` do
    /// (`io.c:3698-3790`, `3841-3950`): the port leaves the table, the
    /// owner, which has the only link to the port, gets a link exit signal
    /// from the port with the reason (`link_port_exit`, `io.c:3802-3813`),
    /// each monitor of the port gets its `DOWN` signal, and then the
    /// driver stops (see [`Self::stop_port`]) and the pipes of the port
    /// close.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.close_port(port, Term::static_atom("normal"))?;
    /// ```
    ///
    /// # Errors
    ///
    /// A reason other than `kill` for a port with queued output (see
    /// [`check_flush`]).
    pub(in crate::runtime) fn close_port(
        &mut self,
        id: PortId,
        reason: Term,
    ) -> Result<(), BeamError> {
        let ports = self.resources.borrow_mut::<Ports>();
        check_flush(ports.get(id).expect("the port to close is open"), &reason)?;
        let mut port = ports.remove(id).expect("the port to close is open");
        let monitors = std::mem::take(&mut port.monitors);
        let exit = Signal::Exit {
            from: LinkId::Port(id),
            reason: reason.clone(),
            linked: true,
            normal_kills: false,
            alias: None,
        };
        // An owner that does not exist drops the signal.
        let _ = self.send_signal(None, port.owner, exit);
        self.port_monitors_down(&monitors, &reason);
        self.stop_port(id, port);
        Ok(())
    }

    /// Send the `DOWN` signal of each monitor of `monitors` on a port that
    /// closed with `reason`, after its link exit signal, as
    /// `erts_deliver_port_exit` does with `monitor_port_exit`
    /// (io.c:3922-3936). The watcher gets `{'DOWN', Ref, port, Port,
    /// Reason}`.
    fn port_monitors_down(&mut self, monitors: &[(ProcessId, Reference)], reason: &Term) {
        for &(watcher, reference) in monitors {
            self.send_monitor_down(None, watcher, reference, reason.clone());
        }
    }

    /// Add the monitor `reference` of `watcher` to the open port `id`, or
    /// return false when the port is not open.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.monitor_port(port, watcher, reference));
    /// ```
    pub(super) fn monitor_port(
        &mut self,
        id: PortId,
        watcher: ProcessId,
        reference: Reference,
    ) -> bool {
        let ports = self.resources.borrow_mut::<Ports>();
        ports
            .get_mut(id)
            .map(|port| port.monitors.push((watcher, reference)))
            .is_some()
    }

    /// Remove the monitor `reference` from the port `id`, as the demonitor
    /// of a port does (`erts_port_demonitor`, io.c). A port that closed
    /// changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.demonitor_port(port, reference);
    /// ```
    pub(super) fn demonitor_port(&mut self, id: PortId, reference: Reference) {
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        // A port that closed has no monitor to remove.
        port.into_iter()
            .for_each(|port| port.monitors.retain(|&(_, item)| item != reference));
    }

    /// Stop the driver of the port `id` that left the table, after its
    /// link exit signal and its `DOWN` signals, as `terminate_port` calls
    /// the `stop` callback of the driver (`io.c:3746`): a `tcp_inet` port
    /// ends the receive and each accept that waits (see
    /// [`Self::tcp_stop`]). Dropping the port closes its pipes or its
    /// socket.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let port = ports.remove(id).unwrap();
    /// scheduler.stop_port(id, port);
    /// ```
    fn stop_port(&mut self, id: PortId, port: Port) {
        if let Driver::Inet(tcp) = port.driver {
            self.tcp_stop(id, tcp);
        }
    }

    /// Send the link exit signal of the exiting process `from` to the port
    /// `id`, as `erts_proc_exit_handle_link` and `call_deliver_port_exit`
    /// do (`erl_process.c:14084-14099`, `io.c:2164-2209`): when the
    /// process is the owner, which has the only link to the port, the link
    /// goes, and the port closes, also for the reason `normal`
    /// (`io.c:3876-3879`). The port then has no link, so the close sends no
    /// signal. A port that closed changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.port_link_exit(port, owner, &Term::static_atom("normal"))?;
    /// ```
    ///
    /// # Errors
    ///
    /// A reason other than `kill` for a port with queued output (see
    /// [`check_flush`]).
    pub(in crate::runtime) fn port_link_exit(
        &mut self,
        id: PortId,
        from: ProcessId,
        reason: &Term,
    ) -> Result<(), BeamError> {
        let ports = self.resources.borrow_mut::<Ports>();
        let owned = ports.get(id).filter(|port| port.owner == from);
        owned.map_or(Ok(()), |port| check_flush(port, reason))?;
        if owned.is_some() {
            let mut port = ports.remove(id).expect("the port is open");
            let monitors = std::mem::take(&mut port.monitors);
            self.port_monitors_down(&monitors, reason);
            self.stop_port(id, port);
        }
        Ok(())
    }
}

/// Check the close of the port `port` with the exit reason `reason`. With a
/// reason other than `kill` and queued output, ERTS keeps the port in the
/// `CLOSING` state and writes the queue before it terminates the port
/// (`flush_port`, `io.c:3939-3948`). PBeam does not do this yet.
///
/// # Examples
///
/// ```rust,ignore
/// check_flush(port, &Term::static_atom("normal"))?;
/// ```
///
/// # Errors
///
/// A reason other than `kill` for a port with queued output.
fn check_flush(port: &Port, reason: &Term) -> Result<(), BeamError> {
    let queued = match &port.driver {
        Driver::Inet(tcp) => tcp.send_fd().is_some(),
        Driver::Spawn(_) => false,
    };
    if queued && *reason != Term::static_atom("kill") {
        return Err(error(
            "unsupported close of a tcp_inet port with queued output: see \"tcp_inet ports\" in docs/erts-differences.md",
        ));
    }
    Ok(())
}

/// Return the message `{Port, {Tag, Value}}` of the port `id`.
///
/// # Examples
///
/// ```rust,ignore
/// let message = port_message(PortId(0), "exit_status", Term::Integer(0));
/// ```
fn port_message(id: PortId, tag: &str, value: Term) -> Term {
    Term::tuple(vec![
        Term::Port(id),
        Term::tuple(vec![Term::static_atom(tag), value]),
    ])
}
