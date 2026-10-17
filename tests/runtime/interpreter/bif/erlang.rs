//! Unit tests of `src/runtime/interpreter/bif/erlang.rs` for an argument
//! that the fixture probes of their tasks cannot check on PBeam: the public
//! API of the runtime has no constructor of a bitstring that is not a whole
//! number of bytes, and a probe that loads only its own beam reaches no port
//! term, because a port needs `erlang:open_port/2` of `erlang.beam`.

use super::*;
use crate::term::Binary;
use crate::{PortId, ProcessId};

#[test]
fn raises_badarg_for_a_bitstring_that_is_not_a_whole_number_of_bytes() {
    // `erts_get_aligned_binary_bytes` (`erl_unicode.c:1969-1972`) gives NULL
    // for a term that is not a byte-aligned binary, so `binary_to_atom/2`
    // raises `badarg`. The pinned ERTS gives `badarg` with the frame
    // `{erlang,binary_to_atom,[<<1:1>>,latin1],[{error_info,#{module =>
    // erl_erts_errors}}]}` (measured with `mise run erts-eval`).
    let mut atoms = AtomTable::new();
    let bitstring = Term::Binary(Binary::from(vec![1]).bits(0, 1));
    let result = binary_to_atom(&mut atoms, &bitstring, &Term::static_atom("latin1")).unwrap();
    assert!(matches!(
        result,
        BifResult::Raise { class: ExceptionClass::Error, reason }
            if reason == Term::static_atom("badarg")
    ));
}

#[test]
fn raises_badarg_for_an_existing_atom_of_a_bitstring() {
    // `erts_get_aligned_binary_bytes` (`erl_unicode.c:1969-1972`) gives NULL
    // for a term that is not a byte-aligned binary, so
    // `binary_to_existing_atom/2` raises `badarg`. The pinned ERTS gives
    // `badarg` (measured with the `bits/0` probe of
    // `fixtures/existing_atom`). The public API of the runtime has no
    // constructor of such a bitstring, so this unit test builds it with
    // `Binary::bits`.
    let atoms = AtomTable::new();
    let bitstring = Term::Binary(Binary::from(vec![1]).bits(0, 1));
    let result = binary_to_existing_atom(&atoms, &bitstring, &Term::static_atom("utf8"));
    assert!(matches!(
        result,
        BifResult::Raise { class: ExceptionClass::Error, reason }
            if reason == Term::static_atom("badarg")
    ));
}

#[test]
fn lists_the_text_of_a_pid_and_of_a_port_as_pinned_erts() {
    // `erlang:pid_to_list/1` and `erlang:port_to_list/1` print the term with
    // `%T` (`term2list_dsprintf`, `bif.c:4442-4457`): the pid data 5 is the
    // pid `<0.5.0>` and the port number 5 is the port `#Port<0.5>`, as the
    // pinned ERTS gives them for `erlang:pid_to_list(erlang:list_to_pid(
    // "<0.5.0>"))` and `erlang:port_to_list(erlang:list_to_port(
    // "#Port<0.5>"))` (measured with `mise run erts-eval`). A probe that
    // loads only its own beam reaches no port term, because a port needs
    // `erlang:open_port/2` of `erlang.beam`.
    let atoms = AtomTable::new();
    let characters = |text: &str| {
        Term::list_from(
            text.bytes()
                .map(|byte| Term::Integer(i64::from(byte)))
                .collect::<Vec<_>>(),
        )
    };
    let pid = pid_to_list(&Term::Pid(ProcessId(5)), &atoms);
    assert!(matches!(pid, BifResult::Value(value) if value == characters("<0.5.0>")));
    let port = port_to_list(&Term::Port(PortId(5)), &atoms);
    assert!(matches!(port, BifResult::Value(value) if value == characters("#Port<0.5>")));
    assert!(matches!(
        pid_to_list(&Term::Port(PortId(5)), &atoms),
        BifResult::Raise { .. }
    ));
    assert!(matches!(
        port_to_list(&Term::Pid(ProcessId(5)), &atoms),
        BifResult::Raise { .. }
    ));
}
