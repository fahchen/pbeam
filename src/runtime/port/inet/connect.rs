//! `INET_REQ_CONNECT` of an IPv4 `tcp_inet` socket: the connect without a
//! wait, and its end when the socket is ready for write or the timer ends
//! (inet_drv.c:12064-12141, 12569-12580, 14242-14288).

use super::{
    Control, Inet, State, errno_reply, error_reply,
    stream::{Event, Receiver},
};
use crate::error::BeamError;

impl Inet {
    /// Run `INET_REQ_CONNECT` of `caller` with `[T3, T2, T1, T0, 1, P1, P0,
    /// A, B, C, D]` (inet_drv.c:12064-12141): a socket that is not open
    /// gives `exbadport`, a connected socket `eisconn`, and a socket that
    /// connects or a request of fewer than 6 bytes `einval`. An error of the
    /// host other than `EINPROGRESS` gives its error reply. Else the socket
    /// connects and joins the write set of the poll (`sock_select(desc,
    /// FD_CONNECT, 1)`), and the scheduler starts the timer and replies
    /// with the async reference (see [`Control::Connect`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let request = [0, 0, 3, 232, 1, 0, 80, 127, 0, 0, 1];
    /// assert_eq!(port.connect(&request, caller)?, Control::Connect(1000));
    /// assert_eq!(port.connect(&request, caller)?, Control::Reply(b"\0einval".to_vec()));
    /// ```
    ///
    /// # Errors
    ///
    /// An address family other than `inet` that PBeam does not run.
    pub(super) fn connect(
        &mut self,
        request: &[u8],
        caller: crate::term::ProcessId,
    ) -> Result<Control, BeamError> {
        let reply = |bytes| Ok(Control::Reply(bytes));
        match self.state {
            State::Closed => return reply(error_reply("exbadport")),
            State::Connected => return reply(error_reply("eisconn")),
            State::Connecting => return reply(error_reply("einval")),
            State::Open | State::Listening => {}
        }
        let Some((&[t3, t2, t1, t0], address)) = request.split_first_chunk() else {
            return reply(error_reply("einval"));
        };
        if address.len() < 2 {
            return reply(error_reply("einval"));
        }
        let timeout = u32::from_be_bytes([t3, t2, t1, t0]);
        let remote = match self.address(address, "connect")? {
            Ok(remote) => remote,
            Err(bytes) => return reply(bytes),
        };
        let socket = self.socket.as_ref().expect("an open port has a socket");
        // A connect that the host ends at once waits for the write select
        // too: see row "tcp_inet ports" of `docs/erts-differences.md`.
        let started = socket.connect(&remote.into()).or_else(|failure| {
            (failure.raw_os_error() == Some(libc::EINPROGRESS))
                .then_some(())
                .ok_or(failure)
        });
        if let Err(failure) = started {
            return reply(errno_reply(&failure));
        }
        self.state = State::Connecting;
        let id = self.next_async_id();
        self.stream.connector = Some(Receiver {
            id,
            caller,
            timer: None,
        });
        Ok(Control::Connect(timeout))
    }

    /// End the connect of a socket that the poll found ready for write, as
    /// `tcp_inet_output` does in `INET_STATE_CONNECTING`
    /// (inet_drv.c:14242-14288): the error of `SO_ERROR` gives `{error,
    /// Reason}` and the socket is open again, else the socket is connected
    /// and the caller gets `ok`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let [Event::Connected { result, .. }] = &port.connect_ready()[..] else { panic!() };
    /// assert_eq!(result, &Err("econnrefused".to_owned()));
    /// ```
    pub(super) fn connect_ready(&mut self) -> Vec<Event> {
        let connector = self.stream.connector.take().expect("the socket connects");
        let socket = self.socket.as_ref().expect("a socket that connects");
        let result = match socket.take_error() {
            Ok(None) => {
                self.state = State::Connected;
                Ok(())
            }
            Ok(Some(failure)) | Err(failure) => {
                self.state = State::Open;
                Err(super::errno_name(&failure))
            }
        };
        vec![Event::Connected { connector, result }]
    }

    /// End the connect of the timer, as `tcp_inet_timeout` does in
    /// `INET_STATE_CONNECTING` (inet_drv.c:12569-12580): the socket closes
    /// (`tcp_desc_close`), and the caller gets `{error, timeout}`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let events = port.connect_timeout();
    /// assert!(matches!(events.last(), Some(Event::Connected { result: Err(_), .. })));
    /// ```
    pub(in crate::runtime) fn connect_timeout(&mut self) -> Vec<Event> {
        let connector = self.stream.connector.take().expect("the socket connects");
        let mut events = self.close_socket();
        events.push(Event::Connected {
            connector,
            result: Err("timeout".to_owned()),
        });
        events
    }
}
