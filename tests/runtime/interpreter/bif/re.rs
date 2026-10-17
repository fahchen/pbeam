//! Unit tests of the parts of `re` that a fixture cannot drive.

use super::*;

#[test]
fn reads_the_offset_pairs_of_a_precompiled_pattern() {
    // A capture count plus one for the whole match, as an `int` keeps the
    // low 32 bits of the sum (`erl_bif_re.c:1607`).
    assert_eq!(ovec_pairs(0), 1);
    assert_eq!(ovec_pairs(1), 2);
    assert_eq!(ovec_pairs(i64::from(u32::MAX) - 1), u32::MAX);
    assert_eq!(ovec_pairs(i64::from(u32::MAX)), 0);
    assert_eq!(ovec_pairs(-1), 0);
}

#[test]
fn returns_the_empty_element_of_each_capture_type() {
    // A group that did not take part in the match gives `{-1, 0}`, an
    // empty list, or an empty binary (`erl_bif_re.c:1092-1093`,
    // `1130-1131`, `1233-1240`).
    assert_eq!(
        empty_element(ReturnType::Index),
        Term::tuple(vec![Term::Integer(-1), Term::Integer(0)])
    );
    assert_eq!(empty_element(ReturnType::List), Term::nil());
    assert_eq!(empty_element(ReturnType::Binary), Term::binary(Vec::new()));
}

#[test]
fn returns_the_element_of_a_group_that_did_not_take_part() {
    // The ovector of a match that has an unset group holds `PCRE2_UNSET`
    // (`pcre2.h:314`), and each type gives its empty element.
    let ovector = [0, 2, pcre2::UNSET, pcre2::UNSET];
    assert_eq!(
        capture_element(ReturnType::Index, &ovector, 1, b"ab"),
        Term::tuple(vec![Term::Integer(-1), Term::Integer(0)])
    );
    assert_eq!(
        capture_element(ReturnType::List, &ovector, 1, b"ab"),
        Term::nil()
    );
    assert_eq!(
        capture_element(ReturnType::Binary, &ovector, 1, b"ab"),
        Term::binary(Vec::new())
    );
}
