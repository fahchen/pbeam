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
            links: [owner].into(),
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
    /// (`io.c:3698-3790`, `3841-3950`): the port leaves the table, each
    /// process linked to the port gets a link exit signal from the port
    /// with the reason, `killed` for `kill` (`link_port_exit`,
    /// `io.c:3802-3813`), each monitor of the port gets its `DOWN` signal,
    /// and then the driver stops (see [`Self::stop_port`]) and the pipes of
    /// the port close.
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
        let reason = if reason == Term::static_atom("kill") {
            Term::static_atom("killed")
        } else {
            reason
        };
        for linked in std::mem::take(&mut port.links) {
            let exit = Signal::Exit {
                from: LinkId::Port(id),
                reason: reason.clone(),
                linked: true,
                normal_kills: false,
                alias: None,
            };
            // A linked process that does not exist drops the signal.
            let _ = self.send_signal(None, linked, exit);
        }
        let monitors = std::mem::take(&mut port.monitors);
        self.port_monitors_down(&monitors, &reason);
        self.stop_port(id, port);
        Ok(())
    }

    /// Close the open port `id` with the reason `reason` from its driver,
    /// as `driver_exit` does (io.c:7228-7249): the port first unlinks its
    /// owner (`unlink_proc`, io.c:2889-2909), so only the other linked
    /// processes get the link exit signal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.driver_exit(port, Term::static_atom("normal"))?;
    /// ```
    ///
    /// # Errors
    ///
    /// The error of [`Self::close_port`].
    pub(super) fn driver_exit(&mut self, id: PortId, reason: Term) -> Result<(), BeamError> {
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        let port = port.expect("the port of a driver exit is open");
        let owner = port.owner;
        port.links.remove(&owner);
        if let Some(process) = self.process_mut(owner) {
            process.links.remove(&LinkId::Port(id));
        }
        self.close_port(id, reason)
    }

    /// Link the current process to the port `id`, as `link_opt` does for a
    /// port (`bif.c:186-232`), and return false when the port is not open
    /// and the process does not trap exits (`noproc`). A trapping process
    /// gets the link exit signal `noproc` from a port that is not open. A
    /// link that exists changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.link_port(port));
    /// ```
    pub(in crate::runtime) fn link_port(&mut self, id: PortId) -> bool {
        let current = self.current_process_mut().id;
        let ports = self.resources.borrow_mut::<Ports>();
        let Some(port) = ports.get_mut(id) else {
            if !self.current_process_mut().trap_exit {
                return false;
            }
            let exit = Signal::Exit {
                from: LinkId::Port(id),
                reason: Term::static_atom("noproc"),
                linked: true,
                normal_kills: false,
                alias: None,
            };
            self.current_process_mut().links.insert(LinkId::Port(id), 0);
            self.send_signal(None, current, exit)
                .expect("the current process gets the exit signal");
            return true;
        };
        port.links.insert(current);
        self.current_process_mut().links.insert(LinkId::Port(id), 0);
        true
    }

    /// Unlink the current process from the port `id`, as `unlink_1` does
    /// for a port (`bif.c:1252-1287`). The port runs the unlink at once, so
    /// both sides of the link go at once. A port without a link to the
    /// process changes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.unlink_port(port);
    /// ```
    pub(in crate::runtime) fn unlink_port(&mut self, id: PortId) {
        let current = self.current_process_mut().id;
        if self.current_process_mut().links.get(&LinkId::Port(id)) != Some(&0) {
            return;
        }
        self.current_process_mut().links.remove(&LinkId::Port(id));
        if let Some(port) = self.resources.borrow_mut::<Ports>().get_mut(id) {
            port.links.remove(&current);
        }
    }

    /// Make `pid` the owner of the open port `id`, as the BIF operation of
    /// `set_port_connected` does (io.c:2355-2400): a port without a link to
    /// `pid` adds the link and sends the link signal [`Signal::PortLink`],
    /// and the old owner keeps its link. Return false when `pid` does not
    /// exist, as the link signal to it fails (`ERTS_PORT_OP_DROPPED`). A
    /// process that exits before it handles the signal closes the port
    /// with `noproc`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.connect_port(port, handler));
    /// ```
    pub(in crate::runtime) fn connect_port(&mut self, id: PortId, pid: ProcessId) -> bool {
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        let port = port.expect("the port to connect is open");
        if !port.links.contains(&pid) {
            let link = Signal::PortLink { port: id };
            if self.send_signal(None, pid, link).is_err() {
                return false;
            }
        }
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        let port = port.expect("the port to connect is open");
        port.links.insert(pid);
        port.owner = pid;
        true
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
    /// do (`erl_process.c:14084-14099`, `io.c:2160-2190`): the link goes,
    /// and the port closes with the reason (see [`Self::close_port`]),
    /// except for the reason `normal` from a process that is not the owner
    /// (`erts_deliver_port_exit`, `io.c:3869-3872`). A port that closed
    /// changes nothing.
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
        let port = self.resources.borrow_mut::<Ports>().get_mut(id);
        // A port that closed sent its link exit signal to `from`.
        let closes = port.is_some_and(|port| {
            assert!(
                port.links.remove(&from),
                "an open port has the link of its linked process"
            );
            *reason != Term::static_atom("normal") || port.owner == from
        });
        if closes {
            self.close_port(id, reason.clone())?;
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
