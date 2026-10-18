use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use super::bif::{BIFS, BifId, BifResult, resolve, resolve_calls};
use super::code_table::export_frame_calls;
use super::{
    BeamProgram, BinaryPart, CodeTable, Issuer, Outcome, Register, Scheduler, StackSlot,
    call_arguments, function_at_label, private_append, read, read_bif_arguments, read_operand,
    read_owned_bif_arguments, read_ref, take_owned, write, write_operand,
};
use crate::{
    BeamModule, Map, Term,
    beam::{
        Import,
        code::{BinaryMatchCommand, Instruction, Operand, TestKind, TestOperand},
    },
    runtime::{
        capability::{AllowAll, Capability, Policy},
        ets::{Access, Busy, Table, TableType},
        import_kind,
        process::{ProcessState, Start, tests::frame},
    },
    term::{
        Atom, AtomTable, Binary, Lambda, ProcessId, Reference, debug_term, test_atom,
        test_atom_table,
    },
};

/// Return the initial capabilities of the default policy.
fn allow_all() -> Vec<Capability> {
    AllowAll.initial()
}

/// Return the runtime error of a root process whose `fixture:main/0` gets
/// an uncaught `error:badarg` from the BIF `module:function` with the `%T`
/// texts `arguments`. ERTS `save_stacktrace` puts the BIF frame with its
/// arguments and the `error_info` location of the format module `format`
/// first, then the frame of the caller (`beam_common.c:903-1005`). An
/// empty `format` gives the location `[]` of a module with no format
/// module.
fn bif_badarg_exit(module: &str, function: &str, arguments: &[&str], format: &str) -> String {
    let location = if format.is_empty() {
        "[]".to_owned()
    } else {
        format!("[{{error_info,#{{module=>{format}}}}}]")
    };
    format!(
        "root process exited with {{badarg,[{{{module},{function},[{}],{location}}},{{fixture,main,0,[]}}]}}",
        arguments.join(",")
    )
}

/// The format module of the BIFs of `erlang` (`beam_common.c:947-948`).
const ERTS_ERRORS: &str = "erl_erts_errors";

/// The format module of the BIFs of `lists` and `unicode`
/// (`beam_common.c:972-986`).
const STDLIB_ERRORS: &str = "erl_stdlib_errors";

/// Return the instructions of `main/function_arity` of the fixture module,
/// with `body` and `extra` after the two entry labels.
fn fixture_instructions(
    function_arity: u32,
    body: Vec<Instruction>,
    extra: Vec<Instruction>,
) -> Vec<Instruction> {
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
    instructions.extend(extra);
    instructions
}

/// Return [`program_of`] with the names of each `func_info` in the atom
/// table of the run, as the loader puts them. The stack trace of
/// `erlang:exit/1` and `erlang:throw/1` reads them.
fn program_with_names(
    instructions: Vec<Instruction>,
    exports: HashMap<(String, u32), u32>,
) -> BeamProgram {
    for instruction in &instructions {
        if let Instruction::FunctionInfo { module, name, .. } = instruction {
            test_atom(module);
            test_atom(name);
        }
    }
    program_of(instructions, exports)
}

/// Return a program of the fixture module with `instructions` and
/// `exports`. The loader resolves each branch label and each call label of
/// the stream when it builds the module.
fn program_of(instructions: Vec<Instruction>, exports: HashMap<(String, u32), u32>) -> BeamProgram {
    BeamProgram {
        modules: HashMap::from([(
            "fixture".into(),
            Rc::new(
                BeamModule::new("fixture".into(), exports, fixture_imports(), instructions)
                    .expect("the fixture module loads"),
            ),
        )]),
        atoms: Arc::new(test_atom_table()),
    }
}

fn make_program(body: Vec<Instruction>, function_arity: u32) -> BeamProgram {
    program_of(
        fixture_instructions(function_arity, body, Vec::new()),
        HashMap::from([(("main".into(), 0), 2)]),
    )
}

thread_local! {
    /// The import table of each hand-built fixture module of this test
    /// thread. [`external`] adds an MFA that no entry holds, so an index that
    /// it gave stays valid for each later module of the thread.
    static FIXTURE_IMPORTS: RefCell<Vec<Import>> = const { RefCell::new(Vec::new()) };
}

/// Return the import table of a hand-built fixture module: each MFA that
/// [`external`] gave an index on this test thread.
fn fixture_imports() -> Vec<Import> {
    FIXTURE_IMPORTS.with_borrow(Clone::clone)
}

/// Return the `call_ext` of `module:name/arity` in a hand-built fixture
/// module, whose import table [`fixture_imports`] gives.
fn external(module: &str, name: &str, arity: u32, tail: bool, deallocate: u32) -> Instruction {
    let import = FIXTURE_IMPORTS.with_borrow_mut(|imports| {
        let found = imports.iter().position(|import| {
            (import.module.as_str(), import.name.as_str(), import.arity) == (module, name, arity)
        });
        found.unwrap_or_else(|| {
            imports.push(Import {
                module: module.into(),
                name: name.into(),
                arity,
                kind: import_kind(module, name, arity),
            });
            imports.len() - 1
        })
    });
    Instruction::CallExternal {
        import: u32::try_from(import).expect("a test makes fewer than 2^32 imports"),
        tail,
        deallocate,
    }
}

fn erlang_call(name: &str, arity: u32, tail: bool) -> Instruction {
    external("erlang", name, arity, tail, 0)
}

fn atom(name: &str) -> Operand {
    Operand::Value(test_atom(name))
}

/// Run `main/0` with a `child/1` function that calls `erlang:exit/1`.
fn run_with_child(body: Vec<Instruction>) -> Result<Term, crate::BeamError> {
    let extra = vec![
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "child".into(),
            arity: 1,
        },
        Instruction::Label(11),
        erlang_call("exit", 1, true),
    ];
    let exports = HashMap::from([(("main".into(), 0), 2), (("child".into(), 1), 11)]);
    program_with_names(fixture_instructions(0, body, extra), exports)
        .execute("fixture", "main", &[])
        .map(|host| host.term)
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
    assert_eq!(run(Term::Integer(1)).term, test_atom("pass"));
    for arity in [Term::Integer(0), Term::Integer(-1), test_atom("a")] {
        assert_eq!(run(arity).term, test_atom("fail"));
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
    let exit = error(vec![
        Instruction::Move(atom("boom"), Operand::X(0)),
        erlang_call("exit", 1, false),
    ]);
    assert!(exit.contains("root process exited with boom"), "{exit}");
    assert!(
        error(vec![
            Instruction::Move(atom("undef"), Operand::X(0)),
            erlang_call("nif_error", 1, false),
        ])
        .contains(
            "root process exited with {undef,[{erlang,nif_error,[undef],\
             [{error_info,#{module=>erl_erts_errors}}]},{fixture,main,0,[]}]}"
        )
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
    assert_eq!(
        error(spawn_link_child(Term::Integer(1))),
        bif_badarg_exit(
            "erlang",
            "spawn_link",
            &["fixture", "child", "1"],
            ERTS_ERRORS
        )
    );
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
    // `{monitor, MonitorOpts}` gives the tag of the `DOWN` message, as
    // `code_server:schedule_on_load/4` uses it (code_server.erl:1395-1396).
    let monitor =
        |options: Vec<Term>| Term::tuple(vec![test_atom("monitor"), Term::list_from(options)]);
    let tag = Term::tuple(vec![test_atom("tag"), test_atom("on_load_down")]);
    assert!(matches!(
        wait_for_abnormal_child(vec![monitor(vec![tag])]),
        Ok(Term::Tuple(ref elements))
            if elements.first() == Some(&test_atom("on_load_down"))
    ));
    assert!(
        spawn_opt(vec![monitor(vec![test_atom("bad")])])
            .unwrap_err()
            .to_string()
            .contains("badarg")
    );
    assert!(
        spawn_opt(vec![monitor(vec![test_atom("priority")])])
            .unwrap_err()
            .to_string()
            .contains("unsupported spawn option Tuple")
    );
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
        ])
        .term,
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
        ])
        .term,
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
        ])
        .term,
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
        ])
        .term,
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
        ])
        .term,
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
            bif: resolve("erlang", "now", 0).unwrap(),
            arguments: Vec::new(),
            destination: Operand::X(0),
        }])
        .contains("unresolved external MFA erlang:now/0")
    );
}

#[test]
fn calls_a_module_export_without_a_bif_search() {
    // A `call_ext` of module code holds only the index of its import, and
    // the loader classifies an MFA that is not a BIF as `ImportKind::Export`.
    // The run then reaches the export lookup with no search of `BIFS`.
    let body = vec![
        Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(0)),
        external("fixture", "guest", 1, false, 0),
        Instruction::Return,
    ];
    let extra = vec![
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "guest".into(),
            arity: 1,
        },
        Instruction::Label(11),
        Instruction::Move(Operand::Value(Term::Integer(7)), Operand::X(0)),
        Instruction::Return,
    ];
    let exports = HashMap::from([(("main".into(), 0), 2), (("guest".into(), 1), 11)]);
    let program = program_of(fixture_instructions(0, body, extra), exports);
    let before = resolve_calls();
    assert_eq!(
        program.execute("fixture", "main", &[]).unwrap().term,
        Term::Integer(7)
    );
    assert_eq!(
        resolve_calls(),
        before,
        "a `call_ext` of module code must reach the export lookup with no search of `BIFS`"
    );
}

/// Run `fixture:main/0` of `program` in a new scheduler and return the
/// scheduler and the result of the run, so that a test reads the code table
/// of the run.
fn run_in_scheduler(program: &BeamProgram) -> (Scheduler, Result<(), crate::BeamError>) {
    let mut scheduler = Scheduler::without_checks((*program.atoms).clone());
    scheduler.code = CodeTable::new(program.modules.clone());
    let result = program.run_root(
        &mut scheduler,
        "fixture",
        "main",
        &[],
        Issuer::Unchecked,
        &mut None,
    );
    (scheduler, result)
}

/// Return the export target of the import `import` of `fixture` in the
/// code table of a run, as the start of the frame that it creates.
fn import_target(scheduler: &Scheduler, import: &Instruction) -> Option<usize> {
    let Instruction::CallExternal { import, .. } = import else {
        panic!("not a call_ext: {import:?}");
    };
    let module = scheduler.code.get("fixture").expect("fixture is loaded");
    let loaded = scheduler.code.loaded_by_id(module.id).unwrap();
    loaded.imports[*import as usize]
        .get()
        .map(|target| target.frame().next_instruction)
}

/// Run `fixture:main/0`, which calls the export `fixture:guest/1` through
/// one import `calls` times. Return the scheduler of the run, the start of
/// `guest/1`, and the number of calls of `CodeTable::export_frame` in the
/// run.
fn run_guest_calls(calls: usize) -> (Scheduler, usize, usize) {
    let guest = external("fixture", "guest", 1, false, 0);
    let mut body = vec![Instruction::Move(
        Operand::Value(Term::Integer(1)),
        Operand::X(0),
    )];
    body.extend(std::iter::repeat_n(guest, calls));
    body.push(Instruction::Return);
    let extra = vec![
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "guest".into(),
            arity: 1,
        },
        Instruction::Label(11),
        Instruction::Return,
    ];
    let exports = HashMap::from([(("main".into(), 0), 2), (("guest".into(), 1), 11)]);
    let program = program_of(fixture_instructions(0, body, extra), exports);
    let start = program.modules["fixture"].tables.jump_targets[&11];
    let before = export_frame_calls();
    let (scheduler, result) = run_in_scheduler(&program);
    result.unwrap();
    (scheduler, start, export_frame_calls() - before)
}

#[test]
fn keeps_the_export_target_of_an_import_after_its_first_call() {
    // The ERTS loader patches the export entry of each import into the code
    // (`jit/asm_load.c:1198-1206`), and `call_ext` branches through it
    // (`jit/arm/instr_call.cpp:150-164`). The first call of an import of
    // module code finds the export and keeps it in the slot of the import,
    // so the second call reads the slot and makes no lookup by name. Each
    // of the two calls enters `guest/1`, which starts after label 11.
    let guest = external("fixture", "guest", 1, false, 0);
    let (_, _, one_call) = run_guest_calls(1);
    let (scheduler, start, two_calls) = run_guest_calls(2);
    assert_eq!(import_target(&scheduler, &guest), Some(start));
    assert_eq!(
        two_calls, one_call,
        "the second call of an import must read its slot with no export lookup"
    );
}

#[test]
fn keeps_no_export_target_for_an_import_with_no_export() {
    // A call of a module that the run does not load finds no export, so the
    // slot stays empty and a later call looks up the export again, as the
    // load of the module sets the code address in the ERTS export entry
    // (`jit/asm_load.c:1180-1196`). A run with no code server stops at the
    // call (see `undefined_function`).
    let absent = external("absent", "run", 0, false, 0);
    let program = make_program(vec![absent.clone(), Instruction::Return], 0);
    let (scheduler, result) = run_in_scheduler(&program);
    assert_eq!(
        result.unwrap_err().to_string(),
        "unresolved external MFA absent:run/0"
    );
    assert_eq!(import_target(&scheduler, &absent), None);
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

/// A local call whose arity differs from the arity of the function that the
/// loader resolved gives the error `call arity does not match M:F`.
#[test]
fn rejects_a_local_call_with_the_wrong_arity() {
    let body = vec![Instruction::Call {
        arity: 0,
        function: 11,
        target: 11,
    }];
    let extra = vec![
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "one".into(),
            arity: 1,
        },
        Instruction::Label(11),
        Instruction::Return,
    ];
    let exports = HashMap::from([(("main".into(), 0), 2), (("one".into(), 1), 11)]);
    assert_eq!(
        program_of(fixture_instructions(0, body, extra), exports)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "call arity does not match fixture:one"
    );
}

/// A local call starts at the exact position of the label that it names, and
/// not at the entry of the function. The pinned ERTS `i_call` saves the
/// continuation pointer and dispatches to the code position of the operand
/// label, and `i_call_only` dispatches to it with no other work
/// (`erts/emulator/beam/emu/instrs.tab:148-180`), so a call to a label inside
/// a function does not run the instructions before that label. `two/0` writes
/// 1 to `x0` between its entry and label 12; the call to label 12 must keep
/// the 2 that `main/0` wrote, and a frame at the entry would return 1.
#[test]
fn starts_a_local_call_at_the_exact_position_of_its_label() {
    let body = vec![
        Instruction::Move(Operand::Value(Term::Integer(2)), Operand::X(0)),
        Instruction::Call {
            arity: 0,
            function: 12,
            target: 12,
        },
        Instruction::Return,
    ];
    let extra = vec![
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "two".into(),
            arity: 0,
        },
        Instruction::Label(11),
        Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(0)),
        Instruction::Label(12),
        Instruction::Return,
    ];
    let exports = HashMap::from([(("main".into(), 0), 2), (("two".into(), 0), 11)]);
    assert_eq!(
        program_of(fixture_instructions(0, body, extra), exports)
            .execute("fixture", "main", &[])
            .unwrap()
            .term,
        Term::Integer(2)
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
    let module = BeamModule::new("mäin_module".into(), exports, Vec::new(), instructions)
        .expect("the fixture module loads");
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
        program.execute("fixture", "main", &[]).unwrap().term,
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
            .unwrap()
            .term,
        test_atom("pid")
    );
    assert_eq!(
        program
            .execute("fixture", "main", &[program.host_term(Term::Integer(1))])
            .unwrap()
            .term,
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
        Operand::Value(Term::Map(
            Map::from_pairs(pairs, &test_atom_table()).unwrap(),
        ))
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
        .unwrap()
        .term,
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
        .unwrap()
        .term,
        test_atom("missing")
    );
    let error = |body| run(body).unwrap_err().to_string();
    assert!(
        error(vec![
            Instruction::GetMapElements {
                fail_label: 3,
                map: atom("a"),
                pairs: vec![],
            },
            Instruction::Label(3),
            Instruction::Return,
        ])
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
fn gets_map_elements_of_a_hashmap_and_matches_an_empty_key_list_of_each_map() {
    let integers = |count: i64| {
        Operand::Value(Term::Map(
            Map::from_pairs(
                (1..=count)
                    .map(|key| (Term::Integer(key), Term::Integer(-key)))
                    .collect(),
                &test_atom_table(),
            )
            .unwrap(),
        ))
    };
    let run = |map: Operand, keys: &[i64]| {
        let pairs = keys
            .iter()
            .zip(0..)
            .map(|(key, index)| (Operand::Value(Term::Integer(*key)), Operand::X(index)))
            .collect();
        make_program(
            vec![
                Instruction::Move(atom("none"), Operand::X(0)),
                Instruction::GetMapElements {
                    fail_label: 3,
                    map,
                    pairs,
                },
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Move(atom("missing"), Operand::X(0)),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
        .unwrap()
        .term
    };
    // A hashmap finds each key in turn.
    assert_eq!(run(integers(40), &[2, 40]), Term::Integer(-2));
    assert_eq!(run(integers(40), &[2, 41]), test_atom("missing"));
    // The ERTS JIT matches an empty key list for each map.
    assert_eq!(run(integers(40), &[]), test_atom("none"));
    assert_eq!(run(integers(2), &[]), test_atom("none"));
}

#[test]
fn tests_map_keys_with_has_map_fields_and_writes_no_register() {
    let map = Operand::Value(Term::Map(
        Map::from_pairs(
            vec![
                (test_atom("input"), test_atom("cooked")),
                (Term::Integer(1), test_atom("one")),
            ],
            &test_atom_table(),
        )
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
        run(vec![atom("input"), Operand::X(1)], map.clone())
            .unwrap()
            .term,
        test_atom("none")
    );
    assert_eq!(
        run(vec![atom("input"), atom("output")], map).unwrap().term,
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
    assert_eq!(run(3, atom("foo")).unwrap().term, test_atom("failed"));
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
    assert_eq!(tuple_key(3).unwrap().term, test_atom("failed"));
    // A reference is a key of the map, so a missing reference key raises
    // `{badkey, Ref}` as another missing key does.
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
    let error = references.unwrap_err().to_string();
    assert!(error.contains("badkey,#Ref<0.0.1.1>"), "{error}");
    // A key that a PBeam map cannot hold is a missing key of the exact
    // change, as ERTS reports a missing key.
    let unsupported = make_program(
        vec![Instruction::PutMapExact {
            fail_label: 0,
            map: Operand::Value(Term::Map(Map::new())),
            destination: Operand::X(0),
            pairs: vec![(
                Operand::Value(Term::tuple(vec![Term::binary(vec![])])),
                atom("a"),
            )],
        }],
        0,
    )
    .execute("fixture", "main", &[]);
    let error = unsupported.unwrap_err().to_string();
    assert!(error.contains("badkey,{<<>>}"), "{error}");
    // An alias reference has no PBeam order, so the exact change reports
    // that order instead of comparing the keys.
    let alias = make_program(
        vec![Instruction::PutMapExact {
            fail_label: 0,
            map: Operand::Value(Term::Map(Map::new())),
            destination: Operand::X(0),
            pairs: vec![
                (
                    Operand::Value(Term::Reference(crate::Reference(1).pid_reference())),
                    atom("a"),
                ),
                (
                    Operand::Value(Term::Reference(crate::Reference(2).pid_reference())),
                    atom("b"),
                ),
            ],
        }],
        0,
    )
    .execute("fixture", "main", &[]);
    let error = alias.unwrap_err().to_string();
    assert!(
        error.contains("unsupported term order of an alias reference"),
        "{error}"
    );
    // The assoc change adds the key, so a key that PBeam cannot hold raises
    // the runtime error of the key. The first change builds a map with one
    // owner, so the second change takes that map and puts it back on the
    // error.
    let unsupported = make_program(
        vec![
            Instruction::Move(Operand::Value(Term::Map(Map::new())), Operand::X(0)),
            Instruction::PutMapAssoc {
                map: Operand::X(0),
                destination: Operand::X(0),
                pairs: vec![(Operand::Value(Term::Integer(1)), atom("a"))],
            },
            Instruction::PutMapAssoc {
                map: Operand::X(0),
                destination: Operand::X(0),
                pairs: vec![(
                    Operand::Value(Term::tuple(vec![Term::binary(vec![])])),
                    atom("a"),
                )],
            },
            Instruction::Return,
        ],
        0,
    )
    .execute("fixture", "main", &[]);
    let error = unsupported.unwrap_err().to_string();
    assert!(error.contains("unsupported map key"), "{error}");
}

#[test]
fn keeps_a_destination_register_when_a_map_change_fails() {
    // ERTS writes the destination register only after the update succeeds
    // (`jit/arm/instr_map.cpp:687-716`), so a fail label finds the value
    // that the destination held before the instruction. The destination is
    // a different slot from the source, so no take removes it.
    let run = |map: Operand, pairs: Vec<(Operand, Operand)>| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(Term::Integer(7)), Operand::X(1)),
                Instruction::PutMapExact {
                    fail_label: 3,
                    map,
                    destination: Operand::X(1),
                    pairs,
                },
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Move(Operand::X(1), Operand::X(0)),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
        .unwrap()
        .term
    };
    // A literal source that is not a map.
    assert_eq!(
        run(atom("foo"), vec![(atom("output"), atom("cooked"))]),
        Term::Integer(7)
    );
    // A map source in another register with a missing key.
    let map = Operand::Value(Term::Map(
        Map::from_pairs(
            vec![(Term::Integer(1), Term::Integer(1))],
            &test_atom_table(),
        )
        .unwrap(),
    ));
    assert_eq!(
        run(
            map,
            vec![(Operand::Value(Term::Integer(2)), atom("cooked"))],
        ),
        Term::Integer(7)
    );
}

#[test]
fn keeps_a_built_map_when_a_same_register_change_fails() {
    // A map that the run built has one owner, so the take removes it. A
    // `put_map_assoc` on a literal map builds a map that no literal holds,
    // and the failing `put_map_exact` then takes that map, puts it back, and
    // jumps to its fail label.
    let source = Term::Map(
        Map::from_pairs(
            vec![(Term::Integer(1), Term::Integer(1))],
            &test_atom_table(),
        )
        .unwrap(),
    );
    let build = |source: Term| {
        vec![
            Instruction::Move(Operand::Value(source), Operand::X(0)),
            Instruction::PutMapAssoc {
                map: Operand::X(0),
                destination: Operand::X(0),
                pairs: vec![(
                    Operand::Value(Term::Integer(3)),
                    Operand::Value(Term::Integer(3)),
                )],
            },
        ]
    };
    let exact = |fail_label, pairs| {
        let mut body = build(source.clone());
        body.push(Instruction::PutMapExact {
            fail_label,
            map: Operand::X(0),
            destination: Operand::X(0),
            pairs,
        });
        body.push(Instruction::Return);
        body.push(Instruction::Label(3));
        body.push(Instruction::Return);
        make_program(body, 0).execute("fixture", "main", &[])
    };
    // A missing key jumps to the fail label, which returns the register.
    assert_eq!(
        exact(3, vec![(Operand::Value(Term::Integer(2)), atom("cooked"))])
            .unwrap()
            .term,
        Term::Map(
            Map::from_pairs(
                vec![
                    (Term::Integer(1), Term::Integer(1)),
                    (Term::Integer(3), Term::Integer(3)),
                ],
                &test_atom_table(),
            )
            .unwrap()
        )
    );
    // The reference keys are missing from the source map, so the exact
    // change fails with `{badkey, Ref}` after the take.
    let error = make_program(
        {
            let mut body = build(source);
            body.push(Instruction::PutMapExact {
                fail_label: 0,
                map: Operand::X(0),
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
            });
            body
        },
        0,
    )
    .execute("fixture", "main", &[]);
    let error = error.unwrap_err().to_string();
    assert!(error.contains("badkey,#Ref<0.0.1.1>"), "{error}");
}

#[test]
fn keeps_a_key_or_value_in_the_destination_slot() {
    // The key and the value operands name the destination register, whose
    // value the instruction overwrites. ERTS reads both before the write, so
    // the result uses the terms that the register held before.
    let run = |held: Term, pairs: Vec<(Operand, Operand)>| {
        make_program(
            vec![
                Instruction::Move(
                    Operand::Value(Term::Map(
                        Map::from_pairs(
                            vec![(Term::Integer(1), Term::Integer(0))],
                            &test_atom_table(),
                        )
                        .unwrap(),
                    )),
                    Operand::X(0),
                ),
                Instruction::Move(Operand::Value(held), Operand::X(2)),
                Instruction::PutMapExact {
                    fail_label: 0,
                    map: Operand::X(0),
                    destination: Operand::X(2),
                    pairs,
                },
                Instruction::Move(Operand::X(2), Operand::X(0)),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
        .unwrap()
        .term
    };
    let expected = || {
        Term::Map(
            Map::from_pairs(
                vec![(Term::Integer(1), Term::Integer(9))],
                &test_atom_table(),
            )
            .unwrap(),
        )
    };
    // The key is the term that the destination register held.
    assert_eq!(
        run(
            Term::Integer(1),
            vec![(Operand::X(2), Operand::Value(Term::Integer(9)))],
        ),
        expected()
    );
    // The value is the term that the destination register held.
    assert_eq!(
        run(
            Term::Integer(9),
            vec![(Operand::Value(Term::Integer(1)), Operand::X(2))],
        ),
        expected()
    );
}

#[test]
fn reports_a_maps_merge_result_key_that_the_map_form_does_not_support() {
    // A flatmap accepts the integer 2^59, and a hashmap does not yet.
    let pairs = |keys: std::ops::RangeInclusive<i64>| {
        let pairs = keys.map(|key| (Term::Integer(key), Term::Integer(0)));
        Term::Map(Map::from_pairs(pairs.collect(), &test_atom_table()).unwrap())
    };
    let merge = make_program(
        vec![
            Instruction::Move(Operand::Value(pairs(1 << 59..=1 << 59)), Operand::X(0)),
            Instruction::Move(Operand::Value(pairs(1..=32)), Operand::X(1)),
            external("maps", "merge", 2, true, 0),
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
            external("maps", "from_list", 1, true, 0),
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
            external("maps", "put", 3, true, 0),
            Instruction::Return,
        ],
        0,
    )
    .execute("fixture", "main", &[]);
    assert_eq!(put.unwrap_err().to_string(), "unsupported map key Map([])");
}

#[test]
fn runs_maps_update_as_a_bif_and_names_a_key_that_the_map_does_not_hold() {
    // ERTS `maps_update_3` sets the value of a key that the map holds, and
    // raises `{badkey, Key}` and `{badmap, Map}` for the other cases
    // (`erl_map.c:2036-2103`, `2241-2253`).
    let map = |pairs: Vec<(i64, &str)>| {
        let pairs = pairs
            .into_iter()
            .map(|(key, value)| (Term::Integer(key), test_atom(value)))
            .collect();
        Operand::Value(Term::Map(
            Map::from_pairs(pairs, &test_atom_table()).unwrap(),
        ))
    };
    let update = |key: Term, value: Term, map: Operand| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(key), Operand::X(0)),
                Instruction::Move(Operand::Value(value), Operand::X(1)),
                Instruction::Move(map, Operand::X(2)),
                external("maps", "update", 3, true, 0),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    let updated = update(
        Term::Integer(2),
        test_atom("b"),
        map(vec![(1, "a"), (2, "a")]),
    );
    assert_eq!(
        updated.unwrap().term,
        Term::Map(
            Map::from_pairs(
                vec![
                    (Term::Integer(1), test_atom("a")),
                    (Term::Integer(2), test_atom("b"))
                ],
                &test_atom_table()
            )
            .unwrap()
        )
    );
    let missing = update(Term::Integer(9), test_atom("b"), map(vec![(1, "a")]))
        .unwrap_err()
        .to_string();
    assert!(missing.contains("{badkey,9}"), "{missing}");
    let empty = update(Term::Integer(9), test_atom("b"), map(vec![]))
        .unwrap_err()
        .to_string();
    assert!(empty.contains("{badkey,9}"), "{empty}");
    let not_a_map = update(
        Term::Integer(1),
        test_atom("b"),
        Operand::Value(Term::Integer(1)),
    )
    .unwrap_err()
    .to_string();
    assert!(not_a_map.contains("{badmap,1}"), "{not_a_map}");
    // A map key is not a key of a PBeam map, so the BIF reports the key as
    // an absent key, as ERTS does for a key that the map does not hold.
    let map_key = update(
        Term::Map(Map::new()),
        test_atom("b"),
        Operand::Value(Term::Map(Map::new())),
    )
    .unwrap_err()
    .to_string();
    assert!(map_key.contains("{badkey"), "{map_key}");
}

#[test]
fn runs_maps_values_as_a_bif_and_raises_badmap_for_a_term_that_is_not_a_map() {
    // ERTS `maps_values_1` gives the values of a map in map order, and
    // raises `{badmap, Map}` for a term that is not a map
    // (`erl_map.c:2262-2310`).
    let values = |argument: Term| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(argument), Operand::X(0)),
                external("maps", "values", 1, true, 0),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    let map = |pairs: Vec<(i64, &str)>| {
        let pairs = pairs
            .into_iter()
            .map(|(key, value)| (Term::Integer(key), test_atom(value)))
            .collect();
        Term::Map(Map::from_pairs(pairs, &test_atom_table()).unwrap())
    };
    assert_eq!(
        values(map(vec![(2, "b"), (1, "a")])).unwrap().term,
        Term::list_from(vec![test_atom("a"), test_atom("b")])
    );
    assert_eq!(values(map(vec![])).unwrap().term, Term::nil());
    let not_a_map = values(Term::Integer(1)).unwrap_err().to_string();
    assert!(not_a_map.contains("{badmap,1}"), "{not_a_map}");
}

#[test]
fn runs_maps_keys_as_a_bif_and_reports_unsupported_arguments() {
    let keys = |argument: Term| {
        make_program(
            vec![
                Instruction::Move(Operand::Value(argument), Operand::X(0)),
                external("maps", "keys", 1, true, 0),
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
        Term::Map(Map::from_pairs(pairs, &test_atom_table()).unwrap())
    };
    let atom = Term::static_atom;
    // ERTS 29.1.1: maps:keys(#{a => V, 2 => V, 1 => V}) is [1,2,a].
    let a = test_atom("a");
    assert_eq!(
        keys(map(vec![a.clone(), Term::Integer(2), Term::Integer(1)]))
            .unwrap()
            .term,
        Term::list_from(vec![Term::Integer(1), Term::Integer(2), a])
    );
    // ERTS 29.1.1: maps:keys(#{true => V, abs_path => V, 1 => V}) is [1,true,abs_path].
    assert_eq!(
        keys(map(vec![atom("abs_path"), atom("true"), Term::Integer(1)]))
            .unwrap()
            .term,
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
                Operand::Value(Term::Map(
                    Map::from_pairs(Vec::new(), &test_atom_table()).unwrap(),
                )),
                Operand::X(0),
            ),
            external("maps", "keys", 1, true, 0),
            Instruction::Return,
        ],
        0,
    );
    program.modules.insert(
        "maps".into(),
        Rc::new(
            BeamModule::new(
                "maps".into(),
                HashMap::from([(("keys".into(), 1), 1)]),
                Vec::new(),
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
            )
            .expect("the fixture module loads"),
        ),
    );
    // The BIF returns the keys of the empty map. The module code would
    // return `module_code`.
    assert_eq!(
        program.execute("fixture", "main", &[]).unwrap().term,
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
            external("atomics", "add_get", 3, true, 0),
            Instruction::Return,
        ],
        0,
    );
    program.modules.insert(
        "atomics".into(),
        Rc::new(
            BeamModule::new(
                "atomics".into(),
                HashMap::from([(("add_get".into(), 3), 1)]),
                Vec::new(),
                vec![
                    Instruction::FunctionInfo {
                        module: "atomics".into(),
                        name: "add_get".into(),
                        arity: 3,
                    },
                    Instruction::Label(1),
                    Instruction::Move(atom("stub"), Operand::X(0)),
                    Instruction::Return,
                ],
            )
            .expect("the fixture module loads"),
        ),
    );
    // `atomics:add_get/3` is in the ERTS 29.1.1 `bif.tab`, and the PBeam table
    // marks it `unsupported`. ERTS runs the BIF, so the BEAM stub body of
    // the loaded `atomics` module must not run.
    assert_eq!(
        program
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "unresolved external MFA atomics:add_get/3"
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
                external("unicode", "characters_to_binary", 2, true, 0),
                Instruction::Return,
            ],
            0,
        )
        .execute("fixture", "main", &[])
    };
    let chardata = || Term::list_from(vec![Term::binary(b"Hello".to_vec()), Term::Integer(10)]);
    assert_eq!(
        convert(chardata(), "unicode").unwrap().term,
        Term::binary(b"Hello\n".to_vec())
    );
    // ERTS 29.1.1 raises badarg for a term that is not chardata.
    assert_eq!(
        convert(Term::Integer(1), "unicode")
            .unwrap_err()
            .to_string(),
        bif_badarg_exit(
            "unicode",
            "characters_to_binary",
            &["1", "unicode"],
            STDLIB_ERRORS
        )
    );
    // ERTS 29.1.1: unicode:characters_to_binary([<<"Hello">>, 10], utf8)
    // is <<"Hello\n">>.
    assert_eq!(
        convert(chardata(), "utf8").unwrap().term,
        Term::binary(b"Hello\n".to_vec())
    );
    // ERTS 29.1.1: unicode:characters_to_binary([<<"Hello">>, 10], latin1)
    // is <<"Hello\n">>, and unicode:characters_to_binary([<<233>>], latin1)
    // is <<195,169>>.
    assert_eq!(
        convert(chardata(), "latin1").unwrap().term,
        Term::binary(b"Hello\n".to_vec())
    );
    assert_eq!(
        convert(Term::list_from(vec![Term::binary(vec![233])]), "latin1")
            .unwrap()
            .term,
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
        list.execute("fixture", "main", &[]).unwrap().term,
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
        improper.execute("fixture", "main", &[]).unwrap().term,
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
        get_list(list(&["a", "b", "c"])).unwrap().term,
        Term::tuple(vec![test_atom("a"), list(&["b", "c"])])
    );
    assert_eq!(
        get_list(list(&["a"])).unwrap().term,
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
    assert_eq!(get_head(list(&["x", "y"])).unwrap().term, test_atom("x"));
    // ERTS: lst([a|x]) is {a,x}, lst([a,b|x]) is {a,[b|x]}, and
    // hd1([x|y]) is x.
    let improper = |names: &[&str], tail: &str| {
        Term::list_with_tail(
            names.iter().map(|name| test_atom(name)).collect(),
            test_atom(tail),
        )
    };
    assert_eq!(
        get_list(improper(&["a"], "x")).unwrap().term,
        Term::tuple(vec![test_atom("a"), test_atom("x")])
    );
    assert_eq!(
        get_list(improper(&["a", "b"], "x")).unwrap().term,
        Term::tuple(vec![test_atom("a"), improper(&["b"], "x")])
    );
    assert_eq!(
        get_head(improper(&["x"], "y")).unwrap().term,
        test_atom("x")
    );
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
    assert_eq!(classify(list(&[])).term, test_atom("nil"));
    assert_eq!(classify(list(&["a"])).term, test_atom("nonempty"));
    assert_eq!(classify(test_atom("a")).term, test_atom("other"));
    // ERTS: nil([a|x]) is nonempty.
    assert_eq!(classify(improper(&["a"], "x")).term, test_atom("nonempty"));
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
        run(Operand::X(0), vec![(3, Operand::X(1)), (4, Operand::X(2))])
            .unwrap()
            .term,
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
        run(Operand::X(1), vec![(2, Operand::X(2)), (3, Operand::X(1))])
            .unwrap()
            .term,
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
        result.term,
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
            format!(
                "invariant error: {} is not a raw stack trace",
                debug_term(&trace, &test_atom_table())
            )
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
    assert_eq!(raw_raise("foo").unwrap().term, Term::static_atom("badarg"));
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
                external("absent", "run", 0, true, 0)
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
        make_program(
            vec![Instruction::Call {
                arity: 1,
                function: 2,
                target: 2
            }],
            0
        )
        .execute("fixture", "main", &[])
        .unwrap_err()
        .to_string()
        .contains("argument x0 is unset")
    );
}

/// The loader answers a bad label and a bad entry with the error text of the
/// removed scans, and the run answers a bad register index with its text.
#[test]
fn keeps_the_error_text_of_a_bad_label_entry_and_register() {
    let load = |body: Vec<Instruction>| {
        BeamModule::new(
            "fixture".into(),
            HashMap::from([(("main".into(), 0), 2)]),
            Vec::new(),
            fixture_instructions(0, body, Vec::new()),
        )
        .unwrap_err()
        .to_string()
    };
    assert_eq!(load(vec![Instruction::Jump(9)]), "missing BEAM label 9");
    assert_eq!(
        load(vec![Instruction::Call {
            arity: 0,
            function: 9,
            target: 9
        }]),
        "entry label 9 not found in fixture"
    );
    // A target of 0 is the "no fail label" operand of a BIF only. Any other
    // instruction keeps the error text of the removed lookup.
    assert_eq!(
        make_program(vec![Instruction::Jump(0)], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "missing BEAM label 0"
    );
    assert_eq!(
        make_program(vec![Instruction::Move(Operand::X(1024), Operand::X(0))], 0)
            .execute("fixture", "main", &[])
            .unwrap_err()
            .to_string(),
        "register x1024 is unset"
    );
}

/// The loader answers a call to a label that no `func_info` of the module
/// precedes with the error text of the removed lookup.
#[test]
fn keeps_the_error_text_of_a_call_with_no_function_info() {
    let instructions = vec![
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "main".into(),
            arity: 0,
        },
        Instruction::Label(1),
        Instruction::FunctionInfo {
            module: "other".into(),
            name: "other".into(),
            arity: 0,
        },
        Instruction::Label(7),
        Instruction::Call {
            arity: 0,
            function: 7,
            target: 7,
        },
    ];
    assert_eq!(
        BeamModule::new("fixture".into(), HashMap::new(), Vec::new(), instructions)
            .unwrap_err()
            .to_string(),
        "entry label 7 has no function info"
    );
}

#[test]
fn rejects_unresolved_exports_and_function_labels() {
    let module = BeamModule::new("external".into(), HashMap::new(), Vec::new(), vec![])
        .expect("the fixture module loads");
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
        Vec::new(),
        vec![
            Instruction::FunctionInfo {
                module: "other".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(2),
        ],
    )
    .expect("the fixture module loads");
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
        let extra = vec![
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: "child".into(),
                arity: 1,
            },
            Instruction::Label(11),
            Instruction::Move(atom("ok"), Operand::X(0)),
            Instruction::Return,
        ];
        let exports = HashMap::from([(("main".into(), 0), 2), (("child".into(), 1), 11)]);
        program_of(fixture_instructions(0, body, extra), exports)
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
    assert_eq!(run(vec![external("fixture", "child", 1, false, 0)]), escape);
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
            destination: Some(Operand::X(1)),
        }]),
        "invariant error: Value(Atom(\"a\")) holds Atom(\"a\"), not a match context"
    );
}

#[test]
fn writes_the_operand_of_a_match_context_error_with_each_atom_name() {
    // `Debug for Term` writes the atom index, because a bare term carries no
    // table. The invariant error of a match context holds the table, so it
    // writes the name of a `Value` operand and its index for a register.
    assert_eq!(
        super::debug_operand(&Operand::Value(test_atom("a")), &test_atom_table()),
        "Value(Atom(\"a\"))"
    );
    assert_eq!(
        super::debug_operand(&Operand::X(0), &AtomTable::new()),
        "X(0)"
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
                destination: Some(Operand::X(2)),
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
                destination: Some(Operand::X(4)),
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
        )
        .term,
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
        )
        .term,
        expected
    );
    assert_eq!(
        run(
            Instruction::BinaryMatchResume {
                source: Operand::X(0),
                destination: Operand::X(1),
            },
            Operand::X(1)
        )
        .term,
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
        .map(|host| host.term)
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
            .map(|host| host.term)
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
    let operands = [Operand::X(0), Operand::Value(Term::Integer(9))];
    let arguments =
        read_bif_arguments(&process, &operands).expect("two set operands are valid arguments");
    assert_eq!(
        arguments.with(|terms| terms.to_vec()),
        vec![&Term::Integer(7), &Term::Integer(9)]
    );
    assert!(read_bif_arguments(&process, &[Operand::X(1)]).is_err());
    assert!(
        read_bif_arguments(
            &process,
            &[Operand::X(0), Operand::X(0), Operand::X(0), Operand::X(0)]
        )
        .is_err()
    );
    let owned = read_owned_bif_arguments(&process, &operands)
        .expect("two set operands are valid arguments");
    assert_eq!(
        owned.with(|terms| terms.to_vec()),
        vec![Term::Integer(7), Term::Integer(9)]
    );
    assert!(read_owned_bif_arguments(&process, &[Operand::X(1)]).is_err());
    assert!(
        read_owned_bif_arguments(
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
        select(Term::tuple(vec![Term::Integer(1)])).term,
        test_atom("one")
    );
    assert_eq!(select(Term::tuple(vec![])).term, test_atom("other"));
    assert_eq!(select(test_atom("a")).term, test_atom("other"));

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
                function: 2,
                target: 2,
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
    // loaded modules, so `pbeam_alpha` gets index 1168 after
    // `pbeam_zeta` (1167). The table of the loaded modules does not
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
        1167
    );
    for _ in 0..2 {
        let result = program.execute("fixture", "main", &[]).unwrap();
        let Term::Atom(alpha) = &result.term else {
            panic!("the run returns the atom pbeam_alpha");
        };
        assert_eq!(result.atoms().name_of(alpha), "pbeam_alpha");
        assert_eq!(alpha.index(), 1168);
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
    assert_eq!(run(hello.clone()).term, Term::list_from(vec![hello]));
    assert_eq!(
        run(Term::list_from(vec![
            Term::Integer(1),
            Term::list_from(vec![Term::binary(vec![2])]),
        ]))
        .term,
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
    // The catch gives {'EXIT', {undef, Stack}}, and the first frame of
    // Stack is the BIF frame of `erlang:nif_error/1`.
    let caught = run(body).unwrap().term;
    let Term::Tuple(exit) = &caught else {
        panic!("catch gives a tuple");
    };
    let Term::Tuple(reason) = &exit[1] else {
        panic!("the reason is {{undef, Stack}}");
    };
    let stack: Vec<Term> = reason[1].proper_list().unwrap().cloned().collect();
    let error_info = Map::from_pairs(
        vec![(
            Term::static_atom("module"),
            Term::static_atom("erl_erts_errors"),
        )],
        &test_atom_table(),
    )
    .unwrap();
    let frame = Term::tuple(vec![
        Term::static_atom("erlang"),
        Term::static_atom("nif_error"),
        Term::list_from(vec![Term::static_atom("undef")]),
        Term::list_from(vec![Term::tuple(vec![
            Term::static_atom("error_info"),
            Term::Map(error_info),
        ])]),
    ]);
    assert_eq!(
        (&exit[0], &reason[0], &stack[0], stack.len()),
        (
            &Term::static_atom("EXIT"),
            &Term::static_atom("undef"),
            &frame,
            2
        )
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
            .contains("root process exited with {undef,[{erlang,nif_error,[undef],")
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
    // ERTS 29.1.1: catch element(1, {}) is {'EXIT', {badarg, Stack}},
    // and the first frame of Stack is the BIF frame
    // {erlang, element, [1, {}], [{error_info, #{module => erl_erts_errors}}]}.
    let caught = element(1).unwrap().term;
    let Term::Tuple(exit) = &caught else {
        panic!("catch gives a tuple");
    };
    let Term::Tuple(reason) = &exit[1] else {
        panic!("the reason is {{badarg, Stack}}");
    };
    let stack: Vec<Term> = reason[1].proper_list().unwrap().cloned().collect();
    let Term::Tuple(frame) = &stack[0] else {
        panic!("the first frame is a tuple");
    };
    assert_eq!(
        (&exit[0], &reason[0], &frame[2], stack.len()),
        (
            &test_atom("EXIT"),
            &test_atom("badarg"),
            &Term::list_from(vec![Term::Integer(1), Term::tuple(vec![])]),
            2
        )
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
        bif_badarg_exit("erlang", "element", &["1", "{}"], ERTS_ERRORS)
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
        node(BifId::ErlangNode0, Vec::new()).unwrap().term,
        Term::static_atom("nonode@nohost")
    );
    assert_eq!(
        node(BifId::ErlangNode1, vec![Operand::X(1)]).unwrap().term,
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
        Rc::new(
            BeamModule::new(
                "erts_internal".into(),
                HashMap::from([(("flush_monitor_messages".into(), 3), 2)]),
                Vec::new(),
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
            )
            .expect("the fixture module loads"),
        ),
    );
    assert_eq!(
        program.execute("fixture", "main", &[]).unwrap().term,
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
        .map(|host| host.term)
}

/// Return the program of `run_with_apply_targets`.
fn apply_targets_program(body: Vec<Instruction>) -> BeamProgram {
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
    let mut extra = Vec::new();
    extra.extend(function("zero", 0, 11));
    extra.extend([
        Instruction::Move(atom("zero"), Operand::X(0)),
        Instruction::Return,
    ]);
    extra.extend(function("id", 3, 12));
    extra.extend([
        Instruction::PutTuple {
            elements: vec![Operand::X(0), Operand::X(1), Operand::X(2)],
            destination: Operand::X(0),
        },
        Instruction::Return,
    ]);
    extra.extend(function("last", 2, 13));
    extra.extend([
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
    let exports = HashMap::from([
        (("main".into(), 0), 2),
        (("zero".into(), 0), 11),
        (("id".into(), 3), 12),
        (("last".into(), 2), 13),
    ]);
    program_with_names(fixture_instructions(0, body, extra), exports)
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
                function: 13,
                target: 13
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
        bif_badarg_exit("erlang", "apply", &["fixture", "id"], ERTS_ERRORS)
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
    let program = apply_targets_program(body);
    // The atoms of the `undef` frame come from the atom table of the run,
    // which holds the atoms of the hand-built code.
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
    // ERTS 29.1.1 gives the BIF frame
    // {erlang, apply, [fixture, missing, L1024], [{error_info, #{module => erl_erts_errors}}]}.
    assert_eq!(
        run_with_apply_targets(body).unwrap_err().to_string(),
        format!(
            "root process exited with {{system_limit,[{{erlang,apply,[fixture,missing,[{}]],\
             [{{error_info,#{{module=>erl_erts_errors}}}}]}},{{fixture,main,0,[]}}]}}",
            ["x"; 1024].join(",")
        )
    );
}

#[test]
fn applies_a_pbeam_bif_through_the_name_path() {
    // `pbeam_cap:revoke/2` is a PBeam BIF and no `bif.tab` entry, so an
    // `apply` reaches it through the name path of `call_mfa`. A holder that
    // is not a pid gives `badarg`.
    let mut body = vec![
        Instruction::Move(atom("x"), Operand::X(0)),
        Instruction::Move(atom("env"), Operand::X(1)),
    ];
    body.extend(apply_body(2, atom("pbeam_cap"), atom("revoke")));
    // ERTS has no format module for `pbeam_cap`, so the location is `[]`.
    assert_eq!(
        run_with_apply_targets(body).unwrap_err().to_string(),
        bif_badarg_exit("pbeam_cap", "revoke", &["x", "env"], "")
    );
}

#[test]
fn rejects_an_apply_with_a_stack_mismatch_and_raises_badarg() {
    let error = |body| run_with_apply_targets(body).unwrap_err().to_string();
    // ERTS 29.1.1 raises `badarg` for `M:F()` with `1` and `f`, or with
    // `m` and `1`.
    // `fixed_apply` raises with the BIF frame of `erlang:apply/3`
    // (`beam_common.c:1529-1551`).
    assert_eq!(
        error(apply_body(0, Operand::Value(Term::Integer(1)), atom("f"))),
        bif_badarg_exit("erlang", "apply", &["1", "f", "[]"], ERTS_ERRORS)
    );
    assert_eq!(
        error(apply_body(0, atom("m"), Operand::Value(Term::Integer(1)))),
        bif_badarg_exit("erlang", "apply", &["m", "1", "[]"], ERTS_ERRORS)
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
    body.push(external(module, name, arity, true, 0));
    body.push(Instruction::Return);
    make_program(body, 0)
        .execute("fixture", "main", &[])
        .map(|host| host.term)
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
            .unwrap()
            .term,
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
    let extra = vec![
        Instruction::FunctionInfo {
            module: "fixture".into(),
            name: "fresh".into(),
            arity: 0,
        },
        Instruction::Label(11),
        get_bif("k", Operand::X(0)),
        erlang_call("exit", 1, true),
    ];
    let exports = HashMap::from([(("main".into(), 0), 2), (("fresh".into(), 0), 11)]);
    let program = program_of(fixture_instructions(0, body, extra), exports);
    // ERTS 29.1.1: after put(k, parent), a spawned process gets
    // undefined for get(k).
    assert!(matches!(
        program.execute("fixture", "main", &[]).map(|host| host.term),
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
    for (right, text) in [(Term::nil(), "[]"), (test_atom("x"), "x")] {
        assert_eq!(
            append(test_atom("a"), right).unwrap_err().to_string(),
            bif_badarg_exit("erlang", "'++'", &["a", text], ERTS_ERRORS)
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
    let exit = |arguments: &[&str]| bif_badarg_exit("lists", "keyfind", arguments, STDLIB_ERRORS);
    for (position, text) in [
        (Term::Integer(0), "0"),
        (Term::Integer(-1), "-1"),
        (test_atom("x"), "x"),
        (Term::Integer(1 << 59), "576460752303423488"),
    ] {
        assert_eq!(
            badarg(a.clone(), position, one.clone()),
            exit(&["a", text, "[{a}]"])
        );
    }
    for (list, text) in [(a.clone(), "a"), (tuple(vec![a.clone()]), "{a}")] {
        assert_eq!(
            badarg(a.clone(), Term::Integer(1), list),
            exit(&["a", "1", text])
        );
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
        bif_badarg_exit("lists", "member", &["a", "x"], STDLIB_ERRORS)
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
            .unwrap()
            .term,
        b
    );
    let element = |index: Term, tuple: Term| call_ext_with("erlang", "element", vec![index, tuple]);
    let one = Term::tuple(vec![a.clone()]);
    // ERTS 29.1.1: element(1, {a}) is a.
    assert_eq!(element(Term::Integer(1), one.clone()).unwrap(), a);
    // ERTS 29.1.1 raises badarg for element(0, {a}), element(2, {a}),
    // element(-1, {a}), element(a, {a}), and element(1, [a]).
    for (index, tuple, texts) in [
        (Term::Integer(0), one.clone(), ["0", "{a}"]),
        (Term::Integer(2), one.clone(), ["2", "{a}"]),
        (Term::Integer(-1), one.clone(), ["-1", "{a}"]),
        (a.clone(), one, ["a", "{a}"]),
        (Term::Integer(1), Term::list_from(vec![a]), ["1", "[a]"]),
    ] {
        assert_eq!(
            element(index, tuple).unwrap_err().to_string(),
            bif_badarg_exit("erlang", "element", &texts, ERTS_ERRORS)
        );
    }
}

#[test]
fn reads_the_tuple_size_as_pinned_erts() {
    let tuple_size = |term: Term| call_ext_with("erlang", "tuple_size", vec![term]);
    // ERTS 29.1.1: tuple_size({}) is 0, and tuple_size(a) raises badarg.
    assert_eq!(tuple_size(Term::tuple(vec![])).unwrap(), Term::Integer(0));
    assert_eq!(
        tuple_size(test_atom("a")).unwrap_err().to_string(),
        bif_badarg_exit("erlang", "tuple_size", &["a"], ERTS_ERRORS)
    );
}

#[test]
fn orders_two_references_by_their_number_words() {
    // ERTS orders two references by their number words
    // (`erts_cmp_compound`, utils.c:2239-2290); this is the "Term order"
    // row in `docs/erts-differences.md`. The reference 1 comes first.
    let reference = |number| Operand::Value(Term::Reference(crate::Reference(number)));
    let outcome = |test| {
        let body = vec![
            test,
            Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(0)),
            Instruction::Return,
            Instruction::Label(3),
            Instruction::Move(Operand::Value(Term::Integer(2)), Operand::X(0)),
            Instruction::Return,
        ];
        make_program(body, 0)
            .execute("fixture", "main", &[])
            .unwrap()
            .term
    };
    assert_eq!(
        outcome(Instruction::TestLess {
            fail_label: 3,
            left: reference(1),
            right: reference(2),
        }),
        Term::Integer(1)
    );
    assert_eq!(
        outcome(Instruction::TestGreaterEqual {
            fail_label: 3,
            left: reference(1),
            right: reference(2),
        }),
        Term::Integer(2)
    );
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
            .unwrap()
            .term,
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
    for (process, item, texts) in [
        (
            test_atom("a"),
            test_atom("current_stacktrace"),
            ["a", "current_stacktrace"],
        ),
        (test_atom("a"), test_atom("bogus_item"), ["a", "bogus_item"]),
        (
            Term::Pid(crate::ProcessId(99)),
            test_atom("bogus_item"),
            ["<0.99.0>", "bogus_item"],
        ),
        (root(), test_atom("bogus_item"), ["<0.0.0>", "bogus_item"]),
        (root(), Term::Integer(5), ["<0.0.0>", "5"]),
        (
            root(),
            Term::list_from(vec![test_atom("registered_name"), test_atom("bogus")]),
            ["<0.0.0>", "[registered_name,bogus]"],
        ),
        (
            root(),
            Term::tuple(vec![test_atom("bogus"), test_atom("k")]),
            ["<0.0.0>", "{bogus,k}"],
        ),
    ] {
        assert_eq!(
            process_info(process, item).unwrap_err().to_string(),
            bif_badarg_exit("erlang", "process_info", &texts, ERTS_ERRORS)
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
            format!(
                "unsupported erlang:process_info/2 arguments {process:?}, {}",
                debug_term(&item, &test_atom_table())
            )
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
    // ERTS adds the `error_info` causes `none`, `none`, and `notalive`,
    // which PBeam does not give (row "Exception stack trace" of
    // `docs/erts-differences.md`).
    for (name, id, texts) in [
        (Term::Integer(1), root, ["1", "<0.0.0>"]),
        (test_atom("a"), test_atom("x"), ["a", "x"]),
        (
            test_atom("a"),
            Term::Pid(crate::ProcessId(99)),
            ["a", "<0.99.0>"],
        ),
    ] {
        assert_eq!(
            register(name, id).unwrap_err().to_string(),
            bif_badarg_exit("erlang", "register", &texts, ERTS_ERRORS)
        );
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
    for (arguments, texts) in [
        (
            vec![Term::Integer(1), self_name.clone(), Term::Integer(0)],
            ["1", "self", "0"],
        ),
        (
            vec![erlang.clone(), Term::Integer(1), Term::Integer(0)],
            ["erlang", "1", "0"],
        ),
        (
            vec![erlang.clone(), self_name.clone(), test_atom("x")],
            ["erlang", "self", "x"],
        ),
        (
            vec![erlang.clone(), self_name.clone(), Term::Integer(1 << 59)],
            ["erlang", "self", "576460752303423488"],
        ),
        (
            vec![erlang, self_name, Term::Integer(-(1 << 59) - 1)],
            ["erlang", "self", "-576460752303423489"],
        ),
    ] {
        assert_eq!(
            call_ext_with("erlang", "function_exported", arguments)
                .unwrap_err()
                .to_string(),
            bif_badarg_exit("erlang", "function_exported", &texts, ERTS_ERRORS)
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
        let mut extra = Vec::new();
        extra.extend(function("burner", 0, 21));
        extra.extend(std::iter::repeat_n(Instruction::TestYield, entries));
        extra.extend([
            Instruction::Call {
                arity: 0,
                function: 23,
                target: 23,
            },
            Instruction::Return,
        ]);
        extra.extend(function("inner", 0, 23));
        extra.extend([
            Instruction::TestYield,
            Instruction::Move(atom("ok"), Operand::X(0)),
            Instruction::Return,
        ]);
        extra.extend(function("sender", 1, 25));
        extra.extend([
            Instruction::Move(atom("tag"), Operand::X(1)),
            Instruction::Send,
            Instruction::Return,
        ]);
        let exports = HashMap::from([
            (("main".into(), 0), 2),
            (("burner".into(), 0), 21),
            (("sender".into(), 1), 25),
        ]);
        let program = program_of(
            fixture_instructions(0, [body, wait_forever()].concat(), extra),
            exports,
        );
        program.execute("fixture", "main", &[]).unwrap()
    };
    // 3998 entries, the entry of `inner`, and its return leave 0. The
    // last return makes the budget -1: `burner` switches out, `sender`
    // runs, and then `burner` exits.
    assert_eq!(first_message(3998).term, test_atom("tag"));
    // With 3997 entries, the last return leaves 0, and `burner` exits at
    // once. The exit bumps 100 reductions and finds none left, so it
    // switches out before the monitor (`erl_process.c:14229`, `14315`).
    assert_eq!(first_message(3997).term, test_atom("tag"));
    // The exit of a process with 150 reductions left uses 100 and 50, and
    // switches out before the monitor (`erl_process.c:14475`, `14543`).
    assert_eq!(first_message(3847).term, test_atom("tag"));
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
                "unsupported erlang:{name}/1 of a bignum of 60 bits with 0 reductions left: \
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
    scheduler.ets.create(
        Reference(0),
        None,
        Table::new(owner, Access::Public, 1, TableType::Set),
    );
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
fn resumes_a_list_insert_without_a_walk_of_the_checked_objects() {
    // The YCF state of `ets_insert_2_list_check` keeps the list cell that it
    // reads (`erl_db.c:1812-1827`), so a resume does not check an object
    // again. The objects here are not good: a resume that walks or checks
    // the whole list again panics or raises `badarg`.
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let owner = scheduler.current_process_mut().id;
    let id = Term::Reference(Reference(0));
    scheduler.ets.create(
        Reference(0),
        None,
        Table::new(owner, Access::Public, 1, TableType::Bag),
    );
    let object = Term::tuple(vec![Term::Integer(1)]);
    let rest = Term::list_from([object.clone(), object]);
    let objects = Term::list_with_tail(vec![Term::Integer(7)], rest.clone());
    // A budget of 0 checks one object and yields after it.
    scheduler.current_process_mut().fcalls = 0;
    let arguments = vec![
        id.clone(),
        objects,
        Term::Integer(1),
        Term::Integer(1),
        Term::nil(),
        Term::tuple(vec![rest.clone()]),
    ];
    let result = program
        .bif_trap(&mut scheduler, "ets", "insert", arguments)
        .unwrap();
    let BifResult::TrapBif { arguments, .. } = result else {
        panic!("the check did not yield");
    };
    let (_, unchecked) = rest.cons_cell().unwrap();
    assert_eq!(arguments[2], Term::Integer(2));
    assert_eq!(arguments[5], Term::tuple(vec![unchecked.clone()]));
    // The table takes the copies and the position of the steps after the
    // take (`erl_db.c:2072-2110`), so a resume runs the objects of the
    // table and does not read the objects of the call, which are not a
    // list here.
    let Some(table) = scheduler.ets.table_by_id(Reference(0)) else {
        panic!("the table is live");
    };
    let object = Term::tuple(vec![Term::Integer(1)]);
    table.busy = Some(Busy::new(
        owner,
        vec![vec![object.clone()], vec![object]],
        false,
    ));
    scheduler.current_process_mut().fcalls = 1;
    let arguments = vec![
        id,
        Term::Integer(7),
        Term::Integer(5),
        Term::Integer(1),
        Term::list_from([Term::Integer(0)]),
        Term::Integer(2),
    ];
    let result = program
        .bif_trap(&mut scheduler, "ets", "insert", arguments)
        .unwrap();
    assert!(matches!(result, BifResult::Value(value) if value == Term::static_atom("true")));
}

#[test]
fn completes_a_busy_list_insert_before_the_operation_of_a_process() {
    // `handle_lacking_permission` completes the list insert of a table with
    // `DB_BUSY` before the operation of the process that uses it, and it
    // charges the units of the work to that process (`erl_db.c:769-798`,
    // `2152-2197`). The operation of the process then traps and runs again
    // in a later turn (`erl_db.c:2223-2253`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let caller = scheduler.current_process_mut().id;
    let id = Term::Reference(Reference(0));
    let object = Term::tuple(vec![Term::Integer(5), Term::Integer(6)]);
    // The table runs the list insert of another process: the walk of its
    // one object did not finish (`erl_db.c:2310-2319`).
    let busy = |scheduler: &mut Scheduler| {
        scheduler
            .ets
            .table_by_id(Reference(0))
            .expect("the table is live")
            .busy = Some(Busy::new(
            ProcessId(99),
            vec![vec![Term::Integer(5)]],
            false,
        ));
    };
    let type_atom = program.atom("type").expect("the atom type exists");
    let arguments = [
        ("lookup", vec![id.clone(), Term::Integer(5)]),
        ("member", vec![id.clone(), Term::Integer(5)]),
        (
            "lookup_element",
            vec![
                id.clone(),
                Term::Integer(5),
                Term::Integer(1),
                Term::Integer(0),
            ],
        ),
        ("delete", vec![id.clone(), Term::Integer(5)]),
        ("info", vec![id.clone(), Term::Atom(type_atom)]),
        ("insert", vec![id.clone(), object.clone()]),
        ("insert_new", vec![id.clone(), object.clone()]),
        // A list of two objects takes the table without a copy
        // (`erl_db.c:2050-2055`).
        (
            "insert",
            vec![
                id.clone(),
                Term::list_from([object.clone(), object.clone()]),
            ],
        ),
    ];
    let mut table = Table::new(caller, Access::Public, 1, TableType::Set);
    table.busy = None;
    scheduler.ets.create(Reference(0), None, table);
    for (function, arguments) in &arguments {
        busy(&mut scheduler);
        let result = program
            .bif(&mut scheduler, "ets", function, arguments.clone())
            .unwrap();
        assert!(
            matches!(result, BifResult::TrapBif { function: name, .. } if name == *function),
            "ets:{function}/2 did not complete the insert and run again"
        );
    }
    // The retry of each call runs on a table without a busy insert.
    for (function, arguments) in &arguments {
        let result = program
            .bif_trap(&mut scheduler, "ets", function, arguments.clone())
            .unwrap();
        assert!(
            matches!(result, BifResult::Value(_) | BifResult::Raise { .. }),
            "ets:{function}/2 did not run again"
        );
    }
    // A short list insert that the units cover runs `can_insert_without_yield`
    // (`erl_db.c:2002-2014`), also in the turn that follows a trap in the
    // take of the table (`erl_db.c:1963-1994`).
    scheduler.current_process_mut().fcalls = 1;
    busy(&mut scheduler);
    let result = program
        .bif_trap(
            &mut scheduler,
            "ets",
            "insert",
            vec![
                id.clone(),
                Term::list_from([object.clone(), object.clone()]),
                Term::Integer(2),
                Term::Integer(1),
                Term::static_atom("false"),
                Term::Integer(2),
            ],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::TrapBif { .. }),
        "the short list insert did not complete the insert of the other process"
    );
    let result = program
        .bif_trap(
            &mut scheduler,
            "ets",
            "insert",
            vec![
                id.clone(),
                Term::list_from([object.clone(), object.clone()]),
                Term::Integer(2),
                Term::Integer(1),
                Term::static_atom("false"),
                Term::Integer(2),
            ],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::Value(value) if value == Term::static_atom("true")),
        "the short list insert did not take the objects without a copy"
    );
    // `ets:match/2,3` completes the insert of the busy table before it
    // reads the objects, so it sees each of them (`erl_db.c:769-787`).
    scheduler.current_process_mut().fcalls = 1;
    busy(&mut scheduler);
    let pattern = Term::static_atom("_");
    let result = program
        .bif(
            &mut scheduler,
            "ets",
            "match",
            vec![id.clone(), pattern.clone()],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::TrapBif { .. }),
        "ets:match/2 did not complete the insert of the other process"
    );
    let result = program
        .bif_trap(
            &mut scheduler,
            "ets",
            "match",
            vec![id.clone(), pattern.clone()],
        )
        .unwrap();
    assert!(matches!(
        result,
        BifResult::Value(_) | BifResult::Raise { .. }
    ));
    // `ets:match/3` with a limit does the same.
    scheduler.current_process_mut().fcalls = 1;
    busy(&mut scheduler);
    let result = program
        .bif(
            &mut scheduler,
            "ets",
            "match",
            vec![id.clone(), pattern.clone(), Term::Integer(10)],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::TrapBif { .. }),
        "ets:match/3 did not complete the insert of the other process"
    );
    let result = program
        .bif_trap(
            &mut scheduler,
            "ets",
            "match",
            vec![id.clone(), pattern.clone(), Term::Integer(10)],
        )
        .unwrap();
    assert!(matches!(
        result,
        BifResult::Value(_) | BifResult::Raise { .. }
    ));
    // The resume of `ets:match/3` does the same (`ets:select_trap/7`).
    scheduler.current_process_mut().fcalls = 1;
    busy(&mut scheduler);
    let result = program
        .bif_trap(
            &mut scheduler,
            "ets",
            "select_trap",
            vec![
                id.clone(),
                pattern,
                Term::Integer(0),
                Term::Integer(0),
                Term::nil(),
                Term::Integer(0),
                Term::Integer(0),
            ],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::TrapBif { .. }),
        "ets:select_trap/7 did not complete the insert of the other process"
    );
    // A list whose objects are bad raises `badarg`, and it completes the
    // list insert of a busy table first, as ERTS takes the table before it
    // raises the error (`erl_db.c:2036-2047`).
    scheduler.current_process_mut().fcalls = 1;
    busy(&mut scheduler);
    let bad = Term::list_from([Term::Integer(1), Term::Integer(1)]);
    let result = program
        .bif(
            &mut scheduler,
            "ets",
            "insert",
            vec![id.clone(), bad.clone()],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::TrapBif { .. }),
        "the bad list did not complete the insert of the other process"
    );
    let result = program
        .bif_trap(&mut scheduler, "ets", "insert", vec![id.clone(), bad])
        .unwrap();
    assert!(
        matches!(result, BifResult::Raise { .. }),
        "the bad list did not raise badarg"
    );
    // A plain list insert that uses the last of its budget on its last
    // object traps with all units of the insert charged, and it returns
    // `true` in its next turn (`erl_db.c:1936-1940`, `2180-2196`).
    scheduler.ets.create(
        Reference(0),
        None,
        Table::new(caller, Access::Public, 1, TableType::Set),
    );
    let objects = |key: i64| Term::tuple(vec![Term::Integer(key)]);
    scheduler
        .ets
        .table_by_id(Reference(0))
        .expect("the table is live")
        .busy = Some(Busy::new(
        caller,
        vec![
            vec![Term::Integer(1), Term::Integer(1)],
            vec![Term::Integer(2), Term::Integer(2)],
        ],
        false,
    ));
    scheduler
        .ets
        .table_by_id(Reference(0))
        .expect("the table is live")
        .step_busy(&scheduler.atoms.clone(), 4);
    scheduler.current_process_mut().fcalls = 1;
    let result = program
        .bif_trap(
            &mut scheduler,
            "ets",
            "insert",
            vec![
                id.clone(),
                Term::list_from([objects(1), objects(2)]),
                Term::Integer(8),
                Term::Integer(1),
                Term::static_atom("false"),
                Term::Integer(2),
            ],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::Value(value) if value == Term::static_atom("true")),
        "the insert that charged all of its units did not return true"
    );
    // A list insert that ended in another turn gives its result to the
    // process that started it, also after the table went away
    // (`erl_db.c:2142-2150`, `2223-2244`).
    scheduler.ets.delete_table(Reference(0), caller);
    scheduler.current_process_mut().insert_result = Some(Term::static_atom("true"));
    let result = program
        .bif_trap(
            &mut scheduler,
            "ets",
            "insert",
            vec![
                id.clone(),
                Term::Integer(7),
                Term::Integer(5),
                Term::Integer(1),
                Term::static_atom("false"),
                Term::Integer(2),
            ],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::Value(value) if value == Term::static_atom("true")),
        "the process did not read the result of the insert that ended"
    );
    // The process that runs the insert takes its own table: the steps of
    // the walk and of the insert are its own (`erl_db.c:2077-2107`).
    scheduler.ets.create(
        Reference(0),
        None,
        Table::new(caller, Access::Public, 1, TableType::Set),
    );
    scheduler.current_process_mut().fcalls = 1;
    scheduler
        .ets
        .table_by_id(Reference(0))
        .expect("the table is live")
        .busy = Some(Busy::new(caller, vec![vec![Term::Integer(5)]], false));
    let result = program
        .bif(
            &mut scheduler,
            "ets",
            "lookup",
            vec![id.clone(), Term::Integer(5)],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::Value(_)),
        "the process that runs the insert found the table busy"
    );
    // A list insert that takes the table of another list insert completes
    // it first, and its own call runs again (`erl_db.c:2077`, `2223-2253`).
    scheduler.current_process_mut().fcalls = 1;
    busy(&mut scheduler);
    let result = program
        .bif_trap(
            &mut scheduler,
            "ets",
            "insert",
            vec![
                id.clone(),
                Term::list_from([object.clone(), object.clone()]),
                Term::Integer(0),
                Term::Integer(1),
                Term::static_atom("false"),
                Term::Integer(2),
            ],
        )
        .unwrap();
    assert!(
        matches!(result, BifResult::TrapBif { .. }),
        "the list insert did not complete the insert of the other process"
    );
    // A list insert of the same table runs its own steps while another
    // process holds the table: it reads no result of its own there
    // (`erl_db.c:2223-2253`).
    scheduler.current_process_mut().fcalls = 1;
    busy(&mut scheduler);
    let result = program
        .bif_trap(
            &mut scheduler,
            "ets",
            "insert",
            vec![
                id,
                Term::list_from([object.clone(), object]),
                Term::Integer(4),
                Term::Integer(1),
                Term::list_from([Term::Integer(0)]),
                Term::Integer(2),
            ],
        )
        .unwrap();
    assert!(matches!(result, BifResult::TrapBif { .. }));
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
    let value = Term::Map(
        Map::from_pairs(
            vec![(Term::Integer(1), Term::Integer(2))],
            &test_atom_table(),
        )
        .unwrap(),
    );
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
    // The loader replaces the fail label of the test with the index of the
    // instruction after the label, so the operand is the clause position.
    let clause = fail_label as usize;
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
/// argument vector of a `Bif`. The first number counts the operands that
/// the instruction reads, and the second counts the storage that the loader
/// allocated for them. Every other operand lives inside the slot of its
/// instruction.
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
        if let Instruction::Bif { arguments, .. } = &module.instructions[*index] {
            read += arguments.len() * std::mem::size_of::<Operand>();
            allocated += arguments.capacity() * std::mem::size_of::<Operand>();
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

#[path = "../common/mod.rs"]
mod common;

/// The probe whose child exits with a graph that the interpreter loop still
/// has to release when the root waits for a message (task #559).
const RELEASE_PROBE: &[u8] = include_bytes!("../../fixtures/release/release_probe.beam");

/// The number of cells of the graph of `release_probe:exiter/1`, which is
/// large enough that the release work of the child needs more than one step.
const RELEASE_CELLS: i64 = 50_000;

#[test]
fn drains_the_release_work_of_an_exited_process_while_the_root_waits() {
    // Row 11 of docs/term-performance-design-codex-v2.md and task #559. The
    // child builds a graph of 50000 cells in its process dictionary and
    // exits; the root then waits for a message that never arrives. The run
    // loop must keep stepping until the release work list is empty. A step
    // that reported `Idle` with work on the list would let the loop wait for
    // input with most of the graph retained, and the list would not drain.
    let erlang = common::pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, RELEASE_PROBE]).unwrap();
    let mut scheduler = Scheduler::without_checks((*program.atoms).clone());
    scheduler.code = CodeTable::new(program.modules.clone());
    program
        .run_root(
            &mut scheduler,
            "release_probe",
            "waits",
            &[Term::Integer(RELEASE_CELLS)],
            Issuer::Unchecked,
            &mut None,
        )
        .expect("the loop ends with the root that waits");
    assert!(
        scheduler.release_len() == 0,
        "the release work of the child drains before the loop waits"
    );
}

#[test]
fn runs_a_handler_after_a_put_map_exact_badmap() {
    // The map operand is a literal that is not a map, and y0 is a live catch
    // tag. ERTS raises `{badmap, Map}` and the process continues at the tag
    // (`beam_common.c:2224-2235`, `handle_error`). The fixture reaches the
    // raise with a catch tag, so the process runs its handler.
    let program = make_program(
        vec![
            Instruction::Allocate { stack_size: 1 },
            Instruction::Catch {
                register: 0,
                label: 7,
            },
            Instruction::PutMapExact {
                fail_label: 0,
                map: atom("foo"),
                destination: Operand::X(0),
                pairs: vec![(atom("output"), atom("cooked"))],
            },
            Instruction::Return,
            Instruction::Label(7),
            Instruction::Move(atom("caught"), Operand::X(0)),
            Instruction::Return,
        ],
        0,
    );
    assert_eq!(
        program.execute("fixture", "main", &[]).unwrap().term,
        test_atom("caught")
    );
}

#[test]
fn gives_up_the_map_of_a_y_register_with_one_owner() {
    // A `put_map_exact` of a Y source and a Y destination is a valid form of
    // the compiler. The take gives up the map only when the process owns
    // the only reference to it, and leaves a shared map in its slot.
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    process.frames.last_mut().unwrap().stack = vec![StackSlot::Term(Term::nil())];
    write(&mut process, &Operand::Y(0), Term::Map(Map::new())).unwrap();
    assert!(matches!(
        take_owned(&mut process, Register::Y(0)),
        Some(Term::Map(_))
    ));
    assert!(take_owned(&mut process, Register::Y(0)).is_none());
    // A shared map keeps its slot.
    write(&mut process, &Operand::Y(0), Term::Map(Map::new())).unwrap();
    let held = read_ref(&process, &Operand::Y(0)).unwrap().clone();
    assert!(take_owned(&mut process, Register::Y(0)).is_none());
    assert!(matches!(
        read_ref(&process, &Operand::Y(0)),
        Ok(Term::Map(_))
    ));
    drop(held);
}

#[test]
fn writes_a_private_append_in_the_array_of_a_binary_with_one_owner() {
    // ERTS changes the size of the writable binary term in place
    // (erl_bits.c:1978, 2017), so the source register holds the result.
    // PBeam writes in the array when the source register owns the only
    // reference, and builds a new binary when another term shares it.
    let mut process = ProcessState::new(ProcessId(0), Start::Entry(frame()), vec![]);
    let parts = [BinaryPart::Utf8('z')];
    write(&mut process, &Operand::X(0), Term::binary(b"a".to_vec())).unwrap();
    let start = match read_ref(&process, &Operand::X(0)).unwrap() {
        Term::Binary(binary) => Arc::as_ptr(binary.bytes()),
        other => panic!("not a binary: {other:?}"),
    };
    let result = private_append(&mut process, &Operand::X(0), &parts).unwrap();
    assert_eq!(result, Ok(Term::binary(b"az".to_vec())));
    let Ok(Term::Binary(appended)) = &result else {
        unreachable!()
    };
    assert_eq!(Arc::as_ptr(appended.bytes()), start);
    assert_eq!(
        read(&process, &Operand::X(0)).unwrap(),
        Term::binary(b"az".to_vec())
    );
    drop(result);

    // A shared binary keeps its bytes.
    let held = read(&process, &Operand::X(0)).unwrap();
    let result = private_append(&mut process, &Operand::X(0), &parts).unwrap();
    assert_eq!(result, Ok(Term::binary(b"azz".to_vec())));
    assert_eq!(held, Term::binary(b"az".to_vec()));

    // The compiler gives the segment only a binary, so another term is a
    // runtime invariant error.
    write(&mut process, &Operand::X(0), Term::Integer(1)).unwrap();
    let error = private_append(&mut process, &Operand::X(0), &parts).unwrap_err();
    assert_eq!(
        error.to_string(),
        "bs_create_bin private_append segment holds a term that is not a binary: Integer(1)"
    );
}

#[test]
fn raises_the_reason_of_an_error_with_cause_that_no_call_path_converted() {
    // A call path that knows the BIF turns `ErrorWithCause` into the stack
    // trace of ERTS with `bif_error`. Without that step, `finish_bif`
    // raises `error:Reason` with the stack trace `[]`.
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
    let outcome = program
        .finish_bif(&mut scheduler, BifResult::badopt(), false)
        .unwrap();
    let Some(Outcome::Exit { reason, .. }) = outcome else {
        panic!("the process exits: {outcome:?}");
    };
    assert_eq!(
        reason,
        Term::tuple(vec![Term::static_atom("badarg"), Term::nil()])
    );
}

#[test]
fn resumes_a_trap_target_through_its_loaded_export() {
    // A trap that waited for the next turn enters the export of its target
    // when the process runs again. A target that a module exports needs no
    // error handler. `fun_probe:tail_wait/1` has a known entry.
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
    write(
        scheduler.current_process_mut(),
        &Operand::X(0),
        Term::Integer(1),
    )
    .unwrap();
    let outcome = program
        .resume_trap(&mut scheduler, "fun_probe", "tail_wait", 1, false)
        .unwrap();
    assert!(outcome.is_none());
    let frames = &scheduler.current_process_mut().frames;
    assert_eq!(frames.len(), 2);
    let (module_id, function) = (frames[1].module_id, frames[1].function);
    let module = scheduler.code.module_by_id(module_id).unwrap();
    assert_eq!(module.tables.name(function), "tail_wait");
}

#[test]
fn traps_a_call_that_meets_a_fixed_clear() {
    // A call that uses a table while a fixed clear does not finish marks a
    // bounded group of buckets and traps to itself
    // (`handle_lacking_permission`, `erl_db.c:769-798`, `3300-3320`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let owner = scheduler.current_process_mut().id;
    let calls = [
        ("next", vec![Term::Integer(0)]),
        ("update_element", vec![Term::Integer(0), Term::nil()]),
        (
            "update_element",
            vec![
                Term::Integer(0),
                Term::nil(),
                Term::tuple(vec![Term::Integer(0)]),
            ],
        ),
        ("info", vec![]),
        ("internal_select_delete", vec![Term::nil()]),
        ("delete", vec![]),
    ];
    for (number, (function, mut arguments)) in calls.into_iter().enumerate() {
        let id = Reference(number as u64);
        let mut table = Table::new(owner, Access::Public, 1, TableType::Set);
        for key in 0..300 {
            table.insert(&scheduler.atoms, vec![Term::Integer(key)]);
        }
        table.fix(owner);
        table.step_clear(&scheduler.atoms, owner, 300, 8);
        scheduler.ets.create(id, None, table);
        arguments.insert(0, Term::Reference(id));
        let result = program
            .bif(&mut scheduler, "ets", function, arguments)
            .unwrap();
        assert!(
            matches!(result, BifResult::TrapBif { function: trapped, .. } if trapped == function),
            "{function}"
        );
    }
}

#[test]
fn traps_a_select_continuation_that_meets_a_fixed_clear() {
    // `ets:select/1` takes the table of its continuation with
    // `DB_GET_TABLE`, which helps a clear that does not finish and traps
    // (`erl_db.c:89-114`, `769-798`, `3965`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let owner = scheduler.current_process_mut().id;
    let mut table = Table::new(owner, Access::Public, 1, TableType::Set);
    for key in 0..300 {
        table.insert(&scheduler.atoms, vec![Term::Integer(key)]);
    }
    table.fix(owner);
    table.step_clear(&scheduler.atoms, owner, 300, 1);
    scheduler.ets.create(Reference(0), None, table);
    let continuation = Term::tuple(vec![
        Term::Reference(Reference(0)),
        Term::Integer(0),
        Term::Integer(1),
        Term::Reference(Reference(999)),
        Term::nil(),
        Term::Integer(1),
    ]);
    let result = program
        .bif(&mut scheduler, "ets", "select", vec![continuation.clone()])
        .unwrap();
    assert!(matches!(
        result,
        BifResult::TrapBif { function: "select", arguments, .. }
            if arguments == vec![continuation]
    ));
}

#[test]
fn schedules_a_waiting_process_for_the_fixation_cleanup() {
    // A waiting process that fixed a deleted table runs the cleanup in a
    // turn of its own, without resuming its Erlang code
    // (`erts_schedule_ets_free_fixation`, `erl_db.c:5263-5301`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let fixer = ProcessId(1);
    let mut process = ProcessState::new(fixer, Start::Entry(frame()), Vec::new());
    process.waiting = true;
    scheduler.insert(process);
    // The fixer runs once and parks.
    scheduler.schedule_next().unwrap();
    scheduler.ets.create(
        Reference(0),
        None,
        Table::new(ProcessId(99), Access::Public, 1, TableType::Set),
    );
    scheduler.ets.fix_table(Reference(0), fixer);
    assert_eq!(scheduler.ets.delete_table(Reference(0), ProcessId(99)), 1);
    scheduler.schedule_pending_cleanups();
    scheduler.schedule_next().unwrap();
    assert_eq!(scheduler.ets.pending_count(fixer), 0);
    assert!(scheduler.process(fixer).unwrap().waiting);
}

#[test]
fn requeues_a_waiting_process_with_more_cleanup_than_a_turn() {
    // The scheduler keeps a waiting process scheduled while its cleanup
    // remains (`notify_sys_task_executed`, `erl_process.c:10989`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let fixer = ProcessId(1);
    let mut process = ProcessState::new(fixer, Start::Entry(frame()), Vec::new());
    process.waiting = true;
    scheduler.insert(process);
    scheduler.schedule_next().unwrap();
    let turn = crate::runtime::scheduler::CONTEXT_REDS as u64;
    for number in 0..=turn {
        let id = Reference(number);
        scheduler.ets.create(
            id,
            None,
            Table::new(ProcessId(99), Access::Public, 1, TableType::Set),
        );
        scheduler.ets.fix_table(id, fixer);
        assert_eq!(scheduler.ets.delete_table(id, ProcessId(99)), 1);
    }
    scheduler.schedule_pending_cleanups();
    scheduler.schedule_next().unwrap();
    assert!(scheduler.process(fixer).unwrap().waiting);
    assert_eq!(scheduler.ets.pending_count(fixer), 1);
    scheduler.schedule_next().unwrap();
    assert_eq!(scheduler.ets.pending_count(fixer), 0);
}

#[test]
fn cleans_a_pending_fixation_when_the_process_is_scheduled() {
    // The cleanup of a fixation on a table that another process deleted runs
    // as process system work at each scheduling
    // (`erts_schedule_ets_free_fixation`, `erl_db.c:5263-5301`,
    // `erl_process.c:10932-10989`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let caller = scheduler.current_process_mut().id;
    scheduler.ets.create(
        Reference(0),
        None,
        Table::new(ProcessId(99), Access::Public, 1, TableType::Set),
    );
    scheduler.ets.fix_table(Reference(0), caller);
    assert_eq!(scheduler.ets.delete_table(Reference(0), ProcessId(99)), 1);
    assert_eq!(scheduler.ets.pending_count(caller), 1);
    scheduler.yield_current();
    scheduler.schedule_next().unwrap();
    assert_eq!(scheduler.ets.pending_count(caller), 0);
}

#[test]
fn helps_a_trapped_insert_from_internal_delete_all() {
    // `ets:internal_delete_all/2` first completes the list insert that
    // holds the table busy (`handle_lacking_permission`, `erl_db.c:769-798`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let owner = scheduler.current_process_mut().id;
    let mut table = Table::new(owner, Access::Public, 1, TableType::Set);
    table.insert(&scheduler.atoms, vec![Term::Integer(1)]);
    // The insert belongs to another process, so the call helps it.
    table.busy = Some(Busy::new(ProcessId(999), Vec::new(), false));
    scheduler.ets.create(Reference(0), None, table);
    let result = program
        .bif(
            &mut scheduler,
            "ets",
            "internal_delete_all",
            vec![
                Term::Reference(Reference(0)),
                Term::static_atom("undefined"),
            ],
        )
        .unwrap();
    assert!(matches!(result, BifResult::Value(value) if value == Term::Integer(1)));
}

#[test]
fn traps_the_initial_clear_with_the_table_reference() {
    // The clear of a fixed table that does not finish traps with the table
    // reference and the object count, so a reused name cannot resume
    // against another table (`erl_db.c:3268-3279`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let owner = scheduler.current_process_mut().id;
    let mut table = Table::new(owner, Access::Public, 1, TableType::Set);
    for key in 0..300 {
        table.insert(&scheduler.atoms, vec![Term::Integer(key)]);
    }
    table.fix(owner);
    scheduler.ets.create(Reference(0), None, table);
    scheduler.current_process_mut().fcalls = 1;
    let result = program
        .bif(
            &mut scheduler,
            "ets",
            "internal_delete_all",
            vec![
                Term::Reference(Reference(0)),
                Term::static_atom("undefined"),
            ],
        )
        .unwrap();
    assert!(matches!(
        result,
        BifResult::TrapBif { function: "internal_delete_all", arguments, .. }
            if arguments == vec![Term::Reference(Reference(0)), Term::Integer(300)]
    ));
}

#[test]
fn accepts_the_line_delimiter_of_the_line_type() {
    // ERTS accepts a delimiter of at most 255 for the `line` type, so the
    // call reaches the runtime error of the unsupported type and not
    // `badarg` (`erl_bif_port.c:1537-1541`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let result = program.bif(
        &mut scheduler,
        "erlang",
        "decode_packet",
        vec![
            Term::static_atom("line"),
            Term::binary(b"abc;rest".to_vec()),
            Term::list_from(vec![Term::tuple(vec![
                Term::static_atom("line_delimiter"),
                Term::Integer(59),
            ])]),
        ],
    );
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("unsupported erlang:decode_packet/3 type line"),
        "{error}"
    );
}

#[test]
fn rejects_a_line_delimiter_that_is_too_large() {
    // A delimiter above 255 raises `badarg` (`erl_bif_port.c:1537-1541`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let result = program
        .bif(
            &mut scheduler,
            "erlang",
            "decode_packet",
            vec![
                Term::static_atom("line"),
                Term::binary(b"abc;rest".to_vec()),
                Term::list_from(vec![Term::tuple(vec![
                    Term::static_atom("line_delimiter"),
                    Term::Integer(256),
                ])]),
            ],
        )
        .unwrap();
    assert!(matches!(result, BifResult::Raise { .. }));
}

#[test]
fn rejects_a_packet_bitstring_that_is_not_a_byte() {
    // A subject that is not a whole number of bytes raises `badarg`
    // (`erts_get_aligned_binary_bytes`, `erl_bif_port.c:1550-1556`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let subject = Term::Binary(Binary::from(&[0x10_u8]).bits(0, 4));
    let result = program
        .bif(
            &mut scheduler,
            "erlang",
            "decode_packet",
            vec![Term::static_atom("http_bin"), subject, Term::nil()],
        )
        .unwrap();
    assert!(matches!(result, BifResult::Raise { .. }));
}

#[test]
fn rejects_a_select_delete_of_a_private_table_of_another_process() {
    // `ets:internal_select_delete/2` takes the table with `DB_WRITE`, so a
    // process that does not own a `private` table raises `badarg`
    // (`erl_db.c:3433-3475`, `769-798`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    scheduler.ets.create(
        Reference(0),
        None,
        Table::new(ProcessId(999), Access::Private, 1, TableType::Set),
    );
    let spec = Term::list_from([Term::tuple(vec![
        Term::static_atom("_"),
        Term::nil(),
        Term::list_from([Term::static_atom("true")]),
    ])]);
    let result = program
        .bif(
            &mut scheduler,
            "ets",
            "internal_select_delete",
            vec![Term::Reference(Reference(0)), spec],
        )
        .unwrap();
    assert!(matches!(result, BifResult::Raise { .. }));
}

#[test]
fn rejects_an_update_default_at_the_zero_key_position() {
    // A key position of 0 comes from a truncated `{keypos, 1 bsl 32}`. The
    // unsigned arity check passes, and the default object has no key
    // position to replace, so the call raises `badarg` (`erl_db.c:1483-1484`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let owner = scheduler.current_process_mut().id;
    scheduler.ets.create(
        Reference(0),
        None,
        Table::new(owner, Access::Public, 0, TableType::Set),
    );
    let object = Term::tuple(vec![Term::Integer(1), Term::Integer(0)]);
    let result = program
        .bif(
            &mut scheduler,
            "ets",
            "update_element",
            vec![
                Term::Reference(Reference(0)),
                Term::Integer(1),
                Term::tuple(vec![Term::Integer(2), Term::Integer(9)]),
                object,
            ],
        )
        .unwrap();
    assert!(matches!(result, BifResult::Raise { .. }));
}

#[test]
fn gives_the_owner_as_the_heir_of_a_table() {
    // `ets:new/2` keeps `{heir, self()}` as the heir, and `ets:info/1,2`
    // gives the owner pid (`erl_db.c:5478-5479`, `5319-5337`).
    let program = make_program(vec![], 0);
    let mut scheduler = Scheduler::new((*program.atoms).clone());
    scheduler.start(frame(), vec![], allow_all()).unwrap();
    scheduler.schedule_next().unwrap();
    let owner = scheduler.current_process_mut().id;
    let heir = Term::Atom(scheduler.atoms.put("heir").unwrap());
    let name = Term::Atom(scheduler.atoms.put("ets_probe_heir").unwrap());
    let options = Term::list_from([
        Term::static_atom("named_table"),
        Term::tuple(vec![heir.clone(), Term::Pid(owner)]),
    ]);
    let table = program
        .bif(&mut scheduler, "ets", "new", vec![name.clone(), options])
        .unwrap();
    let BifResult::Value(table) = table else {
        panic!("ets:new/2 did not make a table");
    };
    let item = program
        .bif(
            &mut scheduler,
            "ets",
            "info",
            vec![table.clone(), heir.clone()],
        )
        .unwrap();
    assert!(matches!(item, BifResult::Value(Term::Pid(pid)) if pid == owner));
    let BifResult::Value(items) = program
        .bif(&mut scheduler, "ets", "info", vec![table])
        .unwrap()
    else {
        panic!("ets:info/1 did not return a list");
    };
    let heir_entry = Term::tuple(vec![heir, Term::Pid(owner)]);
    assert!(items.proper_list().unwrap().any(|item| *item == heir_entry));
}
