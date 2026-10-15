use std::collections::HashMap;

use super::{BeamModule, Instruction, Operand, Term, checked_arity};

#[test]
fn reports_oversized_arity() {
    assert!(checked_arity(usize::MAX).is_err());
}

#[test]
fn execution_skips_another_functions_labels_and_reports_missing_entry_label() {
    let module = BeamModule::new(
        "constant".into(),
        HashMap::from([(("main".into(), 0), 8)]),
        Vec::new(),
        vec![
            Instruction::FunctionInfo {
                module: "other".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(4),
            Instruction::FunctionInfo {
                module: "constant".into(),
                name: "main".into(),
                arity: 0,
            },
            Instruction::Label(7),
            Instruction::End,
        ],
    )
    .expect("the fixture module loads");
    assert!(
        module
            .execute("main", &[])
            .unwrap_err()
            .to_string()
            .contains("entry label 8 not found")
    );
}

#[test]
fn rejects_unimplemented_entry_shapes() {
    let module = |instructions| {
        BeamModule::new(
            "constant".into(),
            HashMap::from([(("main".into(), 0), 2)]),
            Vec::new(),
            instructions,
        )
        .expect("the fixture module loads")
    };
    let function = Instruction::FunctionInfo {
        module: "constant".into(),
        name: "main".into(),
        arity: 0,
    };
    assert!(
        module(vec![
            function.clone(),
            Instruction::Label(2),
            Instruction::Move(Operand::Value(Term::static_atom("ok")), Operand::X(1))
        ])
        .execute("main", &[])
        .unwrap_err()
        .to_string()
        .contains("unsupported entry instruction")
    );
    assert!(
        module(vec![function, Instruction::Label(2), Instruction::Return])
            .execute("main", &[])
            .unwrap_err()
            .to_string()
            .contains("return before setting x0")
    );
}

#[test]
fn rejects_empty_module_and_missing_export() {
    let module = BeamModule::new(String::new(), HashMap::new(), Vec::new(), vec![])
        .expect("the fixture module loads");
    assert!(module.execute("main", &[]).is_err());
    assert!(BeamModule::load(&assemble_empty_beam()).is_err());
}

fn assemble_empty_beam() -> Vec<u8> {
    let mut code = vec![0; 20];
    code[..4].copy_from_slice(&16u32.to_be_bytes());
    code.push(3);
    let chunks = vec![
        (*b"AtU8", vec![0, 0, 0, 0]),
        (*b"Code", code),
        (*b"ExpT", vec![0, 0, 0, 0]),
    ];
    let mut body = b"BEAM".to_vec();
    for (id, data) in chunks {
        body.extend(id);
        body.extend((data.len() as u32).to_be_bytes());
        body.extend(&data);
        body.resize(body.len() + (4 - data.len() % 4) % 4, 0);
    }
    let mut result = b"FOR1".to_vec();
    result.extend((body.len() as u32).to_be_bytes());
    result.extend(body);
    result
}
