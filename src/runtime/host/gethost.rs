//! The `inet_gethost` port program of ERTS
//! (`erts/etc/common/inet_gethost.c`), which `inet_gethost_native` opens
//! with `open_port({spawn_executable, "<bindir>/inet_gethost"}, ...)`
//! (inet_gethost_native.erl:412-421). PBeam runs it in a thread of the
//! node (see [`super::spawn::serve`]), not as an external program, so a
//! bundle needs no `inet_gethost` file.
//!
//! The thread reads the requests from the input pipe and writes the
//! replies to the output pipe, each with a 4-byte big-endian size, as the
//! program does with its fds 0 and 1 (`inet_gethost.c:139-146`). It does
//! each lookup with the host call that the worker of the program uses on
//! a host with `getaddrinfo` and `getnameinfo` (`inet_gethost.c:1707-1992`).
//! It runs the worker pool and the request queues of the program with
//! one thread for each worker (see [`pool`]).

use std::{
    collections::VecDeque,
    ffi::{CStr, CString, c_char, c_int},
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        mpsc::{self, Sender},
    },
    thread,
};

/// The size of the name buffer of `getnameinfo` (`NI_MAXHOST`).
#[expect(
    clippy::as_conversions,
    reason = "NI_MAXHOST is a small libc constant, and TryFrom is not const"
)]
const HOST_NAME_LEN: usize = libc::NI_MAXHOST as usize;

/// `OP_GETHOSTBYNAME` (`inet_gethost.c:154`).
const OP_GETHOSTBYNAME: u8 = 1;
/// `OP_GETHOSTBYADDR` (`inet_gethost.c:155`).
const OP_GETHOSTBYADDR: u8 = 2;
/// `OP_CANCEL_REQUEST` (`inet_gethost.c:156`).
const OP_CANCEL_REQUEST: u8 = 3;
/// `OP_CONTROL` (`inet_gethost.c:157`).
const OP_CONTROL: u8 = 4;
/// `PROTO_IPV4` (`inet_gethost.c:162`).
const PROTO_IPV4: u8 = 1;
/// `PROTO_IPV6` (`inet_gethost.c:163`).
const PROTO_IPV6: u8 = 2;
/// `INVALID_SERIAL` (`inet_gethost.c:149`), the serial of `OP_CONTROL`.
const INVALID_SERIAL: u32 = u32::MAX;

/// The `h_errno` values of `<netdb.h>`, the same on Linux and macOS.
const HOST_NOT_FOUND: c_int = 1;
/// `TRY_AGAIN` of `<netdb.h>`.
const TRY_AGAIN: c_int = 2;
/// `NO_RECOVERY` of `<netdb.h>`.
const NO_RECOVERY: c_int = 3;
/// `NO_DATA` (and `NO_ADDRESS`) of `<netdb.h>`.
const NO_DATA: c_int = 4;

/// `EAI_NODATA` of glibc, which ERTS sees because it builds with
/// `-D_GNU_SOURCE` on Linux (erts/configure.ac:543).
#[cfg(target_os = "linux")]
const EAI_NODATA: c_int = -5;
/// `EAI_NODATA` of macOS.
#[cfg(not(target_os = "linux"))]
const EAI_NODATA: c_int = libc::EAI_NODATA;

/// The lock of `gethostbyname` and `gethostbyaddr`, which return a static
/// `hostent` and set a `h_errno` that is not thread-local on macOS. The
/// program calls them in a worker process of its own.
static NETDB: Mutex<()> = Mutex::new(());

unsafe extern "C" {
    fn gethostbyname(name: *const c_char) -> *mut libc::hostent;
    fn gethostbyaddr(
        addr: *const libc::c_void,
        len: libc::socklen_t,
        kind: c_int,
    ) -> *mut libc::hostent;
    #[cfg(target_os = "linux")]
    fn __h_errno_location() -> *mut c_int;
    #[cfg(not(target_os = "linux"))]
    static h_errno: c_int;
}

/// The answer of a worker to a query: the reply after the serial.
pub(crate) type Answer = Arc<dyn Fn(&Query) -> Vec<u8> + Send + Sync>;

/// Serve the requests of `input` with the arguments `args` of the program
/// and write the replies to `output`, with the host calls of [`answer`].
/// See [`pool`].
///
/// # Examples
///
/// ```rust,ignore
/// let status = serve(&args, input, &mut output);
/// ```
pub(crate) fn serve(
    args: &[Vec<u8>],
    input: impl Read + Send + 'static,
    output: &mut impl Write,
) -> i32 {
    pool(args, input, output, Arc::new(answer))
}

/// Serve the requests of `input` and write the replies to `output` until
/// the end of file of `input`, as `main` and the main loop of the program
/// do (`inet_gethost.c:449-538`, `727-1043`): a thread reads the requests,
/// and a pool of worker threads runs `answer`. Return the exit status of
/// the program: 0 at the end of file, and 1 for a fatal error (`fatal`,
/// `inet_gethost.c:2643-2667`): a worker count below 1, which `malloc`
/// cannot allocate (`inet_gethost.c:1049-1060`, `2669-2677`), a request
/// with a size below 5 or a part of a request (`inet_gethost.c:1325-1351`),
/// `OP_CONTROL` with a serial that is not `INVALID_SERIAL`
/// (`inet_gethost.c:896-901`), and a failed write of a reply
/// (`inet_gethost.c:1477-1492`). The program writes the message of a fatal
/// error to its fd 2; the thread writes nothing.
///
/// # Examples
///
/// ```rust,ignore
/// let status = pool(&[b"4".to_vec()], input, &mut output, Arc::new(answer));
/// ```
pub(crate) fn pool(
    args: &[Vec<u8>],
    mut input: impl Read + Send + 'static,
    output: &mut impl Write,
    answer: Answer,
) -> i32 {
    let Some((max, threshold)) = options(args) else {
        return 1;
    };
    let (events, receiver) = mpsc::channel();
    let requests = events.clone();
    thread::spawn(move || {
        loop {
            let event = read_request(&mut input);
            let end = !matches!(event, Event::Request(_));
            if requests.send(event).is_err() || end {
                break;
            }
        }
    });
    let mut pool = Pool {
        events,
        answer,
        max,
        threshold,
        next_id: 0,
        free: Vec::new(),
        busy: Vec::new(),
        queue: VecDeque::new(),
    };
    loop {
        let request = match receiver.recv().expect("the pool keeps a sender") {
            Event::Request(request) => request,
            Event::End => return 0,
            Event::Fatal => return 1,
            Event::Reply(id, packet) => {
                if let Some(index) = pool.busy.iter().position(|worker| worker.id == id) {
                    if output.write_all(&packet).is_err() {
                        return 1;
                    }
                    pool.replied(index);
                }
                pool.check_queue();
                continue;
            }
        };
        let serial = u32::from_be_bytes(request[..4].try_into().expect("4 bytes"));
        match request[4] {
            OP_CANCEL_REQUEST => pool.cancel(serial),
            // `SETOPT_DEBUG_LEVEL` and other controls change no reply.
            OP_CONTROL if serial == INVALID_SERIAL => {}
            OP_CONTROL => return 1,
            op @ (OP_GETHOSTBYNAME | OP_GETHOSTBYADDR) => {
                if let Some((query, domain)) = parse(op, &request[5..]) {
                    pool.start(Job { serial, query }, domain);
                }
            }
            // "Unknown operation requested from erlang", discarded.
            _ => {}
        }
    }
}

/// Return the worker count and the greedy threshold of the arguments, as
/// `main` reads them (`inet_gethost.c:449-502`), or `None` for a worker
/// count below 1, which `init_workers` cannot allocate
/// (`inet_gethost.c:1049-1060`). `-d` changes no reply. An argument that
/// `atoi` reads as 0 is a usage message and changes nothing.
fn options(args: &[Vec<u8>]) -> Option<(usize, i32)> {
    let mut workers = 1;
    let mut threshold = 0;
    let mut greedy = true;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_slice() {
            b"-d" => {}
            b"-g" if rest.len() > 0 => match rest.next().map_or(0, |next| atoi(next)) {
                0 => {}
                value => threshold = value,
            },
            b"-ng" => greedy = false,
            _ => match atoi(arg) {
                0 => {}
                value => workers = value,
            },
        }
    }
    if threshold == 0 {
        threshold = match workers.wrapping_mul(3) / 4 {
            0 => workers,
            value => value,
        };
    }
    if !greedy {
        threshold = workers.wrapping_add(1);
    }
    Some((
        usize::try_from(workers).ok().filter(|&max| max > 0)?,
        threshold,
    ))
}

/// Read a number as glibc `atoi` does: `(int) strtol(text, NULL, 10)`,
/// with leading white space, a sign, the digits, and a clamp to `long`.
fn atoi(text: &[u8]) -> i32 {
    // The C `isspace` set, with the `\v` that `trim_ascii_start` does not skip.
    let start = text
        .iter()
        .position(|byte| !b" \t\n\x0b\x0c\r".contains(byte))
        .unwrap_or(text.len());
    let text = &text[start..];
    let (negative, digits) = match text.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, text),
    };
    let magnitude = digits
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .fold(0i128, |value, &digit| {
            (value * 10 + i128::from(digit - b'0')).min(i128::from(i64::MAX) + 1)
        });
    let value = if negative { -magnitude } else { magnitude };
    // `strtol` clamps to `long`; the cast to `int` keeps the low 32 bits.
    let long = i64::try_from(value).unwrap_or(i64::MAX);
    let [b0, b1, b2, b3, ..] = long.to_le_bytes();
    i32::from_le_bytes([b0, b1, b2, b3])
}

/// An event of the main loop.
enum Event {
    /// A request of the input.
    Request(Vec<u8>),
    /// The end of file of the input.
    End,
    /// A failed read or a short request.
    Fatal,
    /// The packet of the reply of the worker with the given id.
    Reply(u64, Vec<u8>),
}

/// Read a request as `read_request` does (`inet_gethost.c:1325-1351`).
fn read_request(input: &mut impl Read) -> Event {
    let mut size = [0; 4];
    match read_exact(input, &mut size) {
        Some(0) => return Event::End,
        Some(4) => {}
        _ => return Event::Fatal,
    }
    let mut request =
        vec![0; usize::try_from(u32::from_be_bytes(size)).expect("a u32 fits in usize")];
    if read_exact(input, &mut request) != Some(request.len()) || request.len() < 5 {
        return Event::Fatal;
    }
    Event::Request(request)
}

/// A lookup request: the serial and the query.
struct Job {
    serial: u32,
    query: Query,
}

/// A worker thread of the pool, as the `Worker` of the program
/// (`inet_gethost.c:276-292`).
struct Worker {
    /// The id of the thread, which its replies carry.
    id: u64,
    /// The channel of the jobs of the thread.
    jobs: Sender<Job>,
    /// The serial of the running job.
    serial: u32,
    /// The domain of the last job that the main loop gave to the worker,
    /// which a job of the global queue does not change
    /// (`inet_gethost.c:643-649`, `1037-1040`).
    domain: Option<Vec<u8>>,
    /// The jobs of the same domain that wait for the worker.
    queue: VecDeque<Job>,
}

/// The worker pool and the global request queue of the main loop.
struct Pool {
    events: Sender<Event>,
    answer: Answer,
    max: usize,
    threshold: i32,
    next_id: u64,
    free: Vec<Worker>,
    busy: Vec<Worker>,
    queue: VecDeque<Job>,
}

impl Pool {
    /// Start a worker thread with the given queue, as `create_worker` does.
    fn create(&mut self, domain: Option<Vec<u8>>, queue: VecDeque<Job>) -> Worker {
        let (jobs, receiver) = mpsc::channel::<Job>();
        let (id, events, answer) = (self.next_id, self.events.clone(), Arc::clone(&self.answer));
        self.next_id += 1;
        thread::spawn(move || {
            while let Ok(job) = receiver.recv() {
                let reply = answer(&job.query);
                let mut packet = Vec::with_capacity(8 + reply.len());
                packet
                    .extend_from_slice(&u32::try_from(4 + reply.len()).unwrap_or(0).to_be_bytes());
                packet.extend_from_slice(&job.serial.to_be_bytes());
                packet.extend_from_slice(&reply);
                // The thread ends when the pool drops `jobs`, so a reply
                // after the end of the main loop goes nowhere.
                let _ = events.send(Event::Reply(id, packet));
            }
        });
        Worker {
            id,
            jobs,
            serial: 0,
            domain,
            queue,
        }
    }

    /// Return the index of a busy worker for a job, as `pick_worker` does
    /// (`inet_gethost.c:1149-1180`): a free worker, or a new worker when
    /// fewer than the maximum are busy.
    fn pick(&mut self) -> Option<usize> {
        let worker = match self.free.pop() {
            Some(worker) => worker,
            None if self.busy.len() == self.max => return None,
            None => self.create(None, VecDeque::new()),
        };
        self.busy.push(worker);
        Some(self.busy.len() - 1)
    }

    /// Give `job` to the busy worker at `index`.
    fn send(&mut self, index: usize, job: Job) {
        let worker = &mut self.busy[index];
        worker.serial = job.serial;
        // The thread ends only after the pool drops `jobs`.
        let _ = worker.jobs.send(job);
    }

    /// Start a job of the domain `domain`, as the main loop does
    /// (`inet_gethost.c:982-1040`): when `BEE_GREEDY`, queue it to the busy
    /// worker of the same domain with the shortest queue; else give it to
    /// a worker, or queue it.
    fn start(&mut self, job: Job, domain: Vec<u8>) {
        if i32::try_from(self.busy.len()).unwrap_or(i32::MAX) >= self.threshold {
            let mut found: Option<usize> = None;
            for (index, worker) in self.busy.iter().enumerate() {
                if worker.domain.as_ref() == Some(&domain)
                    && found.is_none_or(|best| worker.queue.len() < self.busy[best].queue.len())
                {
                    found = Some(index);
                }
            }
            if let Some(index) = found {
                self.busy[index].queue.push_back(job);
                return;
            }
        }
        match self.pick() {
            Some(index) => {
                self.send(index, job);
                self.busy[index].domain = Some(domain);
            }
            None => self.queue.push_back(job),
        }
    }

    /// Handle the reply of the busy worker at `index`, as `handle_io_busy`
    /// does (`inet_gethost.c:552-585`).
    fn replied(&mut self, index: usize) {
        if let Some(job) = self.busy[index].queue.pop_front() {
            self.send(index, job);
        } else {
            let worker = self.busy.swap_remove(index);
            self.free.push(worker);
        }
    }

    /// Give the jobs of the global queue to workers, as `check_que` does
    /// (`inet_gethost.c:620-662`).
    fn check_queue(&mut self) {
        while !self.queue.is_empty() {
            let Some(index) = self.pick() else { break };
            let job = self.queue.pop_front().expect("a job");
            self.send(index, job);
        }
    }

    /// Cancel the request `serial`, as the main loop does
    /// (`inet_gethost.c:876-891`, `664-725`): remove it from the global
    /// queue or a worker queue, or stop the worker that runs it. The
    /// program kills a stopped worker process; the thread of a stopped
    /// worker ends its lookup, and the pool drops its reply.
    fn cancel(&mut self, serial: u32) {
        if let Some(position) = self.queue.iter().position(|job| job.serial == serial) {
            self.queue.remove(position);
            return;
        }
        for worker in &mut self.busy {
            if let Some(position) = worker.queue.iter().position(|job| job.serial == serial) {
                worker.queue.remove(position);
                return;
            }
        }
        let Some(index) = self.busy.iter().position(|worker| worker.serial == serial) else {
            return;
        };
        if self.busy[index].queue.is_empty() {
            self.busy.swap_remove(index);
            self.check_queue();
        } else {
            let old = &mut self.busy[index];
            let (domain, mut queue) = (old.domain.take(), std::mem::take(&mut old.queue));
            let job = queue.pop_front().expect("a job");
            self.busy[index] = self.create(domain, queue);
            self.send(index, job);
        }
    }
}

/// A validated lookup query.
pub(crate) enum Query {
    /// `OP_GETHOSTBYNAME` with the protocol and the name.
    Name(u8, CString),
    /// `OP_GETHOSTBYADDR` of `PROTO_IPV4`.
    Address4([u8; 4]),
    /// `OP_GETHOSTBYADDR` of `PROTO_IPV6`.
    Address6([u8; 16]),
}

/// Return the query and the domain of the operation `op` with the protocol
/// and the data of `rest`, or `None` for a request that the main loop
/// discards: a protocol that is not `PROTO_IPV4` or `PROTO_IPV6`, a name
/// without a byte 0, and an address of the wrong size
/// (`inet_gethost.c:963-979`, `1548-1585`). The domain is the operation,
/// the protocol, and the name from its first `.` with at most 255 bytes, or
/// the address without its last byte (`domaincopy`,
/// `inet_gethost.c:1504-1527`).
fn parse(op: u8, rest: &[u8]) -> Option<(Query, Vec<u8>)> {
    let (&proto, data) = rest.split_first()?;
    let mut domain = vec![op, proto];
    let query = match (op, proto) {
        (OP_GETHOSTBYNAME, PROTO_IPV4 | PROTO_IPV6) => {
            let name = CStr::from_bytes_until_nul(data).ok()?;
            let bytes = name.to_bytes();
            let dot = bytes.iter().position(|&byte| byte == b'.');
            let suffix = &bytes[dot.unwrap_or(bytes.len())..];
            domain.extend_from_slice(&suffix[..suffix.len().min(255)]);
            Query::Name(proto, name.to_owned())
        }
        (OP_GETHOSTBYADDR, PROTO_IPV4) => {
            let address = <[u8; 4]>::try_from(data).ok()?;
            domain.extend_from_slice(&address[..3]);
            Query::Address4(address)
        }
        (OP_GETHOSTBYADDR, PROTO_IPV6) => {
            let address = <[u8; 16]>::try_from(data).ok()?;
            domain.extend_from_slice(&address[..15]);
            Query::Address6(address)
        }
        _ => return None,
    };
    Some((query, domain))
}

/// Return the reply of `query` with the host calls of the worker
/// (`inet_gethost.c:1791-1976`).
fn answer(query: &Query) -> Vec<u8> {
    let result = match query {
        Query::Name(PROTO_IPV4, name) => host_by_name(name),
        Query::Name(_, name) => address_info(name),
        Query::Address4(address) => host_by_address(*address),
        Query::Address6(address) => name_info(*address),
    };
    result.unwrap_or_else(error_reply)
}

/// Read bytes into `buffer` until it is full or `input` ends, and return
/// the number of bytes, or `None` for a failed read.
fn read_exact(input: &mut impl Read, buffer: &mut [u8]) -> Option<usize> {
    let mut done = 0;
    while done < buffer.len() {
        match input.read(&mut buffer[done..]) {
            Ok(0) => break,
            Ok(read) => done += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(done)
}

/// The reply of an error code: the unit 0 and the string of
/// `errcode_to_string` with a byte 0 (`inet_gethost.c:2074-2124`).
fn error_reply(code: Code) -> Vec<u8> {
    let text: &[u8] = match code {
        Code::HostNotFound => b"notfound",
        Code::TryAgain => b"try_again",
        Code::NoRecovery => b"no_recovery",
        Code::NoData => b"no_data",
        Code::Internal => b"netdb_internal",
    };
    [&[0], text, &[0]].concat()
}

/// The internal error codes of the program (`inet_gethost.c:251-256`).
/// `ERRCODE_NOTSUP` has no case: each supported operation has a lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Code {
    HostNotFound,
    TryAgain,
    NoRecovery,
    NoData,
    Internal,
}

/// Map a `h_errno` value, as `map_netdb_error` does
/// (`inet_gethost.c:2008-2037`).
fn netdb_code(value: c_int) -> Code {
    match value {
        HOST_NOT_FOUND => Code::HostNotFound,
        TRY_AGAIN => Code::TryAgain,
        NO_RECOVERY => Code::NoRecovery,
        NO_DATA => Code::NoData,
        _ => Code::Internal,
    }
}

/// Map a `getaddrinfo` or `getnameinfo` error, as `map_netdb_error_ai`
/// does (`inet_gethost.c:2039-2070`).
fn address_info_code(value: c_int) -> Code {
    match value {
        libc::EAI_AGAIN => Code::TryAgain,
        libc::EAI_FAIL | libc::EAI_NONAME | EAI_NODATA => Code::HostNotFound,
        _ => Code::Internal,
    }
}

/// The reply of addresses of `unit` bytes and names, as `build_reply` and
/// `build_reply_ai` write it (`inet_gethost.c:2129-2267`).
fn hosts_reply(unit: u8, addresses: &[Vec<u8>], names: &[Vec<u8>]) -> Vec<u8> {
    let mut reply = vec![unit];
    reply.extend_from_slice(&u32::try_from(addresses.len()).unwrap_or(0).to_be_bytes());
    for address in addresses {
        reply.extend_from_slice(address);
    }
    reply.extend_from_slice(&u32::try_from(names.len()).unwrap_or(0).to_be_bytes());
    for name in names {
        reply.extend_from_slice(name);
        reply.push(0);
    }
    reply
}

/// Read the current `h_errno`.
fn h_errno_value() -> c_int {
    #[cfg(target_os = "linux")]
    // SAFETY: glibc and musl return the address of the `h_errno` of the
    // thread, which is valid to read for the life of the thread.
    unsafe {
        *__h_errno_location()
    }
    #[cfg(not(target_os = "linux"))]
    // SAFETY: macOS has one global `int` `h_errno`, which is always
    // initialized. The caller holds `NETDB`, so no call of the network
    // database functions writes it during this read.
    unsafe {
        h_errno
    }
}

/// Run `gethostbyname(name)`, as the worker does for `PROTO_IPV4`
/// (`inet_gethost.c:1837-1847`).
fn host_by_name(name: &CStr) -> Result<Vec<u8>, Code> {
    let _lock = NETDB
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: `name` is a NUL-terminated string. The caller holds `NETDB`.
    let host = unsafe { gethostbyname(name.as_ptr()) };
    hostent_reply(host)
}

/// Run `gethostbyaddr(address, 4, AF_INET)`, as the worker does for
/// `PROTO_IPV4` (`inet_gethost.c:1929-1941`).
fn host_by_address(address: [u8; 4]) -> Result<Vec<u8>, Code> {
    let _lock = NETDB
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: `address` holds the 4 bytes that the call reads. The caller
    // holds `NETDB`.
    let host = unsafe { gethostbyaddr(address.as_ptr().cast(), 4, libc::AF_INET) };
    hostent_reply(host)
}

/// Return the reply of the static `hostent` of `host`, or the code of
/// `h_errno` when `host` is null. The caller holds `NETDB`.
fn hostent_reply(host: *mut libc::hostent) -> Result<Vec<u8>, Code> {
    if host.is_null() {
        return Err(netdb_code(h_errno_value()));
    }
    // SAFETY: a `hostent` that is not null has a NUL-terminated name, a
    // null-terminated alias list, and a null-terminated list of addresses
    // of `h_length` bytes (POSIX `<netdb.h>`). `NETDB` keeps it valid.
    unsafe {
        let host = &*host;
        let unit = u8::try_from(host.h_length).unwrap_or(0);
        let addresses = c_list(host.h_addr_list)
            .map(|address| std::slice::from_raw_parts(address.cast::<u8>(), unit.into()).to_vec())
            .collect::<Vec<_>>();
        let names = std::iter::once(host.h_name)
            .chain(c_list(host.h_aliases))
            .map(|name| CStr::from_ptr(name).to_bytes().to_vec())
            .collect::<Vec<_>>();
        Ok(hosts_reply(unit, &addresses, &names))
    }
}

/// Return the elements of the null-terminated pointer list `list`. POSIX
/// gives `h_aliases` as a list, so the null check of `build_reply`
/// (`inet_gethost.c:2145`) has no case here.
///
/// # Safety
///
/// `list` is a valid null-terminated list.
unsafe fn c_list(list: *mut *mut c_char) -> impl Iterator<Item = *mut c_char> {
    (0..)
        // SAFETY: the list ends at its first null element. The list of
        // macOS `gethostbyaddr` can be misaligned, so the read is
        // unaligned, as the C code of the program reads it.
        .map(move |index| unsafe { list.add(index).read_unaligned() })
        .take_while(|element| !element.is_null())
}

/// Run `getaddrinfo(name)` with `AI_CANONNAME`, `SOCK_STREAM`, and
/// `AF_INET6`, as the worker does for `PROTO_IPV6`
/// (`inet_gethost.c:1796-1809`, `1862-1866`).
fn address_info(name: &CStr) -> Result<Vec<u8>, Code> {
    // SAFETY: a zeroed `addrinfo` is a valid hint without flags.
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_flags = libc::AI_CANONNAME;
    hints.ai_socktype = libc::SOCK_STREAM;
    hints.ai_family = libc::AF_INET6;
    let mut list = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated, and `getaddrinfo` writes the list
    // to `list`, which `freeaddrinfo` frees below.
    let result = unsafe {
        libc::getaddrinfo(
            name.as_ptr(),
            std::ptr::null(),
            &raw const hints,
            &raw mut list,
        )
    };
    if result != 0 {
        return Err(address_info_code(result));
    }
    let mut addresses = Vec::new();
    let mut names = Vec::new();
    let mut entry = list;
    // SAFETY: each entry of the list is valid until `freeaddrinfo`. An
    // `ai_addr` of the family `AF_INET6` is a `sockaddr_in6`.
    unsafe {
        while let Some(info) = entry.as_ref() {
            if !info.ai_addr.is_null() && c_int::from((*info.ai_addr).sa_family) == libc::AF_INET6 {
                let socket = &*info.ai_addr.cast::<libc::sockaddr_in6>();
                addresses.push(socket.sin6_addr.s6_addr.to_vec());
            }
            if !info.ai_canonname.is_null() && info.ai_family == libc::AF_INET6 {
                names.push(CStr::from_ptr(info.ai_canonname).to_bytes().to_vec());
            }
            entry = info.ai_next;
        }
        libc::freeaddrinfo(list);
    }
    Ok(hosts_reply(16, &addresses, &names))
}

/// Run `getnameinfo` with `NI_NAMEREQD` for the IPv6 address `address`, as
/// the worker does for `PROTO_IPV6` (`inet_gethost.c:1877-1899`,
/// `1954-1964`): the reply has the address and the name.
fn name_info(address: [u8; 16]) -> Result<Vec<u8>, Code> {
    // SAFETY: a zeroed `sockaddr_in6` is valid.
    let mut socket: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
    #[cfg(not(target_os = "linux"))]
    {
        socket.sin6_len = u8::try_from(size_of::<libc::sockaddr_in6>()).unwrap_or(0);
    }
    socket.sin6_family = libc::sa_family_t::try_from(libc::AF_INET6).unwrap_or(0);
    socket.sin6_addr.s6_addr = address;
    let mut name = [0; HOST_NAME_LEN];
    // SAFETY: `socket` is a `sockaddr_in6` of the given size, and the call
    // writes at most `name.len()` bytes to `name`.
    let result = unsafe {
        libc::getnameinfo(
            (&raw const socket).cast(),
            libc::socklen_t::try_from(size_of::<libc::sockaddr_in6>()).unwrap_or(0),
            name.as_mut_ptr(),
            libc::socklen_t::try_from(name.len()).unwrap_or(0),
            std::ptr::null_mut(),
            0,
            libc::NI_NAMEREQD,
        )
    };
    if result != 0 {
        return Err(address_info_code(result));
    }
    // SAFETY: `getnameinfo` wrote a NUL-terminated name.
    let name = unsafe { CStr::from_ptr(name.as_ptr()) }.to_bytes().to_vec();
    Ok(hosts_reply(16, &[address.to_vec()], &[name]))
}

#[cfg(test)]
#[path = "../../../tests/runtime/host/gethost.rs"]
mod tests;
