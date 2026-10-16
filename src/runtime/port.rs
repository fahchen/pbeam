//! The ports of the node, as the ERTS port table (`erl_ptab.c`) holds the
//! ports of the spawn driver (`sys/unix/sys_drivers.c`). The scheduler
//! keeps them as the `Ports` value of its `Resources`, and all processes of
//! a run share them. PBeam has `spawn_executable` ports and `tcp_inet` and `udp_inet`
//! driver ports (see [`inet`]).

pub(super) mod inet;

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::c_int,
};

use super::host::spawn::Child;
use crate::{
    error::{BeamError, error},
    term::{PortId, ProcessId, Reference, Term},
};
use inet::Inet;

/// The open ports of the node, by port number, and the number of the next
/// port. A port leaves the table when it closes, as `kill_port` removes it
/// from the ERTS port table (`io.c:3697-3790`).
#[derive(Debug, Default)]
pub(super) struct Ports {
    /// Each open port.
    by_id: BTreeMap<PortId, Port>,
    /// The number of the next port. The first port of a run is `#Port<0.0>`.
    next: u64,
}

/// One open port: the ERTS `Port` (`erl_port.h`) with the data of its
/// driver.
#[derive(Debug)]
pub(super) struct Port {
    /// The connected process, which gets the messages of the port
    /// (`ERTS_PORT_GET_CONNECTED`): the process that opened the port, or
    /// the process of the last `erlang:port_connect/2` (io.c:2355-2400).
    pub(super) owner: ProcessId,
    /// The processes that have a link to the port (`ERTS_P_LINKS`), in pid
    /// order. ERTS keeps them in a red-black tree in pid order, and
    /// `port_info/2` gives them in a different order (see "Ports" in
    /// `docs/erts-differences.md`). `open_port/2` links the owner
    /// (`erl_bif_port.c:92-100`), and `link/1`, `unlink/1`, and
    /// `erlang:port_connect/2` change the set. `link/1` and `unlink/1`
    /// change both sides at once. `erlang:port_connect/2` adds the side of
    /// the port, and the new owner adds its side when it handles the link
    /// signal.
    pub(super) links: BTreeSet<ProcessId>,
    /// The native bytes of the program path or of the driver name, as
    /// `port_info/1` gives them in `name` (`erl_bif_info.c:3767-3776`).
    pub(super) name: Vec<u8>,
    /// The term of `erlang:port_set_data/2`, `undefined` at the open
    /// (`erl_bif_port.c:646`). The port owns a copy of the term; it goes
    /// with the port when the port closes.
    pub(super) data: Term,
    /// The bytes that the port sent to the owner (`bytes_in`).
    pub(super) input: u64,
    /// The bytes that the port wrote to the child (`bytes_out`).
    pub(super) output: u64,
    /// The driver of the port and its data.
    pub(super) driver: Driver,
    /// The monitors of processes on the port, from `erlang:monitor(port,
    /// Port)`, as the watcher and the monitor reference, in creation
    /// order. The close of the port sends the `DOWN` message of each
    /// (`erts_port_task_handle_exit`, io.c).
    pub(super) monitors: Vec<(ProcessId, Reference)>,
    /// True in the `CLOSING` state (`ERTS_PORT_SFLG_CLOSING`): the port
    /// closed with queued output, sent its exit signals, and writes its
    /// queue until it terminates (io.c:3939-3948). `erlang:port_info/1,2`
    /// and `erlang:port_control/3` find the port (`sig_lookup_port`,
    /// erl_bif_port.c:145-148). The other port BIFs give `badarg`, and
    /// `link/1` and `monitor/2` see a closed port (`data_lookup_port`,
    /// erl_bif_port.c:157-160; `link_opt`, bif.c:192-196). The driver sends
    /// no message (io.c:6212-6227).
    pub(super) closing: bool,
}

/// The driver of a port and the data of the driver.
#[derive(Debug)]
pub(super) enum Driver {
    /// A port of `open_port({spawn_executable, Name}, Settings)`.
    Spawn(Spawn),
    /// A port of `open_port({spawn_driver, "tcp_inet"}, [binary])` or of
    /// `open_port({spawn_driver, "udp_inet"}, [binary])`.
    Inet(Inet),
}

/// The data of the spawn driver (`ErtsSysDriverData`,
/// `sys_drivers.c:108-121`).
#[derive(Debug)]
pub(super) struct Spawn {
    /// True with the `exit_status` option: the port sends
    /// `{Port, {exit_status, Status}}` before it closes at the end of file.
    pub(super) exit_status: bool,
    /// The started program and the parent ends of its pipes. Dropping the
    /// port closes the pipes.
    pub(super) child: Child,
    /// True after the output pipe gave its end of file, while the port
    /// waits for the exit status of the child (`dd->alive == 1`,
    /// `sys_drivers.c:1232-1240`).
    pub(super) end_of_file: bool,
}

/// What the IO poll found on one fd of a port.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PortEvent {
    /// The child wrote these bytes.
    Data(Vec<u8>),
    /// The output pipe of a port without `exit_status` gave its end of
    /// file.
    EndOfFile,
    /// The child exited with this exit status, or `None` when the host
    /// reaped the child (see [`Child::read_status`]).
    ExitStatus(Option<i32>),
}

impl Port {
    /// Return the descriptor of a `tcp_inet` or `udp_inet` port, or `None` for a port of
    /// the spawn driver.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(port.inet_mut().is_some());
    /// ```
    pub(super) fn inet_mut(&mut self) -> Option<&mut Inet> {
        match &mut self.driver {
            Driver::Inet(tcp) => Some(tcp),
            Driver::Spawn(_) => None,
        }
    }
}

impl Ports {
    /// Put `port` into the table with the next port number, and return the
    /// number.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ports.insert(port)?, PortId(0));
    /// ```
    ///
    /// # Errors
    ///
    /// The port number overflows.
    pub(super) fn insert(&mut self, port: Port) -> Result<PortId, BeamError> {
        let id = PortId(self.next);
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| error("port identifier overflow"))?;
        self.by_id.insert(id, port);
        Ok(id)
    }

    /// Return true when the port `id` is in the `CLOSING` state.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!ports.closing(PortId(0)));
    /// ```
    pub(super) fn closing(&self, id: PortId) -> bool {
        self.by_id.get(&id).is_some_and(|port| port.closing)
    }

    /// Return the open port `id`, or `None` when it closed.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(ports.get(PortId(0)).is_some());
    /// ```
    pub(super) fn get(&self, id: PortId) -> Option<&Port> {
        self.by_id.get(&id)
    }

    /// Return the open port `id` for a change, or `None` when it closed.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// ports.get_mut(PortId(0)).unwrap().output += 5;
    /// ```
    pub(super) fn get_mut(&mut self, id: PortId) -> Option<&mut Port> {
        self.by_id.get_mut(&id)
    }

    /// Remove the port `id` from the table and return it. Dropping the port
    /// closes its pipes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(ports.remove(PortId(0)).is_some());
    /// assert!(ports.get(PortId(0)).is_none());
    /// ```
    pub(super) fn remove(&mut self, id: PortId) -> Option<Port> {
        self.by_id.remove(&id)
    }

    /// Return the fds that the IO poll waits for: the output pipe of each
    /// spawn port before its end of file, and then its status pipe, and the
    /// socket of each `tcp_inet` listen port with a caller in its accept
    /// queue, and the socket of each active `udp_inet` port. Only a port with `exit_status` stays open after its end of
    /// file.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ports.wait_fds(), vec![child.output_fd()]);
    /// ```
    pub(super) fn wait_fds(&self) -> Vec<c_int> {
        self.by_id
            .values()
            .filter_map(|port| match &port.driver {
                Driver::Spawn(spawn) if spawn.end_of_file => Some(spawn.child.status_fd()),
                Driver::Spawn(spawn) => Some(spawn.child.output_fd()),
                Driver::Inet(inet) => inet
                    .accept_fd()
                    .or_else(|| inet.receive_fd())
                    .or_else(|| inet.stream_receive_fd()),
            })
            .collect()
    }

    /// Return the fd of each `tcp_inet` port with queued output, which the
    /// idle wait selects for write (`sock_select(desc, FD_WRITE, 1)`,
    /// inet_drv.c:13846).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(ports.write_fds().is_empty());
    /// ```
    pub(super) fn write_fds(&self) -> Vec<c_int> {
        self.by_id
            .values()
            .filter_map(|port| match &port.driver {
                Driver::Inet(inet) => inet.send_fd(),
                Driver::Spawn(_) => None,
            })
            .collect()
    }

    /// Return the `tcp_inet` port whose socket is `fd` and has a receive
    /// that waits, or `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ports.stream_receiving(fd), Some(port));
    /// ```
    pub(super) fn stream_receiving(&self, fd: c_int) -> Option<PortId> {
        self.inet_port(fd, Inet::stream_receive_fd)
    }

    /// Return the `tcp_inet` port whose socket is `fd` and has queued
    /// output, or `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ports.sending(fd), Some(port));
    /// ```
    pub(super) fn sending(&self, fd: c_int) -> Option<PortId> {
        self.inet_port(fd, Inet::send_fd)
    }

    /// Return the inet port for which `select` gives `fd`.
    fn inet_port(&self, fd: c_int, select: impl Fn(&Inet) -> Option<c_int>) -> Option<PortId> {
        self.by_id
            .iter()
            .find_map(|(&id, port)| match &port.driver {
                Driver::Inet(inet) if select(inet) == Some(fd) => Some(id),
                _ => None,
            })
    }

    /// Return the `tcp_inet` listen port that waits for a connection on the
    /// fd `fd`, or `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ports.accepting(fd), Some(PortId(0)));
    /// ```
    pub(super) fn accepting(&self, fd: c_int) -> Option<PortId> {
        self.by_id
            .iter()
            .find_map(|(&id, port)| match &port.driver {
                Driver::Inet(tcp) if tcp.accept_fd() == Some(fd) => Some(id),
                _ => None,
            })
    }

    /// Return whether an active `udp_inet` port waits for a datagram on the
    /// fd `fd`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(ports.receiving(fd));
    /// ```
    pub(super) fn receiving(&self, fd: c_int) -> bool {
        self.by_id
            .values()
            .any(|port| matches!(&port.driver, Driver::Inet(inet) if inet.receive_fd() == Some(fd)))
    }

    /// Read the fd `fd` that the IO poll found ready, and return its port
    /// and what the read found, as `ready_input` does
    /// (`sys_drivers.c:1267-1347`). Return `None` when no port waits for
    /// the fd, and when the read of the output pipe fails, for example with
    /// `EINTR`: the port then waits again (`sys_drivers.c:1340-1342`).
    /// The end of file of a port with `exit_status` also returns `None`:
    /// the port then waits for the exit status of the child (`dd->alive ==
    /// 1`, `sys_drivers.c:1232-1240`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ports.read(fd), Some((PortId(0), PortEvent::Data(b"hi".to_vec()))));
    /// ```
    pub(super) fn read(&mut self, fd: c_int) -> Option<(PortId, PortEvent)> {
        let (id, spawn) = self
            .by_id
            .iter_mut()
            .find_map(|(&id, port)| match &mut port.driver {
                Driver::Spawn(spawn)
                    if spawn.child.output_fd() == fd
                        || (spawn.end_of_file && spawn.child.status_fd() == fd) =>
                {
                    Some((id, spawn))
                }
                _ => None,
            })?;
        if spawn.end_of_file {
            return Some((id, PortEvent::ExitStatus(spawn.child.read_status())));
        }
        let bytes = spawn.child.read_output().ok()?;
        let event = if bytes.is_empty() && spawn.exit_status {
            spawn.end_of_file = true;
            return None;
        } else if bytes.is_empty() {
            PortEvent::EndOfFile
        } else {
            PortEvent::Data(bytes)
        };
        Some((id, event))
    }
}
