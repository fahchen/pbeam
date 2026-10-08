use std::collections::HashMap;

use super::bif::BIFS;
use super::{
    BeamProgram, Outcome, Scheduler, call_arguments, function_at_label, read_operand, write_operand,
};
use crate::{
    BeamModule, Map, Term,
    beam::code::{Instruction, Operand, TestKind},
    term::{Atom, AtomTable, Lambda, test_atom},
};

/// The runtime error of a root process that exits with an uncaught
/// `error:badarg`. PBeam gives the stack trace `[]`.
const BADARG_EXIT: &str = "root process exited with {badarg,[]}";

fn make_program(body: Vec<Instruction>, function_arity: u32) -> BeamProgram {
    let mut instructions = vec![
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "main".into(),
            arity: function_arity,
        },
        Instruction::Label(1),
        Instruction::Label(2),
    ];
    instructions.extend(body);
    BeamProgram {
        modules: HashMap::from([(
            "fixture".into(),
            BeamModule {
                name: "fixture".into(),
                exports: HashMap::from([(("main".into(), 0), 2)]),
                instructions,
            },
        )]),
        atoms: AtomTable::new(),
    }
}

fn erlang_call(name: &str, arity: u32, tail: bool) -> Instruction {
    Instruction::CallExternal {
        arity,
        module: "erlang".into(),
        name: name.into(),
        tail,
        deallocate: 0,
    }
}

fn atom(name: &str) -> Operand {
    Operand::Value(test_atom(name))
}

/// Run `main/0` with a `child/1` function that calls `erlang:exit/1`.
fn run_with_child(body: Vec<Instruction>) -> Result<Term, crate::BeamError> {
    let mut program = make_program(body, 0);
    let module = program.modules.get_mut("fixture").unwrap();
    module.exports.insert(("child".into(), 1), 11);
    module.instructions.extend([
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "child".into(),
            arity: 1,
        },
        Instruction::Label(11),
        erlang_call("exit", 1, true),
    ]);
    program.execute("fixture", "main", &[])
}

fn spawn_link_child(arguments: Term) -> Vec<Instruction> {
    vec![
        Instruction::Move(atom("fixture"), Operand::X(0)),
        Instruction::Move(atom("child"), Operand::X(1)),
        Instruction::Move(Operand::Value(arguments), Operand::X(2)),
        erlang_call("spawn_link", 3, false),
    ]
}

fn wait_forever() -> Vec<Instruction> {
    vec![
        Instruction::Label(5),
        Instruction::LoopReceive {
            fail_label: 6,
            destination: Operand::X(0),
        },
        Instruction::Return,
        Instruction::Label(6),
        Instruction::Wait(5),
    ]
}

#[test]
fn tests_a_fun_with_a_register_arity_as_erl_is_function() {
    // The OTP 29.1.1 compiler emits `bif is_function` for an arity that
    // is not a literal, but the loader accepts `is_function2` with a
    // register arity (`jit/arm/ops.tab:605`). The JIT then calls
    // `erl_is_function` and passes only on `true`
    // (`jit/arm/instr_common.cpp:1266-1283`), so a `badarg` arity fails
    // the test and raises nothing.
    let run = |arity| {
        let lambda = Lambda::new(Atom::from_static("erlang").unwrap(), 0, 0, 2, 1);
        make_program(
            vec![
                Instruction::MakeFun {
                    lambda,
                    free: Vec::new(),
                    destination: Operand::X(0),
                },
                Instruction::Move(Operand::Value(arity), Operand::X(1)),
                Instruction::TestFunctionArity {
                    fail_label: 3,
                    value: Operand::X(0),
                    arity: Operand::X(1),
                },
                Instruction::Move(atom("pass"), Operand::X(0)),
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Move(atom("fail"), Operand::X(0)),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
        .unwrap()
    };
    assert_eq!(run(Term::Integer(1)), test_atom("pass"));
    for arity in [Term::Integer(0), Term::Integer(-1), test_atom("a")] {
        assert_eq!(run(arity), test_atom("fail"));
    }
}

#[test]
fn calls_an_external_fun_of_a_missing_function_as_call_ext() {
    // A call of an external fun goes to the export entry of its
    // function, as `call_ext` does (`beam_common.c:1605-1731`). ERTS 29.1.1
    // raises `undef` for `(erlang:make_fun(missing, run, 0))()`; PBeam
    // gives the runtime error of the row "Exception raise sites".
    let program = make_program(
        vec![
            Instruction::Move(atom("missing"), Operand::X(0)),
            Instruction::Move(atom("run"), Operand::X(1)),
            Instruction::Move(Operand::Value(Term::Integer(0)), Operand::X(2)),
            erlang_call("make_fun", 3, false),
            Instruction::CallFun {
                arity: 0,
                function: Operand::X(0),
                tail: false,
                deallocate: 0,
            },
            Instruction::Return,
        ],
        0,
    );
    assert_eq!(
        program
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "unresolved external MFA missing:run/0"
    );
}

#[test]
fn replaces_the_caller_frame_for_a_fun_call_in_a_tail_position() {
    // `fun_probe:tail_wait/1` ends with `call_fun 0`, `deallocate 1`, and
    // `return`, which ERTS runs as `i_call_fun_last`
    // (`jit/arm/ops.tab:868`). The lambda replaces the frame of
    // `tail_wait/1`, so the process that waits in the lambda has one
    // frame, and a loop of tail fun calls uses constant stack, as in ERTS.
    // This test loads a `.beam` file but stays a unit test: it reads the
    // frames of the scheduler, which `tests/` cannot see.
    let program =
        BeamProgram::load(&[include_bytes!("../../../fixtures/funs/fun_probe.beam")]).unwrap();
    let label = program.modules["fun_probe"].exports[&("tail_wait".to_owned(), 1)];
    let mut scheduler = Scheduler::new(program.atoms.clone());
    let frame = program.frame("fun_probe", label, 1).unwrap();
    scheduler.start(frame, vec![Term::Integer(1)]).unwrap();
    scheduler.schedule_next().unwrap();
    assert!(matches!(
        program.run(&mut scheduler).unwrap(),
        Outcome::Wait
    ));
    let frames = &scheduler.current_process_mut().frames;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].function, "-tail_wait/1-fun-0-");
}

#[test]
fn requires_the_frame_size_for_a_fun_call_in_a_tail_position() {
    // `i_call_fun_last` releases exactly the Y registers of the frame.
    let program = make_program(
        vec![
            Instruction::Move(atom("notfun"), Operand::X(0)),
            Instruction::CallFun {
                arity: 0,
                function: Operand::X(0),
                tail: true,
                deallocate: 1,
            },
            Instruction::Return,
        ],
        0,
    );
    assert_eq!(
        program
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "invalid BEAM tail-call stack size"
    );
}

#[test]
fn exits_the_process_with_an_uncaught_badfun() {
    // A fun call that no `try` or `catch` handles exits the process with
    // `{{badfun, Term}, Stack}`, as ERTS `call_fun` raises it
    // (`beam_common.c:1617-1621`). PBeam gives the stack trace `[]`.
    let program = make_program(
        vec![
            Instruction::Move(atom("notfun"), Operand::X(0)),
            Instruction::CallFun {
                arity: 0,
                function: Operand::X(0),
                tail: false,
                deallocate: 0,
            },
            Instruction::Return,
        ],
        0,
    );
    assert_eq!(
        program
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "root process exited with {{badfun,notfun},[]}"
    );
}

#[test]
fn reports_process_exits_and_link_exits() {
    let error = |body| run_with_child(body).unwrap_err().to_string();
    assert!(
        error(vec![
            Instruction::Move(atom("boom"), Operand::X(0)),
            erlang_call("exit", 1, false),
        ])
        .contains("root process exited with boom")
    );
    assert!(
        error(vec![
            Instruction::Move(atom("undef"), Operand::X(0)),
            erlang_call("nif_error", 1, false),
        ])
        .contains("root process exited with {undef,[]}")
    );
    let mut body = spawn_link_child(Term::List(vec![Term::Integer(1)]));
    body.extend(wait_forever());
    // The link exit signal of the child ends the root process.
    assert_eq!(error(body), "root process exited with 1");
    let mut body = spawn_link_child(Term::List(vec![test_atom("normal")]));
    body.extend(wait_forever());
    assert!(error(body).contains("waits for a message that cannot arrive"));
    let mut body = spawn_link_child(Term::List(vec![Term::Integer(1)]));
    body.push(Instruction::Return);
    assert!(matches!(run_with_child(body), Ok(Term::Pid(_))));
    // ERTS 29.1.1: spawn_link(M, F, 1) raises badarg.
    assert_eq!(error(spawn_link_child(Term::Integer(1))), BADARG_EXIT);
    // `erlang:is_alive/0` is not a BIF: it runs the code of the `erlang`
    // module, which this program does not load.
    assert!(
        error(vec![erlang_call("is_alive", 0, false)])
            .contains("unresolved external MFA erlang:is_alive/0")
    );
    // An external call of `erlang:yield/0` that the loader did not turn
    // into `Instruction::Yield`, as through `apply`, runs the code of the
    // `erlang` module, as in ERTS.
    assert!(
        error(vec![erlang_call("yield", 0, false)])
            .contains("unresolved external MFA erlang:yield/0")
    );
}

#[test]
fn spawns_with_link_and_monitor_options_and_rejects_other_options() {
    let spawn_opt_body = |reason: Term, options: Vec<Term>| {
        let mut body = spawn_link_child(Term::List(vec![reason]));
        body.pop();
        body.push(Instruction::Move(
            Operand::Value(Term::List(options)),
            Operand::X(3),
        ));
        body.push(erlang_call("spawn_opt", 4, false));
        body
    };
    let spawn_opt = |options: Vec<Term>| {
        let mut body = spawn_opt_body(test_atom("normal"), options);
        body.push(Instruction::Return);
        run_with_child(body)
    };
    // A link sends the abnormal exit of the child to the parent as an
    // exit signal, and a monitor sends it as a `'DOWN'` message.
    let wait_for_abnormal_child = |options: Vec<Term>| {
        let mut body = spawn_opt_body(Term::Integer(1), options);
        body.extend(wait_forever());
        run_with_child(body)
    };
    assert_eq!(
        wait_for_abnormal_child(vec![test_atom("link")])
            .unwrap_err()
            .to_string(),
        "root process exited with 1"
    );
    assert!(matches!(
        wait_for_abnormal_child(vec![test_atom("monitor")]),
        Ok(Term::Tuple(ref elements))
            if elements.first() == Some(&test_atom("DOWN"))
    ));
    assert!(
        wait_for_abnormal_child(vec![])
            .unwrap_err()
            .to_string()
            .contains("waits for a message that cannot arrive")
    );
    assert!(matches!(
        spawn_opt(vec![test_atom("link"), test_atom("monitor")]),
        Ok(Term::Tuple(ref elements))
            if matches!(elements.as_slice(), [Term::Pid(_), Term::Reference(_)])
    ));
    assert!(matches!(spawn_opt(vec![]), Ok(Term::Pid(_))));
    assert!(
        spawn_opt(vec![test_atom("priority")])
            .unwrap_err()
            .to_string()
            .contains("unsupported spawn option Atom(\"priority\")")
    );
}

#[test]
fn returns_a_tail_called_bif_value_and_drops_messages_to_missing_processes() {
    assert!(matches!(
        run_with_child(vec![erlang_call("make_ref", 0, true), Instruction::Return]),
        Ok(Term::Reference(_))
    ));
    assert_eq!(
        run_with_child(vec![
            Instruction::Move(
                Operand::Value(Term::Pid(crate::ProcessId(99))),
                Operand::X(0)
            ),
            Instruction::Move(atom("lost"), Operand::X(1)),
            Instruction::Send,
            Instruction::Return,
        ])
        .unwrap(),
        test_atom("lost")
    );
}

#[test]
fn handles_tuple_tests_timeouts_and_unbound_receive_markers() {
    assert_eq!(
        run_with_child(vec![
            Instruction::PutTuple {
                elements: vec![atom("a")],
                destination: Operand::X(0),
            },
            Instruction::TestTaggedTuple {
                fail_label: 3,
                value: Operand::X(0),
                arity: 2,
                tag: test_atom("a"),
            },
            Instruction::Label(3),
            Instruction::TestExactEqual {
                fail_label: 4,
                left: atom("a"),
                right: atom("b"),
            },
            Instruction::Label(4),
            Instruction::Timeout,
            Instruction::ReceiveMarkerBind {
                marker: atom("none"),
                reference: atom("none"),
            },
            Instruction::ReceiveMarkerUse(Operand::Value(Term::Reference(crate::Reference(9)))),
            Instruction::Return,
        ])
        .unwrap(),
        Term::Tuple(vec![test_atom("a")])
    );
}

#[test]
fn rejects_invalid_tuple_access_receives_and_bifs() {
    let error = |body| run_with_child(body).unwrap_err().to_string();
    let element = |tuple, index| Instruction::GetTupleElement {
        tuple: Operand::Value(tuple),
        index,
        destination: Operand::X(0),
    };
    assert!(error(vec![Instruction::LoopReceiveEnd(1)]).contains("loop_rec_end"));
    assert!(error(vec![Instruction::RemoveMessage]).contains("remove_message"));
    assert!(error(vec![element(Term::Integer(1), 0)]).contains("not a tuple"));
    assert!(error(vec![element(Term::Tuple(vec![]), 0)]).contains("no element 0"));
    assert!(
        error(vec![Instruction::Bif {
            fail_label: 0,
            module: "erlang".into(),
            name: "date".into(),
            arguments: Vec::new(),
            destination: Operand::X(0),
        }])
        .contains("unresolved external MFA erlang:date/0")
    );
    assert!(
        error(vec![Instruction::Bif {
            fail_label: 0,
            module: "lists".into(),
            name: "node".into(),
            arguments: Vec::new(),
            destination: Operand::X(0),
        }])
        .contains("unsupported BIF lists:node/0")
    );
}

#[test]
fn rejects_missing_modules_exports_and_function_arities() {
    let program = make_program(vec![Instruction::Return], 0);
    assert!(
        program
            .execute("missing", "main", &[])
            .unwrap_err()
            .to_string()
            .contains("missing BEAM module")
    );
    assert!(
        program
            .execute("fixture", "missing", &[])
            .unwrap_err()
            .to_string()
            .contains("missing export")
    );
    assert!(
        make_program(vec![Instruction::Return], 1)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
            .contains("arity does not match")
    );
}

#[test]
fn executes_a_loaded_function_and_returns_x0() {
    let program = make_program(
        vec![
            Instruction::Move(Operand::Value(Term::Integer(7)), Operand::X(0)),
            Instruction::Return,
        ],
        0,
    );

    assert_eq!(
        program.execute("fixture", "main", &[]).unwrap(),
        Term::Integer(7)
    );
}

#[test]
fn recognizes_runtime_process_identifiers_as_pids() {
    use crate::ProcessId;

    let mut program = make_program(
        vec![
            Instruction::Test {
                kind: crate::beam::code::TestKind::Pid,
                fail_label: 3,
                value: Operand::X(0),
            },
            Instruction::Move(Operand::Value(test_atom("pid")), Operand::X(0)),
            Instruction::Return,
            Instruction::Label(3),
            Instruction::Move(Operand::Value(test_atom("other")), Operand::X(0)),
            Instruction::Return,
        ],
        1,
    );
    program
        .modules
        .get_mut("fixture")
        .unwrap()
        .exports
        .insert(("main".into(), 1), 2);

    assert_eq!(
        program
            .execute("fixture", "main", &[Term::Pid(ProcessId(1))])
            .unwrap(),
        test_atom("pid")
    );
    assert_eq!(
        program
            .execute("fixture", "main", &[Term::Integer(1)])
            .unwrap(),
        test_atom("other")
    );
}

#[test]
fn updates_and_reads_maps_and_reports_unsupported_map_operands() {
    let map = |pairs: Vec<(&str, i64)>| {
        let pairs = pairs
            .into_iter()
            .map(|(key, value)| (test_atom(key), Term::Integer(value)))
            .collect();
        Operand::Value(Term::Map(Map::from_pairs(pairs).unwrap()))
    };
    let run = |body: Vec<Instruction>| make_program(body, 0).execute("fixture", "main", &[]);
    // Replace an existing key, then read it back.
    assert_eq!(
        run(vec![
            Instruction::PutMapAssoc {
                map: map(vec![("a", 1), ("b", 2)]),
                destination: Operand::X(0),
                pairs: vec![(atom("b"), Operand::Value(Term::Integer(3)))],
            },
            Instruction::GetMapElements {
                fail_label: 3,
                map: Operand::X(0),
                pairs: vec![(atom("b"), Operand::X(0))],
            },
            Instruction::Return,
            Instruction::Label(3),
            Instruction::Move(atom("missing"), Operand::X(0)),
            Instruction::Return,
        ])
        .unwrap(),
        Term::Integer(3)
    );
    // A missing key jumps to the fail label.
    assert_eq!(
        run(vec![
            Instruction::GetMapElements {
                fail_label: 3,
                map: map(vec![("a", 1)]),
                pairs: vec![(atom("a"), Operand::X(1)), (atom("b"), Operand::X(0))],
            },
            Instruction::Return,
            Instruction::Label(3),
            Instruction::Move(atom("missing"), Operand::X(0)),
            Instruction::Return,
        ])
        .unwrap(),
        test_atom("missing")
    );
    let error = |body| run(body).unwrap_err().to_string();
    assert!(
        error(vec![Instruction::GetMapElements {
            fail_label: 3,
            map: atom("a"),
            pairs: vec![],
        }])
        .contains("get_map_elements source is not a map")
    );
    assert!(
        error(vec![Instruction::PutMapAssoc {
            map: atom("a"),
            destination: Operand::X(0),
            pairs: vec![],
        }])
        .contains("put_map_assoc source: not a map")
    );
    assert!(
        error(vec![Instruction::PutMapAssoc {
            map: map(vec![]),
            destination: Operand::X(0),
            pairs: vec![(
                Operand::Value(Term::Tuple(vec![])),
                Operand::Value(Term::Integer(1))
            )],
        }])
        .contains("unsupported map key Tuple([])")
    );
}

#[test]
fn tests_map_keys_with_has_map_fields_and_writes_no_register() {
    let map = Operand::Value(Term::Map(
        Map::from_pairs(vec![
            (test_atom("input"), test_atom("cooked")),
            (Term::Integer(1), test_atom("one")),
        ])
        .unwrap(),
    ));
    let run = |keys: Vec<Operand>, map: Operand| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(1)),
                Instruction::Move(atom("none"), Operand::X(0)),
                Instruction::HasMapFields {
                    fail_label: 3,
                    map,
                    keys,
                },
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Move(atom("missing"), Operand::X(0)),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    // All keys present: x0 keeps its value.
    assert_eq!(
        run(vec![atom("input"), Operand::X(1)], map.clone()).unwrap(),
        test_atom("none")
    );
    assert_eq!(
        run(vec![atom("input"), atom("output")], map).unwrap(),
        test_atom("missing")
    );
    assert!(
        run(vec![atom("input")], atom("raw"))
            .unwrap_err()
            .to_string()
            .contains("get_map_elements source is not a map")
    );
}

#[test]
fn raises_or_jumps_when_put_map_exact_gets_no_map_or_no_key() {
    // erlc puts an `is_map` test before each `put_map_exact`, so compiled
    // code does not reach this case. ERTS still raises `{badmap, Map}`
    // with fail label 0 and jumps to another fail label
    // (`beam_common.c:2231-2234`).
    let run = |fail_label, map| {
        make_program(
            vec![
                Instruction::PutMapExact {
                    fail_label,
                    map,
                    destination: Operand::X(0),
                    pairs: vec![(atom("output"), atom("cooked"))],
                },
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Move(atom("failed"), Operand::X(0)),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    assert_eq!(
        run(0, atom("foo")).unwrap_err().to_string(),
        "root process exited with {{badmap,foo},[]}"
    );
    assert_eq!(run(3, atom("foo")).unwrap(), test_atom("failed"));
    // A key of a type that a PBeam map cannot hold is a missing key.
    let tuple_key = |fail_label| {
        make_program(
            vec![
                Instruction::PutMapExact {
                    fail_label,
                    map: Operand::Value(Term::Map(Map::new())),
                    destination: Operand::X(0),
                    pairs: vec![(Operand::Value(Term::Tuple(vec![])), atom("cooked"))],
                },
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Move(atom("failed"), Operand::X(0)),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    assert_eq!(
        tuple_key(0).unwrap_err().to_string(),
        "root process exited with {{badkey,{}},[]}"
    );
    assert_eq!(tuple_key(3).unwrap(), test_atom("failed"));
    // Two different references have no PBeam order.
    let references = make_program(
        vec![Instruction::PutMapExact {
            fail_label: 0,
            map: Operand::Value(Term::Map(Map::new())),
            destination: Operand::X(0),
            pairs: vec![
                (
                    Operand::Value(Term::Reference(crate::Reference(1))),
                    atom("a"),
                ),
                (
                    Operand::Value(Term::Reference(crate::Reference(2))),
                    atom("b"),
                ),
            ],
        }],
        0,
    )
    .execute("fixture", "main", &[]);
    assert!(
        references
            .unwrap_err()
            .to_string()
            .contains("unsupported term order of two references")
    );
}

#[test]
fn reports_a_maps_merge_result_key_that_the_map_form_does_not_support() {
    // A flatmap accepts the integer 2^59, and a hashmap does not yet.
    let pairs = |keys: std::ops::RangeInclusive<i64>| {
        let pairs = keys.map(|key| (Term::Integer(key), Term::Integer(0)));
        Term::Map(Map::from_pairs(pairs.collect()).unwrap())
    };
    let merge = make_program(
        vec![
            Instruction::Move(Operand::Value(pairs(1 << 59..=1 << 59)), Operand::X(0)),
            Instruction::Move(Operand::Value(pairs(1..=32)), Operand::X(1)),
            Instruction::CallExternal {
                arity: 2,
                module: "maps".into(),
                name: "merge".into(),
                tail: true,
                deallocate: 0,
            },
            Instruction::Return,
        ],
        0,
    )
    .execute("fixture", "main", &[]);
    assert_eq!(
        merge.unwrap_err().to_string(),
        "unsupported map key Integer(576460752303423488)"
    );
}

#[test]
fn reports_a_maps_from_list_key_that_the_map_form_does_not_support() {
    // ERTS accepts a tuple key; a PBeam map does not hold one yet.
    let pair = Term::Tuple(vec![Term::Tuple(vec![]), Term::Integer(0)]);
    let from_list = make_program(
        vec![
            Instruction::Move(Operand::Value(Term::List(vec![pair])), Operand::X(0)),
            Instruction::CallExternal {
                arity: 1,
                module: "maps".into(),
                name: "from_list".into(),
                tail: true,
                deallocate: 0,
            },
            Instruction::Return,
        ],
        0,
    )
    .execute("fixture", "main", &[]);
    assert_eq!(
        from_list.unwrap_err().to_string(),
        "unsupported map key Tuple([])"
    );
}

#[test]
fn runs_maps_keys_as_a_bif_and_reports_unsupported_arguments() {
    let keys = |argument: Term| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(argument), Operand::X(0)),
                Instruction::CallExternal {
                    arity: 1,
                    module: "maps".into(),
                    name: "keys".into(),
                    tail: true,
                    deallocate: 0,
                },
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    let map = |keys: Vec<Term>| {
        let pairs = keys
            .into_iter()
            .map(|key| (key, Term::Integer(0)))
            .collect();
        Term::Map(Map::from_pairs(pairs).unwrap())
    };
    let atom = Term::static_atom;
    // ERTS 29.1.1: maps:keys(#{a => V, 2 => V, 1 => V}) is [1,2,a].
    let a = test_atom("a");
    assert_eq!(
        keys(map(vec![a.clone(), Term::Integer(2), Term::Integer(1)])).unwrap(),
        Term::List(vec![Term::Integer(1), Term::Integer(2), a])
    );
    // ERTS 29.1.1: maps:keys(#{true => V, abs_path => V, 1 => V}) is [1,true,abs_path].
    assert_eq!(
        keys(map(vec![atom("abs_path"), atom("true"), Term::Integer(1)])).unwrap(),
        Term::List(vec![Term::Integer(1), atom("true"), atom("abs_path")])
    );
    assert_eq!(
        keys(atom("ok")).unwrap_err().to_string(),
        "unsupported maps:keys/1 argument: not a map"
    );
}

#[test]
fn runs_a_table_bif_before_a_loaded_module_that_exports_the_same_mfa() {
    let mut program = make_program(
        vec![
            Instruction::Move(
                Operand::Value(Term::Map(Map::from_pairs(Vec::new()).unwrap())),
                Operand::X(0),
            ),
            Instruction::CallExternal {
                arity: 1,
                module: "maps".into(),
                name: "keys".into(),
                tail: true,
                deallocate: 0,
            },
            Instruction::Return,
        ],
        0,
    );
    program.modules.insert(
        "maps".into(),
        BeamModule {
            name: "maps".into(),
            exports: HashMap::from([(("keys".into(), 1), 1)]),
            instructions: vec![
                Instruction::FunctionInfo {
                    module: "maps".into(),
                    name: "keys".into(),
                    arity: 1,
                },
                Instruction::Label(1),
                Instruction::Move(atom("module_code"), Operand::X(0)),
                Instruction::Return,
            ],
        },
    );
    // The BIF returns the keys of the empty map. The module code would
    // return `module_code`.
    assert_eq!(
        program.execute("fixture", "main", &[]).unwrap(),
        Term::List(Vec::new())
    );
}

#[test]
fn never_runs_the_stub_of_a_table_bif_that_pbeam_does_not_implement() {
    let mut program = make_program(
        vec![
            Instruction::Move(atom("a"), Operand::X(0)),
            Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(1)),
            Instruction::Move(Operand::Value(Term::List(Vec::new())), Operand::X(2)),
            Instruction::CallExternal {
                arity: 3,
                module: "lists".into(),
                name: "keysearch".into(),
                tail: true,
                deallocate: 0,
            },
            Instruction::Return,
        ],
        0,
    );
    program.modules.insert(
        "lists".into(),
        BeamModule {
            name: "lists".into(),
            exports: HashMap::from([(("keysearch".into(), 3), 1)]),
            instructions: vec![
                Instruction::FunctionInfo {
                    module: "lists".into(),
                    name: "keysearch".into(),
                    arity: 3,
                },
                Instruction::Label(1),
                Instruction::Move(atom("stub"), Operand::X(0)),
                Instruction::Return,
            ],
        },
    );
    // `lists:keysearch/3` is in the ERTS 29.1.1 `bif.tab`. ERTS runs the
    // BIF, so the BEAM stub body of the loaded `lists` module must not run.
    assert_eq!(
        program
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "unresolved external MFA lists:keysearch/3"
    );
    // `is_bif` finds an entry with a binary search.
    // Strictly increasing: sorted for a binary search, with no duplicate entry.
    assert!(BIFS.windows(2).all(|pair| pair[0] < pair[1]));
}

/// The BIF table holds the BIFs of the pinned ERTS.
/// `fixtures/bifs/is_builtin.txt` lists each exported function of the
/// OTP 29.1.1 installation for which `erlang:is_builtin/3` returns
/// `true`, and `tests/bifs.rs` checks this list on the pinned ERTS. The
/// list has each table entry and no other function, except
/// `erlang:apply/2`, `erlang:apply/3`, and `erlang:yield/0`.
/// `erts_is_builtin` (`beam_common.c`) marks these three, but they are
/// not `bif.tab` entries: the loader turns a call to them into the
/// `i_apply_fun`, `i_apply`, and `i_yield` instructions (`ops.tab`).
#[test]
fn bif_table_equals_erts_is_builtin() {
    const INSTRUCTION_CALLS: [(&str, &str, u32); 3] = [
        ("erlang", "apply", 2),
        ("erlang", "apply", 3),
        ("erlang", "yield", 0),
    ];
    let mut builtin: Vec<(&str, &str, u32)> = include_str!("../../../fixtures/bifs/is_builtin.txt")
        .lines()
        .map(|line| {
            let mut field = line.split('\t');
            (
                field.next().unwrap(),
                field.next().unwrap(),
                field.next().unwrap().parse().unwrap(),
            )
        })
        .collect();
    for mfa in INSTRUCTION_CALLS {
        assert!(builtin.contains(&mfa));
    }
    builtin.retain(|mfa| !INSTRUCTION_CALLS.contains(mfa));
    assert_eq!(BIFS, builtin.as_slice());
}

#[test]
fn converts_unicode_chardata_and_rejects_other_encodings() {
    let convert = |chardata: Term, encoding: &str| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(chardata), Operand::X(0)),
                Instruction::Move(Operand::Value(Term::static_atom(encoding)), Operand::X(1)),
                Instruction::CallExternal {
                    arity: 2,
                    module: "unicode".into(),
                    name: "characters_to_binary".into(),
                    tail: true,
                    deallocate: 0,
                },
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    let chardata = || Term::List(vec![Term::Binary(b"Hello".to_vec()), Term::Integer(10)]);
    assert_eq!(
        convert(chardata(), "unicode").unwrap(),
        Term::Binary(b"Hello\n".to_vec())
    );
    // ERTS 29.1.1 raises badarg for a term that is not chardata.
    assert_eq!(
        convert(Term::Integer(1), "unicode")
            .unwrap_err()
            .to_string(),
        BADARG_EXIT
    );
    // ERTS 29.1.1: unicode:characters_to_binary([<<"Hello">>, 10], utf8)
    // is <<"Hello\n">>.
    assert_eq!(
        convert(chardata(), "utf8").unwrap(),
        Term::Binary(b"Hello\n".to_vec())
    );
    assert_eq!(
        convert(chardata(), "latin1").unwrap_err().to_string(),
        "unsupported unicode:characters_to_binary/2 encoding Atom(\"latin1\")"
    );
}

#[test]
fn builds_a_proper_and_an_improper_list() {
    let list = make_program(
        vec![
            Instruction::PutList {
                head: Operand::Value(Term::Integer(10)),
                tail: Operand::Value(Term::List(Vec::new())),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
        0,
    );
    assert_eq!(
        list.execute("fixture", "main", &[]).unwrap(),
        Term::List(vec![Term::Integer(10)])
    );

    // ERTS: [10 | 0] is the improper list [10|0].
    let improper = make_program(
        vec![
            Instruction::PutList {
                head: Operand::Value(Term::Integer(10)),
                tail: Operand::Value(Term::Integer(0)),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
        0,
    );
    assert_eq!(
        improper.execute("fixture", "main", &[]).unwrap(),
        Term::ImproperList(vec![Term::Integer(10)], Box::new(Term::Integer(0)))
    );
}

/// The values come from a pinned OTP 29.1.1 run of `lst/1`, `hd1/1`, and
/// `nil/1` in a module compiled with `erlc +no_line_info`:
/// `lst([H | T]) -> {H, T}.`, `hd1([H | _]) -> H.`, and
/// `nil([]) -> nil; nil([_ | _]) -> nonempty; nil(_) -> other.`
#[test]
fn reads_and_tests_lists_as_pinned_erts() {
    let list = |names: &[&str]| Term::List(names.iter().map(|name| test_atom(name)).collect());
    let run = |argument: Term, body: Vec<Instruction>| {
        let mut instructions = vec![Instruction::Move(Operand::Value(argument), Operand::X(0))];
        instructions.extend(body);
        make_program(instructions, 0).execute("fixture", "main", &[])
    };
    // lst/1 compiles to {get_list,{x,0},{x,1},{x,0}}, then put_tuple2.
    let get_list = |argument: Term| {
        run(
            argument,
            vec![
                Instruction::GetList {
                    source: Operand::X(0),
                    head: Operand::X(1),
                    tail: Operand::X(0),
                },
                Instruction::PutTuple {
                    elements: vec![Operand::X(1), Operand::X(0)],
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
        )
    };
    // ERTS: lst([a,b,c]) is {a,[b,c]}, and lst([a]) is {a,[]}.
    assert_eq!(
        get_list(list(&["a", "b", "c"])).unwrap(),
        Term::Tuple(vec![test_atom("a"), list(&["b", "c"])])
    );
    assert_eq!(
        get_list(list(&["a"])).unwrap(),
        Term::Tuple(vec![test_atom("a"), list(&[])])
    );
    // ERTS: hd1([x,y]) is x.
    let get_head = |argument: Term| {
        run(
            argument,
            vec![
                Instruction::GetHead {
                    source: Operand::X(0),
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
        )
    };
    assert_eq!(get_head(list(&["x", "y"])).unwrap(), test_atom("x"));
    // ERTS: lst([a|x]) is {a,x}, lst([a,b|x]) is {a,[b|x]}, and
    // hd1([x|y]) is x.
    let improper = |names: &[&str], tail: &str| {
        Term::ImproperList(
            names.iter().map(|name| test_atom(name)).collect(),
            Box::new(test_atom(tail)),
        )
    };
    assert_eq!(
        get_list(improper(&["a"], "x")).unwrap(),
        Term::Tuple(vec![test_atom("a"), test_atom("x")])
    );
    assert_eq!(
        get_list(improper(&["a", "b"], "x")).unwrap(),
        Term::Tuple(vec![test_atom("a"), improper(&["b"], "x")])
    );
    assert_eq!(get_head(improper(&["x"], "y")).unwrap(), test_atom("x"));
    // ERTS: nil([]) is nil, nil([a]) is nonempty, and nil(a) is other.
    let classify = |argument: Term| {
        run(
            argument,
            vec![
                Instruction::Test {
                    kind: TestKind::Nil,
                    fail_label: 3,
                    value: Operand::X(0),
                },
                Instruction::Move(atom("nil"), Operand::X(0)),
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Test {
                    kind: TestKind::NonEmptyList,
                    fail_label: 4,
                    value: Operand::X(0),
                },
                Instruction::Move(atom("nonempty"), Operand::X(0)),
                Instruction::Return,
                Instruction::Label(4),
                Instruction::Move(atom("other"), Operand::X(0)),
                Instruction::Return,
            ],
        )
        .unwrap()
    };
    assert_eq!(classify(list(&[])), test_atom("nil"));
    assert_eq!(classify(list(&["a"])), test_atom("nonempty"));
    assert_eq!(classify(test_atom("a")), test_atom("other"));
    // ERTS: nil([a|x]) is nonempty.
    assert_eq!(classify(improper(&["a"], "x")), test_atom("nonempty"));
    // The BEAM validator makes sure that the source is a nonempty list.
    assert_eq!(
        get_list(list(&[])).unwrap_err().to_string(),
        "BEAM list source List([]) is not a nonempty list"
    );
    assert_eq!(
        get_head(test_atom("a")).unwrap_err().to_string(),
        "BEAM list source Atom(\"a\") is not a nonempty list"
    );
}

/// The value comes from a pinned OTP 29.1.1 run of `rec/2` with
/// `-record(r, {a, b, c})`: `rec(#r{a = 1, b = 2, c = 3}, 9)` is
/// `{r,1,9,1}`. `rec(R, X) -> R#r{b = X, c = R#r.a}.` compiles to
/// `{get_tuple_element,{x,0},1,{x,2}}` and
/// `{update_record,{atom,copy},4,{tr,{x,0},_},{x,0},{list,[3,{x,1},4,{x,2}]}}`.
#[test]
fn updates_record_positions_as_pinned_erts() {
    let record = || {
        Operand::Value(Term::Tuple(vec![
            test_atom("r"),
            Term::Integer(1),
            Term::Integer(2),
            Term::Integer(3),
        ]))
    };
    let run = |destination: Operand, updates: Vec<(u32, Operand)>| {
        make_program(
            vec![
                Instruction::Move(record(), Operand::X(0)),
                Instruction::Move(Operand::Value(Term::Integer(9)), Operand::X(1)),
                Instruction::GetTupleElement {
                    tuple: Operand::X(0),
                    index: 1,
                    destination: Operand::X(2),
                },
                Instruction::UpdateRecord {
                    source: Operand::X(0),
                    destination: destination.clone(),
                    updates,
                },
                Instruction::Move(destination, Operand::X(0)),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    assert_eq!(
        run(Operand::X(0), vec![(3, Operand::X(1)), (4, Operand::X(2))]).unwrap(),
        Term::Tuple(vec![
            test_atom("r"),
            Term::Integer(1),
            Term::Integer(9),
            Term::Integer(1),
        ])
    );
    // The destination x1 is also a value register. The update uses the
    // value of x1 before the write, as the ERTS `SCRATCH_X_REG` rewrite does.
    assert_eq!(
        run(Operand::X(1), vec![(2, Operand::X(2)), (3, Operand::X(1))]).unwrap(),
        Term::Tuple(vec![
            test_atom("r"),
            Term::Integer(1),
            Term::Integer(9),
            Term::Integer(3),
        ])
    );
    for position in [0, 5] {
        assert_eq!(
            run(Operand::X(0), vec![(position, Operand::X(1))])
                .unwrap_err()
                .to_string(),
            format!("BEAM update_record has no position {position}")
        );
    }
    assert_eq!(
        make_program(
            vec![Instruction::UpdateRecord {
                source: atom("r"),
                destination: Operand::X(0),
                updates: Vec::new(),
            }],
            0,
        )
        .execute("fixture", "main", &[])
        .unwrap_err()
        .to_string(),
        "BEAM update_record source is not a tuple"
    );
}

/// ERTS 29.1.1 `beam_jit_wait_timeout` (`jit/beam_jit_common.cpp:1208-1240`):
/// `infinity` waits as `wait` does, so the `DOWN` message of a monitored
/// child continues the receive at its label. `0` continues at once at
/// `timeout`. A value that is not valid and that no `try` catches exits
/// the process with `{timeout_value, Stack}`.
#[test]
fn waits_or_continues_at_a_receive_timeout_and_raises_timeout_value() {
    let wait = |timeout: Term| {
        let mut body = spawn_link_child(Term::List(vec![Term::Integer(1)]));
        body.pop();
        body.extend([
            Instruction::Move(
                Operand::Value(Term::List(vec![test_atom("monitor")])),
                Operand::X(3),
            ),
            erlang_call("spawn_opt", 4, false),
            Instruction::Label(5),
            Instruction::LoopReceive {
                fail_label: 6,
                destination: Operand::X(0),
            },
            Instruction::RemoveMessage,
            Instruction::Return,
            Instruction::Label(6),
            Instruction::WaitTimeout {
                label: 5,
                source: Operand::Value(timeout),
            },
            Instruction::Timeout,
            Instruction::Move(atom("timeout"), Operand::X(0)),
            Instruction::Return,
        ]);
        run_with_child(body)
    };
    assert!(matches!(
        wait(test_atom("infinity")),
        Ok(Term::Tuple(ref fields))
            if fields[0] == test_atom("DOWN")
                && fields[2] == test_atom("process")
                && fields[4] == Term::Integer(1)
    ));
    assert_eq!(wait(Term::Integer(0)).unwrap(), test_atom("timeout"));
    assert_eq!(
        wait(test_atom("foo")).unwrap_err().to_string(),
        "root process exited with {timeout_value,[]}"
    );
}

#[test]
fn rejects_call_stack_and_return_errors() {
    assert!(
        make_program(
            vec![
                Instruction::Allocate { stack_size: 1 },
                Instruction::Deallocate { stack_size: 0 }
            ],
            0
        )
        .execute("fixture", "main", &[])
        .is_err()
    );
    assert!(
        make_program(
            vec![
                Instruction::Allocate { stack_size: 1 },
                Instruction::Trim {
                    count: 1,
                    remaining: 1
                }
            ],
            0
        )
        .execute("fixture", "main", &[])
        .unwrap_err()
        .to_string()
        .contains("invalid BEAM stack size")
    );
    assert!(
        make_program(
            vec![
                Instruction::Allocate { stack_size: 1 },
                Instruction::CallExternal {
                    arity: 0,
                    module: "absent".into(),
                    name: "run".into(),
                    tail: true,
                    deallocate: 0
                }
            ],
            0
        )
        .execute("fixture", "main", &[])
        .unwrap_err()
        .to_string()
        .contains("tail-call stack size")
    );
    assert!(
        make_program(vec![Instruction::Return], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
            .contains("return before setting x0")
    );
    assert!(
        make_program(vec![Instruction::End], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
            .contains("did not return")
    );
    assert!(
        make_program(vec![Instruction::Unsupported(6)], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
            .contains("unsupported BEAM opcode 6")
    );
    assert!(
        make_program(vec![Instruction::Label(3)], 0)
            .execute("fixture", "main", &[])
            .is_err()
    );
    assert!(
        make_program(vec![Instruction::Call { arity: 1, label: 2 }], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
            .contains("argument x0 is unset")
    );
}

#[test]
fn rejects_unresolved_exports_and_function_labels() {
    let module = BeamModule {
        name: "external".into(),
        exports: HashMap::new(),
        instructions: vec![],
    };
    let program = BeamProgram {
        modules: HashMap::from([("external".into(), module)]),
        atoms: AtomTable::new(),
    };
    assert!(
        program
            .mfa_frame("missing", "run", 0)
            .unwrap_err()
            .to_string()
            .contains("unresolved external MFA missing:run/0")
    );
    assert!(
        program
            .mfa_frame("external", "missing", 0)
            .unwrap_err()
            .to_string()
            .contains("external:missing/0")
    );
    let module = BeamModule {
        name: "fixture".into(),
        exports: HashMap::new(),
        instructions: vec![
            Instruction::FunctionInfo {
                module: "other".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(2),
        ],
    };
    assert!(
        function_at_label(&module, 2)
            .unwrap_err()
            .to_string()
            .contains("no function info")
    );
    assert!(
        function_at_label(&module, 3)
            .unwrap_err()
            .to_string()
            .contains("not found")
    );
}

/// PBeam lets a match context reach only the binary-match instructions
/// and local calls. It gives a runtime invariant error for each other
/// use, and for a binary-match instruction that gets another term.
#[test]
fn gives_an_invariant_error_for_a_match_context_outside_binary_matching() {
    let run = |rest: Vec<Instruction>| {
        let mut body = vec![
            Instruction::Move(Operand::Value(Term::Binary(b"a".to_vec())), Operand::X(0)),
            Instruction::BinaryMatchStart {
                fail_label: 1,
                source: Operand::X(0),
                destination: Operand::X(0),
            },
        ];
        body.extend(rest);
        // `child/1` does not read its argument.
        let mut program = make_program(body, 0);
        let module = program.modules.get_mut("fixture").unwrap();
        module.exports.insert(("child".into(), 1), 11);
        module.instructions.extend([
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: "child".into(),
                arity: 1,
            },
            Instruction::Label(11),
            Instruction::Move(atom("ok"), Operand::X(0)),
            Instruction::Return,
        ]);
        program
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
    };
    let escape = "invariant error: the match context in X(0) reached an instruction that \
                  is not a binary match";
    assert_eq!(
        run(vec![Instruction::Move(Operand::X(0), Operand::X(1))]),
        escape
    );
    assert_eq!(run(vec![erlang_call("hd", 1, false)]), escape);
    // An external call to a loaded function also checks its arguments.
    assert_eq!(
        run(vec![Instruction::CallExternal {
            arity: 1,
            module: "fixture".into(),
            name: "child".into(),
            tail: false,
            deallocate: 0,
        }]),
        escape
    );
    // A reuse in another register would share the position in ERTS.
    assert_eq!(
        run(vec![Instruction::BinaryMatchStart {
            fail_label: 1,
            source: Operand::X(0),
            destination: Operand::X(1),
        }]),
        "unsupported bs_start_match3 that copies the match context in X(0) to X(1)"
    );
    assert_eq!(
        run(vec![Instruction::BinarySetPosition {
            context: Operand::X(0),
            position: atom("a"),
        }]),
        "invariant error: bs_set_position position Atom(\"a\") is not a position"
    );
    assert_eq!(
        run(vec![Instruction::BinaryGetUtf8 {
            fail_label: 1,
            context: atom("a"),
            destination: Operand::X(1),
        }]),
        "invariant error: Value(Atom(\"a\")) holds Atom(\"a\"), not a match context"
    );
}

#[test]
fn handles_register_values_and_reports_invalid_registers() {
    let mut registers = vec![Some(Term::Integer(7))];
    assert_eq!(
        read_operand(&Operand::X(0), &registers, &[]).unwrap(),
        Term::Integer(7)
    );
    assert!(read_operand(&Operand::X(1), &registers, &[]).is_err());
    assert!(read_operand(&Operand::Y(0), &registers, &[]).is_err());
    assert!(read_operand(&Operand::UnsupportedLiteral(0), &registers, &[]).is_err());
    assert!(call_arguments(&[], 1).is_err());
    assert!(
        write_operand(
            &Operand::UnsupportedLiteral(0),
            Term::Integer(1),
            &mut registers,
            &mut []
        )
        .is_err()
    );
    assert!(write_operand(&Operand::Y(0), Term::Integer(1), &mut registers, &mut []).is_err());
    assert!(
        write_operand(
            &Operand::Value(Term::Integer(1)),
            Term::Integer(1),
            &mut registers,
            &mut []
        )
        .is_err()
    );
}

#[test]
fn selects_tuple_arity_and_rejects_call_last_stack_size() {
    let select = |value: Term| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(value), Operand::X(0)),
                Instruction::SelectTupleArity {
                    value: Operand::X(0),
                    fail_label: 4,
                    choices: vec![(1, 3)],
                },
                Instruction::Label(3),
                Instruction::Move(atom("one"), Operand::X(0)),
                Instruction::Return,
                Instruction::Label(4),
                Instruction::Move(atom("other"), Operand::X(0)),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
        .unwrap()
    };
    assert_eq!(
        select(Term::Tuple(vec![Term::Integer(1)])),
        test_atom("one")
    );
    assert_eq!(select(Term::Tuple(vec![])), test_atom("other"));
    assert_eq!(select(test_atom("a")), test_atom("other"));

    let error = |body| {
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
    };
    assert!(
        error(vec![
            Instruction::Allocate { stack_size: 1 },
            Instruction::CallLast {
                arity: 0,
                label: 2,
                deallocate: 0,
            },
        ])
        .contains("tail-call stack size")
    );
}

#[test]
fn puts_a_new_atom_into_the_atom_table_of_the_run() {
    // ERTS has one atom table for each emulator, and
    // `erlang:list_to_atom/1` gives a new atom the next index
    // (`erts_atom_put`). A run starts with a copy of the table of the
    // loaded modules, so `pbeam_alpha` gets index 1151 after
    // `pbeam_zeta` (1150). The table of the loaded modules does not
    // change, so each run gives the same index.
    let mut program = make_program(
        vec![
            Instruction::Move(
                Operand::Value(Term::List(
                    "pbeam_alpha"
                        .bytes()
                        .map(|byte| Term::Integer(i64::from(byte)))
                        .collect(),
                )),
                Operand::X(0),
            ),
            erlang_call("list_to_atom", 1, true),
            Instruction::Return,
        ],
        0,
    );
    assert_eq!(program.atoms.put("pbeam_zeta").unwrap().index(), 1150);
    for _ in 0..2 {
        assert!(matches!(
            program.execute("fixture", "main", &[]).unwrap(),
            Term::Atom(ref alpha) if alpha.name() == "pbeam_alpha" && alpha.index() == 1151
        ));
    }
    assert_eq!(program.atom("pbeam_alpha"), None);
}

#[test]
fn returns_the_iovec_of_a_binary_and_of_a_list() {
    // On pinned ERTS, `erlang:iolist_to_iovec(<<"Hello">>)` returns
    // `[<<"Hello">>]`, and `[1, [<<2>>]]` returns `[<<1,2>>]` through the
    // iolist engine (`iolist_to_iovec_engine` in `erl_iolist.c`).
    let run = |argument| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(argument), Operand::X(0)),
                erlang_call("iolist_to_iovec", 1, true),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
        .unwrap()
    };
    let hello = Term::Binary(b"Hello".to_vec());
    assert_eq!(run(hello.clone()), Term::List(vec![hello]));
    assert_eq!(
        run(Term::List(vec![
            Term::Integer(1),
            Term::List(vec![Term::Binary(vec![2])]),
        ])),
        Term::List(vec![Term::Binary(vec![1, 2])])
    );
}

#[test]
fn finds_a_live_catch_tag_after_other_slots_and_skips_a_cleared_tag() {
    let run = |body| make_program(body, 0).execute("fixture", "main", &[]);
    let nif_error = [
        Instruction::Move(atom("undef"), Operand::X(0)),
        erlang_call("nif_error", 1, false),
    ];
    // y0 is empty, so the scan must look past it to the tag in y1.
    let mut body = vec![
        Instruction::Allocate { stack_size: 2 },
        Instruction::Catch {
            register: 1,
            label: 3,
        },
    ];
    body.extend(nif_error.clone());
    body.extend([
        Instruction::Label(3),
        Instruction::CatchEnd { register: 1 },
        Instruction::Return,
    ]);
    assert_eq!(
        run(body).unwrap(),
        Term::Tuple(vec![
            Term::static_atom("EXIT"),
            Term::Tuple(vec![Term::static_atom("undef"), Term::List(Vec::new())]),
        ])
    );
    // `try_end` writes `[]` over the tag, so the exception is not caught.
    let mut body = vec![
        Instruction::Allocate { stack_size: 1 },
        Instruction::Catch {
            register: 0,
            label: 3,
        },
        Instruction::TryEnd { register: 0 },
    ];
    body.extend(nif_error);
    body.extend([Instruction::Label(3), Instruction::Return]);
    assert!(
        run(body)
            .unwrap_err()
            .to_string()
            .contains("root process exited with {undef,[]}")
    );
}

#[test]
fn rejects_a_catch_tag_operand_and_an_unallocated_catch_register() {
    let error = |body| {
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
    };
    assert!(
        error(vec![
            Instruction::Allocate { stack_size: 1 },
            Instruction::Catch {
                register: 0,
                label: 2,
            },
            Instruction::Move(Operand::Y(0), Operand::X(0)),
        ])
        .contains("stack register y0 holds a catch tag")
    );
    assert!(
        error(vec![Instruction::Catch {
            register: 0,
            label: 2,
        }])
        .contains("register y0 is outside allocated range")
    );
}

#[test]
fn raises_the_exception_of_a_bif_instruction_and_rejects_a_trap() {
    let element = |index: i64| {
        make_program(
            vec![
                Instruction::Allocate { stack_size: 1 },
                Instruction::Catch {
                    register: 0,
                    label: 3,
                },
                element_bif(index, Operand::Value(Term::Tuple(vec![])), Operand::X(0)),
                Instruction::Label(3),
                Instruction::CatchEnd { register: 0 },
                Instruction::Deallocate { stack_size: 1 },
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    // ERTS 29.1.1: catch element(1, {}) is {'EXIT', {badarg, Stack}}.
    // PBeam gives the stack trace [].
    assert_eq!(
        element(1).unwrap(),
        Term::Tuple(vec![
            test_atom("EXIT"),
            Term::Tuple(vec![test_atom("badarg"), Term::List(Vec::new())]),
        ])
    );
    // Without a catch, the exception exits the process.
    let body = vec![
        element_bif(1, Operand::Value(Term::Tuple(vec![])), Operand::X(0)),
        Instruction::Return,
    ];
    assert_eq!(
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        BADARG_EXIT
    );
    // demonitor(Ref, [flush]) of a reference with no monitor traps. No
    // compiler emits it as a `bif` instruction.
    let body = vec![
        erlang_call("make_ref", 0, false),
        Instruction::Bif {
            fail_label: 0,
            module: "erlang".into(),
            name: "demonitor".into(),
            arguments: vec![
                Operand::X(0),
                Operand::Value(Term::List(vec![test_atom("flush")])),
            ],
            destination: Operand::X(0),
        },
        Instruction::Return,
    ];
    assert!(
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
            .starts_with("unsupported erlang:demonitor/2 result Trap")
    );
}

#[test]
fn returns_the_local_node_for_node_0_and_a_pid() {
    let node = |arguments: Vec<Operand>| {
        make_program(
            vec![
                Instruction::Bif {
                    fail_label: 0,
                    module: "erlang".into(),
                    name: "self".into(),
                    arguments: Vec::new(),
                    destination: Operand::X(1),
                },
                Instruction::Bif {
                    fail_label: 0,
                    module: "erlang".into(),
                    name: "node".into(),
                    arguments,
                    destination: Operand::X(0),
                },
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    // ERTS 29.1.1 without distribution: node() and node(self()) are nonode@nohost.
    assert_eq!(
        node(Vec::new()).unwrap(),
        Term::static_atom("nonode@nohost")
    );
    assert_eq!(
        node(vec![Operand::X(1)]).unwrap(),
        Term::static_atom("nonode@nohost")
    );
    assert_eq!(
        node(vec![atom("foo")]).unwrap_err().to_string(),
        "unsupported erlang:node/1 argument Atom(\"foo\"): not a pid"
    );
}

#[test]
fn traps_a_tail_called_demonitor_flush_into_erts_internal() {
    let mut program = make_program(
        vec![
            erlang_call("make_ref", 0, false),
            Instruction::Move(
                Operand::Value(Term::List(vec![test_atom("flush")])),
                Operand::X(1),
            ),
            erlang_call("demonitor", 2, true),
            Instruction::Return,
        ],
        0,
    );
    program.modules.insert(
        "erts_internal".into(),
        BeamModule {
            name: "erts_internal".into(),
            exports: HashMap::from([(("flush_monitor_messages".into(), 3), 2)]),
            instructions: vec![
                Instruction::FunctionInfo {
                    module: "erts_internal".into(),
                    name: "flush_monitor_messages".into(),
                    arity: 3,
                },
                Instruction::Label(1),
                Instruction::Label(2),
                Instruction::Move(Operand::X(2), Operand::X(0)),
                Instruction::Return,
            ],
        },
    );
    assert_eq!(
        program.execute("fixture", "main", &[]).unwrap(),
        test_atom("true")
    );
}

#[test]
fn removes_an_active_monitor_without_a_trap() {
    // monitor(process, Child), then demonitor(Ref, [flush]) before the
    // child runs: the monitor is still active, so ERTS returns true
    // without flushing.
    let mut body = spawn_link_child(Term::List(vec![test_atom("normal")]));
    body.pop();
    body.push(erlang_call("spawn", 3, false));
    body.extend([
        Instruction::Move(Operand::X(0), Operand::X(1)),
        Instruction::Move(atom("process"), Operand::X(0)),
        erlang_call("monitor", 2, false),
        Instruction::Move(
            Operand::Value(Term::List(vec![test_atom("flush")])),
            Operand::X(1),
        ),
        erlang_call("demonitor", 2, true),
        Instruction::Return,
    ]);
    assert_eq!(run_with_child(body).unwrap(), test_atom("true"));
}

/// Run `main/0` with the functions `zero/0`, `id/3`, and `last/2` of
/// the probe that gives the ERTS values of the `apply` tests:
/// `zero() -> zero.`, `id(A, B, C) -> {A, B, C}.`, and
/// `last(M, F) -> M:F(bye).`, which compiles to `{apply_last,1,0}`.
fn run_with_apply_targets(body: Vec<Instruction>) -> Result<Term, crate::BeamError> {
    apply_targets_program(body).execute("fixture", "main", &[])
}

/// Return the program of `run_with_apply_targets`.
fn apply_targets_program(body: Vec<Instruction>) -> BeamProgram {
    let mut program = make_program(body, 0);
    let module = program.modules.get_mut("fixture").unwrap();
    module.exports.extend([
        (("zero".into(), 0), 11),
        (("id".into(), 3), 12),
        (("last".into(), 2), 13),
    ]);
    let function = |name: &str, arity, label| {
        [
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: name.into(),
                arity,
            },
            Instruction::Label(label),
        ]
    };
    module.instructions.extend(function("zero", 0, 11));
    module.instructions.extend([
        Instruction::Move(atom("zero"), Operand::X(0)),
        Instruction::Return,
    ]);
    module.instructions.extend(function("id", 3, 12));
    module.instructions.extend([
        Instruction::PutTuple {
            elements: vec![Operand::X(0), Operand::X(1), Operand::X(2)],
            destination: Operand::X(0),
        },
        Instruction::Return,
    ]);
    module.instructions.extend(function("last", 2, 13));
    module.instructions.extend([
        Instruction::Allocate { stack_size: 0 },
        Instruction::Move(Operand::X(1), Operand::X(2)),
        Instruction::Move(Operand::X(0), Operand::X(1)),
        Instruction::Move(atom("bye"), Operand::X(0)),
        Instruction::Apply {
            arity: 1,
            tail: true,
            deallocate: 0,
        },
    ]);
    program
}

/// Return the body that moves the module and the function name of an
/// `apply` with `arity` arguments to `x(arity)` and `x(arity + 1)`, then
/// runs the `apply`.
fn apply_body(arity: u32, module: Operand, function: Operand) -> Vec<Instruction> {
    vec![
        Instruction::Move(module, Operand::X(arity)),
        Instruction::Move(function, Operand::X(arity + 1)),
        Instruction::Apply {
            arity,
            tail: false,
            deallocate: 0,
        },
        Instruction::Return,
    ]
}

#[test]
fn applies_module_functions_and_bifs_as_pinned_erts() {
    // OTP 29.1.1: `M:F()` compiles to `{apply,0}`, and it gives `zero`
    // with `fixture` and `zero`.
    assert_eq!(
        run_with_apply_targets(apply_body(0, atom("fixture"), atom("zero"))).unwrap(),
        test_atom("zero")
    );
    // `M:F()` with `erlang` and `self` gives a pid: the BIF runs.
    assert!(matches!(
        run_with_apply_targets(apply_body(0, atom("erlang"), atom("self"))),
        Ok(Term::Pid(_))
    ));
    // In a tail position, `M:F()` compiles to `{apply_last,0,0}`. The BIF
    // value returns from the last frame, so the root process exits.
    assert!(matches!(
        run_with_apply_targets(vec![
            Instruction::Move(atom("erlang"), Operand::X(0)),
            Instruction::Move(atom("self"), Operand::X(1)),
            Instruction::Apply {
                arity: 0,
                tail: true,
                deallocate: 0,
            },
        ]),
        Ok(Term::Pid(_))
    ));
    // `M:F(A, b, c)` compiles to `{apply,3}`, and it gives `{a,b,c}`
    // with `fixture`, `id`, and `a`.
    let mut body = vec![
        Instruction::Move(atom("a"), Operand::X(0)),
        Instruction::Move(atom("b"), Operand::X(1)),
        Instruction::Move(atom("c"), Operand::X(2)),
    ];
    body.extend(apply_body(3, atom("fixture"), atom("id")));
    assert_eq!(
        run_with_apply_targets(body).unwrap(),
        Term::Tuple(vec![test_atom("a"), test_atom("b"), test_atom("c")])
    );
    // `catch last(erlang, exit)` gives `{'EXIT',bye}`: `apply_last` runs
    // the `erlang:exit/1` BIF, and the exception reaches the catch.
    assert_eq!(
        run_with_apply_targets(vec![
            Instruction::Allocate { stack_size: 1 },
            Instruction::Catch {
                register: 0,
                label: 3,
            },
            Instruction::Move(atom("erlang"), Operand::X(0)),
            Instruction::Move(atom("exit"), Operand::X(1)),
            Instruction::Call {
                arity: 2,
                label: 13
            },
            Instruction::Label(3),
            Instruction::CatchEnd { register: 0 },
            Instruction::Return,
        ])
        .unwrap(),
        Term::Tuple(vec![test_atom("EXIT"), test_atom("bye")])
    );
}

#[test]
fn applies_erlang_apply_3_as_the_i_apply_instruction() {
    let xyz = || Term::List(vec![test_atom("x"), test_atom("y"), test_atom("z")]);
    let apply_of = |module: Term, function: Term, arguments: Term| {
        vec![
            Instruction::Move(Operand::Value(module), Operand::X(0)),
            Instruction::Move(Operand::Value(function), Operand::X(1)),
            Instruction::Move(Operand::Value(arguments), Operand::X(2)),
        ]
    };
    // `M:F(fixture, id, [x, y, z])` with `erlang` and `apply` compiles to
    // `{apply,3}`. ERTS `fixed_apply` runs `apply` (`beam_common.c`),
    // which calls `fixture:id/3` and gives `{x,y,z}`.
    let mut body = apply_of(test_atom("fixture"), test_atom("id"), xyz());
    body.extend(apply_body(3, atom("erlang"), atom("apply")));
    let tuple = Term::Tuple(vec![test_atom("x"), test_atom("y"), test_atom("z")]);
    assert_eq!(run_with_apply_targets(body).unwrap(), tuple);
    // `erlang:apply(erlang, apply, [fixture, id, [x, y, z]])` as a tail
    // `call_ext` runs the inner call.
    let mut body = apply_of(
        test_atom("erlang"),
        test_atom("apply"),
        Term::List(vec![test_atom("fixture"), test_atom("id"), xyz()]),
    );
    body.push(erlang_call("apply", 3, true));
    assert_eq!(run_with_apply_targets(body).unwrap(), tuple);
    // An apply of `erlang:apply` with a list that is not three elements
    // calls `erlang:apply/2`. ERTS 29.1.1 raises `badarg` for
    // `apply(erlang, apply, [fixture, id])`, because `id` is not a list.
    let mut body = apply_of(
        test_atom("erlang"),
        test_atom("apply"),
        Term::List(vec![test_atom("fixture"), test_atom("id")]),
    );
    body.push(erlang_call("apply", 3, true));
    assert_eq!(
        run_with_apply_targets(body).unwrap_err().to_string(),
        BADARG_EXIT
    );
    // `apply` copies at most 1023 arguments to the X registers. ERTS
    // raises `undef` for `apply(fixture, missing, L1023)`, because the
    // loaded module `fixture` does not export `missing/1023`, and raises
    // `system_limit` for a list of 1024 elements.
    let mut body = apply_of(
        test_atom("fixture"),
        test_atom("missing"),
        Term::List(vec![test_atom("x"); 1023]),
    );
    body.push(erlang_call("apply", 3, true));
    let mut program = apply_targets_program(body);
    // The atoms of the `undef` frame come from the atom table of the run.
    assert_eq!(
        program
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "atom table does not hold fixture"
    );
    for name in ["fixture", "missing"] {
        program.atoms.put(name).unwrap();
    }
    assert_eq!(
        program
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        format!(
            "root process exited with {{undef,[{{fixture,missing,[{}],[]}}]}}",
            ["x"; 1023].join(",")
        )
    );
    let mut body = apply_of(
        test_atom("fixture"),
        test_atom("missing"),
        Term::List(vec![test_atom("x"); 1024]),
    );
    body.push(erlang_call("apply", 3, true));
    assert_eq!(
        run_with_apply_targets(body).unwrap_err().to_string(),
        "root process exited with {system_limit,[]}"
    );
}

#[test]
fn rejects_an_apply_with_a_stack_mismatch_and_raises_badarg() {
    let error = |body| run_with_apply_targets(body).unwrap_err().to_string();
    // ERTS 29.1.1 raises `badarg` for `M:F()` with `1` and `f`, or with
    // `m` and `1`.
    assert_eq!(
        error(apply_body(0, Operand::Value(Term::Integer(1)), atom("f"))),
        BADARG_EXIT
    );
    assert_eq!(
        error(apply_body(0, atom("m"), Operand::Value(Term::Integer(1)))),
        BADARG_EXIT
    );
    // `apply_last` releases the Y registers of the frame, so the count
    // must match the allocated stack, as for `call_last`.
    assert_eq!(
        error(vec![
            Instruction::Allocate { stack_size: 1 },
            Instruction::Move(atom("fixture"), Operand::X(0)),
            Instruction::Move(atom("zero"), Operand::X(1)),
            Instruction::Apply {
                arity: 0,
                tail: true,
                deallocate: 0,
            },
        ]),
        "invalid BEAM tail-call stack size"
    );
}

/// Run `main/0` as a tail `call_ext` of `module:name` with `arguments`
/// in `x0` onwards.
fn call_ext_with(module: &str, name: &str, arguments: Vec<Term>) -> Result<Term, crate::BeamError> {
    let arity = arguments.len() as u32;
    let mut body: Vec<Instruction> = arguments
        .into_iter()
        .zip(0..)
        .map(|(argument, register)| {
            Instruction::Move(Operand::Value(argument), Operand::X(register))
        })
        .collect();
    body.push(Instruction::CallExternal {
        arity,
        module: module.into(),
        name: name.into(),
        tail: true,
        deallocate: 0,
    });
    body.push(Instruction::Return);
    make_program(body, 0).execute("fixture", "main", &[])
}

fn get_bif(key: &str, destination: Operand) -> Instruction {
    Instruction::Bif {
        fail_label: 0,
        module: "erlang".into(),
        name: "get".into(),
        arguments: vec![atom(key)],
        destination,
    }
}

#[test]
fn puts_and_gets_process_dictionary_values_as_pinned_erts() {
    let put = |value: i64| {
        [
            Instruction::Move(atom("k"), Operand::X(0)),
            Instruction::Move(Operand::Value(Term::Integer(value)), Operand::X(1)),
            erlang_call("put", 2, false),
        ]
    };
    let mut body = vec![Instruction::Allocate { stack_size: 3 }];
    body.extend(put(1));
    body.push(Instruction::Move(Operand::X(0), Operand::Y(0)));
    body.extend(put(2));
    body.push(Instruction::Move(Operand::X(0), Operand::Y(1)));
    // `gen:get_parent/0` reads `get('$ancestors')` with
    // `{bif,get,{f,0},[{atom,'$ancestors'}],{x,0}}`.
    body.push(get_bif("k", Operand::Y(2)));
    body.push(get_bif("other", Operand::X(0)));
    body.extend([
        Instruction::PutTuple {
            elements: vec![Operand::Y(0), Operand::Y(1), Operand::Y(2), Operand::X(0)],
            destination: Operand::X(0),
        },
        Instruction::Deallocate { stack_size: 3 },
        Instruction::Return,
    ]);
    // ERTS 29.1.1: [put(k, 1), put(k, 2), get(k), get(other)] is
    // [undefined,1,2,undefined].
    let undefined = Term::static_atom("undefined");
    assert_eq!(
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap(),
        Term::Tuple(vec![
            undefined.clone(),
            Term::Integer(1),
            Term::Integer(2),
            undefined
        ])
    );
}

#[test]
fn starts_a_spawned_process_with_an_empty_dictionary() {
    let mut body = vec![
        Instruction::Move(atom("k"), Operand::X(0)),
        Instruction::Move(atom("parent"), Operand::X(1)),
        erlang_call("put", 2, false),
        Instruction::Move(atom("fixture"), Operand::X(0)),
        Instruction::Move(atom("fresh"), Operand::X(1)),
        Instruction::Move(Operand::Value(Term::List(Vec::new())), Operand::X(2)),
        Instruction::Move(
            Operand::Value(Term::List(vec![test_atom("monitor")])),
            Operand::X(3),
        ),
        erlang_call("spawn_opt", 4, false),
    ];
    body.extend(wait_forever());
    let mut program = make_program(body, 0);
    let module = program.modules.get_mut("fixture").unwrap();
    module.exports.insert(("fresh".into(), 0), 11);
    module.instructions.extend([
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "fresh".into(),
            arity: 0,
        },
        Instruction::Label(11),
        get_bif("k", Operand::X(0)),
        erlang_call("exit", 1, true),
    ]);
    // ERTS 29.1.1: after put(k, parent), a spawned process gets
    // undefined for get(k).
    assert!(matches!(
        program.execute("fixture", "main", &[]),
        Ok(Term::Tuple(ref elements))
            if elements.first() == Some(&test_atom("DOWN"))
                && elements.last() == Some(&Term::static_atom("undefined"))
    ));
}

#[test]
fn appends_lists_as_pinned_erts() {
    let integers = |values: &[i64]| Term::List(values.iter().copied().map(Term::Integer).collect());
    let append = |left: Term, right: Term| call_ext_with("erlang", "++", vec![left, right]);
    // ERTS 29.1.1: [] ++ x is x, and [1,2] ++ [3] is [1,2,3].
    assert_eq!(
        append(Term::List(Vec::new()), test_atom("x")).unwrap(),
        test_atom("x")
    );
    assert_eq!(
        append(integers(&[1, 2]), integers(&[3])).unwrap(),
        integers(&[1, 2, 3])
    );
    // ERTS 29.1.1: [1] ++ x is the improper list [1|x], and a ++ []
    // and a ++ x raise badarg.
    assert_eq!(
        append(integers(&[1]), test_atom("x")).unwrap(),
        Term::ImproperList(vec![Term::Integer(1)], Box::new(test_atom("x")))
    );
    for right in [Term::List(Vec::new()), test_atom("x")] {
        assert_eq!(
            append(test_atom("a"), right).unwrap_err().to_string(),
            BADARG_EXIT
        );
    }
}

#[test]
fn reverses_a_list_onto_a_tail_that_is_not_a_list() {
    // ERTS 29.1.1: lists:reverse([a], x) is the improper list [a|x].
    let reverse = call_ext_with(
        "lists",
        "reverse",
        vec![Term::List(vec![test_atom("a")]), test_atom("x")],
    );
    assert_eq!(
        reverse.unwrap(),
        Term::ImproperList(vec![test_atom("a")], Box::new(test_atom("x")))
    );
}

#[test]
fn finds_key_tuples_as_pinned_erts() {
    let tuple = |elements: Vec<Term>| Term::Tuple(elements);
    let keyfind = |key: Term, position: Term, list: Vec<Term>| {
        call_ext_with("lists", "keyfind", vec![key, position, Term::List(list)])
    };
    let badarg = |key: Term, position: Term, list: Term| {
        call_ext_with("lists", "keyfind", vec![key, position, list])
            .unwrap_err()
            .to_string()
    };
    let (a, b) = (test_atom("a"), test_atom("b"));
    // ERTS 29.1.1: lists:keyfind(b, 1, [{a,1},{b,2},{b,3}]) is {b,2}.
    assert_eq!(
        keyfind(
            b.clone(),
            Term::Integer(1),
            vec![
                tuple(vec![a.clone(), Term::Integer(1)]),
                tuple(vec![b.clone(), Term::Integer(2)]),
                tuple(vec![b.clone(), Term::Integer(3)]),
            ]
        )
        .unwrap(),
        tuple(vec![b, Term::Integer(2)])
    );
    // ERTS 29.1.1: lists:keyfind(x, 1, []) is false.
    assert_eq!(
        keyfind(test_atom("x"), Term::Integer(1), Vec::new()).unwrap(),
        Term::static_atom("false")
    );
    // ERTS 29.1.1: lists:keyfind(a, 2, [{1,a}, x, {a}, {2,a}]) is {1,a}:
    // a term that is not a tuple and a tuple that is too short do not match.
    assert_eq!(
        keyfind(
            a.clone(),
            Term::Integer(2),
            vec![
                tuple(vec![Term::Integer(1), a.clone()]),
                test_atom("x"),
                tuple(vec![a.clone()]),
                tuple(vec![Term::Integer(2), a.clone()]),
            ]
        )
        .unwrap(),
        tuple(vec![Term::Integer(1), a.clone()])
    );
    // ERTS 29.1.1: lists:keyfind(a, (1 bsl 59) - 1, [{a}]) is false: the
    // position is a small integer.
    assert_eq!(
        keyfind(
            a.clone(),
            Term::Integer((1 << 59) - 1),
            vec![tuple(vec![a.clone()])]
        )
        .unwrap(),
        Term::static_atom("false")
    );
    // ERTS 29.1.1 raises badarg for the positions 0, -1, x, and the
    // bignum 1 bsl 59, and for the lists a and {a}.
    let one = Term::List(vec![tuple(vec![a.clone()])]);
    for position in [
        Term::Integer(0),
        Term::Integer(-1),
        test_atom("x"),
        Term::Integer(1 << 59),
    ] {
        assert_eq!(badarg(a.clone(), position, one.clone()), BADARG_EXIT);
    }
    for list in [a.clone(), tuple(vec![a.clone()])] {
        assert_eq!(badarg(a.clone(), Term::Integer(1), list), BADARG_EXIT);
    }
}

#[test]
fn finds_list_members_as_pinned_erts() {
    let member = |element: Term, list: Term| call_ext_with("lists", "member", vec![element, list]);
    // ERTS 29.1.1: lists:member(a, []) is false, and lists:member(1, [1])
    // is true.
    assert_eq!(
        member(test_atom("a"), Term::List(Vec::new())).unwrap(),
        Term::static_atom("false")
    );
    assert_eq!(
        member(Term::Integer(1), Term::List(vec![Term::Integer(1)])).unwrap(),
        Term::static_atom("true")
    );
    // ERTS 29.1.1: lists:member(a, x) raises badarg.
    assert_eq!(
        member(test_atom("a"), test_atom("x"))
            .unwrap_err()
            .to_string(),
        BADARG_EXIT
    );
}

fn element_bif(index: i64, tuple: Operand, destination: Operand) -> Instruction {
    Instruction::Bif {
        fail_label: 0,
        module: "erlang".into(),
        name: "element".into(),
        arguments: vec![Operand::Value(Term::Integer(index)), tuple],
        destination,
    }
}

#[test]
fn reads_tuple_elements_as_pinned_erts() {
    let (a, b, c) = (test_atom("a"), test_atom("b"), test_atom("c"));
    // The pinned `gen_statem` runs `element/2` through
    // `{bif,element,{f,0},[{integer,2},{x,0}],{x,0}}`.
    // ERTS 29.1.1: element(2, {a,b,c}) is b.
    let body = vec![
        Instruction::Move(
            Operand::Value(Term::Tuple(vec![a.clone(), b.clone(), c])),
            Operand::X(0),
        ),
        element_bif(2, Operand::X(0), Operand::X(0)),
        Instruction::Return,
    ];
    assert_eq!(
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap(),
        b
    );
    let element = |index: Term, tuple: Term| call_ext_with("erlang", "element", vec![index, tuple]);
    let one = Term::Tuple(vec![a.clone()]);
    // ERTS 29.1.1: element(1, {a}) is a.
    assert_eq!(element(Term::Integer(1), one.clone()).unwrap(), a);
    // ERTS 29.1.1 raises badarg for element(0, {a}), element(2, {a}),
    // element(-1, {a}), element(a, {a}), and element(1, [a]).
    for (index, tuple) in [
        (Term::Integer(0), one.clone()),
        (Term::Integer(2), one.clone()),
        (Term::Integer(-1), one.clone()),
        (a.clone(), one),
        (Term::Integer(1), Term::List(vec![a])),
    ] {
        assert_eq!(element(index, tuple).unwrap_err().to_string(), BADARG_EXIT);
    }
}

#[test]
fn reads_the_tuple_size_as_pinned_erts() {
    let tuple_size = |term: Term| call_ext_with("erlang", "tuple_size", vec![term]);
    // ERTS 29.1.1: tuple_size({}) is 0, and tuple_size(a) raises badarg.
    assert_eq!(tuple_size(Term::Tuple(vec![])).unwrap(), Term::Integer(0));
    assert_eq!(
        tuple_size(test_atom("a")).unwrap_err().to_string(),
        BADARG_EXIT
    );
}

#[test]
fn rejects_is_ge_and_is_lt_on_two_different_references() {
    // ERTS orders two references by the scheduler thread that made each
    // one; this is the "Term order" row in `docs/erts-differences.md`.
    let reference = |number| Operand::Value(Term::Reference(crate::Reference(number)));
    let is_ge = Instruction::TestGreaterEqual {
        fail_label: 3,
        left: reference(1),
        right: reference(2),
    };
    let is_lt = Instruction::TestLess {
        fail_label: 3,
        left: reference(1),
        right: reference(2),
    };
    for test in [is_ge, is_lt] {
        let body = vec![test, Instruction::Label(3), Instruction::Return];
        assert_eq!(
            make_program(body, 0)
                .execute("fixture", "main", &[])
                .unwrap_err()
                .to_string(),
            "unsupported term order of two references Reference(Reference(1)) and \
             Reference(Reference(2))"
        );
    }
}

#[test]
fn reads_an_empty_current_stacktrace_of_the_calling_process() {
    // `STACKTRACE()` in the pinned `gen_statem` is
    // `element(2, erlang:process_info(self(), current_stacktrace))`.
    let body = vec![
        Instruction::Bif {
            fail_label: 0,
            module: "erlang".into(),
            name: "self".into(),
            arguments: Vec::new(),
            destination: Operand::X(0),
        },
        Instruction::Move(atom("current_stacktrace"), Operand::X(1)),
        erlang_call("process_info", 2, false),
        Instruction::Move(Operand::X(0), Operand::X(1)),
        element_bif(2, Operand::X(0), Operand::X(0)),
        Instruction::PutTuple {
            elements: vec![Operand::X(1), Operand::X(0)],
            destination: Operand::X(0),
        },
        Instruction::Return,
    ];
    // ERTS 29.1.1 gives the frames of the caller; PBeam records none.
    let stacktrace = Term::Tuple(vec![
        test_atom("current_stacktrace"),
        Term::List(Vec::new()),
    ]);
    assert_eq!(
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap(),
        Term::Tuple(vec![stacktrace, Term::List(Vec::new())])
    );
    let process_info =
        |process: Term, item: Term| call_ext_with("erlang", "process_info", vec![process, item]);
    let root = || Term::Pid(crate::ProcessId(0));
    let dictionary = || Term::Tuple(vec![test_atom("dictionary"), test_atom("k")]);
    // ERTS 29.1.1 checks the item before the process. It raises badarg for
    // process_info(a, current_stacktrace), process_info(a, bogus_item),
    // process_info(self(), bogus_item), process_info(self(), 5),
    // process_info(self(), [registered_name, bogus]), and
    // process_info(self(), {bogus, k}).
    for (process, item) in [
        (test_atom("a"), test_atom("current_stacktrace")),
        (test_atom("a"), test_atom("bogus_item")),
        (Term::Pid(crate::ProcessId(99)), test_atom("bogus_item")),
        (root(), test_atom("bogus_item")),
        (root(), Term::Integer(5)),
        (
            root(),
            Term::List(vec![test_atom("registered_name"), test_atom("bogus")]),
        ),
        (
            root(),
            Term::Tuple(vec![test_atom("bogus"), test_atom("k")]),
        ),
    ] {
        assert_eq!(
            process_info(process, item).unwrap_err().to_string(),
            BADARG_EXIT
        );
    }
    // ERTS 29.1.1: a process that is not alive gives undefined for each
    // valid item, also a list of items (`erl_bif_info.c:1384-1386`).
    for item in [
        test_atom("current_stacktrace"),
        Term::List(vec![test_atom("registered_name")]),
    ] {
        assert_eq!(
            process_info(Term::Pid(crate::ProcessId(99)), item).unwrap(),
            test_atom("undefined")
        );
    }
    // ERTS 29.1.1: process_info(self(), {dictionary, k}) is
    // {{dictionary, k}, undefined}, process_info(self(), [registered_name])
    // is [{registered_name, []}], and process_info(self(), status) is
    // {status, running}. PBeam does not implement them yet.
    for (process, item) in [
        (root(), dictionary()),
        (root(), Term::List(vec![test_atom("registered_name")])),
        (root(), test_atom("status")),
    ] {
        assert_eq!(
            process_info(process.clone(), item.clone())
                .unwrap_err()
                .to_string(),
            format!("unsupported erlang:process_info/2 arguments {process:?}, {item:?}")
        );
    }
    // ERTS 29.1.1: process_info(self(), registered_name) is [] for a
    // process without a name.
    assert_eq!(
        call_ext_with(
            "erlang",
            "process_info",
            vec![Term::Pid(crate::ProcessId(0)), test_atom("registered_name")]
        )
        .unwrap(),
        Term::List(Vec::new())
    );
}

#[test]
fn rejects_the_current_stacktrace_of_another_process() {
    // ERTS 29.1.1 returns the frames of another live process. PBeam
    // records no stack frames, so it returns a runtime error.
    let error = run_with_child(vec![
        Instruction::Move(atom("fixture"), Operand::X(0)),
        Instruction::Move(atom("child"), Operand::X(1)),
        Instruction::Move(
            Operand::Value(Term::List(vec![test_atom("normal")])),
            Operand::X(2),
        ),
        erlang_call("spawn", 3, false),
        Instruction::Move(atom("current_stacktrace"), Operand::X(1)),
        erlang_call("process_info", 2, true),
        Instruction::Return,
    ])
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported erlang:process_info/2 arguments Pid(ProcessId(1)), Atom(\"current_stacktrace\")"
    );
}

#[test]
fn registers_a_name_or_raises_badarg_as_pinned_erts() {
    let register = |name: Term, id: Term| call_ext_with("erlang", "register", vec![name, id]);
    let root = Term::Pid(crate::ProcessId(0));
    // ERTS 29.1.1: register(a, self()) is true.
    assert_eq!(
        register(test_atom("a"), root.clone()).unwrap(),
        test_atom("true")
    );
    // ERTS 29.1.1 raises badarg for register(1, self()), register(a, x),
    // and register(a, Pid) with a process that is not alive.
    for (name, id) in [
        (Term::Integer(1), root),
        (test_atom("a"), test_atom("x")),
        (test_atom("a"), Term::Pid(crate::ProcessId(99))),
    ] {
        assert_eq!(register(name, id).unwrap_err().to_string(), BADARG_EXIT);
    }
}

#[test]
fn finds_exported_functions_as_pinned_erts() {
    let exported = |module: &str, name: &str, arity: i64| {
        call_ext_with(
            "erlang",
            "function_exported",
            vec![test_atom(module), test_atom(name), Term::Integer(arity)],
        )
    };
    let (yes, no) = (test_atom("true"), test_atom("false"));
    // ERTS 29.1.1: function_exported/3 is true for a loaded export
    // (lists:map/2), a BIF (erlang:self/0), and erlang:apply/2,
    // erlang:apply/3, and erlang:yield/0. It is false for a missing
    // function (lists:nomatch/2), a module that is not loaded, and the
    // arity -1. Only the low 32 bits of the arity count:
    // function_exported(erlang, self, 1 bsl 32) is true.
    for (module, name, arity, value) in [
        ("fixture", "main", 0, &yes),
        ("erlang", "self", 0, &yes),
        ("erlang", "apply", 2, &yes),
        ("erlang", "apply", 3, &yes),
        ("erlang", "yield", 0, &yes),
        ("erlang", "self", 1 << 32, &yes),
        ("fixture", "main", (1 << 32) + 1, &no),
        ("fixture", "nomatch", 0, &no),
        ("nomod", "f", 0, &no),
        ("erlang", "self", -1, &no),
        ("erlang", "yield", 1, &no),
    ] {
        assert_eq!(
            &exported(module, name, arity).unwrap(),
            value,
            "{module}:{name}/{arity}"
        );
    }
    // ERTS 29.1.1: an arity in the small-integer range counts with its
    // low 32 bits, so -(1 bsl 59) finds erlang:self/0, and
    // (1 bsl 59) - 1 finds no function.
    let arity = |module: &str, name: &str, arity: i64| {
        call_ext_with(
            "erlang",
            "function_exported",
            vec![test_atom(module), test_atom(name), Term::Integer(arity)],
        )
    };
    assert_eq!(arity("erlang", "self", -(1 << 59)).unwrap(), yes);
    assert_eq!(arity("erlang", "self", (1 << 59) - 1).unwrap(), no);
    // ERTS 29.1.1 raises badarg for a module or a name that is not an
    // atom, and for an arity that is not a small integer: x, the bignum
    // 1 bsl 59, and the bignum -(1 bsl 59) - 1.
    let (erlang, self_name) = (test_atom("erlang"), test_atom("self"));
    for arguments in [
        vec![Term::Integer(1), self_name.clone(), Term::Integer(0)],
        vec![erlang.clone(), Term::Integer(1), Term::Integer(0)],
        vec![erlang.clone(), self_name.clone(), test_atom("x")],
        vec![erlang.clone(), self_name.clone(), Term::Integer(1 << 59)],
        vec![erlang, self_name, Term::Integer(-(1 << 59) - 1)],
    ] {
        assert_eq!(
            call_ext_with("erlang", "function_exported", arguments)
                .unwrap_err()
                .to_string(),
            BADARG_EXIT
        );
    }
}

#[test]
fn exits_in_the_next_turn_after_a_switch_at_the_return_of_the_last_frame() {
    // `emit_dispatch_return` switches when the budget is below 0 after the
    // return, and the process continues at the return address
    // (`jit/arm/instr_call.cpp:30-54`). For the last frame, that is the
    // normal exit. The child `burner` returns from `inner` and then from
    // its last frame. The root gets the first message of its queue.
    let first_message = |entries: usize| {
        let body = vec![
            Instruction::Move(atom("fixture"), Operand::X(0)),
            Instruction::Move(atom("burner"), Operand::X(1)),
            Instruction::Move(Operand::Value(Term::List(Vec::new())), Operand::X(2)),
            Instruction::Move(
                Operand::Value(Term::List(vec![test_atom("monitor")])),
                Operand::X(3),
            ),
            erlang_call("spawn_opt", 4, false),
            Instruction::Bif {
                fail_label: 0,
                module: "erlang".into(),
                name: "self".into(),
                arguments: Vec::new(),
                destination: Operand::X(1),
            },
            Instruction::PutList {
                head: Operand::X(1),
                tail: Operand::Value(Term::List(Vec::new())),
                destination: Operand::X(2),
            },
            Instruction::Move(atom("fixture"), Operand::X(0)),
            Instruction::Move(atom("sender"), Operand::X(1)),
            erlang_call("spawn", 3, false),
        ];
        let mut program = make_program([body, wait_forever()].concat(), 0);
        let module = program.modules.get_mut("fixture").unwrap();
        module.exports.insert(("burner".into(), 0), 21);
        module.exports.insert(("sender".into(), 1), 25);
        let function = |name: &str, arity, label| {
            [
                Instruction::FunctionInfo {
                    module: "fixture".into(),
                    name: name.into(),
                    arity,
                },
                Instruction::Label(label),
            ]
        };
        module.instructions.extend(function("burner", 0, 21));
        module
            .instructions
            .extend(std::iter::repeat_n(Instruction::TestYield, entries));
        module.instructions.extend([
            Instruction::Call {
                arity: 0,
                label: 23,
            },
            Instruction::Return,
        ]);
        module.instructions.extend(function("inner", 0, 23));
        module.instructions.extend([
            Instruction::TestYield,
            Instruction::Move(atom("ok"), Operand::X(0)),
            Instruction::Return,
        ]);
        module.instructions.extend(function("sender", 1, 25));
        module.instructions.extend([
            Instruction::Move(atom("tag"), Operand::X(1)),
            Instruction::Send,
            Instruction::Return,
        ]);
        program.execute("fixture", "main", &[]).unwrap()
    };
    // 3998 entries, the entry of `inner`, and its return leave 0. The
    // last return makes the budget -1: `burner` switches out, `sender`
    // runs, and then `burner` exits.
    assert_eq!(first_message(3998), test_atom("tag"));
    // With 3997 entries, the last return leaves 0, and `burner` exits at
    // once. The exit bumps 100 reductions and finds none left, so it
    // switches out before the monitor (`erl_process.c:14229`, `14315`).
    assert_eq!(first_message(3997), test_atom("tag"));
    // The exit of a process with 150 reductions left uses 100 and 50, and
    // switches out before the monitor (`erl_process.c:14475`, `14543`).
    assert_eq!(first_message(3847), test_atom("tag"));
    // With 151 reductions left, the exit sends the monitor signal at once.
    assert!(matches!(
        first_message(3846),
        Term::Tuple(ref elements) if elements.first() == Some(&test_atom("DOWN"))
    ));
}

#[test]
fn rejects_integer_to_list_of_a_bignum_with_no_reductions_left() {
    // `bump_reductions(4000)` leaves no reductions, and a `bif` instruction
    // has no reduction test. ERTS then runs `integer_to_list/1` of a bignum
    // on a dirty CPU scheduler (`bif.c:3289-3296`).
    let body = vec![
        Instruction::Move(Operand::Value(Term::Integer(4000)), Operand::X(0)),
        erlang_call("bump_reductions", 1, false),
        Instruction::Bif {
            fail_label: 0,
            module: "erlang".into(),
            name: "integer_to_list".into(),
            arguments: vec![Operand::Value(Term::Integer(1 << 59))],
            destination: Operand::X(0),
        },
        Instruction::Return,
    ];
    let error = make_program(body, 0)
        .execute("fixture", "main", &[])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported erlang:integer_to_list/1 of 576460752303423488 with 0 reductions left: \
         ERTS runs it on a dirty CPU scheduler"
    );
}
