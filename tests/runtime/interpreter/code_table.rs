//! The unit tests of `src/runtime/interpreter/code_table.rs`.

use std::{collections::HashMap, sync::Arc};

use super::{CodeTable, Finish, OnLoadCode};
use crate::{
    BeamModule,
    beam::{
        ModuleId,
        code::{Instruction, Operand},
    },
    runtime::process::InitialCall,
    term::{Atom, AtomTable, Lambda, Reference},
};

/// Return the code table of one module `fixture` with the function `main/0`.
///
/// # Examples
///
/// ```rust,ignore
/// let code = fixture();
/// assert!(code.get("fixture").is_some());
/// ```
fn fixture() -> CodeTable {
    let lambda = Lambda::new(
        AtomTable::new().put("fixture").expect("the atom fits"),
        0,
        7,
        2,
        1,
    );
    let module = BeamModule::new(
        "fixture".into(),
        HashMap::new(),
        Vec::new(),
        vec![
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(1),
            Instruction::MakeFun {
                lambda,
                free: Vec::new(),
                destination: Operand::X(0),
            },
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: "-fun-".into(),
                arity: 1,
            },
            Instruction::Label(2),
        ],
    )
    .expect("the fixture module loads");
    CodeTable::new(HashMap::from([("fixture".into(), Arc::new(module))]))
}

/// The identity of a module reads back its code, and an identity that no
/// loaded module has is a runtime invariant error.
#[test]
fn returns_the_loaded_module_of_an_identity() {
    let code = fixture();
    let module = code.get("fixture").unwrap();
    assert_eq!(code.module_by_id(module.id).unwrap().name, "fixture");
    let error = code.module_by_id(ModuleId::next()).unwrap_err().to_string();
    assert!(error.contains("invariant error: no loaded module has identity"));
}

/// An entry start resolves its names from the loader tables of the module,
/// and an undefined start resolves them from its atoms.
#[test]
fn returns_the_names_of_a_start_mfa() {
    let code = fixture();
    let module = code.get("fixture").unwrap();
    let entry = InitialCall::Entry {
        module: module.id,
        function: module.tables.name_id("main").unwrap(),
        arity: 2,
    };
    assert_eq!(
        code.initial_call_names(&AtomTable::new(), &entry).unwrap(),
        ("fixture".to_owned(), "main".to_owned(), 2)
    );
    let undefined = InitialCall::Undefined {
        module: Atom::from_static("error_handler").unwrap(),
        function: Atom::from_static("undefined_function").unwrap(),
        arity: 0,
    };
    assert_eq!(
        code.initial_call_names(&AtomTable::new(), &undefined)
            .unwrap(),
        (
            "error_handler".to_owned(),
            "undefined_function".to_owned(),
            0
        )
    );
}

/// Return the module `name` with no export.
///
/// # Examples
///
/// ```rust,ignore
/// let module = module("a");
/// assert_eq!(module.name, "a");
/// ```
fn module(name: &str) -> BeamModule {
    BeamModule::new(
        name.to_owned(),
        HashMap::new(),
        Vec::new(),
        vec![Instruction::End],
    )
    .unwrap()
}

#[test]
fn returns_not_purged_in_reverse_input_order() {
    // `exception_list` prepends each module of the input order
    // (`beam_bif_load.c:322-337`), so the result is in reverse order.
    let mut code = CodeTable::new(HashMap::new());
    let (a1, b1) = (Reference(1), Reference(2));
    code.prepare(a1, module("a"));
    code.prepare(b1, module("b"));
    code.finish(&[a1, b1], vec!["a".into(), "b".into()])
        .unwrap();
    assert_eq!(code.delete("a"), Some(true));
    assert_eq!(code.delete("b"), Some(true));
    let (a2, b2) = (Reference(3), Reference(4));
    code.prepare(a2, module("a"));
    code.prepare(b2, module("b"));
    code.finish(&[a2, b2], vec!["a".into(), "b".into()])
        .unwrap();
    let (a3, b3) = (Reference(5), Reference(6));
    code.prepare(a3, module("a"));
    code.prepare(b3, module("b"));
    assert_eq!(
        code.finish(&[a3, b3], vec!["a".into(), "b".into()])
            .unwrap(),
        Finish::NotPurged(vec!["b".into(), "a".into()])
    );
}

#[test]
fn rejects_a_delete_when_the_module_has_old_code() {
    // `delete_module_1` raises `badarg` before it changes code when the
    // module has old code (`beam_bif_load.c:702-708`).
    let mut code = CodeTable::new(HashMap::new());
    let first = Reference(1);
    code.prepare(first, module("a"));
    code.finish(&[first], vec!["a".into()]).unwrap();
    assert_eq!(code.delete("a"), Some(true));
    let second = Reference(2);
    code.prepare(second, module("a"));
    code.finish(&[second], vec!["a".into()]).unwrap();
    assert_eq!(code.delete("a"), Some(false));
    assert!(code.get("a").is_some());
}

/// A fun of the old code of a deleted module has the name of its function,
/// also after a load of another version, and it has no name while a purge of
/// that code runs: the preparation of the purge clears the fun address
/// (`erl_fun.c:159-175`).
#[test]
fn gives_no_name_of_a_fun_of_code_in_a_purge() {
    let mut code = fixture();
    let lambda = Lambda::new(
        AtomTable::new().put("fixture").expect("the atom fits"),
        0,
        7,
        2,
        1,
    );
    assert_eq!(
        code.function_name_of("fixture", &lambda)
            .unwrap()
            .as_deref(),
        Some("-fun-")
    );
    code.delete("fixture");
    assert_eq!(
        code.function_name_of("fixture", &lambda)
            .unwrap()
            .as_deref(),
        Some("-fun-")
    );
    let module = Arc::clone(&code.by_id.values().next().expect("the old code").code);
    code.purging = Some(("fixture".to_owned(), module));
    assert_eq!(code.function_name_of("fixture", &lambda).unwrap(), None);
}

/// The code of an `-on_load` purge is old code from the preparation until
/// the purge completes: ERTS moves the `on_load` code to `modp->old`
/// (`beam_bif_load.c:2134-2141`), so `erlang:loaded/0`, which lists each
/// module with current or old code (`beam_bif_load.c:762-791`), names the
/// module in this window, and another process can call it during the
/// asynchronous purge checks. The test drives the code table directly, so
/// it does not depend on the timing of the purge checks.
#[test]
fn names_the_module_of_an_on_load_purge_until_it_completes() {
    let mut code = fixture();
    let module = code.modules.remove("fixture").expect("the fixture module");
    code.on_load.insert(
        "fixture".to_owned(),
        OnLoadCode {
            code: module,
            deferred: false,
        },
    );
    assert_eq!(code.names().count(), 0);
    assert_eq!(code.prepare_on_load_purge("fixture").unwrap(), Some(true));
    assert_eq!(code.names().collect::<Vec<_>>(), ["fixture"]);
    assert!(code.complete_purge("fixture"));
    assert_eq!(code.names().count(), 0);
}
