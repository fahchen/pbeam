//! The ports of the node, as the ERTS port table (`erl_ptab.c`) holds the
//! ports of the spawn driver (`sys/unix/sys_drivers.c`). The scheduler
//! keeps them as the `Ports` value of its `Resources`, and all processes of
//! a run share them. PBeam has spawn ports, fd ports, and `tcp_inet` and `udp_inet`
//! driver ports (see [`inet`]).

pub(super) mod inet;

use std::{
    collections::BTreeMap,
    ffi::c_int,
    sync::{Arc, Mutex, MutexGuard},
};

use super::{
    host::spawn::{Child, Fds},
    process::MonitorLinkTree,
    scheduler::poll::{Direction, NodePoll},
};
use crate::term::{PortId, ProcessId, Reference, Term};
use inet::Inet;

/// The open ports of the node, by port number. A port leaves the table when
/// it closes, as `kill_port` removes it from the ERTS port table
/// (`io.c:3697-3790`).
#[derive(Debug, Default)]
pub(super) struct Ports {
    /// Each open port.
    /// Each open port, with the scheduler that serves it, which does not
    /// change, so that a scan of one scheduler locks only its own ports.
    by_id: BTreeMap<PortId, (usize, PortCell)>,
}

/// One open port: the ERTS `Port` (`erl_port.h`) with the data of its
/// driver.
#[derive(Debug)]
pub(super) struct Port {
    /// The connected process, which gets the messages of the port
    /// (`ERTS_PORT_GET_CONNECTED`): the process that opened the port, or
    /// the process of the last `erlang:port_connect/2` (io.c:2355-2400).
    pub(super) owner: ProcessId,
    /// The processes that have a link to the port (`ERTS_P_LINKS`), as the
    /// ERTS link tree: the same red-black tree as the links of a process,
    /// with the key order `ml_cmp_keys` (`erl_monitor_link.c:91-233`), the
    /// insert, delete, and rebalance steps of `erl_rbtree.h`, and the
    /// post-order walk of `foreach_unordered__` (`erl_rbtree.h:1343-1458`).
    /// Thus the tree has the ERTS shape after each sequence of links and
    /// unlinks: the `links` item of `erlang:port_info/1,2` conses the walk
    /// to the front (`erl_bif_info.c:3676-3696`), and the close of the port
    /// sends the link exit signals in the walk order (`io.c:3922-3925`).
    /// `open_port/2` links the owner (`erl_bif_port.c:92-100`), and
    /// `link/1`, `unlink/1`, and `erlang:port_connect/2` change the set.
    /// `link/1` and `unlink/1` change both sides at once.
    /// `erlang:port_connect/2` adds the side of the port, and the new owner
    /// adds its side when it handles the link signal.
    pub(super) links: MonitorLinkTree<ProcessId, ()>,
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
    /// Send `{Port, closed}` to the owner when the port terminates: a
    /// `{Pid, close}` message closed it (`ERTS_PORT_SFLG_SEND_CLOSED`,
    /// io.c:3713-3719).
    pub(super) send_closed: bool,
    /// The scheduler that opened the port and serves its I/O: its poll
    /// waits for the fds of the port. The port itself is in the node port
    /// table, so a BIF of any scheduler reads and changes it.
    pub(super) scheduler: usize,
    /// The poll thread of the node, which a new fd of the port wakes (see
    /// [`PortGuard`]).
    pub(super) poll: Arc<NodePoll>,
    /// The identifier of the port, which names its fds in the poll.
    pub(super) id: PortId,
}

/// The driver of a port and the data of the driver.
#[derive(Debug)]
pub(super) enum Driver {
    /// A port of `open_port({spawn_executable, Name}, Settings)`, of
    /// `{spawn, Command}`, or of `{fd, In, Out}`.
    Spawn(Spawn),
    /// A port of `open_port({spawn_driver, "tcp_inet"}, [binary])` or of
    /// `open_port({spawn_driver, "udp_inet"}, [binary])`.
    Inet(Inet),
}

/// The data of the spawn driver (`ErtsSysDriverData`,
/// `sys_drivers.c:108-121`).
#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the independent port flags of ERTS (`ERTS_PORT_SFLG_SOFT_EOF`, `ERTS_PORT_SFLG_BINARY_IO`, `ERTS_PORT_SFLG_PARALLELISM`) and `exit_status` of `SysDriverOpts`, which `open_port` sets one by one"
)]
pub(super) struct Spawn {
    /// True with the `exit_status` option: the port sends
    /// `{Port, {exit_status, Status}}` before it closes at the end of file.
    pub(super) exit_status: bool,
    /// True with the `eof` option (`ERTS_PORT_SFLG_SOFT_EOF`,
    /// `erl_bif_port.c:995-996`): at the end of file, the port sends
    /// `{Port, eof}` and stays open (`driver_failure_term`,
    /// `io.c:7208-7209`).
    pub(super) soft_eof: bool,
    /// The packet framing of the `{packet, N}` option.
    pub(super) packets: Packets,
    /// True with the `binary` option (`ERTS_PORT_SFLG_BINARY_IO`): the data
    /// of a message is a binary, else a list of bytes
    /// (`deliver_read_message`, `io.c:3399-3417`).
    pub(super) binary: bool,
    /// The line buffer of the `{line, L}` and `line` options
    /// (`ERTS_PORT_SFLG_LINEBUF_IO`, `erl_bif_port.c:1139-1142`), or `None`
    /// without them.
    pub(super) lines: Option<Lines>,
    /// True with `{parallelism, true}` (`ERTS_PORT_SFLG_PARALLELISM`,
    /// `erl_bif_port.c:913-919`), which `port_info/2` gives.
    pub(super) parallelism: bool,
    /// The started program and the parent ends of its pipes, or the fds of
    /// an fd port.
    pub(super) program: Program,
    /// The state of the output pipe of the child.
    pub(super) input: Input,
}

/// What a port of the spawn driver or of the fd driver reads and writes.
#[derive(Debug)]
pub(super) enum Program {
    /// A started program. Dropping the port closes its pipes.
    Child(Child),
    /// The fds of `open_port({fd, In, Out}, Settings)`. Dropping the port
    /// does not close them, as `fd_stop` does not
    /// (`sys_drivers.c:1002-1027`).
    Fds(Fds),
}

impl Program {
    /// Return the started program, or `None` for an fd port.
    pub(super) fn child(&self) -> Option<&Child> {
        match self {
            Program::Child(child) => Some(child),
            Program::Fds(_) => None,
        }
    }

    /// Return the fd that the port reads, or `None` with the `out` option.
    pub(super) fn output_fd(&self) -> Option<c_int> {
        match self {
            Program::Child(child) => child.output_fd(),
            Program::Fds(fds) => Some(fds.input),
        }
    }

    /// Read the fd of [`Program::output_fd`] with one `read` of at most
    /// 64 KiB, as `ready_input` does (`sys_drivers.c:1336-1347`).
    pub(super) fn read_output(&self) -> Result<Vec<u8>, c_int> {
        match self {
            Program::Child(child) => child.read_output(),
            Program::Fds(fds) => fds.read(),
        }
    }

    /// Write all bytes of `bytes` to the fd that the port writes.
    pub(super) fn write_input(&self, bytes: &[u8]) -> Result<(), c_int> {
        match self {
            Program::Child(child) => child.write_input(bytes),
            Program::Fds(fds) => fds.write(bytes),
        }
    }
}

/// The state of the output pipe of a spawn port, which the port reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Input {
    /// The port reads the pipe.
    Open,
    /// The `out` option opened the port for writing only: ERTS closes the
    /// read end after the handshake and keeps no read data
    /// (`sys_drivers.c:1307-1311`), so no data message, no end of file,
    /// and no exit status arrives.
    Write,
    /// The pipe gave its end of file, and the port waits for the exit
    /// status of the child (`dd->alive == 1`, `sys_drivers.c:1232-1240`).
    Status,
    /// The port with `eof` sent `{Port, eof}`, and it reads nothing more:
    /// `port_inp_failure` stops the select of the pipe
    /// (`sys_drivers.c:1211-1214`).
    Ended,
}

/// The packet framing of a spawn port (`packet_bytes` of
/// `ErtsSysDriverData`, `sys_drivers.c:112`): each packet has a header of
/// 1, 2, or 4 bytes with the size of its data, big-endian. With no header
/// (the default), the data has no framing.
#[derive(Debug, Default)]
pub(super) struct Packets {
    /// The size of the header: 0, 1, 2, or 4.
    header: usize,
    /// The bytes of a packet that the child did not write completely: its
    /// header, or a part of it, and a part of its data.
    pending: Vec<u8>,
}

impl Packets {
    /// Return the framing of the header size `header`: 0, 1, 2, or 4.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let packets = Packets::new(4);
    /// ```
    pub(super) fn new(header: usize) -> Self {
        Self {
            header,
            pending: Vec::new(),
        }
    }

    /// Return the data of each complete packet of the bytes that the child
    /// wrote, after the bytes of earlier reads, as `ready_input` does
    /// (`sys_drivers.c:1336-1440`). Keep the bytes of a packet that is not
    /// complete for the next read. Without a header, return the bytes as
    /// one part.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut packets = Packets::new(1);
    /// assert_eq!(packets.split(vec![2, b'h']), Vec::<Vec<u8>>::new());
    /// assert_eq!(packets.split(vec![b'i', 0]), vec![b"hi".to_vec(), Vec::new()]);
    /// ```
    pub(super) fn split(&mut self, bytes: Vec<u8>) -> Vec<Vec<u8>> {
        if self.header == 0 {
            return vec![bytes];
        }
        self.pending.extend(bytes);
        let mut parts = Vec::new();
        let mut start = 0;
        while let Some(header) = self.pending.get(start..start + self.header) {
            let size = header
                .iter()
                .fold(0, |size, &byte| (size << 8) | usize::from(byte));
            let data = start + self.header;
            let Some(packet) = self.pending.get(data..data + size) else {
                break;
            };
            parts.push(packet.to_vec());
            start = data + size;
        }
        self.pending.drain(..start);
        parts
    }

    /// Return the bytes that the port writes for the data `bytes`: the
    /// header with the size and then the data, as `output` does
    /// (`sys_drivers.c:1140-1203`). Return `None` when the size does not
    /// fit in a header of 1 or 2 bytes (`sys_drivers.c:1153-1158`). A
    /// header of 4 bytes keeps the low 32 bits of the size (`put_int32`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Packets::new(2).frame(b"hi".to_vec()), Some(vec![0, 2, b'h', b'i']));
    /// assert_eq!(Packets::new(1).frame(vec![0; 256]), None);
    /// ```
    pub(super) fn frame(&self, bytes: Vec<u8>) -> Option<Vec<u8>> {
        let limit = match self.header {
            0 => return Some(bytes),
            1 => 0xff,
            2 => 0xffff,
            _ => usize::MAX,
        };
        if bytes.len() > limit {
            return None;
        }
        // The header keeps the low 32 bits of the length, as `put_int32` does.
        let [b0, b1, b2, b3, ..] = bytes.len().to_le_bytes();
        let size = [b3, b2, b1, b0];
        let mut framed = size[4 - self.header..].to_vec();
        framed.extend(bytes);
        Some(framed)
    }
}

impl Spawn {
    /// Return the data term of a message with the bytes `bytes`: a binary
    /// with `binary`, else a list of bytes, in `{eol, Data}` or
    /// `{noeol, Data}` in line mode (`io.c:3399-3427`).
    fn data(&self, eol: Option<bool>, bytes: Vec<u8>) -> Term {
        let data = if self.binary {
            Term::binary(bytes)
        } else {
            Term::list_from(bytes.into_iter().map(|byte| Term::Integer(byte.into())))
        };
        match eol {
            Some(eol) => Term::tuple(vec![
                Term::static_atom(if eol { "eol" } else { "noeol" }),
                data,
            ]),
            None => data,
        }
    }

    /// Return the data terms of the messages of the parts `parts`, each in
    /// lines in line mode, as `driver_output2` delivers each part
    /// (`io.c:6425-6430`, `3448-3460`).
    fn messages(&mut self, parts: Vec<Vec<u8>>) -> Vec<Term> {
        let mut lines = Vec::new();
        let Some(buffer) = &mut self.lines else {
            return parts
                .into_iter()
                .map(|bytes| self.data(None, bytes))
                .collect();
        };
        for bytes in parts {
            buffer.read(&bytes, &mut lines);
        }
        lines
            .into_iter()
            .map(|(eol, bytes)| self.data(Some(eol), bytes))
            .collect()
    }

    /// Take the end of file of the port: a port with `eof` reads nothing
    /// more (`io.c:7191-7209`), and the line that is not complete goes out.
    fn end_of_file(&mut self) -> End {
        if self.soft_eof {
            self.input = Input::Ended;
        }
        (self.soft_eof, self.flush_lines())
    }

    /// Return the data terms of the lines that the line buffer holds at the
    /// end of file, as `flush_linebuf_messages` does before the port closes
    /// or sends `{Port, eof}` (`io.c:3469-3493`, `7204-7205`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The child wrote "ab" without a newline.
    /// assert_eq!(spawn.flush_lines().len(), 1);
    /// ```
    fn flush_lines(&mut self) -> Vec<Term> {
        let mut lines = Vec::new();
        if let Some(buffer) = &mut self.lines {
            buffer.flush(&mut lines);
        }
        lines
            .into_iter()
            .map(|(eol, bytes)| self.data(Some(eol), bytes))
            .collect()
    }
}

/// The state of a line buffer (`io.c:3138-3141`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineState {
    /// `LINEBUF_MAIN`: the buffer takes bytes.
    Main,
    /// `LINEBUF_FULL`: the buffer holds `L` bytes.
    Full,
    /// `LINEBUF_CR_INSIDE`: a `\r` came after the bytes of the buffer.
    CrInside,
    /// `LINEBUF_CR_AFTER`: a `\r` came after a full buffer.
    CrAfter,
}

/// The line buffer of a spawn port in line mode (`LineBuf`, `io.c:3143-3301`).
#[derive(Debug)]
pub(super) struct Lines {
    /// The maximum size of a line, `L` of `{line, L}`.
    size: usize,
    /// The state of the buffer.
    state: LineState,
    /// The bytes of the line that is not complete.
    pending: Vec<u8>,
}

impl Lines {
    /// Return an empty line buffer for lines of at most `size` bytes
    /// (`allocate_linebuf`, `io.c:3149-3159`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let lines = Lines::new(512);
    /// ```
    pub(super) fn new(size: usize) -> Self {
        Self {
            size,
            state: LineState::Main,
            pending: Vec::new(),
        }
    }

    /// Add to `lines` each line of `bytes` that is complete, with true for
    /// `eol`, and each part of `L` bytes of a longer line, with false for
    /// `noeol`, as `read_linebuf` does (`io.c:3246-3301`). A `\n` or a
    /// `\r\n` ends a line.
    fn read(&mut self, bytes: &[u8], lines: &mut Vec<(bool, Vec<u8>)>) {
        let mut rest = bytes;
        while let Some((&byte, after)) = rest.split_first() {
            if byte == b'\n' {
                self.state = LineState::Main;
                rest = after;
                lines.push((true, std::mem::take(&mut self.pending)));
                continue;
            }
            match self.state {
                LineState::Main if self.pending.len() == self.size => self.state = LineState::Full,
                LineState::Main if byte == b'\r' => {
                    rest = after;
                    self.state = LineState::CrInside;
                }
                LineState::Main => {
                    self.pending.push(byte);
                    rest = after;
                }
                LineState::Full if byte == b'\r' => {
                    rest = after;
                    self.state = LineState::CrAfter;
                }
                LineState::Full => {
                    lines.push((false, std::mem::take(&mut self.pending)));
                    self.state = LineState::Main;
                }
                LineState::CrInside => {
                    self.pending.push(b'\r');
                    self.state = LineState::Main;
                }
                LineState::CrAfter => {
                    lines.push((false, std::mem::take(&mut self.pending)));
                    self.state = LineState::CrInside;
                }
            }
        }
    }

    /// Add to `lines` each `noeol` part that the buffer holds, as the
    /// calls of `flush_linebuf` until `LINEBUF_EMPTY` do (`io.c:3197-3224`,
    /// `3484`).
    fn flush(&mut self, lines: &mut Vec<(bool, Vec<u8>)>) {
        loop {
            match self.state {
                LineState::CrInside => {
                    self.pending.push(b'\r');
                    self.state = LineState::Main;
                }
                LineState::Main | LineState::Full => self.state = LineState::Main,
                LineState::CrAfter => self.state = LineState::CrInside,
            }
            if self.pending.is_empty() {
                return;
            }
            lines.push((false, std::mem::take(&mut self.pending)));
        }
    }
}

/// What the IO poll found on one fd of a port.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PortEvent {
    /// The child wrote the data terms of these messages: one part without
    /// a packet header, else each complete packet, each in lines in line
    /// mode.
    Data(Vec<Term>),
    /// The output pipe of a port without `exit_status` gave its end of
    /// file. The bytes of a packet that is not complete go.
    EndOfFile(End),
    /// The child exited with this exit status, or `None` when the host
    /// reaped the child (see [`Child::read_status`]).
    ExitStatus(Option<i32>, End),
}

/// The end of file of a spawn port: true when the port has `eof` and
/// stays open, and the data terms of the line that is not complete in
/// line mode (`driver_failure_eof`, `io.c:7191-7219`).
pub(super) type End = (bool, Vec<Term>);

impl Port {
    /// Return the fd that the poll waits to read for the port, as the read
    /// select of its driver: the output pipe of a spawn port before its end
    /// of file, and then its status pipe, the socket of a `tcp_inet` listen
    /// port with a caller in its accept queue, or the socket of an active
    /// `udp_inet` port.
    fn wait_fd(&self) -> Option<c_int> {
        match &self.driver {
            Driver::Spawn(spawn) => match spawn.input {
                Input::Open => spawn.program.output_fd(),
                Input::Write | Input::Ended => None,
                Input::Status => spawn.program.child().map(Child::status_fd),
            },
            Driver::Inet(inet) => inet
                .accept_fd()
                .or_else(|| inet.receive_fd())
                .or_else(|| inet.stream_receive_fd()),
        }
    }

    /// Return the fd that the poll waits to write for the port: the socket
    /// of a `tcp_inet` port with queued output.
    fn write_fd(&self) -> Option<c_int> {
        match &self.driver {
            Driver::Inet(inet) => inet.send_fd(),
            Driver::Spawn(_) => None,
        }
    }

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

/// The lock of one port, as each ERTS port has its own lock
/// (`erts_port_lock`, io.c:923-1002). The slot holds `None` once the port
/// closed: a holder of the handle that locks it after the close sees the
/// closed port, as `erts_port_lookup` checks the state of the port under
/// its lock (`erl_port.h`). Code takes a port lock before the table lock or
/// after it is released, and never holds two port locks.
pub(super) type PortCell = Arc<Mutex<Option<Port>>>;

/// Lock the port of `cell`.
///
/// # Examples
///
/// ```rust,ignore
/// let slot = lock(&cell);
/// assert!(slot.is_some());
/// ```
pub(super) fn lock(cell: &PortCell) -> PortGuard<'_> {
    let slot = cell.lock().expect("a port lock is not poisoned");
    let before = slot.as_ref().map(Interest::of);
    PortGuard { slot, before }
}

/// The fds that the poll waits on for a port, with the port.
#[derive(Debug)]
struct Interest {
    read: Option<c_int>,
    write: Option<c_int>,
    id: PortId,
    scheduler: usize,
    poll: Arc<NodePoll>,
}

impl Interest {
    fn of(port: &Port) -> Self {
        Self {
            read: port.wait_fd(),
            write: port.write_fd(),
            id: port.id,
            scheduler: port.scheduler,
            poll: Arc::clone(&port.poll),
        }
    }
}

/// The held lock of one port. At the release, the guard gives each fd of
/// the port that changed to the poll thread of the node (see
/// [`NodePoll::register`]), and a closed port removes its fds.
#[derive(Debug)]
pub(super) struct PortGuard<'a> {
    slot: MutexGuard<'a, Option<Port>>,
    before: Option<Interest>,
}

impl std::ops::Deref for PortGuard<'_> {
    type Target = Option<Port>;

    fn deref(&self) -> &Option<Port> {
        &self.slot
    }
}

impl std::ops::DerefMut for PortGuard<'_> {
    fn deref_mut(&mut self) -> &mut Option<Port> {
        &mut self.slot
    }
}

impl Drop for PortGuard<'_> {
    fn drop(&mut self) {
        let after = self.slot.as_ref().map(Interest::of);
        let Some(owner) = after.as_ref().or(self.before.as_ref()) else {
            return;
        };
        let fd = |interest: &Option<Interest>, read: bool| {
            interest
                .as_ref()
                .and_then(|interest| if read { interest.read } else { interest.write })
        };
        for (direction, read) in [(Direction::Read, true), (Direction::Write, false)] {
            owner.poll.register(
                owner.id,
                owner.scheduler,
                direction,
                fd(&self.before, read),
                fd(&after, read),
            );
        }
    }
}

impl Ports {
    /// Put `port` under the identifier `id`, which no open port has.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// ports.insert_at(PortId(7), port);
    /// ```
    pub(super) fn insert_at(&mut self, id: PortId, port: Port) {
        port.poll
            .register(id, port.scheduler, Direction::Read, None, port.wait_fd());
        port.poll
            .register(id, port.scheduler, Direction::Write, None, port.write_fd());
        self.by_id
            .insert(id, (port.scheduler, Arc::new(Mutex::new(Some(port)))));
    }

    /// Return the identifiers of the open ports, in no order, as
    /// `erts_ptab_list` (`erl_ptab.c:738-...`) gives them to
    /// `erlang:ports/0`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ports.ids(), vec![PortId(0)]);
    /// ```
    pub(super) fn ids(&self) -> Vec<PortId> {
        self.by_id.keys().copied().collect()
    }

    /// Return the handle of the port `id`, or `None` when it is not in the
    /// table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(ports.get(PortId(0)).is_some());
    /// ```
    pub(super) fn get(&self, id: PortId) -> Option<PortCell> {
        self.by_id.get(&id).map(|(_, cell)| Arc::clone(cell))
    }

    /// Remove the port `id` from the table. The port closed before: its
    /// slot holds `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// ports.remove(PortId(0));
    /// assert!(ports.get(PortId(0)).is_none());
    /// ```
    pub(super) fn remove(&mut self, id: PortId) {
        self.by_id.remove(&id);
    }

    /// Return the handles of the ports that the scheduler `index` serves.
    /// The caller releases the table lock before it locks a port.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ports.handles(0).len(), 1);
    /// ```
    pub(super) fn handles(&self, index: usize) -> Vec<(PortId, PortCell)> {
        self.by_id
            .iter()
            .filter(|(_, (scheduler, _))| *scheduler == index)
            .map(|(&id, (_, cell))| (id, Arc::clone(cell)))
            .collect()
    }
}

/// Lock the ports of `handles` (see [`Ports::handles`]), one at a time,
/// after the table lock is released.
fn served(handles: &[(PortId, PortCell)]) -> impl Iterator<Item = (PortId, PortGuard<'_>)> {
    handles.iter().map(|(id, cell)| (*id, lock(cell)))
}

/// Return the `tcp_inet` port of `handles` whose fd for `select` is `fd`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(inet_port(&handles, fd, Inet::send_fd), Some(port));
/// ```
pub(super) fn inet_port(
    handles: &[(PortId, PortCell)],
    fd: c_int,
    select: impl Fn(&Inet) -> Option<c_int>,
) -> Option<PortId> {
    served(handles).find_map(|(id, slot)| match &slot.as_ref()?.driver {
        Driver::Inet(inet) if select(inet) == Some(fd) => Some(id),
        _ => None,
    })
}

/// Read the pipe `fd` of a spawn port of `handles`, and return the port
/// and the event of the read, or `None` when the read gives nothing to
/// deliver. The read adds the bytes to the input count of the port. The
/// lock of the port stays held (see [`Read`]).
///
/// # Examples
///
/// ```rust,ignore
/// let (id, event) = read(&handles, fd, Some(port)).unwrap();
/// ```
///
/// Only the port `reader`, which the poll waits to read `fd` for, reads
/// it: two fd ports can read the same fd, and the last `driver_select`
/// takes the fd from the first port (`erl_check_io.c:1080-1085`, `1711-1719`).
pub(super) fn read(
    handles: &[(PortId, PortCell)],
    fd: c_int,
    reader: Option<PortId>,
) -> Option<Read<'_>> {
    served(handles)
        .filter(|&(id, _)| Some(id) == reader)
        .find_map(|(id, mut slot)| {
            let port = slot.as_mut()?;
            let owner = port.owner;
            let event = match &mut port.driver {
                Driver::Spawn(spawn)
                    if (spawn.input == Input::Open && spawn.program.output_fd() == Some(fd))
                        || (spawn.input == Input::Status
                            && spawn.program.child().map(Child::status_fd) == Some(fd)) =>
                {
                    read_spawn(spawn, &mut port.input)
                }
                _ => return None,
            };
            Some(event.map(|event| (id, owner, event, slot)))
        })
        .flatten()
}

/// A read of a spawn port: the port, its owner, the event, and the still
/// held lock of the port, so that the data goes out before a close on
/// another scheduler sends the exit signals of the port, as the driver
/// sends its data in the locked `ready_input` (`sys_drivers.c:1336-1348`).
pub(super) type Read<'a> = (PortId, ProcessId, PortEvent, PortGuard<'a>);

/// Read the pipe of `spawn` that is ready, and add the bytes of the data
/// to `input`.
fn read_spawn(spawn: &mut Spawn, input: &mut u64) -> Option<PortEvent> {
    if spawn.input == Input::Status {
        let child = spawn.program.child()?;
        let status = child.read_status();
        return Some(PortEvent::ExitStatus(status, spawn.end_of_file()));
    }
    let bytes = match spawn.program.read_output() {
        Ok(bytes) => bytes,
        // A read that fails with an errno other than `EINTR` and `EAGAIN`
        // stops the read select, and the port stays open
        // (`sys_drivers.c:1339-1342`, `1211-1214`).
        Err(errno) => {
            if errno != libc::EINTR && errno != libc::EAGAIN {
                spawn.input = Input::Ended;
            }
            return None;
        }
    };
    if bytes.is_empty() && spawn.exit_status {
        spawn.input = Input::Status;
        return None;
    } else if bytes.is_empty() {
        return Some(PortEvent::EndOfFile(spawn.end_of_file()));
    }
    let packets = spawn.packets.split(bytes);
    *input += packets
        .iter()
        .map(|bytes| u64::try_from(bytes.len()).expect("a usize fits in u64"))
        .sum::<u64>();
    let messages = spawn.messages(packets);
    (!messages.is_empty()).then_some(PortEvent::Data(messages))
}

#[cfg(test)]
#[path = "../../tests/runtime/port.rs"]
mod tests;
