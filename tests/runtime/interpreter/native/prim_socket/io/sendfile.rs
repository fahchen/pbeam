use super::*;

/// Run [`send_loop`] with the results `results` of the host calls, and
/// return its end, the counters, and the `(offset, chunk)` of each call.
fn run(count: u64, results: &[(u64, Option<i32>)]) -> (Sent, SendfileCounters, Vec<(i64, usize)>) {
    let mut counters = SendfileCounters::default();
    let mut calls = Vec::new();
    let mut results = results.iter();
    let sent = send_loop(&mut counters, 5, count, |offset, chunk| {
        calls.push((offset, chunk));
        *results.next().expect("a result for each call")
    });
    (sent, counters, calls)
}

#[test]
fn sends_again_after_eintr() {
    // `EINTR` continues the loop (unix_socket_syncio.c:9333-9334); a
    // later call that sends the rest ends it.
    let (sent, counters, calls) = run(10, &[(4, Some(libc::EINTR)), (6, None)]);
    assert_eq!(sent, Sent::Done(10));
    assert_eq!(calls, vec![(5, 10), (9, 6)]);
    assert_eq!((counters.calls, counters.pkg, counters.byte), (2, 2, 10));
    assert_eq!((counters.pkg_max, counters.max), (10, 10));
}

#[test]
fn sends_the_whole_file_in_chunks_until_its_end() {
    // A count of 0 sends chunks of 0x20000000 bytes until a call sends no
    // byte (unix_socket_syncio.c:9172-9179, 9339-9342).
    let (sent, counters, calls) = run(0, &[(7, None), (0, None)]);
    assert_eq!(sent, Sent::Done(7));
    assert_eq!(calls, vec![(5, 0x2000_0000), (12, 0x2000_0000)]);
    assert_eq!((counters.calls, counters.pkg), (2, 1));
}

#[test]
fn gives_the_select_and_the_errno() {
    let (sent, counters, _) = run(0, &[(3, Some(libc::EAGAIN))]);
    assert_eq!(sent, Sent::Select(3));
    assert_eq!(counters.byte, 3);
    let (sent, counters, _) = run(0, &[(0, Some(libc::EPIPE))]);
    assert_eq!(sent, Sent::Failed(libc::EPIPE));
    assert_eq!((counters.calls, counters.pkg), (1, 0));
}
