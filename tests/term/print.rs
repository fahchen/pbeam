use super::print_term;
use crate::term::{MatchContext, ProcessId, Reference, Term};

#[test]
#[should_panic(expected = "a match context reached a term operation")]
fn panics_for_a_match_context() {
    // The interpreter gives a runtime error before a match context can
    // reach a term operation.
    print_term(&Term::MatchContext(MatchContext::new(b"a")));
}

#[test]
fn prints_a_deep_term_without_recursion() {
    let depth = 1_000_000;
    let deep = (0..depth).fold(Term::Tuple(vec![]), |inner, _| Term::Tuple(vec![inner]));
    let text = print_term(&deep);
    assert_eq!(text.len(), 2 * (depth + 1));
    assert!(text[..=depth].iter().all(|&byte| byte == b'{'));
    assert!(text[depth + 1..].iter().all(|&byte| byte == b'}'));
}

#[test]
fn prints_pids_and_references_as_pinned_erts() {
    // The pid data holds the number in its low 28 bits and the serial
    // above them (`erl_term.h:646-656`).
    assert_eq!(print_term(&Term::Pid(ProcessId(0))), b"<0.0.0>");
    assert_eq!(print_term(&Term::Pid(ProcessId(1 << 28))), b"<0.0.1>");
    assert_eq!(
        print_term(&Term::Pid(ProcessId((1 << 28) - 1))),
        b"<0.268435455.0>"
    );
    // ERTS 29.1.1 prints the reference with the counter value
    // 0xF55F56815BA88196 of scheduler 1 as
    // #Ref<0.4116665985.1537736705.33174>.
    assert_eq!(
        print_term(&Term::Reference(Reference(0xF55F_5681_5BA8_8196))),
        b"#Ref<0.4116665985.1537736705.33174>"
    );
    assert_eq!(print_term(&Term::Reference(Reference(0))), b"#Ref<0.0.1.0>");
}
