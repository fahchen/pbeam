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
        host::spawn::Child,
        port::{Port, PortEvent, Ports},
        signal::Signal,
    },
    term::{PortId, ProcessId, Term},
};

impl Scheduler {
    /// Open a port for the program `child`, owned by and linked to the
    /// current process, as `erts_internal_open_port_2` does
    /// (`erl_bif_port.c:60-130`). The link goes into both the port and the
    /// link tree of the process at once, as `erts_link_tree_insert` does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let port = scheduler.open_port(b"/bin/sh".to_vec(), true, child)?;
    /// assert_eq!(port, PortId(0));
    /// ```
    ///
    /// # Errors
    ///
    /// The port number overflows.
    pub(in crate::runtime) fn open_port(
        &mut self,
        name: Vec<u8>,
        exit_status: bool,
        child: Child,
    ) -> Result<PortId, BeamError> {
        let owner = self.current_process_mut().id;
        let port = Port {
            owner,
            name,
            exit_status,
            child,
            end_of_file: false,
            input: 0,
            output: 0,
        };
        let id = self.resources.borrow_mut::<Ports>().insert(port)?;
        self.current_process_mut().links.insert(LinkId::Port(id), 0);
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
    /// `os:set_signal(sigchld, ignore)`.
    pub(in crate::runtime) fn port_ready(&mut self, fd: libc::c_int) -> Result<(), BeamError> {
        let ports = self.resources.borrow_mut::<Ports>();
        let Some((id, event)) = ports.read(fd) else {
            return Ok(());
        };
        let port = ports.get_mut(id).expect("the port of a read is open");
        let owner = port.owner;
        match event {
            PortEvent::Data(bytes) => {
                port.input += bytes.len() as u64;
                self.deliver(owner, port_message(id, "data", Term::Binary(bytes)));
            }
            PortEvent::EndOfFile if port.exit_status => port.end_of_file = true,
            PortEvent::EndOfFile => self.close_port(id, Term::static_atom("normal")),
            PortEvent::ExitStatus(Some(status)) => {
                let status = Term::Integer(status.into());
                self.deliver(owner, port_message(id, "exit_status", status));
                self.close_port(id, Term::static_atom("normal"));
            }
            PortEvent::ExitStatus(None) => {
                return Err(error(
                    "unsupported exit status of a port program after os:set_signal(sigchld, ignore): see \"Ports\" in docs/erts-differences.md",
                ));
            }
        }
        Ok(())
    }

    /// Close the open port `id` with the exit reason `reason`, as
    /// `erts_deliver_port_exit` and `terminate_port` do
    /// (`io.c:3698-3790`, `3841-3950`): the port leaves the table, and its
    /// pipes close. The owner, which has the only link to the port, gets a
    /// link exit signal from the port with the reason (`link_port_exit`,
    /// `io.c:3802-3813`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.close_port(port, Term::static_atom("normal"));
    /// ```
    pub(in crate::runtime) fn close_port(&mut self, id: PortId, reason: Term) {
        let port = self
            .resources
            .borrow_mut::<Ports>()
            .remove(id)
            .expect("the port to close is open");
        let exit = Signal::Exit {
            from: LinkId::Port(id),
            reason,
            linked: true,
            normal_kills: false,
            alias: None,
        };
        // An owner that does not exist drops the signal.
        let _ = self.send_signal(None, port.owner, exit);
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
    /// scheduler.port_link_exit(port, owner);
    /// ```
    pub(in crate::runtime) fn port_link_exit(&mut self, id: PortId, from: ProcessId) {
        let ports = self.resources.borrow_mut::<Ports>();
        if ports.get(id).is_some_and(|port| port.owner == from) {
            ports.remove(id);
        }
    }
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
