use super::*;

#[test]
fn reads_the_entries_of_a_protocols_file_as_getprotoent() {
    // A host file of the kernel boot test has only well-formed lines. A
    // comment, a blank line, a line without a number, and a line with a
    // number that is not decimal give no entry.
    let text = b"# Internet protocols\n\nip\t0\tIP\t\t# internet protocol\n\
        hopopt 0 HOPOPT\r\nnone\nbad x BAD\nipv6 41 IPv6 v6 # ipv6\n";
    assert_eq!(
        protocol_entries(text),
        [
            (vec![&b"ip"[..], b"IP"], 0),
            (vec![&b"hopopt"[..], b"HOPOPT"], 0),
            (vec![&b"ipv6"[..], b"IPv6", b"v6"], 41),
        ]
    );
}
