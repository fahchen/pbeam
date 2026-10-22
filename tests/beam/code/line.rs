use super::LineTable;
use crate::term::Term;

/// Return a `Line` chunk with the version, the declared entry count, the
/// item count, the name count, the compact item bytes, and the name bytes.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(chunk(0, 1, 1, 0, &[0x51], &[]).len(), 21);
/// ```
fn chunk(version: u32, entries: u32, items: u32, names: u32, body: &[u8], tail: &[u8]) -> Vec<u8> {
    [version, 0, entries, items, names]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .chain(body.iter().copied())
        .chain(tail.iter().copied())
        .collect()
}

/// Return the location `[{file, File}, {line, Line}]`.
fn location(file: &str, line: i64) -> Term {
    let file = Term::list_from(
        file.chars()
            .map(|code| Term::Integer(u32::from(code).into())),
    );
    Term::list_from(vec![
        Term::tuple(vec![Term::static_atom("file"), file]),
        Term::tuple(vec![Term::static_atom("line"), Term::Integer(line)]),
    ])
}

#[test]
fn reads_the_items_and_the_file_names_of_a_line_chunk() {
    // Items: 1 = probe.erl:5, then file 1: 2 = a.erl:7, then file 2 (the
    // empty name): 3 = probe.erl:0, 4 = line 2^24 - 1 (rejected), then
    // file 0: 5 = line 0 (rejected).
    let body = [
        0x51, 0x12, 0x71, 0x22, 0x01, 0x59, 0x00, 0xff, 0xff, 0xff, 0x02, 0x01,
    ];
    let names = [0, 5, b'a', b'.', b'e', b'r', b'l', 0, 0];
    let mut table = LineTable::parse(&chunk(0, 6, 5, 2, &body, &names), "probe").unwrap();
    table.function(0, Some(1)).unwrap();
    table.line(2, 2).unwrap();
    // A duplicate `line` records no entry.
    table.line(3, 2).unwrap();
    table.line(4, 3).unwrap();
    table.line(5, 4).unwrap();
    table.function(6, None).unwrap();
    table.line(7, 5).unwrap();
    table.function(8, Some(2)).unwrap();
    assert_eq!(table.entries.len(), 7);
    let rest = Term::static_atom("ok");
    assert_eq!(table.location(0, Term::nil()), location("probe.erl", 5));
    assert_eq!(table.location(3, Term::nil()), location("a.erl", 7));
    assert_eq!(table.location(4, Term::nil()), location("probe.erl", 0));
    assert_eq!(table.location(5, rest.clone()), rest);
    assert_eq!(table.location(6, rest.clone()), rest);
    assert_eq!(table.location(7, rest.clone()), rest);
    assert_eq!(table.location(9, Term::nil()), location("a.erl", 7));
    // The declared entry count bounds the entries.
    assert!(
        table
            .line(9, 1)
            .unwrap_err()
            .to_string()
            .contains("table overflow (6/6)")
    );
    assert!(
        table
            .line(9, 6)
            .unwrap_err()
            .to_string()
            .contains("index overflow (6)")
    );
}

#[test]
fn gives_no_location_without_a_line_table() {
    let mut table = LineTable::parse(&chunk(1, 1, 1, 0, &[0x51], &[]), "probe").unwrap();
    table.line(0, 9).unwrap();
    table.function(1, None).unwrap();
    assert!(table.entries.is_empty());
    assert_eq!(table.location(0, Term::nil()), Term::nil());
}

#[test]
fn rejects_a_corrupt_line_chunk() {
    let corrupt = |data: &[u8]| {
        let error = LineTable::parse(data, "probe").unwrap_err();
        assert!(
            error.to_string().contains("corrupt BEAM line table"),
            "{error}"
        );
    };
    corrupt(&[0, 0, 0, 0, 0, 0, 0, 0, 0]);
    corrupt(&chunk(0, 1, 1, 0, &[], &[]));
    corrupt(&chunk(0, 1, 1, 0, &[0x50], &[]));
    // File 2 is after the implicit file 0 and its count 1.
    corrupt(&chunk(0, 1, 1, 0, &[0x22, 0x51], &[]));
    corrupt(&chunk(
        0,
        1,
        1,
        0,
        &[0xd9, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        &[],
    ));
    // The long integer `ff ff` is -1.
    corrupt(&chunk(0, 1, 1, 0, &[0x19, 0xff, 0xff], &[]));
    corrupt(&chunk(0, 1, 1, 1, &[0x51], &[0]));
    corrupt(&chunk(0, 1, 1, 1, &[0x51], &[0xff, 0xff]));
    corrupt(&chunk(0, 1, 1, 1, &[0x51], &[0, 2, b'a']));
    corrupt(&chunk(0, 1, 1, 1, &[0x51], &[0, 1, 0xff]));
}

/// Return the location of each item of a table, from an entry per item.
fn locations(table: &mut LineTable) -> Vec<Term> {
    let items = table.items.len();
    (1..items)
        .map(|item| {
            table.line(item, item as u64).unwrap();
            table.location(item, Term::nil())
        })
        .collect()
}

#[test]
fn keeps_the_low_16_bits_of_a_location_with_no_file_name() {
    // With no file name, file 1 passes the ERTS check `name_index <=
    // name_count` (beam_file.c:475, 515-517), and the 16-bit table drops the
    // file of `MAKE_LOCATION(1, 5)` (beam_file.c:501, asm_load.c:857-870).
    // `MAKE_LOCATION(1, 0)` keeps 0, the undefined location.
    let body = [0x12, 0x51, 0x01];
    let mut table = LineTable::parse(&chunk(0, 2, 2, 0, &body, &[]), "m").unwrap();
    assert_eq!(locations(&mut table), [location("m.erl", 5), Term::nil()]);
}

#[test]
fn cuts_the_file_index_and_the_line_to_32_bits() {
    // ERTS keeps both as `Sint32` (beam_file.h:112-115). The file index
    // 2^32 + 1 is file 1, the line 2^32 + 5 is line 5, and the file index
    // 2^32 - 1 is -1, which `MAKE_LOCATION` rejects (beam_code.h:33-39).
    let body = [
        0x7a, 0x01, 0x00, 0x00, 0x00, 0x01, 0x79, 0x01, 0x00, 0x00, 0x00, 0x05, 0x5a, 0xff, 0xff,
        0xff, 0xff, 0x71,
    ];
    let names = [0, 5, b'a', b'.', b'e', b'r', b'l'];
    let mut table = LineTable::parse(&chunk(0, 2, 2, 1, &body, &names), "m").unwrap();
    assert_eq!(locations(&mut table), [location("a.erl", 5), Term::nil()]);
}

#[test]
fn gives_no_location_for_the_file_index_of_the_name_count() {
    // ERTS accepts file 2 of a chunk with one file name and reads the word
    // after its file name table (beam_file.c:515-517, utils.c:3436).
    let names = [0, 5, b'a', b'.', b'e', b'r', b'l'];
    let mut table = LineTable::parse(&chunk(0, 1, 1, 1, &[0x22, 0x51], &names), "m").unwrap();
    assert_eq!(locations(&mut table), [Term::nil()]);
}
