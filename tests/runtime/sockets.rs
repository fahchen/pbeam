use super::Counters;

#[cfg(target_os = "linux")]
#[test]
fn counts_an_mptcp_socket_as_tcp() {
    // `esock_inc_socket` counts `IPPROTO_MPTCP` in `num_ptcp`
    // (prim_socket_nif.c:17157-17162).
    let mut counters = Counters::default();
    counters.count(libc::AF_INET, libc::SOCK_STREAM, libc::IPPROTO_MPTCP, 1);
    assert_eq!((counters.tcp, counters.sctp), (1, 0));
}

#[test]
fn counts_an_sctp_socket_in_no_protocol_counter() {
    // The pinned build has no `HAVE_SCTP` (prim_socket_nif.c:17165-17168).
    let mut counters = Counters::default();
    counters.count(libc::AF_INET, libc::SOCK_SEQPACKET, libc::IPPROTO_SCTP, 1);
    assert_eq!(
        (
            counters.sockets,
            counters.seqpackets,
            counters.sctp,
            counters.tcp
        ),
        (1, 1, 0, 0)
    );
}
