use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BINARY_TO_INTEGER_PROBE: &[u8] =
    include_bytes!("../../fixtures/binary_to_integer/binary_to_integer_probe.beam");

/// The `~w` output of `binary_to_integer_probe:main()` on the pinned ERTS.
const EXPECTED: &[u8] =
    include_bytes!("../../fixtures/binary_to_integer/binary_to_integer_probe.expected");

/// The `~w` output of `binary_to_integer_probe:bits()` on the pinned ERTS.
const BITS_EXPECTED: &[u8] = b"[badarg]\n";

#[test]
fn reads_integers_from_binaries_as_pinned_erts() {
    // `erts_internal:binary_to_integer/2` follows
    // `erts_internal_binary_to_integer_2` with `chars_to_integer`
    // (big.c:4162-4256): the base must be a small integer from 2 to 36, the
    // binary must hold a whole number of bytes, and the text reads one
    // optional `-` or `+` and then the digits of the base. It gives the
    // atom `big` for a text above `digits_per_small_lookup` digits
    // (big.c:4115-4127) before it reads a digit, so a text of 18 decimal
    // digits gives `big` also when its value fits in a small integer, and
    // leading zeros do not count toward the limit. It gives the atom
    // `badarg` for each other failure; `erlang:binary_to_integer/1,2` of
    // `erlang.erl` raise that `badarg` and read `big` with
    // `big_binary_to_int` (erlang.erl:1056-1129).
    let program = BeamProgram::load(&[BINARY_TO_INTEGER_PROBE]).unwrap();
    let result = program
        .execute("binary_to_integer_probe", "main", &[])
        .unwrap();
    let atom = |name: &str| Term::Atom(result.atom(name).unwrap());
    let ok = |value: Term| Term::tuple(vec![atom("ok"), value]);
    let mut expected = vec![
        ok(Term::Integer(1023)),
        ok(Term::Integer(2)),
        ok(Term::Integer(-12)),
        ok(Term::Integer(33)),
        ok(Term::Integer(0)),
        ok(Term::Integer(35)),
        ok(Term::Integer(35)),
        ok(Term::Integer(1)),
        ok(Term::Integer(0)),
        ok(Term::Integer(12345678901234567)),
        ok(atom("big")),
        ok(atom("big")),
    ];
    expected.extend(std::iter::repeat_n(ok(atom("badarg")), 18));
    assert_eq!(result.term, Term::list_from(expected));
    // `binary_to_integer_probe:bits()` calls the BIF with a bit string of
    // one bit, which ERTS rejects, because it is not a whole number of bytes
    // (`erl_bits.h:606-633`). Only the reference run of this test reads that
    // case: PBeam has no bitstring term of a size that is not a multiple of
    // 8 bits (row "Bitstring terms" of `docs/erts-differences.md`), so the
    // program of this test cannot build the argument. `binary_to_integer`
    // tests the size of its argument (`bif/erts_internal.rs`), and the case
    // `<<1:1>>` waits for a bitstring term, as task #761 records.

    for (function, expected) in [("main", EXPECTED), ("bits", BITS_EXPECTED)] {
        let reference = Command::new("erl")
            .args([
                "-noshell",
                "-pa",
                "fixtures/binary_to_integer",
                "-eval",
                &format!("io:format(\"~w~n\", [binary_to_integer_probe:{function}()]), halt()."),
            ])
            .output()
            .unwrap();
        assert!(
            reference.status.success() && reference.stderr.is_empty(),
            "{reference:?}"
        );
        assert_eq!(reference.stdout, expected, "{function}");
    }
}
