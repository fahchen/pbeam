//! The unit tests of `src/runtime/interpreter/code_table.rs`.

use std::{collections::HashMap, rc::Rc};

use super::CodeTable;
use crate::{
    BeamModule,
    beam::{ModuleId, code::Instruction},
    runtime::process::InitialCall,
    term::{Atom, AtomTable},
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
    let module = BeamModule::new(
        "fixture".into(),
        HashMap::new(),
        vec![
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(1),
        ],
    )
    .expect("the fixture module loads");
    CodeTable::new(HashMap::from([("fixture".into(), Rc::new(module))]))
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
