use super::find;

#[test]
fn finds_the_last_entry_of_a_key_as_the_erts_copy_does() {
    // Measured on ERTS 29.1.1: `erlexec` started with the raw
    // environment `PBEAM_DUP=first`, `PBEAM_DUP=second`, and
    // `=lead=x` gives "second" for os:getenv("PBEAM_DUP") and "lead=x"
    // for os:getenv(""). A `Command` cannot make a duplicate entry,
    // and the `erl` script removes these entries, so this test gives
    // the raw entries to `find`.
    let entries = [&b"A=1"[..], b"A=2", b"=lead=x", b"NOEQ", b"=x"];
    assert_eq!(find(entries, b"A"), Some(&b"2"[..]));
    assert_eq!(find(entries, b""), Some(&b"x"[..]));
    assert_eq!(
        find(entries[..3].iter().copied(), b""),
        Some(&b"lead=x"[..])
    );
    assert_eq!(find(entries, b"NOEQ"), None);
    assert_eq!(find(entries, b"B"), None);
}
