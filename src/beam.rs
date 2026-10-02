use std::{collections::HashMap, fmt};

mod format;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeamError(String);

impl fmt::Display for BeamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BeamError {}

pub(super) fn error(message: impl Into<String>) -> BeamError {
    BeamError(message.into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Atom(String),
    Integer(i64),
}

#[derive(Debug)]
pub struct BeamModule {
    pub(super) name: String,
    pub(super) exports: HashMap<(String, u32), u32>,
    pub(super) instructions: Vec<Instruction>,
}

#[derive(Debug, Clone)]
pub(super) enum Instruction {
    Label(u32),
    FunctionInfo {
        module: String,
        name: String,
        arity: u32,
    },
    Move(Term, u32),
    Return,
    End,
}

impl BeamModule {
    pub fn load(bytes: &[u8]) -> Result<Self, BeamError> {
        format::load(bytes)
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
    use super::*;
    use format::{
        atom_at, parse_atoms, parse_code, parse_exports, read_integer, read_integer_after_first,
        read_operand, read_u32, read_value,
    };

    #[test]
    fn reports_oversized_arity() {
        assert!(checked_arity(usize::MAX).is_err());
    }

    #[test]
    fn formats_runtime_errors() {
        assert_eq!(error("bad module").to_string(), "bad module");
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
    fn rejects_operand_tag_and_truncated_integer_fields() {
        let mut cursor = 0;
        assert!(read_integer(&[0x12], &mut cursor, 1).is_err());
        assert!(read_u32(&[0, 1]).is_err());
        assert!(parse_atoms(&[]).is_err());
        assert!(parse_atoms(&(-1i32).to_be_bytes()).is_err());
        assert!(atom_at(&[], 0).is_err());
        let mut cursor = 0;
        assert!(read_integer_after_first(&[], &mut cursor, 0x19).is_err());
        assert!(read_integer_after_first(&[], &mut cursor, 0x09).is_err());
        assert!(read_operand(&[], &mut cursor, 0).is_err());
        let mut malformed_atoms = (-2i32).to_be_bytes().to_vec();
        malformed_atoms.extend([0x30, b'a', b'b', b'c', 0xa0]);
        assert!(parse_atoms(&malformed_atoms).is_err());
        assert!(parse_atoms(&[0xff, 0xff, 0xff, 0xff, 0x10, 0xff]).is_err());
        assert!(read_value(&[0x03], &mut 0, &[]).is_err());
        assert!(read_value(&[0x12], &mut 0, &[]).is_err());
        assert!(parse_exports(&[], &[]).is_err());
        assert!(parse_exports(&[0, 0, 0, 1], &[]).is_err());
        assert!(parse_code(&[], &[]).is_err());
        assert!(parse_code(&[0; 20], &[]).is_err());
        assert!(
            parse_code(
                &[0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                &[]
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_empty_atom_table_and_invalid_export_indices() {
        assert!(parse_atoms(&0i32.to_be_bytes()).unwrap().is_empty());
        let empty_module = BeamModule {
            name: String::new(),
            exports: HashMap::new(),
            instructions: vec![],
        };
        assert!(BeamModule::load(&assemble_empty_beam()).is_err());
        assert!(empty_module.execute("main", &[]).is_err());

        let exports = [0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 1];
        assert!(parse_exports(&exports, &["constant".into()]).is_err());
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
}
