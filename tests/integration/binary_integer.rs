use std::process::{Command, Stdio};

use pbeam_runtime::{AtomTable, BeamProgram, HostTerm, Term};

const BINARY_INTEGER_PROBE: &[u8] =
    include_bytes!("../../fixtures/binary_integer/binary_integer_probe.beam");

/// `binary_integer_probe:unsigned/0` on the pinned ERTS: the bytes of
/// `binary:encode_unsigned/1,2` in the fewest bytes, and the integers of
/// `binary:decode_unsigned/1,2` with the leading zeros skipped, for `big`
/// and `little`, a bignum, and a sub-binary that does not start on a byte
/// boundary (`erl_bif_binary.c:2572-2816`). A negative integer, a term that
/// is not an integer or a binary, a bitstring, and an endianness that is
/// not `big` or `little` raise `badarg`.
const UNSIGNED: &str = "[<<0>>,<<1,2>>,<<2,1>>,<<8,0,0,0,0,0,0,0>>,<<0,0,0,0,0,0,0,128>>,\
                        <<64,0,0,0,0,0,0,1,2>>,<<2,1,0,0,0,0,0,0,64>>,{error,badarg},\
                        {error,badarg},{error,badarg},{error,badarg},{error,badarg},0,0,258,\
                        513,18446744073709551615,1180591620717411303682,\
                        1180591620717411303682,528408,403703808,{error,badarg},{error,badarg},\
                        {error,badarg},{error,badarg}]";

/// `binary_integer_probe:common/0` on the pinned ERTS: the pairs of
/// `binary:longest_common_prefix/1` and `binary:longest_common_suffix/1`
/// (`erl_bif_binary.c:2078-2239`), with binaries longer than one step of 64
/// bytes and a sub-binary that does not start on a byte boundary, and
/// `badarg` for the empty list, a bitstring, an improper list, an element
/// that is not a binary, and a term that is not a list.
const COMMON: &str = "[{2,0},{3,3},{0,0},{0,0},{2,0},{150,49},{150,49},{200,200},{150,49},\
                      {100,100},{2,2},{{error,badarg},{error,badarg}},\
                      {{error,badarg},{error,badarg}},{{error,badarg},{error,badarg}},\
                      {{error,badarg},{error,badarg}},{{error,badarg},{error,badarg}},\
                      {{error,badarg},{error,badarg}},{{error,badarg},{error,badarg}}]";

/// `binary_integer_probe:referenced/0` on the pinned ERTS: the size of a
/// heap binary of at most 64 bytes is its own size, and a refc binary or a
/// sub-binary of more than 64 bytes gives the size of the refc binary
/// (`erl_bif_binary.c:2508-2534`). A term that is not a bitstring raises
/// `badarg`.
const REFERENCED: &str = "[0,3,64,63,200,200,10,200,200,64,200,65,1,0,1,{error,badarg},\
                          {error,badarg}]";

/// `binary_integer_probe:costs/1` on the pinned ERTS with `erl +S 1`: the
/// result and the reductions of `binary:longest_common_prefix/1` and
/// `binary:longest_common_suffix/1` for two equal binaries of `N` bytes,
/// with 1 for the `call_ext`. A step uses one reduction for each 10 bits,
/// and a call longer than the budget of one turn traps and continues in
/// the next turns (`erl_bif_binary.c:2098-2117`, `2178`).
const COSTS: &str = "[{1,prefix,1,1},{1,suffix,1,1},{64,prefix,64,52},{64,suffix,64,52},\
                     {65,prefix,65,53},{65,suffix,65,53},{1000,prefix,1000,801},\
                     {1000,suffix,1000,801},{4000,prefix,4000,3201},{4000,suffix,4000,3201},\
                     {100000,prefix,100000,79146},{100000,suffix,100000,79146},\
                     {1000000,prefix,1000000,791172},{1000000,suffix,1000000,791172}]";

/// The spawn options of the measured processes in the ERTS reference run: a
/// heap of 4,000,000 words, so that no measured process collects garbage.
/// PBeam has no process heap and runs with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// Return a term as `io_lib:write/1` gives it for the atoms, integers,
/// binaries, lists, and tuples of the probe results. The atom names come
/// from `atoms`, the table that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::binary(vec![97])]), &atoms), "[<<97>>]");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::BigInt(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        Term::Binary(bytes) => {
            let bytes: Vec<String> = bytes.bytes().unwrap().iter().map(u8::to_string).collect();
            format!("<<{}>>", bytes.join(","))
        }
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn runs_binary_integer_and_common_affix_bifs_as_pinned_erts() {
    let program = BeamProgram::load(&[BINARY_INTEGER_PROBE]).unwrap();
    let run = |function, arguments: &[HostTerm]| {
        let result = program
            .execute("binary_integer_probe", function, arguments)
            .unwrap();
        text(&result.term, result.atoms())
    };
    assert_eq!(run("unsigned", &[]), UNSIGNED);
    assert_eq!(run("common", &[]), COMMON);
    assert_eq!(run("referenced", &[]), REFERENCED);
    assert_eq!(run("costs", &[program.host_term(Term::nil())]), COSTS);

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/binary_integer",
            "-eval",
            &format!(
                "M = binary_integer_probe, io:format(\"~w~n~w~n~w~n~w~n\", \
                 [M:unsigned(), M:common(), M:referenced(), M:costs({ERTS_OPTIONS})]), halt()."
            ),
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{UNSIGNED}\n{COMMON}\n{REFERENCED}\n{COSTS}\n")
    );
}
