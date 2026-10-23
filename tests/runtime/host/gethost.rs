use super::*;
use std::{os::unix::net::UnixStream, sync::Condvar, thread::JoinHandle, time::Duration};

#[test]
fn maps_the_netdb_errors_as_map_netdb_error() {
    // inet_gethost.c:2008-2037; the resolver of the host gives these
    // values only for a failing name server.
    let codes = [HOST_NOT_FOUND, TRY_AGAIN, NO_RECOVERY, NO_DATA, 99].map(netdb_code);
    assert_eq!(
        codes,
        [
            Code::HostNotFound,
            Code::TryAgain,
            Code::NoRecovery,
            Code::NoData,
            Code::Internal
        ]
    );
}

#[test]
fn maps_the_address_info_errors_as_map_netdb_error_ai() {
    // inet_gethost.c:2039-2070.
    let codes = [
        libc::EAI_AGAIN,
        libc::EAI_FAIL,
        libc::EAI_NONAME,
        EAI_NODATA,
        libc::EAI_MEMORY,
    ]
    .map(address_info_code);
    assert_eq!(
        codes,
        [
            Code::TryAgain,
            Code::HostNotFound,
            Code::HostNotFound,
            Code::HostNotFound,
            Code::Internal
        ]
    );
}

#[test]
fn gives_the_error_strings_of_errcode_to_string() {
    // inet_gethost.c:2074-2097.
    let replies = [
        Code::HostNotFound,
        Code::TryAgain,
        Code::NoRecovery,
        Code::NoData,
        Code::Internal,
    ]
    .map(error_reply);
    assert_eq!(
        replies,
        [
            b"\0notfound\0".to_vec(),
            b"\0try_again\0".to_vec(),
            b"\0no_recovery\0".to_vec(),
            b"\0no_data\0".to_vec(),
            b"\0netdb_internal\0".to_vec()
        ]
    );
}

#[test]
fn stops_at_a_failed_read_or_write() {
    // A read error is a fatal error of the main loop
    // (inet_gethost.c:1330-1347); a failed write ends the worker
    // (inet_gethost.c:1989-1990).
    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("read"))
        }
    }
    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("write"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(serve(&[], Failing, &mut Vec::new()), 1);
    // The input stays open, so the main loop waits for the reply.
    let (mut erlang, input) = UnixStream::pair().expect("pair");
    erlang
        .write_all(b"\0\0\0\x0a\0\0\0\x01\x02\x01\x7f\0\0\x01")
        .expect("write");
    assert_eq!(serve(&[], input, &mut Failing), 1);
}

#[test]
fn reads_again_after_an_interrupted_read() {
    // read_exact retries EINTR (inet_gethost.c:2504-2506).
    struct Interrupted(bool);
    impl Read for Interrupted {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            if std::mem::replace(&mut self.0, true) {
                Ok(0)
            } else {
                Err(std::io::ErrorKind::Interrupted.into())
            }
        }
    }
    assert_eq!(serve(&[], Interrupted(false), &mut Vec::new()), 0);
}

/// A pool with a fake answer: a name that starts with `slow` waits for
/// `open`, and the reply is the name.
struct Fake {
    erlang: UnixStream,
    replies: UnixStream,
    gate: Arc<(Mutex<bool>, Condvar)>,
    status: JoinHandle<i32>,
}

impl Fake {
    fn start(args: &[&str]) -> Self {
        let (erlang, input) = UnixStream::pair().expect("pair");
        let (replies, mut output) = UnixStream::pair().expect("pair");
        replies
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("timeout");
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let shared = gate.clone();
        let answer: Answer = Arc::new(move |query| {
            let Query::Name(_, name) = query else {
                return Vec::new();
            };
            if name.to_bytes().starts_with(b"slow") {
                let (open, signal) = &*shared;
                let mut open = open.lock().expect("lock");
                while !*open {
                    open = signal.wait(open).expect("wait");
                }
            }
            name.to_bytes().to_vec()
        });
        let args = args
            .iter()
            .map(|arg| arg.as_bytes().to_vec())
            .collect::<Vec<_>>();
        let status = thread::spawn(move || pool(&args, input, &mut output, answer));
        Self {
            erlang,
            replies,
            gate,
            status,
        }
    }

    fn lookup(&mut self, serial: u32, name: &str) {
        let mut packet = u32::try_from(name.len() + 7)
            .expect("size")
            .to_be_bytes()
            .to_vec();
        packet.extend_from_slice(&serial.to_be_bytes());
        packet.extend_from_slice(&[OP_GETHOSTBYNAME, PROTO_IPV4]);
        packet.extend_from_slice(name.as_bytes());
        packet.push(0);
        self.erlang.write_all(&packet).expect("write");
    }

    fn cancel(&mut self, serial: u32) {
        let mut packet = vec![0, 0, 0, 5];
        packet.extend_from_slice(&serial.to_be_bytes());
        packet.push(OP_CANCEL_REQUEST);
        self.erlang.write_all(&packet).expect("write");
    }

    fn open(&self) {
        let (open, signal) = &*self.gate;
        *open.lock().expect("lock") = true;
        signal.notify_all();
    }

    fn reply(&mut self) -> (u32, String) {
        let mut size = [0; 4];
        self.replies.read_exact(&mut size).expect("size");
        let mut reply = vec![0; u32::from_be_bytes(size) as usize];
        self.replies.read_exact(&mut reply).expect("reply");
        let serial = u32::from_be_bytes(reply[..4].try_into().expect("serial"));
        (
            serial,
            String::from_utf8(reply[4..].to_vec()).expect("name"),
        )
    }

    fn end(self) -> i32 {
        drop(self.erlang);
        self.status.join().expect("join")
    }
}

#[test]
fn answers_a_parallel_request_before_a_slow_one() {
    // Two workers (inet_gethost.c:474-480); the greedy threshold is 1
    // (inet_gethost.c:493-498), and `b.y` has another domain, so a second
    // worker runs it (inet_gethost.c:982-1040, 1149-1180).
    let mut fake = Fake::start(&["2"]);
    fake.lookup(1, "slow.x");
    fake.lookup(2, "b.y");
    assert_eq!(fake.reply(), (2, "b.y".into()));
    fake.open();
    assert_eq!(fake.reply(), (1, "slow.x".into()));
    assert_eq!(fake.end(), 0);
}

#[test]
fn queues_a_request_of_the_same_domain_to_the_busy_worker() {
    // With 2 workers and one busy, `BEE_GREEDY` holds, and `b.x` goes to
    // the queue of the worker of `.x` (inet_gethost.c:982-1002); `c.y`
    // goes to a free worker. `-ng` turns the queue off
    // (inet_gethost.c:500-502).
    let mut fake = Fake::start(&["-d", "2"]);
    fake.lookup(1, "slow.x");
    fake.lookup(2, "b.x");
    fake.lookup(3, "c.y");
    assert_eq!(fake.reply(), (3, "c.y".into()));
    fake.open();
    assert_eq!(fake.reply(), (1, "slow.x".into()));
    assert_eq!(fake.reply(), (2, "b.x".into()));
    assert_eq!(fake.end(), 0);
    let mut fake = Fake::start(&["-ng", "2"]);
    fake.lookup(1, "slow.x");
    fake.lookup(2, "b.x");
    assert_eq!(fake.reply(), (2, "b.x".into()));
    fake.open();
    assert_eq!(fake.reply(), (1, "slow.x".into()));
    assert_eq!(fake.end(), 0);
}

#[test]
fn cancels_a_request_of_the_global_queue_or_a_worker_queue() {
    // One worker: `b.y` waits in the global queue and `c.x` in the queue
    // of the worker; a cancel removes them (inet_gethost.c:664-725), so
    // the next reply after `slow.x` is that of `d.z`.
    let mut fake = Fake::start(&[]);
    fake.lookup(1, "slow.x");
    fake.lookup(2, "b.y");
    fake.lookup(3, "c.x");
    fake.lookup(4, "e.y");
    fake.cancel(2);
    fake.cancel(3);
    fake.cancel(9);
    fake.open();
    assert_eq!(fake.reply(), (1, "slow.x".into()));
    assert_eq!(fake.reply(), (4, "e.y".into()));
    fake.lookup(5, "d.z");
    assert_eq!(fake.reply(), (5, "d.z".into()));
    assert_eq!(fake.end(), 0);
}

#[test]
fn cancels_a_running_request() {
    // One worker: a cancel of the running `slow.x` stops the worker
    // (inet_gethost.c:884-887), so a new worker runs `b.y` from the global
    // queue, and the reply of `slow.x` never comes.
    let mut fake = Fake::start(&[]);
    fake.lookup(1, "slow.x");
    fake.lookup(2, "b.y");
    fake.cancel(1);
    assert_eq!(fake.reply(), (2, "b.y".into()));
    fake.open();
    fake.lookup(3, "c.z");
    assert_eq!(fake.reply(), (3, "c.z".into()));
    assert_eq!(fake.end(), 0);
}

#[test]
fn restarts_a_cancelled_worker_with_a_queue() {
    // One worker: `b.x` waits in the queue of the worker of `slow.x`. A
    // cancel of `slow.x` restarts the worker and starts `b.x`
    // (inet_gethost.c:881-883, 1200-1250).
    let mut fake = Fake::start(&[]);
    fake.lookup(1, "slow.x");
    fake.lookup(2, "b.x");
    fake.lookup(3, "c.x");
    fake.cancel(1);
    assert_eq!(fake.reply(), (2, "b.x".into()));
    assert_eq!(fake.reply(), (3, "c.x".into()));
    fake.open();
    assert_eq!(fake.end(), 0);
}

#[test]
fn reads_the_arguments_as_main() {
    // inet_gethost.c:449-502: the worker count, `-g`, `-ng`, and `-d`; an
    // argument that `atoi` reads as 0 changes nothing.
    let options = |args: &[&str]| {
        options(
            &args
                .iter()
                .map(|arg| arg.as_bytes().to_vec())
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(options(&[]), Some((1, 1)));
    assert_eq!(options(&["4"]), Some((4, 3)));
    assert_eq!(options(&[" 8x", "-d", "abc"]), Some((8, 6)));
    assert_eq!(options(&["4", "-g", "2"]), Some((4, 2)));
    assert_eq!(options(&["4", "-g", "0"]), Some((4, 3)));
    assert_eq!(options(&["4", "-g"]), Some((4, 3)));
    assert_eq!(options(&["-ng", "4"]), Some((4, 5)));
    // A count below 1 is a fatal `malloc` failure (inet_gethost.c:1049-1060).
    assert_eq!(options(&["-3"]), None);
    assert_eq!(
        serve(&[b"-3".to_vec()], std::io::empty(), &mut Vec::new()),
        1
    );
    assert_eq!(atoi(b"+12"), 12);
    assert_eq!(atoi(b"4294967297"), 1);
    assert_eq!(atoi(b"99999999999999999999"), -1);
    assert_eq!(atoi(b"-99999999999999999999"), 0);
}

#[test]
fn keeps_the_domain_of_a_request() {
    // domaincopy (inet_gethost.c:1504-1527).
    let domain = |op, rest: &[u8]| parse(op, rest).map(|(_, domain)| domain);
    assert_eq!(
        domain(OP_GETHOSTBYNAME, b"\x01a.b.c\0"),
        Some(b"\x01\x01.b.c".to_vec())
    );
    assert_eq!(
        domain(OP_GETHOSTBYNAME, b"\x02ab\0"),
        Some(b"\x01\x02".to_vec())
    );
    let long = [b"\x01a.".as_slice(), &[b'b'; 300], b"\0"].concat();
    assert_eq!(domain(OP_GETHOSTBYNAME, &long).map(|d| d.len()), Some(257));
    assert_eq!(
        domain(OP_GETHOSTBYADDR, b"\x01\x7f\0\0\x01"),
        Some(b"\x02\x01\x7f\0\0".to_vec())
    );
    let six = [[PROTO_IPV6].as_slice(), &[7; 16]].concat();
    assert_eq!(domain(OP_GETHOSTBYADDR, &six).map(|d| d.len()), Some(17));
    assert_eq!(domain(OP_GETHOSTBYADDR, b"\x03\x7f\0\0\x01"), None);
}
