use std::process::Command;
use std::sync::Arc;

use num_bigint::BigInt;
use pbeam_runtime::{BeamProgram, Map, Term};

#[path = "../common/mod.rs"]
mod common;

const ATOMICS_PROBE: &[u8] = include_bytes!("../../fixtures/atomics/atomics_probe.beam");

/// The `~w` output of `atomics_probe:main()` on the pinned ERTS.
const EXPECTED: &str = include_str!("../../fixtures/atomics/atomics_probe.expected");

/// Load the probe with the preloaded `counters` and `atomics` modules of
/// the pinned OTP. The two modules hold the BEAM code that calls the BIFs
/// of `erl_bif_atomics.c`, and a run of the example loads them on demand
/// through the code server.
fn program() -> BeamProgram {
    let counters = common::pinned_beam("counters");
    let atomics = common::pinned_beam("atomics");
    BeamProgram::load(&[&counters, &atomics, ATOMICS_PROBE]).unwrap()
}

/// Run `atomics_probe:Function()` under the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/atomics",
            "-eval",
            &format!("io:format(\"~w~n\", [atomics_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    output.stdout
}

/// The terms of the exceptions of `atomics_probe:main/0`, in the order of
/// the cases of that function.
struct Expected<'a> {
    program: &'a BeamProgram,
}

impl Expected<'_> {
    fn atom(&self, name: &str) -> Term {
        Term::Atom(self.program.atom(name).unwrap())
    }

    fn big(&self, value: impl Into<BigInt>) -> Term {
        Term::BigInt(Arc::new(value.into()))
    }

    /// The location of a raise of `counters:new/2` and of `atomics:new/2`,
    /// whose error format module is `erl_erts_errors`
    /// (`beam_common.c:939-991`), with the cause of an option error.
    fn location(&self, cause: Option<&str>) -> Term {
        let mut pairs = vec![(self.atom("module"), self.atom("erl_erts_errors"))];
        if let Some(cause) = cause {
            pairs.push((self.atom("cause"), self.atom(cause)));
        }
        Term::list_from(vec![Term::tuple(vec![
            self.atom("error_info"),
            Term::Map(Map::from_pairs(pairs, self.program.atoms()).unwrap()),
        ])])
    }

    /// The exception of a call of `module:function/arity` that raises
    /// `reason`, with `arguments` and the location of its first frame.
    fn failed(
        &self,
        reason: &str,
        module: &str,
        function: &str,
        arguments: Vec<Term>,
        location: Term,
    ) -> Term {
        let frame = Term::tuple(vec![
            self.atom(module),
            self.atom(function),
            Term::list_from(arguments),
            location,
        ]);
        Term::tuple(vec![self.atom("error"), self.atom(reason), frame])
    }

    /// The information map of `atomics:info/1`.
    fn info(&self, max: Term, min: Term, size: i64) -> Term {
        let pairs = vec![
            (self.atom("max"), max),
            (self.atom("memory"), Term::Integer(40 + 8 * size)),
            (self.atom("min"), min),
            (self.atom("size"), Term::Integer(size)),
        ];
        Term::Map(Map::from_pairs(pairs, self.program.atoms()).unwrap())
    }

    /// The pair of one case of `atomics_probe:main/0`.
    fn pair(&self, label: &str, value: Term) -> Term {
        Term::tuple(vec![self.atom(label), value])
    }

    /// The result of `counters:new/2` and of `atomics:new/2`, whose
    /// reference a run does not print.
    fn array(&self) -> Term {
        Term::tuple(vec![self.atom("atomics"), self.atom("reference")])
    }
}

#[test]
fn runs_the_counters_calls_of_the_elixir_logger_start_as_pinned_erts() {
    // The Elixir Logger of the plug_hello release starts with
    // `counters:new(1, [atomics])`
    // (`Elixir.Logger.Backends.Internal:init/1`) and writes through
    // `counters:add/3` and `counters:get/2`
    // (`Elixir.Logger.Backends.Handler:threshold/1`,
    // `Elixir.Logger.Backends.Config:update_counter/2`). The default mode
    // of `counters` is the `atomics` module, whose BIFs are
    // `erts_internal:atomics_new/2`, `atomics:get/2`, `atomics:add/3`,
    // `atomics:put/3`, and `atomics:info/1` (`erl_bif_atomics.c`). A
    // counter of an array is 64 bits wide and wraps around (`counters.erl`),
    // a signed array takes the signed range and an unsigned array the
    // unsigned one (`get_value`, `erl_bif_atomics.c:112-117`), and a
    // reference that is not an array, an index outside the array, a size
    // of 0, a negative size, and a bad value raise `badarg`
    // (`erl_bif_atomics.c:56-110`). A size above the machine word divided
    // by the bytes of an atomic raises `system_limit`
    // (`erl_bif_atomics.c:74-75`), and `counters:info/1` reports the memory
    // of the array (`erl_bif_atomics.c:245-247`).
    let program = program();
    let expected = Expected { program: &program };
    let expected = &expected;
    let error_info = || expected.location(None);
    let badopt = || expected.location(Some("badopt"));
    let counter = |function: &str, arguments: Vec<Term>, location: Term| {
        expected.failed("badarg", "counters", function, arguments, location)
    };
    let atomics = |function: &str, arguments: Vec<Term>| {
        expected.failed("badarg", "atomics", function, arguments, error_info())
    };
    let array = expected.array();
    let cases = Term::list_from(vec![
        expected.pair("new_atomics", array.clone()),
        expected.pair("new_default", array.clone()),
        expected.pair(
            "counter",
            Term::list_from(vec![
                Term::Integer(0),
                expected.atom("ok"),
                Term::Integer(5),
                expected.atom("ok"),
                Term::Integer(3),
                expected.atom("ok"),
                Term::Integer(-1),
                expected.atom("ok"),
                expected.atom("ok"),
                Term::Integer(i64::MIN),
            ]),
        ),
        expected.pair(
            "info_signed",
            expected.info(Term::Integer(i64::MAX), Term::Integer(i64::MIN), 2),
        ),
        expected.pair(
            "info_unsigned",
            expected.info(expected.big(u64::MAX), Term::Integer(0), 2),
        ),
        expected.pair("get_signed", Term::Integer(-1)),
        expected.pair("get_unsigned", expected.big(u64::MAX)),
        expected.pair("add_signed", Term::Integer(2)),
        expected.pair("add_wrap", Term::Integer(i64::MIN)),
        expected.pair("add_unsigned_wrap", Term::Integer(0)),
        expected.pair("put_signed", expected.atom("ok")),
        expected.pair("put_unsigned", expected.big(1u128 << 63)),
        expected.pair(
            "get_not_a_reference",
            atomics("get", vec![Term::Integer(5), Term::Integer(1)]),
        ),
        expected.pair(
            "get_unknown_reference",
            atomics("get", vec![expected.atom("reference"), Term::Integer(1)]),
        ),
        expected.pair(
            "get_counter_not_a_reference",
            counter(
                "get",
                vec![Term::Integer(5), Term::Integer(1)],
                error_info(),
            ),
        ),
        expected.pair(
            "index_zero",
            counter("get", vec![array.clone(), Term::Integer(0)], error_info()),
        ),
        expected.pair(
            "index_above_size",
            counter("get", vec![array.clone(), Term::Integer(2)], error_info()),
        ),
        expected.pair(
            "index_not_an_integer",
            counter("get", vec![array.clone(), Term::Float(1.5)], error_info()),
        ),
        expected.pair(
            "put_above_signed_range",
            atomics(
                "put",
                vec![
                    expected.atom("reference"),
                    Term::Integer(1),
                    expected.big(1u128 << 63),
                ],
            ),
        ),
        expected.pair(
            "put_below_unsigned_range",
            atomics(
                "put",
                vec![
                    expected.atom("reference"),
                    Term::Integer(1),
                    Term::Integer(-1),
                ],
            ),
        ),
        expected.pair(
            "put_above_machine_word",
            atomics(
                "put",
                vec![
                    expected.atom("reference"),
                    Term::Integer(1),
                    expected.big(1u128 << 64),
                ],
            ),
        ),
        expected.pair(
            "add_above_both_ranges",
            atomics(
                "add",
                vec![
                    expected.atom("reference"),
                    Term::Integer(1),
                    expected.big(1u128 << 70),
                ],
            ),
        ),
        expected.pair(
            "add_not_an_integer",
            atomics(
                "add",
                vec![
                    expected.atom("reference"),
                    Term::Integer(1),
                    Term::Float(1.0),
                ],
            ),
        ),
        expected.pair(
            "size_zero",
            counter("new", vec![Term::Integer(0), Term::nil()], error_info()),
        ),
        expected.pair(
            "size_negative",
            counter("new", vec![Term::Integer(-1), Term::nil()], error_info()),
        ),
        expected.pair(
            "size_not_an_integer",
            counter("new", vec![Term::Float(1.0), Term::nil()], error_info()),
        ),
        expected.pair(
            "size_negative_bignum",
            counter(
                "new",
                vec![Term::Integer(-(1 << 62)), Term::nil()],
                error_info(),
            ),
        ),
        expected.pair(
            "size_negative_above_i64",
            counter(
                "new",
                vec![expected.big(-(1i128 << 70)), Term::nil()],
                error_info(),
            ),
        ),
        expected.pair(
            "size_above_machine_word",
            expected.failed(
                "system_limit",
                "counters",
                "new",
                vec![expected.big(1u128 << 64), Term::nil()],
                error_info(),
            ),
        ),
        expected.pair(
            "size_above_limit",
            expected.failed(
                "system_limit",
                "counters",
                "new",
                vec![Term::Integer(1 << 61), Term::nil()],
                error_info(),
            ),
        ),
        expected.pair(
            "options_not_a_mode",
            expected.failed(
                "badarg",
                "counters",
                "new",
                vec![Term::Integer(1), Term::list_from(vec![expected.atom("x")])],
                badopt(),
            ),
        ),
        expected.pair(
            "options_not_a_list",
            expected.failed(
                "badarg",
                "counters",
                "new",
                vec![Term::Integer(1), expected.atom("atomics")],
                badopt(),
            ),
        ),
        expected.pair(
            "options_negative",
            expected.failed(
                "badarg",
                "erts_internal",
                "atomics_new",
                vec![Term::Integer(1), Term::Integer(-1)],
                error_info(),
            ),
        ),
        expected.pair(
            "options_not_an_integer",
            expected.failed(
                "badarg",
                "erts_internal",
                "atomics_new",
                vec![Term::Integer(1), Term::Float(1.0)],
                error_info(),
            ),
        ),
        expected.pair(
            "options_above_machine_word",
            expected.failed(
                "system_limit",
                "erts_internal",
                "atomics_new",
                vec![Term::Integer(1), expected.big(1u128 << 64)],
                error_info(),
            ),
        ),
    ]);

    // Each case is a pair `{Label, Result}`, and a case that differs names
    // itself in the failure.
    let actual: Vec<Term> = program
        .execute("atomics_probe", "main", &[])
        .unwrap()
        .term
        .list_iter()
        .unwrap()
        .cloned()
        .collect();
    let cases: Vec<Term> = cases.list_iter().unwrap().cloned().collect();
    assert_eq!(actual.len(), cases.len());
    for (case, expected) in actual.iter().zip(cases.iter()) {
        assert_eq!(
            case,
            expected,
            "case {}",
            pbeam_runtime::debug_term(case, program.atoms())
        );
    }
    assert_eq!(reference("main"), EXPECTED.as_bytes());
}

#[test]
fn gives_system_limit_for_an_array_that_pbeam_cannot_allocate() {
    // `counters:new(1 bsl 60, [])` asks for 8 EiB, which no machine holds.
    // ERTS ends the emulator when that allocation fails
    // (`erts_alloc_n_enomem`, `erl_alloc.c:2105-2109`), so the probe gives
    // this part to a PBeam run only: PBeam raises `error:system_limit` in
    // the calling process, as `docs/erts-differences.md` records.
    let program = program();
    let expected = Expected { program: &program };
    let frame = Term::tuple(vec![
        expected.atom("counters"),
        expected.atom("new"),
        Term::list_from(vec![Term::Integer(1 << 60), Term::nil()]),
        expected.location(None),
    ]);
    let expected = Term::tuple(vec![
        expected.atom("error"),
        expected.atom("system_limit"),
        frame,
    ]);
    assert_eq!(
        program
            .execute("atomics_probe", "allocation_failure", &[])
            .unwrap()
            .term,
        expected
    );
}

#[test]
fn runs_the_write_concurrency_counters_as_pinned_erts() {
    // The `write_concurrency` mode of `counters` calls the BIFs of
    // `erl_bif_counters.c:77-259`. A read gives the signed 64-bit sum, an
    // add wraps at 64 bits and takes the signed and the unsigned ranges
    // (`get_incr`, `erl_bif_counters.c:162-166`), and a put takes the
    // signed range only (`erl_bif_counters.c:221-224`). `memory` counts one
    // cache line of 8 atomics for each scheduler and the base value
    // (`erl_bif_counters.c:95-96`); the run and the pinned ERTS have one
    // scheduler. A reference of an atomic array is not a counter array, and
    // the reverse (`get_ref`, `erl_bif_counters.c:115-126`).
    let program = program();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let info = |memory: i64, size: i64| {
        let pairs = vec![
            (atom("memory"), Term::Integer(memory)),
            (atom("size"), Term::Integer(size)),
        ];
        Term::Map(Map::from_pairs(pairs, program.atoms()).unwrap())
    };
    let error = |reason: &str| Term::tuple(vec![atom("error"), atom(reason)]);
    let mut cases = vec![
        info(160, 1),
        info(288, 9),
        Term::Integer(0),
        atom("ok"),
        Term::Integer(5),
        atom("ok"),
        Term::Integer(3),
        atom("ok"),
        Term::Integer(-1),
        atom("ok"),
        atom("ok"),
        Term::Integer(i64::MIN),
        atom("ok"),
        Term::Integer(i64::MAX),
    ];
    cases.extend(["badarg", "badarg", "badarg", "system_limit", "system_limit"].map(error));
    cases.extend(std::iter::repeat_n(error("badarg"), 10));
    let actual = program
        .execute("atomics_probe", "write_concurrency", &[])
        .unwrap()
        .term;
    assert_eq!(actual, Term::list_from(cases));
    assert_eq!(
        reference("write_concurrency"),
        include_str!("../../fixtures/atomics/write_concurrency.expected").as_bytes()
    );
    // ERTS ends the emulator when the allocation of the largest array
    // fails; PBeam raises `system_limit`, as for the atomic arrays.
    assert_eq!(
        program
            .execute("atomics_probe", "write_concurrency_allocation_failure", &[])
            .unwrap()
            .term,
        error("system_limit")
    );
}

#[test]
fn gives_a_clear_runtime_error_for_the_parts_that_pbeam_does_not_implement() {
    // `atomics:add_get/3` and `atomics:exchange/3` are `atomics` functions
    // that the `counters` module does not call. PBeam does not implement
    // them, and `docs/erts-differences.md` records that each call gives an
    // unresolved MFA error.
    let program = program();
    let error = program
        .execute("atomics_probe", "atomics_only", &[])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unresolved external MFA atomics:add_get/3"),
        "{error}"
    );
}
