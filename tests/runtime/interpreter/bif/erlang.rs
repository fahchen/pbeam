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

use crate::beam::BeamModule;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::Start;
use std::{collections::HashMap, rc::Rc};

/// Return a code table with one module `fixture` and the function `main/0`.
///
/// # Examples
///
/// ```rust,ignore
/// let code = fixture_code();
/// assert!(code.get("fixture").is_some());
/// ```
fn fixture_code() -> CodeTable {
    let module = BeamModule::new(
        "fixture".into(),
        HashMap::from([(("main".to_owned(), 0), 1)]),
        Vec::new(),
        vec![
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(1),
        ],
    )
    .unwrap();
    CodeTable::new(HashMap::from([("fixture".into(), Rc::new(module))]))
}

#[test]
fn reports_the_exiting_status_of_another_process() {
    // The `status` item of ERTS is `exiting` for a process that exits
    // (`erl_bif_info.c:1697-1710`). A process of another state gives
    // `waiting` or `runnable`.
    let code = fixture_code();
    let frame = code.mfa_frame("fixture", "main", 0).unwrap();
    let mut process = ProcessState::new(ProcessId(1), Start::Entry(frame), vec![]);
    process.exiting = Some(Term::static_atom("normal"));
    let values = process_info_values(&code, &process, 0, 0, false).unwrap();
    assert_eq!(values.status, "exiting");
    process.exiting = None;
    process.waiting = true;
    assert_eq!(
        process_info_values(&code, &process, 0, 0, false)
            .unwrap()
            .status,
        "waiting"
    );
    process.waiting = false;
    assert_eq!(
        process_info_values(&code, &process, 0, 0, false)
            .unwrap()
            .status,
        "runnable"
    );
}

#[test]
fn reads_the_current_function_of_a_process_that_did_not_run() {
    // A process with an undefined start gives its start MFA until its first
    // frame (`erl_process.c:12616-12624`). A process with no frame and no
    // start gives `undefined`.
    let code = fixture_code();
    let start = Start::Undefined {
        module: Atom::from_static("erlang").unwrap(),
        function: Atom::from_static("apply").unwrap(),
    };
    let mut process = ProcessState::new(ProcessId(1), start, vec![Term::Integer(1)]);
    let values = process_info_values(&code, &process, 0, 0, false).unwrap();
    assert!(matches!(
        values.current_function,
        Some(MfaNames::Atoms(_, _, 1))
    ));
    process.undefined_start = None;
    let values = process_info_values(&code, &process, 0, 0, false).unwrap();
    assert!(values.current_function.is_none());
}

#[test]
fn returns_the_erlang_unique_integer_sequences() {
    // `unique_integer_bif` gives 2 and then 4 for `[positive]` with one
    // scheduler (`erl_bif_unique.c:474-518`), and
    // `unique_monotonic_integer_bif` gives `MIN_SMALL` and then `MIN_SMALL +
    // 1`, or 1 and then 2 with `positive` (`erl_bif_unique.c:693-734`).
    let mut scheduler = Scheduler::new(AtomTable::new());
    assert_eq!(scheduler.unique_integer_positive().unwrap(), 2);
    assert_eq!(scheduler.unique_integer_positive().unwrap(), 4);
    assert_eq!(
        scheduler.unique_monotonic_integer(false).unwrap(),
        SMALL_INTEGER_MIN
    );
    assert_eq!(
        scheduler.unique_monotonic_integer(false).unwrap(),
        SMALL_INTEGER_MIN + 1
    );
    assert_eq!(scheduler.unique_monotonic_integer(true).unwrap(), 3);
}

#[test]
fn raises_a_runtime_error_for_the_stdin_display_device() {
    // `display_string_2` writes the string to the standard output or the
    // standard error (`bif.c:4192-4285`). The `stdin` device writes to the
    // terminal input with `TIOCSTI`, which PBeam does not run.
    let mut atoms = AtomTable::new();
    let stdin = Term::Atom(atoms.put("stdin").unwrap());
    let error = display_string(&mut atoms, &stdin, &Term::nil()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported erlang:display_string/2 device stdin"),
        "{error}"
    );
    let other = Term::Atom(atoms.put("foo").unwrap());
    assert!(matches!(
        display_string(&mut atoms, &other, &Term::nil()).unwrap(),
        BifResult::Raise { .. }
    ));
}

#[test]
fn raises_badarg_for_a_display_string_that_is_not_whole_bytes() {
    // `erts_get_aligned_binary_bytes` gives NULL for a bitstring that is
    // not byte aligned, so `display_string_2` raises `badarg`
    // (`bif.c:4237-4248`).
    let mut atoms = AtomTable::new();
    let device = Term::Atom(atoms.put("foo").unwrap());
    let bitstring = Term::Binary(Binary::from(vec![1]).bits(0, 1));
    assert!(matches!(
        display_string(&mut atoms, &device, &bitstring).unwrap(),
        BifResult::Raise { .. }
    ));
}

#[test]
fn returns_the_on_load_flag_of_prepared_code() {
    // `has_prepared_code_on_load_1` reads the loader state of the reference
    // (`beam_bif_load.c:268-283`): true with an `-on_load` function, false
    // without one, and badarg for a reference that holds no prepared code.
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.code = fixture_code();
    let frame = scheduler.code.mfa_frame("fixture", "main", 0).unwrap();
    scheduler.start(frame, vec![], AllowAll.initial()).unwrap();
    scheduler.schedule_next().unwrap();
    let plain = BeamModule::new(
        "fixture".into(),
        HashMap::new(),
        Vec::new(),
        vec![Instruction::End],
    )
    .unwrap();
    let plain_reference = scheduler.make_ref().unwrap();
    scheduler.code.prepare(plain_reference, plain);
    let on_load = BeamModule::load(include_bytes!(
        "../../../../fixtures/diagnostics/diagnostics_on_load.beam"
    ))
    .unwrap();
    let on_load_reference = scheduler.make_ref().unwrap();
    scheduler.code.prepare(on_load_reference, on_load);
    assert!(matches!(
        has_prepared_code_on_load(&mut scheduler, &Term::Reference(plain_reference)).unwrap(),
        BifResult::Value(Term::Atom(ref atom)) if atom.is_static("false")
    ));
    assert!(matches!(
        has_prepared_code_on_load(&mut scheduler, &Term::Reference(on_load_reference)).unwrap(),
        BifResult::Value(Term::Atom(ref atom)) if atom.is_static("true")
    ));
    let absent = scheduler.make_ref().unwrap();
    assert!(matches!(
        has_prepared_code_on_load(&mut scheduler, &Term::Reference(absent)).unwrap(),
        BifResult::Raise { .. }
    ));
}

#[test]
fn returns_not_purged_for_prepared_code_of_a_deleted_module() {
    // `finish_loading_1` gives `{not_purged, [Module]}` when the module has
    // current and old code (`beam_bif_load.c:444-455`). `delete_module/1`
    // moves the current code of a module to old code
    // (`beam_bif_load.c:2412-2455`).
    let mut scheduler = Scheduler::new(AtomTable::new());
    let first = BeamModule::new(
        "fixture".into(),
        HashMap::from([(("main".to_owned(), 0), 1)]),
        Vec::new(),
        vec![
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(1),
        ],
    )
    .unwrap();
    scheduler.code = CodeTable::new(HashMap::from([("fixture".into(), Rc::new(first))]));
    let frame = scheduler.code.mfa_frame("fixture", "main", 0).unwrap();
    scheduler.start(frame, vec![], AllowAll.initial()).unwrap();
    scheduler.schedule_next().unwrap();
    assert_eq!(scheduler.code.delete("fixture"), Some(true));
    assert_eq!(scheduler.code.delete("fixture"), Some(false));
    assert_eq!(scheduler.code.delete("nomod"), None);
    let second = BeamModule::new(
        "fixture".into(),
        HashMap::new(),
        Vec::new(),
        vec![Instruction::End],
    )
    .unwrap();
    let reference = scheduler.make_ref().unwrap();
    scheduler.code.prepare(reference, second);
    let result = finish_loading(
        &mut scheduler,
        &Term::list_from(vec![Term::Reference(reference)]),
    )
    .unwrap();
    assert!(matches!(
        result,
        BifResult::Value(Term::Tuple(ref parts))
            if parts.len() == 2
                && parts[0] == Term::static_atom("not_purged")
                && parts[1].proper_list().is_some_and(|modules| modules.count() == 1)
    ));
}

#[test]
fn denies_a_code_delete_without_its_capability() {
    // `delete_module_1` (`beam_bif_load.c:691-693`) runs the capability
    // check after the argument checks. PBeam then checks the `code`
    // capability: a process with no capability gets
    // `error:capability_denied`, and the code table does not change.
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler.code = fixture_code();
    let frame = scheduler.code.mfa_frame("fixture", "main", 0).unwrap();
    scheduler
        .start(frame, vec![], Vec::new())
        .expect("the root starts");
    scheduler.schedule_next().unwrap();
    let fixture = Term::Atom(scheduler.atoms.put("fixture").unwrap());
    let denied = delete_module(&mut scheduler, &fixture).unwrap();
    assert!(matches!(
        denied,
        BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::Atom(ref atom),
        } if scheduler.atoms.name_of(atom) == "capability_denied"
    ));
    assert!(scheduler.code.get("fixture").is_some());
    let module = BeamModule::new(
        "prepared".into(),
        HashMap::new(),
        Vec::new(),
        vec![Instruction::End],
    )
    .unwrap();
    let reference = scheduler.make_ref().unwrap();
    scheduler.code.prepare(reference, module);
    assert!(matches!(
        has_prepared_code_on_load(&mut scheduler, &Term::Reference(reference)).unwrap(),
        BifResult::Value(Term::Atom(ref atom)) if atom.is_static("false")
    ));
}

/// Return process info values with the given current function and
/// initial call.
///
/// # Examples
///
/// ```rust,ignore
/// let values = info_values(Some(MfaNames::Atoms(atom, atom, 2)));
/// assert_eq!(values.status, "running");
/// ```
fn info_values(current_function: Option<MfaNames>) -> ProcessInfoValues {
    let name = || Atom::from_static("erlang").unwrap();
    let function = || Atom::from_static("apply").unwrap();
    ProcessInfoValues {
        registered_name: None,
        current_function,
        initial_call: MfaNames::Atoms(name(), function(), 2),
        status: "running",
        message_queue_len: 0,
        links: Vec::new(),
        dictionary: Vec::new(),
        trap_exit: false,
        error_handler: None,
        priority: "normal",
        group_leader: ProcessId(0),
        total_heap_size: 0,
        heap_size: 0,
        stack_size: 0,
        reductions: 0,
    }
}

#[test]
fn builds_the_process_info_items_of_an_atom_mfa_and_an_undefined_function() {
    // `initial_call` of a process with an undefined start gives its atoms,
    // and `current_function` is `undefined` only without a frame and a
    // start MFA (`erl_bif_info.c:1650-1665`).
    let mut atoms = AtomTable::new();
    let term = process_info_term(
        &mut atoms,
        info_values(Some(MfaNames::Atoms(
            Atom::from_static("erlang").unwrap(),
            Atom::from_static("apply").unwrap(),
            2,
        ))),
    )
    .unwrap();
    assert!(matches!(term, Term::Cons(_)));
    let term = process_info_term(&mut atoms, info_values(None)).unwrap();
    assert!(matches!(term, Term::Cons(_)));
}
