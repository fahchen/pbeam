use std::{collections::HashMap, rc::Rc, sync::Arc};

use super::bif::{BIFS, BifId, BifResult, resolve, resolve_calls};
use super::{
    BeamProgram, CodeTable, Outcome, Scheduler, StackSlot, call_arguments, function_at_label,
    read_bif_arguments, read_operand, write_operand,
};
use crate::{
    BeamModule, Map, Term,
    beam::code::{BinaryMatchCommand, Instruction, Operand, TestKind, TestOperand},
    runtime::{
        capability::{AllowAll, Capability, Policy},
        ets::{Access, Table},
        process::{ProcessState, Start, tests::frame},
    },
    term::{Atom, AtomTable, Lambda, ProcessId, Reference, test_atom},
};

/// Return the initial capabilities of the default policy.
fn allow_all() -> Vec<Capability> {
    AllowAll.initial()
}

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
            Rc::new(BeamModule::new(
                "fixture".into(),
                HashMap::from([(("main".into(), 0), 2)]),
                instructions,
            )),
        )]),
        atoms: Arc::new(AtomTable::new()),
    }
}

fn erlang_call(name: &str, arity: u32, tail: bool) -> Instruction {
    Instruction::CallExternal {
        arity,
        module: "erlang".into(),
        name: name.into(),
        bif: resolve("erlang", name, arity),
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
    let module = Rc::get_mut(program.modules.get_mut("fixture").unwrap()).unwrap();
    module.instructions.extend([
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "child".into(),
            arity: 1,
        },
        Instruction::Label(11),
        erlang_call("exit", 1, true),
    ]);
    module.reindex();
    let child = module.tables.name_id("child").unwrap();
    module.exports.insert((child, 1), 11);
    program
        .execute("fixture", "main", &[])
        .map(|host| host.into_term())
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
        BeamProgram::load(&[include_bytes!("../../fixtures/funs/fun_probe.beam")]).unwrap();
    let label = program.modules["fun_probe"].exports[&(
        program.modules["fun_probe"]
            .tables
            .name_id("tail_wait")
            .unwrap(),
        1,
    )];
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.code = CodeTable::new(program.modules.clone());
    let frame = scheduler.code.frame("fun_probe", label, 1).unwrap();
    scheduler
        .start(frame, vec![Term::Integer(1)], Vec::new())
        .unwrap();
    scheduler.schedule_next().unwrap();
    assert!(matches!(
        program.run(&mut scheduler).unwrap(),
        Outcome::Wait
    ));
    let frames = &scheduler.current_process_mut().frames;
    assert_eq!(frames.len(), 1);
    let (module_id, function) = (frames[0].module_id, frames[0].function);
    let module = scheduler.code.module_by_id(module_id).unwrap();
    assert_eq!(module.tables.name(function), "-tail_wait/1-fun-0-");
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
    let mut body = spawn_link_child(Term::list_from(vec![Term::Integer(1)]));
    body.extend(wait_forever());
    // The link exit signal of the child ends the root process.
    assert_eq!(error(body), "root process exited with 1");
    let mut body = spawn_link_child(Term::list_from(vec![test_atom("normal")]));
    body.extend(wait_forever());
    assert!(error(body).contains("waits for a message that cannot arrive"));
    let mut body = spawn_link_child(Term::list_from(vec![Term::Integer(1)]));
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
        let mut body = spawn_link_child(Term::list_from(vec![reason]));
        body.pop();
        body.push(Instruction::Move(
            Operand::Value(Term::list_from(options)),
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
            if matches!(elements.as_ref(), [Term::Pid(_), Term::Reference(_)])
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
        Term::tuple(vec![test_atom("a")])
    );
}

/// The classified operand form of `test is_eq_exact` compares the same values
/// as the generic form and takes the same fail branch. It reads one register
/// and one immediate, two registers, or a Y register.
#[test]
fn runs_the_classified_operand_form_of_is_eq_exact() {
    let run = |body: Vec<Instruction>| {
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap()
    };
    let test = |left: TestOperand, right: TestOperand| Instruction::TestExactEqualOperands {
        fail_label: 3,
        left,
        right,
    };
    // x0 is 1 and the immediate is 1: equal, so the test does not jump.
    assert_eq!(
        run(vec![
            Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(0)),
            test(TestOperand::X(0), TestOperand::Value(Term::Integer(1))),
            Instruction::Move(atom("equal"), Operand::X(0)),
            Instruction::Return,
            Instruction::Label(3),
            Instruction::Move(atom("different"), Operand::X(0)),
            Instruction::Return,
        ]),
        test_atom("equal")
    );
    // x0 is 1 and the immediate is 2: not equal, so the test jumps.
    assert_eq!(
        run(vec![
            Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(0)),
            test(TestOperand::X(0), TestOperand::Value(Term::Integer(2))),
            Instruction::Move(atom("equal"), Operand::X(0)),
            Instruction::Return,
            Instruction::Label(3),
            Instruction::Move(atom("different"), Operand::X(0)),
            Instruction::Return,
        ]),
        test_atom("different")
    );
    // Two registers that hold the same boxed literal are equal.
    assert_eq!(
        run(vec![
            Instruction::Move(
                Operand::Value(Term::tuple(vec![Term::Integer(1)])),
                Operand::X(0),
            ),
            Instruction::Move(Operand::X(0), Operand::X(1)),
            test(TestOperand::X(0), TestOperand::X(1)),
            Instruction::Move(atom("equal"), Operand::X(0)),
            Instruction::Return,
            Instruction::Label(3),
            Instruction::Move(atom("different"), Operand::X(0)),
            Instruction::Return,
        ]),
        test_atom("equal")
    );
    // A Y register and an immediate.
    assert_eq!(
        run(vec![
            Instruction::Allocate { stack_size: 1 },
            Instruction::Move(Operand::Value(Term::Integer(7)), Operand::Y(0)),
            test(TestOperand::Y(0), TestOperand::Value(Term::Integer(7))),
            Instruction::Move(atom("equal"), Operand::X(0)),
            Instruction::Return,
            Instruction::Label(3),
            Instruction::Move(atom("different"), Operand::X(0)),
            Instruction::Return,
        ]),
        test_atom("equal")
    );
    // The immediate comes first, and a register that holds nil equals nil.
    assert_eq!(
        run(vec![
            Instruction::Move(Operand::Value(Term::nil()), Operand::X(0)),
            test(TestOperand::Value(Term::nil()), TestOperand::X(0)),
            Instruction::Move(atom("equal"), Operand::X(0)),
            Instruction::Return,
            Instruction::Label(3),
            Instruction::Move(atom("different"), Operand::X(0)),
            Instruction::Return,
        ]),
        test_atom("equal")
    );
}

/// The classified operand form keeps the register and match-context error
/// text of the generic form.
#[test]
fn keeps_the_classified_test_operand_errors() {
    let error = |body: Vec<Instruction>| {
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
    };
    let test = |left: TestOperand, right: TestOperand| Instruction::TestExactEqualOperands {
        fail_label: 2,
        left,
        right,
    };
    assert_eq!(
        error(vec![test(
            TestOperand::X(0),
            TestOperand::Value(Term::Integer(0))
        )]),
        "register x0 is unset"
    );
    assert_eq!(
        error(vec![
            Instruction::Allocate { stack_size: 1 },
            test(TestOperand::Y(0), TestOperand::Value(Term::Integer(0))),
        ]),
        "stack register y0 is unset"
    );
    assert_eq!(
        error(vec![test(
            TestOperand::Y(0),
            TestOperand::Value(Term::Integer(0))
        )]),
        "stack register y0 is unset"
    );
    assert_eq!(
        error(vec![
            Instruction::Allocate { stack_size: 1 },
            Instruction::Catch {
                register: 0,
                label: 2,
            },
            test(TestOperand::Y(0), TestOperand::Value(Term::Integer(0))),
        ]),
        "stack register y0 holds a catch tag"
    );
    let binary = Instruction::Move(Operand::Value(Term::binary(b"a".to_vec())), Operand::X(0));
    let start = Instruction::BinaryMatchStart {
        fail_label: 2,
        source: Operand::X(0),
        destination: Operand::X(0),
    };
    assert_eq!(
        error(vec![
            binary.clone(),
            start.clone(),
            test(TestOperand::X(0), TestOperand::Value(Term::Integer(0))),
        ]),
        "invariant error: the match context in X(0) reached an instruction that is not a binary match"
    );
    assert_eq!(
        error(vec![
            Instruction::Allocate { stack_size: 1 },
            binary,
            start,
            Instruction::Move(Operand::X(0), Operand::Y(0)),
            test(TestOperand::Y(0), TestOperand::Value(Term::Integer(0))),
        ]),
        "invariant error: the match context in Y(0) reached an instruction that is not a binary match"
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
    assert!(error(vec![element(Term::tuple(vec![]), 0)]).contains("no element 0"));
    assert!(
        error(vec![Instruction::Bif {
            fail_label: 0,
            bif: resolve("erlang", "date", 0).unwrap(),
            arguments: Vec::new(),
            destination: Operand::X(0),
        }])
        .contains("unresolved external MFA erlang:date/0")
    );
}

#[test]
fn calls_a_module_export_without_a_bif_search() {
    // A `call_ext` of module code holds the classification of its import,
    // and that classification is `None` for an MFA that is not a BIF. The
    // run then reaches the export lookup with no search of `BIFS`.
    let body = vec![
        Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(0)),
        Instruction::CallExternal {
            arity: 1,
            module: "fixture".into(),
            name: "guest".into(),
            bif: resolve("fixture", "guest", 1),
            tail: false,
            deallocate: 0,
        },
        Instruction::Return,
    ];
    let mut program = make_program(body, 0);
    let module = Rc::get_mut(program.modules.get_mut("fixture").unwrap()).unwrap();
    module.instructions.extend([
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "guest".into(),
            arity: 1,
        },
        Instruction::Label(11),
        Instruction::Move(Operand::Value(Term::Integer(7)), Operand::X(0)),
        Instruction::Return,
    ]);
    module.reindex();
    let guest = module.tables.name_id("guest").unwrap();
    module.exports.insert((guest, 1), 11);
    let before = resolve_calls();
    assert_eq!(
        program.execute("fixture", "main", &[]).unwrap(),
        Term::Integer(7)
    );
    assert_eq!(
        resolve_calls(),
        before,
        "a `call_ext` of module code must reach the export lookup with no search of `BIFS`"
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
    assert_eq!(
        make_program(vec![Instruction::Return], 1)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "call arity does not match fixture:main"
    );
}

/// A module name and a function name that hold a non-ASCII character print
/// as the pinned ERTS prints the atoms of the same names. The pinned ERTS
/// prints `~p` of the atom `list_to_atom([109, 228, 105, 110])` as the UTF-8
/// bytes of `mäin` (measured with `mise run erts-eval`), and a Rust `String`
/// holds those bytes.
#[test]
fn prints_a_non_ascii_module_and_function_name_in_a_call_path_error() {
    let instructions = vec![
        Instruction::FunctionInfo {
            module: "mäin_module".into(),
            name: "mäin".into(),
            arity: 0,
        },
        Instruction::Label(1),
        Instruction::Label(2),
        Instruction::End,
    ];
    let exports = HashMap::from([(("mäin".into(), 0), 2)]);
    let module = BeamModule::new("mäin_module".into(), exports, instructions);
    let mut program = BeamProgram {
        modules: HashMap::from([("mäin_module".into(), Rc::new(module))]),
        atoms: Arc::new(AtomTable::new()),
    };
    assert_eq!(
        program
            .execute("mäin_module", "mäin", &[])
            .unwrap_err()
            .to_string(),
        "function mäin_module:mäin did not return"
    );
    // The export names arity 1 and the function holds arity 0, so the frame
    // creation gives the arity error.
    let module = Rc::get_mut(program.modules.get_mut("mäin_module").unwrap()).unwrap();
    module
        .exports
        .insert((module.tables.name_id("mäin").unwrap(), 1), 2);
    assert_eq!(
        program
            .execute(
                "mäin_module",
                "mäin",
                &[program.host_term(Term::Integer(1))]
            )
            .unwrap_err()
            .to_string(),
        "call arity does not match mäin_module:mäin"
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
    let module = Rc::get_mut(program.modules.get_mut("fixture").unwrap()).unwrap();
    let main = module.tables.name_id("main").unwrap();
    module.exports.insert((main, 1), 2);

    assert_eq!(
        program
            .execute(
                "fixture",
                "main",
                &[program.host_term(Term::Pid(ProcessId(1)))]
            )
            .unwrap(),
        test_atom("pid")
    );
    assert_eq!(
        program
            .execute("fixture", "main", &[program.host_term(Term::Integer(1))])
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
                Operand::Value(Term::Map(Map::new())),
                Operand::Value(Term::Integer(1))
            )],
        }])
        .contains("unsupported map key Map([])")
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
                    pairs: vec![(Operand::Value(Term::tuple(vec![])), atom("cooked"))],
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
                bif: resolve("maps", "merge", 2),
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
    // ERTS accepts a map key; a PBeam map does not hold one yet.
    let pair = Term::tuple(vec![Term::Map(Map::new()), Term::Integer(0)]);
    let from_list = make_program(
        vec![
            Instruction::Move(Operand::Value(Term::list_from(vec![pair])), Operand::X(0)),
            Instruction::CallExternal {
                arity: 1,
                module: "maps".into(),
                name: "from_list".into(),
                bif: resolve("maps", "from_list", 1),
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
        "unsupported map key Map([])"
    );
}

#[test]
fn reports_a_maps_put_key_that_the_map_form_does_not_support() {
    // ERTS accepts a map key; a PBeam map does not hold one yet.
    let put = make_program(
        vec![
            Instruction::Move(Operand::Value(Term::Map(Map::new())), Operand::X(0)),
            Instruction::Move(Operand::Value(Term::Integer(0)), Operand::X(1)),
            Instruction::Move(Operand::Value(Term::Map(Map::new())), Operand::X(2)),
            Instruction::CallExternal {
                arity: 3,
                module: "maps".into(),
                name: "put".into(),
                bif: resolve("maps", "put", 3),
                tail: true,
                deallocate: 0,
            },
            Instruction::Return,
        ],
        0,
    )
    .execute("fixture", "main", &[]);
    assert_eq!(put.unwrap_err().to_string(), "unsupported map key Map([])");
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
                    bif: resolve("maps", "keys", 1),
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
        Term::list_from(vec![Term::Integer(1), Term::Integer(2), a])
    );
    // ERTS 29.1.1: maps:keys(#{true => V, abs_path => V, 1 => V}) is [1,true,abs_path].
    assert_eq!(
        keys(map(vec![atom("abs_path"), atom("true"), Term::Integer(1)])).unwrap(),
        Term::list_from(vec![Term::Integer(1), atom("true"), atom("abs_path")])
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
                bif: resolve("maps", "keys", 1),
                tail: true,
                deallocate: 0,
            },
            Instruction::Return,
        ],
        0,
    );
    program.modules.insert(
        "maps".into(),
        Rc::new(BeamModule::new(
            "maps".into(),
            HashMap::from([(("keys".into(), 1), 1)]),
            vec![
                Instruction::FunctionInfo {
                    module: "maps".into(),
                    name: "keys".into(),
                    arity: 1,
                },
                Instruction::Label(1),
                Instruction::Move(atom("module_code"), Operand::X(0)),
                Instruction::Return,
            ],
        )),
    );
    // The BIF returns the keys of the empty map. The module code would
    // return `module_code`.
    assert_eq!(
        program.execute("fixture", "main", &[]).unwrap(),
        Term::nil()
    );
}

#[test]
fn never_runs_the_stub_of_a_table_bif_that_pbeam_does_not_implement() {
    let mut program = make_program(
        vec![
            Instruction::Move(atom("a"), Operand::X(0)),
            Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(1)),
            Instruction::Move(Operand::Value(Term::nil()), Operand::X(2)),
            Instruction::CallExternal {
                arity: 3,
                module: "lists".into(),
                name: "keysearch".into(),
                bif: resolve("lists", "keysearch", 3),
                tail: true,
                deallocate: 0,
            },
            Instruction::Return,
        ],
        0,
    );
    program.modules.insert(
        "lists".into(),
        Rc::new(BeamModule::new(
            "lists".into(),
            HashMap::from([(("keysearch".into(), 3), 1)]),
            vec![
                Instruction::FunctionInfo {
                    module: "lists".into(),
                    name: "keysearch".into(),
                    arity: 3,
                },
                Instruction::Label(1),
                Instruction::Move(atom("stub"), Operand::X(0)),
                Instruction::Return,
            ],
        )),
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
/// `true`, and `tests/integration/bifs.rs` checks this list on the pinned ERTS. The
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
    let mut builtin: Vec<(&str, &str, u32)> = include_str!("../../fixtures/bifs/is_builtin.txt")
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
                    bif: resolve("unicode", "characters_to_binary", 2),
                    tail: true,
                    deallocate: 0,
                },
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    let chardata = || Term::list_from(vec![Term::binary(b"Hello".to_vec()), Term::Integer(10)]);
    assert_eq!(
        convert(chardata(), "unicode").unwrap(),
        Term::binary(b"Hello\n".to_vec())
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
        Term::binary(b"Hello\n".to_vec())
    );
    // ERTS 29.1.1: unicode:characters_to_binary([<<"Hello">>, 10], latin1)
    // is <<"Hello\n">>, and unicode:characters_to_binary([<<233>>], latin1)
    // is <<195,169>>.
    assert_eq!(
        convert(chardata(), "latin1").unwrap(),
        Term::binary(b"Hello\n".to_vec())
    );
    assert_eq!(
        convert(Term::list_from(vec![Term::binary(vec![233])]), "latin1").unwrap(),
        Term::binary(vec![195, 169])
    );
    assert_eq!(
        convert(chardata(), "utf16").unwrap_err().to_string(),
        "unsupported unicode:characters_to_binary/2 encoding Atom(\"utf16\")"
    );
}

#[test]
fn builds_a_proper_and_an_improper_list() {
    let list = make_program(
        vec![
            Instruction::PutList {
                head: Operand::Value(Term::Integer(10)),
                tail: Operand::Value(Term::nil()),
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
        0,
    );
    assert_eq!(
        list.execute("fixture", "main", &[]).unwrap(),
        Term::list_from(vec![Term::Integer(10)])
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
        Term::list_with_tail(vec![Term::Integer(10)], Term::Integer(0))
    );
}

/// The values come from a pinned OTP 29.1.1 run of `lst/1`, `hd1/1`, and
/// `nil/1` in a module compiled with `erlc +no_line_info`:
/// `lst([H | T]) -> {H, T}.`, `hd1([H | _]) -> H.`, and
/// `nil([]) -> nil; nil([_ | _]) -> nonempty; nil(_) -> other.`
#[test]
fn reads_and_tests_lists_as_pinned_erts() {
    let list = |names: &[&str]| Term::list_from(names.iter().map(|name| test_atom(name)));
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
        Term::tuple(vec![test_atom("a"), list(&["b", "c"])])
    );
    assert_eq!(
        get_list(list(&["a"])).unwrap(),
        Term::tuple(vec![test_atom("a"), list(&[])])
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
        Term::list_with_tail(
            names.iter().map(|name| test_atom(name)).collect(),
            test_atom(tail),
        )
    };
    assert_eq!(
        get_list(improper(&["a"], "x")).unwrap(),
        Term::tuple(vec![test_atom("a"), test_atom("x")])
    );
    assert_eq!(
        get_list(improper(&["a", "b"], "x")).unwrap(),
        Term::tuple(vec![test_atom("a"), improper(&["b"], "x")])
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
        Operand::Value(Term::tuple(vec![
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
        Term::tuple(vec![
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
        Term::tuple(vec![
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

/// The source operand keeps its tuple when `update_record` writes another
/// register. `read` gives the source a copy that shares the element array, so
/// the source holds the array too, and `Arc::make_mut` copies the array before
/// the write. ERTS copies the whole record for the `copy` hint
/// (`i_update_record_copy`, emu `instrs.tab:785-802`; JIT `emit_update_record`,
/// `jit/arm/instr_common.cpp:893-998`), so the source survives there as well.
#[test]
fn keeps_the_source_record_of_update_record_as_pinned_erts() {
    let record = |values: &[i64]| {
        let mut elements = vec![test_atom("r")];
        elements.extend(values.iter().copied().map(Term::Integer));
        Term::tuple(elements)
    };
    let result = make_program(
        vec![
            Instruction::Move(Operand::Value(record(&[1, 2, 3])), Operand::X(0)),
            Instruction::Move(Operand::Value(Term::Integer(9)), Operand::X(1)),
            Instruction::UpdateRecord {
                source: Operand::X(0),
                destination: Operand::X(1),
                updates: vec![(3, Operand::X(1))],
            },
            // Both the source and the result leave the run, so the test reads
            // each one.
            Instruction::PutTuple {
                elements: vec![Operand::X(0), Operand::X(1)],
                destination: Operand::X(0),
            },
            Instruction::Return,
        ],
        0,
    )
    .execute("fixture", "main", &[])
    .unwrap();
    assert_eq!(
        result,
        Term::tuple(vec![record(&[1, 2, 3]), record(&[1, 9, 3])])
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
        let mut body = spawn_link_child(Term::list_from(vec![Term::Integer(1)]));
        body.pop();
        body.extend([
            Instruction::Move(
                Operand::Value(Term::list_from(vec![test_atom("monitor")])),
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
fn raises_class_error_for_the_raw_stack_trace_nil() {
    // ERTS `erts_sanitize_freason` gives class `error` for the raw stack
    // trace `NIL`, and `build_stacktrace` gives `[]` for it
    // (`beam_common.c:1089-1097`, `1219-1220`). ERTS then saves a new stack
    // trace from the frames of the callers; PBeam records no frames.
    let raise = |trace: Term| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(trace), Operand::X(0)),
                Instruction::BuildStacktrace,
                Instruction::Raise {
                    trace: Operand::X(0),
                    value: atom("r"),
                },
            ],
            0,
        )
        .execute("fixture", "main", &[])
        .unwrap_err()
        .to_string()
    };
    assert_eq!(raise(Term::nil()), "root process exited with {r,[]}");
    // Compiled code gives `raise` only a raw stack trace. ERTS reads each
    // other term as a raw stack trace with no check.
    for trace in [
        test_atom("foo"),
        Term::tuple(vec![test_atom("foo"), Term::nil()]),
    ] {
        assert_eq!(
            raise(trace.clone()),
            format!("invariant error: {trace:?} is not a raw stack trace")
        );
    }
}

#[test]
fn exits_with_the_error_of_an_error_path_instruction_with_no_catch() {
    // With no catch tag, the exception of `badmatch` and `if_end` ends the
    // process with `{Reason, Stacktrace}` (`beam_common.c:684-692`).
    let exit = |instruction: Instruction| {
        make_program(vec![instruction], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        exit(Instruction::TaggedError {
            tag: "badmatch",
            value: atom("r"),
        }),
        "root process exited with {{badmatch,r},[]}"
    );
    assert_eq!(
        exit(Instruction::IfEnd),
        "root process exited with {if_clause,[]}"
    );
}

#[test]
fn raw_raise_writes_badarg_for_a_class_that_is_not_valid() {
    // ERTS `raw_raise` writes `badarg` to `x0` and continues for a class
    // that is not `error`, `exit`, or `throw` (emu `instrs.tab:1253-1263`,
    // `beam_common.c:1148-1179`). The compiler writes `badarg` to `x0` after
    // `raw_raise` too, so a compiled fixture does not show this write.
    let raw_raise = |class: &str| {
        make_program(
            vec![
                Instruction::Move(atom(class), Operand::X(0)),
                Instruction::Move(atom("r"), Operand::X(1)),
                Instruction::Move(Operand::Value(Term::nil()), Operand::X(2)),
                Instruction::RawRaise,
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    assert_eq!(raw_raise("foo").unwrap(), Term::static_atom("badarg"));
    // The raw stack trace `[]` keeps the class of `x0`.
    assert_eq!(
        raw_raise("exit").unwrap_err().to_string(),
        "root process exited with r"
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
                    bif: resolve("absent", "run", 0),
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
    assert_eq!(
        make_program(vec![Instruction::End], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "function fixture:main did not return"
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

/// The loader tables answer a bad label, a bad entry, and a bad register
/// index with the error text of the removed scans.
#[test]
fn keeps_the_error_text_of_a_bad_label_entry_and_register() {
    assert_eq!(
        make_program(vec![Instruction::Jump(9)], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "missing BEAM label 9"
    );
    assert_eq!(
        make_program(vec![Instruction::Call { arity: 0, label: 9 }], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "entry label 9 not found in fixture"
    );
    assert_eq!(
        make_program(vec![Instruction::Move(Operand::X(1024), Operand::X(0))], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "register x1024 is unset"
    );
}

#[test]
fn rejects_unresolved_exports_and_function_labels() {
    let module = BeamModule::new("external".into(), HashMap::new(), vec![]);
    let program = BeamProgram {
        modules: HashMap::from([("external".into(), Rc::new(module))]),
        atoms: Arc::new(AtomTable::new()),
    };
    assert!(
        CodeTable::new(program.modules.clone())
            .mfa_frame("missing", "run", 0)
            .unwrap_err()
            .to_string()
            .contains("unresolved external MFA missing:run/0")
    );
    assert!(
        CodeTable::new(program.modules.clone())
            .mfa_frame("external", "missing", 0)
            .unwrap_err()
            .to_string()
            .contains("external:missing/0")
    );
    let module = BeamModule::new(
        "fixture".into(),
        HashMap::new(),
        vec![
            Instruction::FunctionInfo {
                module: "other".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(2),
        ],
    );
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

/// PBeam lets a match context reach only the binary-match instructions,
/// `move`, and local calls. It gives a runtime invariant error for each
/// other use, and for a binary-match instruction that gets another term.
#[test]
fn gives_an_invariant_error_for_a_match_context_outside_binary_matching() {
    let run = |rest: Vec<Instruction>| {
        let mut body = vec![
            Instruction::Move(Operand::Value(Term::binary(b"a".to_vec())), Operand::X(0)),
            Instruction::BinaryMatchStart {
                fail_label: 1,
                source: Operand::X(0),
                destination: Operand::X(0),
            },
        ];
        body.extend(rest);
        // `child/1` does not read its argument.
        let mut program = make_program(body, 0);
        let module = Rc::get_mut(program.modules.get_mut("fixture").unwrap()).unwrap();
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
        module.reindex();
        let child = module.tables.name_id("child").unwrap();
        module.exports.insert((child, 1), 11);
        program
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string()
    };
    let escape = "invariant error: the match context in X(0) reached an instruction that \
                  is not a binary match";
    // `swap` of a match context is not implemented yet.
    assert_eq!(
        run(vec![Instruction::Swap(Operand::X(0), Operand::X(1))]),
        escape
    );
    assert_eq!(run(vec![erlang_call("hd", 1, false)]), escape);
    assert_eq!(
        run(vec![Instruction::Bif {
            fail_label: 0,
            bif: BifId::ErlangHd1,
            arguments: vec![Operand::X(0)],
            destination: Operand::X(1),
        }]),
        escape
    );
    // An external call to a loaded function also checks its arguments.
    assert_eq!(
        run(vec![Instruction::CallExternal {
            arity: 1,
            module: "fixture".into(),
            name: "child".into(),
            bif: resolve("fixture", "child", 1),
            tail: false,
            deallocate: 0,
        }]),
        escape
    );
    assert_eq!(
        run(vec![Instruction::BinaryMatchResume {
            source: atom("a"),
            destination: Operand::X(0),
        }]),
        "invariant error: Value(Atom(\"a\")) holds Atom(\"a\"), not a match context"
    );
    // `bs_start_match4` with `no_fail` has the fail label 0. The compiler
    // gives it only a binary or a match context.
    assert_eq!(
        run(vec![Instruction::BinaryMatchStart {
            fail_label: 0,
            source: atom("a"),
            destination: Operand::X(1),
        }]),
        "invariant error: bs_start_match3 with no fail label got Atom(\"a\"), not a binary or a \
         match context"
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

/// A copy of a match context to another register shares the match
/// context, as ERTS copies the term that refers to it (`emit_i_move`,
/// jit/arm/instr_common.cpp:516-519; `emit_i_bs_start_match3`,
/// jit/arm/instr_bs.cpp:158-159, 187-188; jit/arm/ops.tab:1036-1045). A
/// read of the copy moves the position of the source, and a change of the
/// position of the source moves the position of the copy.
#[test]
fn shares_a_match_context_that_an_instruction_copies_to_another_register() {
    let run = |copy: Instruction, to: Operand| {
        let body = vec![
            Instruction::Allocate { stack_size: 1 },
            Instruction::Move(Operand::Value(Term::binary(b"ab".to_vec())), Operand::X(0)),
            Instruction::BinaryMatchStart {
                fail_label: 1,
                source: Operand::X(0),
                destination: Operand::X(0),
            },
            copy,
            // Read `a` with the copy, then the position with the source.
            Instruction::BinaryGetUtf8 {
                fail_label: 1,
                context: to.clone(),
                destination: Operand::X(2),
            },
            Instruction::BinaryGetPosition {
                context: Operand::X(0),
                destination: Operand::X(3),
            },
            // Set the position 0 with the source, then read `a` again with
            // the copy.
            Instruction::BinarySetPosition {
                context: Operand::X(0),
                position: Operand::Value(Term::Integer(0)),
            },
            Instruction::BinaryGetUtf8 {
                fail_label: 1,
                context: to,
                destination: Operand::X(4),
            },
            Instruction::PutTuple {
                destination: Operand::X(0),
                elements: vec![Operand::X(2), Operand::X(3), Operand::X(4)],
            },
            Instruction::Deallocate { stack_size: 1 },
            Instruction::Return,
        ];
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap()
    };
    let expected = Term::tuple(vec![Term::Integer(97), Term::Integer(8), Term::Integer(97)]);
    assert_eq!(
        run(
            Instruction::Move(Operand::X(0), Operand::Y(0)),
            Operand::Y(0)
        ),
        expected
    );
    assert_eq!(
        run(
            Instruction::BinaryMatchStart {
                fail_label: 1,
                source: Operand::X(0),
                destination: Operand::X(1),
            },
            Operand::X(1)
        ),
        expected
    );
    assert_eq!(
        run(
            Instruction::BinaryMatchResume {
                source: Operand::X(0),
                destination: Operand::X(1),
            },
            Operand::X(1)
        ),
        expected
    );
}

/// Run `bs_match` with `commands` on a match context of `bytes` in `x0`,
/// then the instructions of `then`. The fail label 3 returns `failed`.
fn run_binary_match(
    bytes: &[u8],
    commands: Vec<BinaryMatchCommand>,
    then: Vec<Instruction>,
) -> Result<Term, String> {
    let mut body = vec![
        Instruction::Move(Operand::Value(Term::binary(bytes.to_vec())), Operand::X(0)),
        Instruction::BinaryMatchStart {
            fail_label: 1,
            source: Operand::X(0),
            destination: Operand::X(0),
        },
        Instruction::BinaryMatch {
            fail_label: 3,
            context: Operand::X(0),
            commands,
        },
    ];
    body.extend(then);
    body.extend([
        Instruction::Label(3),
        Instruction::Move(atom("failed"), Operand::X(0)),
        Instruction::Return,
    ]);
    make_program(body, 0)
        .execute("fixture", "main", &[])
        .map(|host| host.into_term())
        .map_err(|error| error.to_string())
}

/// The `gc_bif` instruction of `byte_size/1` of `argument` to `x0`.
fn byte_size(argument: Operand) -> Instruction {
    Instruction::Bif {
        fail_label: 0,
        bif: BifId::ErlangByteSize1,
        arguments: vec![argument],
        destination: Operand::X(0),
    }
}

/// `byte_size/1` of a match context gives the bits from its position to
/// its end in bytes, rounded up, as `byte_size/1` of the same bits as a
/// bitstring (`emit_bif_byte_size`, jit/arm/instr_guard_bifs.cpp:458-509).
/// On the pinned ERTS, `byte_size(<<1:12>>)` is 2.
#[test]
fn gives_the_byte_size_of_a_match_context_from_its_position() {
    let unit = std::num::NonZeroU64::new(1).unwrap();
    let skip = |size| {
        vec![
            BinaryMatchCommand::EnsureAtLeast { size, unit },
            BinaryMatchCommand::Skip { size },
        ]
    };
    let size = |size| {
        run_binary_match(
            &[1, 2, 3],
            skip(size),
            vec![byte_size(Operand::X(0)), Instruction::Return],
        )
    };
    assert_eq!(size(0), Ok(Term::Integer(3)));
    assert_eq!(size(4), Ok(Term::Integer(3)));
    assert_eq!(size(8), Ok(Term::Integer(2)));
    assert_eq!(size(20), Ok(Term::Integer(1)));
    assert_eq!(size(24), Ok(Term::Integer(0)));
}

/// A failed `=:=` command of `bs_match` leaves the position unknown until
/// `bs_set_position`. ERTS has it at the end of the read of the command
/// (jit/arm/instr_bs.cpp:2995-3045), which PBeam does not model.
#[test]
fn gives_a_runtime_error_for_a_read_after_a_failed_equal_command() {
    let unit = std::num::NonZeroU64::new(8).unwrap();
    let skip = BinaryMatchCommand::Skip { size: 8 };
    let byte = |destination| BinaryMatchCommand::Integer {
        size: 8,
        little: false,
        signed: false,
        destination,
    };
    // `bs_get_position` x0 x1 before the match, and `bs_set_position` x0
    // x1 at the fail label, as the compiler restores the position.
    let restore = |rest: Vec<Instruction>| {
        let mut body = vec![
            Instruction::Move(Operand::Value(Term::binary(vec![7, 8])), Operand::X(0)),
            Instruction::BinaryMatchStart {
                fail_label: 1,
                source: Operand::X(0),
                destination: Operand::X(0),
            },
            Instruction::BinaryGetPosition {
                context: Operand::X(0),
                destination: Operand::X(1),
            },
            Instruction::BinaryMatch {
                fail_label: 3,
                context: Operand::X(0),
                commands: vec![
                    BinaryMatchCommand::EnsureAtLeast { size: 16, unit },
                    BinaryMatchCommand::Equal { size: 8, value: 1 },
                ],
            },
            Instruction::Move(atom("matched"), Operand::X(0)),
            Instruction::Return,
            Instruction::Label(3),
        ];
        body.extend(rest);
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .map(|host| host.into_term())
            .map_err(|error| error.to_string())
    };
    assert_eq!(
        restore(vec![
            Instruction::BinarySetPosition {
                context: Operand::X(0),
                position: Operand::X(1),
            },
            Instruction::BinaryMatch {
                fail_label: 1,
                context: Operand::X(0),
                commands: vec![byte(Operand::X(0))],
            },
            Instruction::Return,
        ]),
        Ok(Term::Integer(7))
    );
    assert_eq!(
        restore(vec![Instruction::BinaryMatch {
            fail_label: 1,
            context: Operand::X(0),
            commands: vec![skip],
        }]),
        Err(
            "unsupported read of the match context in X(0) after a failed bs_match =:= command"
                .into()
        )
    );
    assert_eq!(
        restore(vec![byte_size(Operand::X(0))]),
        Err(
            "unsupported read of the match context in X(0) after a failed bs_match =:= command"
                .into()
        )
    );
    // A successful `=:=` moves the position, and an ensure command that
    // fails jumps to the fail label.
    assert_eq!(
        run_binary_match(
            &[1, 2],
            vec![
                BinaryMatchCommand::Equal { size: 8, value: 1 },
                byte(Operand::X(0))
            ],
            vec![Instruction::Return]
        ),
        Ok(Term::Integer(2))
    );
    assert_eq!(
        run_binary_match(
            &[1],
            vec![BinaryMatchCommand::EnsureAtLeast { size: 16, unit }],
            vec![Instruction::Return]
        ),
        Ok(test_atom("failed"))
    );
}

/// The compiler puts an ensure command before each command that reads
/// bits, so a read past the end is a runtime invariant error.
#[test]
fn gives_an_invariant_error_for_a_bs_match_read_past_the_end() {
    let past_end = |command| run_binary_match(&[1], vec![command], vec![Instruction::Return]);
    let error = |size: u64| {
        Err(format!(
            "invariant error: a bs_match command needs {size} bits, but 8 bits remain"
        ))
    };
    assert_eq!(
        past_end(BinaryMatchCommand::Integer {
            size: 16,
            little: false,
            signed: false,
            destination: Operand::X(1),
        }),
        error(16)
    );
    assert_eq!(
        past_end(BinaryMatchCommand::Binary {
            size: 16,
            destination: Operand::X(1),
        }),
        error(16)
    );
    assert_eq!(
        past_end(BinaryMatchCommand::Equal { size: 9, value: 0 }),
        error(9)
    );
    assert_eq!(past_end(BinaryMatchCommand::Skip { size: 9 }), error(9));
}

#[test]
fn handles_register_values_and_reports_invalid_registers() {
    let mut registers = vec![Some(Term::Integer(7))];
    assert_eq!(
        read_operand(&Operand::X(0), &registers, &[]).unwrap(),
        &Term::Integer(7)
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
fn reads_bif_arguments_into_a_fixed_buffer_and_reports_invalid_operands() {
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    process.registers = vec![Some(Term::Integer(7))];
    let arguments =
        read_bif_arguments(&process, &[Operand::X(0), Operand::Value(Term::Integer(9))])
            .expect("two set operands are valid arguments");
    assert_eq!(
        arguments.with(|terms| terms.to_vec()),
        vec![Term::Integer(7), Term::Integer(9)]
    );
    assert!(read_bif_arguments(&process, &[Operand::X(1)]).is_err());
    assert!(
        read_bif_arguments(
            &process,
            &[Operand::X(0), Operand::X(0), Operand::X(0), Operand::X(0)]
        )
        .is_err()
    );
}

#[test]
fn borrows_literal_and_register_operands_without_a_copy() {
    // A read-only instruction gets a reference to the literal or the
    // register itself, so a large tuple is not copied.
    let tuple = || Term::tuple(vec![Term::Integer(1); 1000]);
    let literal = Operand::Value(tuple());
    let registers = vec![Some(tuple())];
    let stack = vec![StackSlot::Term(tuple())];
    let (Operand::Value(literal_term), Some(x0), StackSlot::Term(y0)) =
        (&literal, &registers[0], &stack[0])
    else {
        unreachable!("the fixture holds terms");
    };
    let read =
        |operand: &Operand| read_operand(operand, &registers, &stack).unwrap() as *const Term;
    assert_eq!(read(&literal), literal_term as *const Term);
    assert_eq!(read(&Operand::X(0)), x0 as *const Term);
    assert_eq!(read(&Operand::Y(0)), y0 as *const Term);
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
        select(Term::tuple(vec![Term::Integer(1)])),
        test_atom("one")
    );
    assert_eq!(select(Term::tuple(vec![])), test_atom("other"));
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
                Operand::Value(Term::list_from(
                    "pbeam_alpha"
                        .bytes()
                        .map(|byte| Term::Integer(i64::from(byte))),
                )),
                Operand::X(0),
            ),
            erlang_call("list_to_atom", 1, true),
            Instruction::Return,
        ],
        0,
    );
    assert_eq!(
        Arc::get_mut(&mut program.atoms)
            .unwrap()
            .put("pbeam_zeta")
            .unwrap()
            .index(),
        1150
    );
    for _ in 0..2 {
        assert!(matches!(
            program.execute("fixture", "main", &[]).unwrap().term,
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
    let hello = Term::binary(b"Hello".to_vec());
    assert_eq!(run(hello.clone()), Term::list_from(vec![hello]));
    assert_eq!(
        run(Term::list_from(vec![
            Term::Integer(1),
            Term::list_from(vec![Term::binary(vec![2])]),
        ])),
        Term::list_from(vec![Term::binary(vec![1, 2])])
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
        Term::tuple(vec![
            Term::static_atom("EXIT"),
            Term::tuple(vec![Term::static_atom("undef"), Term::nil()]),
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
                element_bif(index, Operand::Value(Term::tuple(vec![])), Operand::X(0)),
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
        Term::tuple(vec![
            test_atom("EXIT"),
            Term::tuple(vec![test_atom("badarg"), Term::nil()]),
        ])
    );
    // Without a catch, the exception exits the process.
    let body = vec![
        element_bif(1, Operand::Value(Term::tuple(vec![])), Operand::X(0)),
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
            bif: BifId::ErlangDemonitor2,
            arguments: vec![
                Operand::X(0),
                Operand::Value(Term::list_from(vec![test_atom("flush")])),
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
    let node = |bif: BifId, arguments: Vec<Operand>| {
        make_program(
            vec![
                Instruction::Bif {
                    fail_label: 0,
                    bif: BifId::ErlangSelf0,
                    arguments: Vec::new(),
                    destination: Operand::X(1),
                },
                Instruction::Bif {
                    fail_label: 0,
                    bif,
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
        node(BifId::ErlangNode0, Vec::new()).unwrap(),
        Term::static_atom("nonode@nohost")
    );
    assert_eq!(
        node(BifId::ErlangNode1, vec![Operand::X(1)]).unwrap(),
        Term::static_atom("nonode@nohost")
    );
    assert_eq!(
        node(BifId::ErlangNode1, vec![atom("foo")])
            .unwrap_err()
            .to_string(),
        "unsupported erlang:node/1 argument Atom(\"foo\"): not a pid"
    );
}

#[test]
fn traps_a_tail_called_demonitor_flush_into_erts_internal() {
    let mut program = make_program(
        vec![
            erlang_call("make_ref", 0, false),
            Instruction::Move(
                Operand::Value(Term::list_from(vec![test_atom("flush")])),
                Operand::X(1),
            ),
            erlang_call("demonitor", 2, true),
            Instruction::Return,
        ],
        0,
    );
    program.modules.insert(
        "erts_internal".into(),
        Rc::new(BeamModule::new(
            "erts_internal".into(),
            HashMap::from([(("flush_monitor_messages".into(), 3), 2)]),
            vec![
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
        )),
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
    let mut body = spawn_link_child(Term::list_from(vec![test_atom("normal")]));
    body.pop();
    body.push(erlang_call("spawn", 3, false));
    body.extend([
        Instruction::Move(Operand::X(0), Operand::X(1)),
        Instruction::Move(atom("process"), Operand::X(0)),
        erlang_call("monitor", 2, false),
        Instruction::Move(
            Operand::Value(Term::list_from(vec![test_atom("flush")])),
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
    apply_targets_program(body)
        .execute("fixture", "main", &[])
        .map(|host| host.into_term())
}

/// Return the program of `run_with_apply_targets`.
fn apply_targets_program(body: Vec<Instruction>) -> BeamProgram {
    let mut program = make_program(body, 0);
    let module = Rc::get_mut(program.modules.get_mut("fixture").unwrap()).unwrap();
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
    module.reindex();
    let zero = module.tables.name_id("zero").unwrap();
    let id = module.tables.name_id("id").unwrap();
    let last = module.tables.name_id("last").unwrap();
    module
        .exports
        .extend([((zero, 0), 11), ((id, 3), 12), ((last, 2), 13)]);
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
        Term::tuple(vec![test_atom("a"), test_atom("b"), test_atom("c")])
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
        Term::tuple(vec![test_atom("EXIT"), test_atom("bye")])
    );
}

#[test]
fn applies_erlang_apply_3_as_the_i_apply_instruction() {
    let xyz = || Term::list_from(vec![test_atom("x"), test_atom("y"), test_atom("z")]);
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
    let tuple = Term::tuple(vec![test_atom("x"), test_atom("y"), test_atom("z")]);
    assert_eq!(run_with_apply_targets(body).unwrap(), tuple);
    // `erlang:apply(erlang, apply, [fixture, id, [x, y, z]])` as a tail
    // `call_ext` runs the inner call.
    let mut body = apply_of(
        test_atom("erlang"),
        test_atom("apply"),
        Term::list_from(vec![test_atom("fixture"), test_atom("id"), xyz()]),
    );
    body.push(erlang_call("apply", 3, true));
    assert_eq!(run_with_apply_targets(body).unwrap(), tuple);
    // An apply of `erlang:apply` with a list that is not three elements
    // calls `erlang:apply/2`. ERTS 29.1.1 raises `badarg` for
    // `apply(erlang, apply, [fixture, id])`, because `id` is not a list.
    let mut body = apply_of(
        test_atom("erlang"),
        test_atom("apply"),
        Term::list_from(vec![test_atom("fixture"), test_atom("id")]),
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
        Term::list_from(vec![test_atom("x"); 1023]),
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
        Arc::get_mut(&mut program.atoms).unwrap().put(name).unwrap();
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
        Term::list_from(vec![test_atom("x"); 1024]),
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
        bif: resolve(module, name, arity),
        tail: true,
        deallocate: 0,
    });
    body.push(Instruction::Return);
    make_program(body, 0)
        .execute("fixture", "main", &[])
        .map(|host| host.into_term())
}

fn get_bif(key: &str, destination: Operand) -> Instruction {
    Instruction::Bif {
        fail_label: 0,
        bif: BifId::ErlangGet1,
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
        Term::tuple(vec![
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
        Instruction::Move(Operand::Value(Term::nil()), Operand::X(2)),
        Instruction::Move(
            Operand::Value(Term::list_from(vec![test_atom("monitor")])),
            Operand::X(3),
        ),
        erlang_call("spawn_opt", 4, false),
    ];
    body.extend(wait_forever());
    let mut program = make_program(body, 0);
    let module = Rc::get_mut(program.modules.get_mut("fixture").unwrap()).unwrap();
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
    module.reindex();
    let fresh = module.tables.name_id("fresh").unwrap();
    module.exports.insert((fresh, 0), 11);
    // ERTS 29.1.1: after put(k, parent), a spawned process gets
    // undefined for get(k).
    assert!(matches!(
        program.execute("fixture", "main", &[]).map(|host| host.into_term()),
        Ok(Term::Tuple(ref elements))
            if elements.first() == Some(&test_atom("DOWN"))
                && elements.last() == Some(&Term::static_atom("undefined"))
    ));
}

#[test]
fn appends_lists_as_pinned_erts() {
    let integers = |values: &[i64]| Term::list_from(values.iter().copied().map(Term::Integer));
    let append = |left: Term, right: Term| call_ext_with("erlang", "++", vec![left, right]);
    // ERTS 29.1.1: [] ++ x is x, and [1,2] ++ [3] is [1,2,3].
    assert_eq!(append(Term::nil(), test_atom("x")).unwrap(), test_atom("x"));
    assert_eq!(
        append(integers(&[1, 2]), integers(&[3])).unwrap(),
        integers(&[1, 2, 3])
    );
    // ERTS 29.1.1: [1] ++ x is the improper list [1|x], and a ++ []
    // and a ++ x raise badarg.
    assert_eq!(
        append(integers(&[1]), test_atom("x")).unwrap(),
        Term::list_with_tail(vec![Term::Integer(1)], test_atom("x"))
    );
    for right in [Term::nil(), test_atom("x")] {
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
        vec![Term::list_from(vec![test_atom("a")]), test_atom("x")],
    );
    assert_eq!(
        reverse.unwrap(),
        Term::list_with_tail(vec![test_atom("a")], test_atom("x"))
    );
}

#[test]
fn finds_key_tuples_as_pinned_erts() {
    let tuple = |elements: Vec<Term>| Term::tuple(elements);
    let keyfind = |key: Term, position: Term, list: Vec<Term>| {
        call_ext_with(
            "lists",
            "keyfind",
            vec![key, position, Term::list_from(list)],
        )
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
    let one = Term::list_from(vec![tuple(vec![a.clone()])]);
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
        member(test_atom("a"), Term::nil()).unwrap(),
        Term::static_atom("false")
    );
    assert_eq!(
        member(Term::Integer(1), Term::list_from(vec![Term::Integer(1)])).unwrap(),
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
        bif: BifId::ErlangElement2,
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
            Operand::Value(Term::tuple(vec![a.clone(), b.clone(), c])),
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
    let one = Term::tuple(vec![a.clone()]);
    // ERTS 29.1.1: element(1, {a}) is a.
    assert_eq!(element(Term::Integer(1), one.clone()).unwrap(), a);
    // ERTS 29.1.1 raises badarg for element(0, {a}), element(2, {a}),
    // element(-1, {a}), element(a, {a}), and element(1, [a]).
    for (index, tuple) in [
        (Term::Integer(0), one.clone()),
        (Term::Integer(2), one.clone()),
        (Term::Integer(-1), one.clone()),
        (a.clone(), one),
        (Term::Integer(1), Term::list_from(vec![a])),
    ] {
        assert_eq!(element(index, tuple).unwrap_err().to_string(), BADARG_EXIT);
    }
}

#[test]
fn reads_the_tuple_size_as_pinned_erts() {
    let tuple_size = |term: Term| call_ext_with("erlang", "tuple_size", vec![term]);
    // ERTS 29.1.1: tuple_size({}) is 0, and tuple_size(a) raises badarg.
    assert_eq!(tuple_size(Term::tuple(vec![])).unwrap(), Term::Integer(0));
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
            bif: BifId::ErlangSelf0,
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
    let stacktrace = Term::tuple(vec![test_atom("current_stacktrace"), Term::nil()]);
    assert_eq!(
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap(),
        Term::tuple(vec![stacktrace, Term::nil()])
    );
    let process_info =
        |process: Term, item: Term| call_ext_with("erlang", "process_info", vec![process, item]);
    let root = || Term::Pid(crate::ProcessId(0));
    let dictionary = || Term::tuple(vec![test_atom("dictionary"), test_atom("k")]);
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
            Term::list_from(vec![test_atom("registered_name"), test_atom("bogus")]),
        ),
        (
            root(),
            Term::tuple(vec![test_atom("bogus"), test_atom("k")]),
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
        Term::list_from(vec![test_atom("registered_name")]),
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
        (root(), Term::list_from(vec![test_atom("registered_name")])),
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
        Term::nil()
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
            Operand::Value(Term::list_from(vec![test_atom("normal")])),
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
            Instruction::Move(Operand::Value(Term::nil()), Operand::X(2)),
            Instruction::Move(
                Operand::Value(Term::list_from(vec![test_atom("monitor")])),
                Operand::X(3),
            ),
            erlang_call("spawn_opt", 4, false),
            Instruction::Bif {
                fail_label: 0,
                bif: BifId::ErlangSelf0,
                arguments: Vec::new(),
                destination: Operand::X(1),
            },
            Instruction::PutList {
                head: Operand::X(1),
                tail: Operand::Value(Term::nil()),
                destination: Operand::X(2),
            },
            Instruction::Move(atom("fixture"), Operand::X(0)),
            Instruction::Move(atom("sender"), Operand::X(1)),
            erlang_call("spawn", 3, false),
        ];
        let mut program = make_program([body, wait_forever()].concat(), 0);
        let module = Rc::get_mut(program.modules.get_mut("fixture").unwrap()).unwrap();
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
        module.reindex();
        let burner = module.tables.name_id("burner").unwrap();
        let sender = module.tables.name_id("sender").unwrap();
        module.exports.insert((burner, 0), 21);
        module.exports.insert((sender, 1), 25);
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
        first_message(3846).term,
        Term::Tuple(ref elements) if elements.first() == Some(&test_atom("DOWN"))
    ));
}

#[test]
fn rejects_integer_conversions_of_a_bignum_with_no_reductions_left() {
    // `bump_reductions(4000)` leaves no reductions, and a `bif` instruction
    // has no reduction test. ERTS then runs `integer_to_list/1` and
    // `integer_to_binary/1` of a bignum on a dirty CPU scheduler
    // (`bif.c:3289-3296`, `binary.c:98-105`).
    for (name, bif) in [
        ("integer_to_list", BifId::ErlangIntegerToList1),
        ("integer_to_binary", BifId::ErlangIntegerToBinary1),
    ] {
        let body = vec![
            Instruction::Move(Operand::Value(Term::Integer(4000)), Operand::X(0)),
            erlang_call("bump_reductions", 1, false),
            Instruction::Bif {
                fail_label: 0,
                bif,
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
            format!(
                "unsupported erlang:{name}/1 of 576460752303423488 with 0 reductions left: \
                 ERTS runs it on a dirty CPU scheduler"
            )
        );
    }
}

#[test]
fn shares_the_stored_term_of_an_ets_lookup() {
    // An ETS insert copies the object into the table and a lookup copies it
    // out on ERTS (`db_store_term`, erl_db_util.c:3559, and
    // `db_copy_from_comp`, `:3729`). PBeam shares the term by owner
    // decision R3 of #450 (row "Message copy" of docs/erts-differences.md).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let owner = scheduler.current_process_mut().id;
    let id = Term::Reference(Reference(0));
    scheduler
        .ets
        .create(Reference(0), None, Table::new(owner, Access::Public, 1));
    // A list for the value: the element of the looked-up object shares the
    // cons cell of the inserted term.
    let value = Term::list_from([Term::Integer(2), Term::Integer(3)]);
    let object = Term::tuple(vec![Term::Integer(1), value.clone()]);
    program
        .bif(&mut scheduler, "ets", "insert", vec![id.clone(), object])
        .unwrap();
    let result = program
        .bif(&mut scheduler, "ets", "lookup", vec![id, Term::Integer(1)])
        .unwrap();
    let BifResult::Value(objects) = result else {
        panic!("ets:lookup/2 did not return a value");
    };
    let (mut objects, _) = objects.list_cells().expect("ets:lookup/2 returns a list");
    let found = objects.next().expect("the table holds the object");
    let Term::Tuple(elements) = found else {
        panic!("the stored object is a tuple");
    };
    assert!(value.shares_storage_with(&elements[1]));
}

#[test]
fn shares_the_stored_term_of_a_persistent_term() {
    // `persistent_term:put/2` copies the term to a literal area on ERTS
    // (`erl_bif_persistent.c:744-761`), and `persistent_term:get/1` does not
    // copy (`:380-388`). PBeam shares the term by owner decision R3 of #450
    // (row "Message copy" of docs/erts-differences.md).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let key = Term::tuple(vec![Term::Integer(7)]);
    let value = Term::Map(Map::from_pairs(vec![(Term::Integer(1), Term::Integer(2))]).unwrap());
    program
        .bif(
            &mut scheduler,
            "persistent_term",
            "put",
            vec![key.clone(), value.clone()],
        )
        .unwrap();
    let result = program
        .bif(&mut scheduler, "persistent_term", "get", vec![key])
        .unwrap();
    let BifResult::Value(stored) = result else {
        panic!("persistent_term:get/1 did not return a value");
    };
    assert!(value.shares_storage_with(&stored));
}

#[test]
fn rejects_a_bif_name_that_no_table_entry_holds() {
    // The name path of `BeamProgram::bif` runs a `bif.tab` entry or a PBeam
    // BIF of `PBEAM_BIFS`. An MFA that neither holds gives the unresolved
    // MFA error, which is the error that the run gave before the loader
    // resolved the identity of a BIF.
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    assert_eq!(
        program
            .bif(&mut scheduler, "erlang", "no_such_bif", vec![])
            .unwrap_err()
            .to_string(),
        "unresolved external MFA erlang:no_such_bif/0"
    );
}

/// The decoded slots that one iteration of the loop `function` retires in
/// `module`: the entry (`TestYield` and the clause test) and the loop clause
/// that the fail label of that test names up to its tail call. A `Label` is
/// not dispatched, and the `Return` that the loader puts after a tail call
/// is not reached.
///
/// # Examples
///
/// ```rust,ignore
/// let probe = program.modules["dispatch_probe"].clone();
/// assert_eq!(loop_body(&probe, "floor_loop").len(), 4);
/// ```
fn loop_body(module: &BeamModule, function: &str) -> Vec<usize> {
    let instructions = &module.instructions;
    let info = instructions
        .iter()
        .position(
            |instruction| matches!(instruction, Instruction::FunctionInfo { name, .. } if name == function),
        )
        .unwrap_or_else(|| panic!("the module has no {function}"));
    // The loader writes `func_info`, the entry `label`, and `TestYield`, so
    // the function starts at `TestYield`.
    let start = info + 2;
    assert!(
        matches!(instructions[start], Instruction::TestYield),
        "the function entry of {function} is not `TestYield`: {:?}",
        instructions[start]
    );
    let fail_label = match &instructions[start + 1] {
        Instruction::TestExactEqual { fail_label, .. }
        | Instruction::TestExactEqualOperands { fail_label, .. } => *fail_label,
        other => panic!("the entry clause test of {function} is not `is_eq_exact`: {other:?}"),
    };
    let clause = *module
        .tables
        .jump_targets
        .get(&fail_label)
        .unwrap_or_else(|| panic!("the loader records the jump target of label {fail_label}"));
    let end = (clause..instructions.len())
        .find(|index| {
            matches!(
                instructions[*index],
                Instruction::FunctionInfo { .. } | Instruction::End
            )
        })
        .expect("the function ends before the stream does");
    // The clause ends with the tail call of the loop. A `Return` after a
    // tail call is dead, because the call replaces the frame.
    let tail = (clause..end)
        .find(|index| {
            matches!(
                instructions[*index],
                Instruction::Call { .. }
                    | Instruction::CallOnly { .. }
                    | Instruction::CallExternal { .. }
                    | Instruction::CallLast { .. }
                    | Instruction::Apply { .. }
                    | Instruction::CallFun { .. }
            )
        })
        .unwrap_or(end);
    (start..start + 2)
        .chain(clause..=tail)
        .filter(|index| !matches!(instructions[*index], Instruction::Label(_)))
        .collect()
}

/// The bytes of the heap operands of the instructions of `slots`: the
/// argument vector of a `Bif` and the two names of a `CallExternal`. The
/// first number counts the operands that the instruction reads, and the
/// second counts the storage that the loader allocated for them. Every other
/// operand lives inside the 136-byte slot of its instruction.
///
/// # Examples
///
/// ```rust,ignore
/// let slots = loop_body(&program.modules["dispatch_probe"], "floor_loop");
/// assert_eq!(heap_operand_bytes(&program.modules["dispatch_probe"], &slots), (80, 160));
/// ```
fn heap_operand_bytes(module: &BeamModule, slots: &[usize]) -> (usize, usize) {
    let mut read = 0;
    let mut allocated = 0;
    for index in slots {
        match &module.instructions[*index] {
            Instruction::Bif { arguments, .. } => {
                read += arguments.len() * std::mem::size_of::<Operand>();
                allocated += arguments.capacity() * std::mem::size_of::<Operand>();
            }
            Instruction::CallExternal { module, name, .. } => {
                read += module.len() + name.len();
                allocated += module.capacity() + name.capacity();
            }
            _ => {}
        }
    }
    (read, allocated)
}

/// Record the decoded form that one iteration of each measured loop retires:
/// the number of instructions, the bytes of their slots, the bytes of the
/// heap operands that they read, and the bytes of the address range that
/// their slots cover. A scratch build of the run loop prints `module.id` and
/// the instruction index of each retired instruction, and its trace gives
/// the slot set that this test derives from the compiled shape.
#[test]
fn records_the_decoded_loop_body() {
    const DISPATCH_PROBE: &[u8] = include_bytes!("../../fixtures/dispatch/dispatch_probe.beam");
    const POSITION_PROBE: &[u8] =
        include_bytes!("../../fixtures/dispatch/dispatch_position_probe.beam");
    let program = BeamProgram::load(&[DISPATCH_PROBE, POSITION_PROBE]).unwrap();

    let shapes = [
        ("dispatch_probe", "floor_ext_loop", 4),
        ("dispatch_probe", "floor_loop", 4),
        ("dispatch_position_probe", "floor_loop", 4),
        ("dispatch_probe", "plus_loop", 5),
        ("dispatch_probe", "tuple_size_loop", 5),
        ("dispatch_probe", "element_loop", 5),
        ("dispatch_probe", "hd_loop", 5),
        ("dispatch_probe", "map_get_loop", 5),
    ];

    println!(
        "| loop | instructions for each iteration | bytes of the slots | bytes of the heap operands read | bytes of the heap operand allocation | bytes of the slot range |"
    );
    println!("| --- | --- | --- | --- | --- | --- |");
    for (name, function, count) in shapes {
        let module = &program.modules[name];
        let slots = loop_body(module, function);
        assert_eq!(
            slots.len(),
            count,
            "the retired instruction count of {name}:{function}"
        );
        let first = *slots.first().unwrap();
        let last = *slots.last().unwrap();
        let range = (last - first + 1) * std::mem::size_of::<Instruction>();
        let (read, allocated) = heap_operand_bytes(module, &slots);
        println!(
            "| `{name}`:`{function}` | {count} | {} | {read} | {allocated} | {range} |",
            count * std::mem::size_of::<Instruction>(),
        );
    }
}
