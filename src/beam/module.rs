use std::collections::HashMap;

use super::{
    code::{self, Instruction},
    error::{BeamError, error},
    term::Term,
};

#[derive(Debug)]
pub struct BeamModule {
    name: String,
    exports: HashMap<(String, u32), u32>,
    instructions: Vec<Instruction>,
}

impl BeamModule {
    pub fn load(bytes: &[u8]) -> Result<Self, BeamError> {
        let parsed = code::parse(bytes)?;
        Ok(Self {
            name: parsed.name,
            exports: parsed.exports,
            instructions: parsed.instructions,
        })
    }

    pub fn execute(&self, function: &str, arguments: &[Term]) -> Result<Term, BeamError> {
        let arity = checked_arity(arguments.len())?;
        let entry = self
            .exports
            .get(&(function.to_owned(), arity))
            .ok_or_else(|| error(format!("missing export {}:{function}/{arity}", self.name)))?;
        let mut active = false;
        let mut label_matches = false;
        let mut value = None;
        for instruction in &self.instructions {
            match instruction {
                Instruction::FunctionInfo {
                    module,
                    name,
                    arity: found,
                } => {
                    active = module == &self.name && name == function && *found == arity;
                    label_matches = false;
                }
                Instruction::Label(label) if active && label == entry => label_matches = true,
                Instruction::Label(_) if active => {}
                Instruction::Move(term, 0) if active && label_matches && value.is_none() => {
                    value = Some(term.clone());
                }
                Instruction::Return if active && label_matches => {
                    return value.ok_or_else(|| error("return before setting x0"));
                }
                Instruction::End => break,
                _ if active && label_matches => return Err(error("unsupported entry instruction")),
                _ => {}
            }
        }
        Err(error(format!(
            "entry label {entry} not found for {}:{function}/{arity}",
            self.name
        )))
    }
}

fn checked_arity(length: usize) -> Result<u32, BeamError> {
    u32::try_from(length).map_err(|_| error("arity exceeds BEAM limit"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{BeamModule, Instruction, Term, checked_arity};

    #[test]
    fn reports_oversized_arity() {
        assert!(checked_arity(usize::MAX).is_err());
    }

    #[test]
    fn execution_skips_another_functions_labels_and_reports_missing_entry_label() {
        let module = BeamModule {
            name: "constant".into(),
            exports: HashMap::from([(("main".into(), 0), 8)]),
            instructions: vec![
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
        };
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
        let module = |instructions| BeamModule {
            name: "constant".into(),
            exports: HashMap::from([(("main".into(), 0), 2)]),
            instructions,
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
                Instruction::Move(Term::Atom("ok".into()), 1)
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
        let module = BeamModule {
            name: String::new(),
            exports: HashMap::new(),
            instructions: vec![],
        };
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
}
