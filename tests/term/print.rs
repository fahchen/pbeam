use std::sync::Arc;

use super::print_term;
use crate::term::{
    Atom, AtomTable, Binary, Fun, Lambda, Map, MatchState, PortId, ProcessId, Reference, Term,
};

/// Return a table that holds each name of `names`.
fn table_of(names: &[&str]) -> AtomTable {
    let mut table = AtomTable::new();
    for name in names {
        table
            .put(name)
            .expect("a test makes fewer atoms than ATOM_LIMIT");
    }
    table
}

/// Return the atom of `name` in `table`.
fn atom_of(table: &AtomTable, name: &str) -> Atom {
    table.get(name).expect("the table holds the name")
}

/// Return a list of the integers of `bytes`.
fn byte_list(bytes: &[u8]) -> Term {
    Term::list_from(bytes.iter().map(|&byte| Term::Integer(byte.into())))
}

#[test]
#[should_panic(expected = "a match context reached a term operation")]
fn panics_for_a_match_context() {
    // The interpreter gives a runtime error before a match context can
    // reach a term operation.
    print_term(
        &Term::MatchContext(Arc::new(MatchState::new(Binary::from(b"a")))),
        &AtomTable::new(),
    );
}

#[test]
fn prints_a_deep_term_without_recursion() {
    let depth = 1_000_000;
    let deep = (0..depth).fold(Term::tuple(vec![]), |inner, _| Term::tuple(vec![inner]));
    let text = print_term(&deep, &AtomTable::new());
    assert_eq!(text.len(), 2 * (depth + 1));
    assert!(text[..=depth].iter().all(|&byte| byte == b'{'));
    assert!(text[depth + 1..].iter().all(|&byte| byte == b'}'));
}

#[test]
fn prints_pids_and_references_as_pinned_erts() {
    let atoms = AtomTable::new();
    // The pid data holds the number in its low 28 bits and the serial
    // above them (`erl_term.h:646-656`).
    assert_eq!(print_term(&Term::Pid(ProcessId(0)), &atoms), b"<0.0.0>");
    // A bignum prints its decimal digits, in the text and in `Debug`.
    let big = Term::from_bigint(-(num_bigint::BigInt::from(1) << 64u32));
    assert_eq!(print_term(&big, &atoms), b"-18446744073709551616");
    assert_eq!(format!("{big:?}"), "Integer(-18446744073709551616)");
    assert_eq!(
        print_term(&Term::Pid(ProcessId(1 << 28)), &atoms),
        b"<0.0.1>"
    );
    assert_eq!(
        print_term(&Term::Pid(ProcessId((1 << 28) - 1)), &atoms),
        b"<0.268435455.0>"
    );
    // ERTS 29.1.1 prints the reference with the counter value
    // 0xF55F56815BA88196 of scheduler 1 as
    // #Ref<0.4116665985.1537736705.33174>.
    assert_eq!(
        print_term(&Term::Reference(Reference(0xF55F_5681_5BA8_8196)), &atoms),
        b"#Ref<0.4116665985.1537736705.33174>"
    );
    assert_eq!(
        print_term(&Term::Reference(Reference(0)), &atoms),
        b"#Ref<0.0.1.0>"
    );
}

#[test]
fn prints_a_port_as_erts() {
    // ERTS prints a local port as `#Port<0.N>` (erl_printf_term.c:521-528).
    // PBeam prints its counter value as `N`, as it prints a pid.
    assert_eq!(
        print_term(&Term::Port(PortId(10)), &AtomTable::new()),
        b"#Port<0.10>"
    );
}

#[test]
fn prints_atoms_as_pinned_erts() {
    // Each expected value is the text of the pinned ERTS OTP 29.1.1 from
    // `erts_internal:term_to_string(Term, 32768)`:
    // `mise run erts-eval '[io:format("~p => ~p~n", [T,
    // erlang:list_to_binary(erts_internal:term_to_string(T, 32768))]) || T <- Terms]'`.
    let cases: &[(&str, &[u8])] = &[
        ("ok", b"ok"),
        ("end", b"end"),
        ("Hello", b"'Hello'"),
        ("it's", b"'it\\'s'"),
        ("a\u{8}", b"'a\\b'"),
        ("a\nb", b"'a\\nb'"),
        ("a\u{1b}b", b"'a\\033b'"),
        ("\\", b"'\\\\'"),
        ("\u{c}", b"'\\f'"),
        ("\t", b"'\\t'"),
        ("\r", b"'\\r'"),
        ("\u{8}", b"'\\b'"),
        ("\u{b}", b"'\\v'"),
        ("", b"''"),
        ("9x", b"'9x'"),
        ("_x", b"'_x'"),
        ("aB_9", b"aB_9"),
        ("\u{c4}", b"'\xc3\x84'"),
        ("\u{e9}", b"\xc3\xa9"),
        ("\u{3bb}", b"'\xce\xbb'"),
        ("pbeam_zeta", b"pbeam_zeta"),
        ("Elixir.Hello", b"'Elixir.Hello'"),
    ];
    let table = table_of(&cases.iter().map(|(name, _)| *name).collect::<Vec<_>>());
    for (name, expected) in cases {
        assert_eq!(
            print_term(&Term::Atom(atom_of(&table, name)), &table),
            *expected,
            "{name:?}"
        );
    }
}

#[test]
fn prints_compound_terms_as_pinned_erts() {
    // The same pinned ERTS run as `prints_atoms_as_pinned_erts` gives each
    // expected value.
    let table = table_of(&["a", "b"]);
    let atom = |name: &str| Term::Atom(atom_of(&table, name));
    assert_eq!(print_term(&Term::Integer(-7), &table), b"-7");
    assert_eq!(print_term(&Term::Integer(0), &table), b"0");
    assert_eq!(print_term(&Term::nil(), &table), b"[]");
    assert_eq!(print_term(&byte_list(b"abc"), &table), b"\"abc\"");
    assert_eq!(
        print_term(&byte_list(b"q\"uo\n\t"), &table),
        b"\"q\\\"uo\\n\t\""
    );
    assert_eq!(
        print_term(
            &Term::list_from(vec![Term::Integer(1), Term::Integer(2), Term::Integer(3)]),
            &table
        ),
        b"[1,2,3]"
    );
    assert_eq!(
        print_term(
            &Term::list_with_tail(vec![Term::Integer(65)], Term::Integer(66)),
            &table
        ),
        b"[65|66]"
    );
    assert_eq!(
        print_term(&Term::list_with_tail(vec![atom("a")], atom("b")), &table),
        b"[a|b]"
    );
    assert_eq!(print_term(&byte_list(&[128]), &table), b"[128]");
    assert_eq!(print_term(&byte_list(&[160, 255]), &table), b"\"\xa0\xff\"");
    assert_eq!(
        print_term(&Term::list_from(vec![Term::Integer(300)]), &table),
        b"[300]"
    );
    assert_eq!(print_term(&Term::tuple(vec![]), &table), b"{}");
    assert_eq!(
        print_term(
            &Term::tuple(vec![atom("a"), Term::tuple(vec![atom("b")])]),
            &table
        ),
        b"{a,{b}}"
    );
    assert_eq!(print_term(&Term::binary(vec![]), &table), b"<<>>");
    assert_eq!(
        print_term(&Term::binary(b"abc".to_vec()), &table),
        b"<<\"abc\">>"
    );
    assert_eq!(
        print_term(&Term::binary(b"a\"b".to_vec()), &table),
        b"<<\"a\\\"b\">>"
    );
    assert_eq!(
        print_term(&Term::binary(vec![1, 2, 200]), &table),
        b"<<1,2,200>>"
    );
    assert_eq!(
        print_term(&Term::binary(vec![97, 10]), &table),
        b"<<97,10>>"
    );
    assert_eq!(
        print_term(
            &Term::Map(Map::from_pairs(vec![], &table).expect("a map takes atom keys")),
            &table
        ),
        b"#{}"
    );
    assert_eq!(
        print_term(
            &Term::Map(
                Map::from_pairs(vec![(atom("a"), Term::Integer(1))], &table)
                    .expect("a map takes atom keys")
            ),
            &table
        ),
        b"#{a=>1}"
    );
}

#[test]
fn prints_funs_as_pinned_erts() {
    // The pinned ERTS OTP 29.1.1 gives `fun lists:map/2` for the external
    // fun and `#Fun<root_exit_probe.0.120231133>` for the local fun of the
    // fixture (`mise run erts-eval --pa fixtures/root_exit '...'`).
    let table = table_of(&["lists", "map", "root_exit_probe"]);
    let external = Term::Fun(Fun::external(
        atom_of(&table, "lists"),
        atom_of(&table, "map"),
        2,
    ));
    assert_eq!(print_term(&external, &table), b"fun lists:map/2");
    let local = Term::Fun(Fun::new(
        Lambda::new(atom_of(&table, "root_exit_probe"), 0, 120_231_133, 53, 1),
        vec![],
    ));
    assert_eq!(
        print_term(&local, &table),
        b"#Fun<root_exit_probe.0.120231133>"
    );
}

#[test]
fn reads_the_name_from_the_given_table() {
    // The atom keeps its own name today, so a table that gives another name
    // at the same index proves that `print_term` reads the table.
    let mut source = AtomTable::new();
    let named = source.put("pbeam_zeta").expect("the table holds the name");
    let mut other = AtomTable::new();
    other.put("pbeam_other").expect("the table holds the name");
    assert_eq!(other.name(named.index()), Some("pbeam_other"));
    assert_eq!(print_term(&Term::Atom(named), &other), b"pbeam_other");
}

#[test]
fn prints_a_float_with_six_digits_as_pinned_erts() {
    // `erlang:display/1` prints a float with `%.6e`
    // (`erl_printf_term.c:596-600`). ERTS 29.1.1 prints `1.500000e+00`,
    // `-0.000000e+00`, and `1.000000e+300`.
    let floats = Term::list_from([1.5, -0.0, 1.0e300].map(Term::Float));
    assert_eq!(
        print_term(&floats, &AtomTable::new()),
        b"[1.500000e+00,-0.000000e+00,1.000000e+300]"
    );
}
