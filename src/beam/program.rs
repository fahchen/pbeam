use std::collections::HashMap;

use super::{
    BeamError,
    code::{Instruction, Operand},
    error::error,
    module::BeamModule,
    term::Term,
};

#[derive(Debug)]
pub struct BeamProgram {
    modules: HashMap<String, BeamModule>,
}

impl BeamProgram {
    pub fn load(modules: &[&[u8]]) -> Result<Self, BeamError> {
        let mut loaded = HashMap::new();
        for bytes in modules {
            let module = BeamModule::load(bytes)?;
            let name = module.name.clone();
            if loaded.insert(name.clone(), module).is_some() {
                return Err(error(format!("duplicate BEAM module {name}")));
            }
        }
        Ok(Self { modules: loaded })
    }

    pub fn execute(
        &self,
        module: &str,
        function: &str,
        arguments: &[Term],
    ) -> Result<Term, BeamError> {
        let loaded = self
            .modules
            .get(module)
            .ok_or_else(|| error(format!("missing BEAM module {module}")))?;
        let arity =
            u32::try_from(arguments.len()).map_err(|_| error("arity exceeds BEAM limit"))?;
        let label = *loaded
            .exports
            .get(&(function.to_owned(), arity))
            .ok_or_else(|| error(format!("missing export {module}:{function}/{arity}")))?;
        self.execute_label(module, label, arguments)
    }

    fn execute_mfa(
        &self,
        module: &str,
        function: &str,
        arity: u32,
        arguments: &[Term],
    ) -> Result<Term, BeamError> {
        let loaded = self.modules.get(module).ok_or_else(|| {
            error(format!(
                "unresolved external MFA {module}:{function}/{}",
                arguments.len()
            ))
        })?;
        let label = *loaded
            .exports
            .get(&(function.to_owned(), arity))
            .ok_or_else(|| {
                error(format!(
                    "unresolved external MFA {module}:{function}/{arity}"
                ))
            })?;
        self.execute_label(module, label, arguments)
    }

    fn execute_label(
        &self,
        module_name: &str,
        label: u32,
        arguments: &[Term],
    ) -> Result<Term, BeamError> {
        let module = self
            .modules
            .get(module_name)
            .ok_or_else(|| error(format!("missing BEAM module {module_name}")))?;
        let function = function_at_label(module, label)?;
        if function.arity as usize != arguments.len() {
            return Err(error(format!(
                "call arity does not match {module_name}:{}",
                function.name
            )));
        }
        let start = function.start;
        let mut registers = arguments.iter().cloned().map(Some).collect::<Vec<_>>();
        let mut stack = Vec::new();
        let mut pc = start;
        while let Some(instruction) = module.instructions.get(pc) {
            pc += 1;
            match instruction {
                Instruction::Move(source, destination) => {
                    let value = read_operand(source, &registers, &stack)?;
                    write_operand(destination, value, &mut registers, &mut stack)?;
                }
                Instruction::Allocate { stack_size } => stack.resize(*stack_size as usize, None),
                Instruction::Deallocate { stack_size } => {
                    if stack.len() != *stack_size as usize {
                        return Err(error("invalid BEAM stack size"));
                    }
                    stack.clear();
                }
                Instruction::Call { arity, label } => {
                    let arguments = call_arguments(&registers, *arity)?;
                    let result = self.execute_label(module_name, *label, &arguments)?;
                    set_x0(result, &mut registers);
                }
                Instruction::CallExternal {
                    arity,
                    module,
                    name,
                    tail,
                    deallocate,
                } => {
                    if *tail && *deallocate as usize != stack.len() {
                        return Err(error("invalid BEAM tail-call stack size"));
                    }
                    let arguments = call_arguments(&registers, *arity)?;
                    let result = self.execute_mfa(module, name, *arity, &arguments)?;
                    if *tail {
                        return Ok(result);
                    }
                    set_x0(result, &mut registers);
                }
                Instruction::Return => {
                    return registers
                        .first()
                        .and_then(Clone::clone)
                        .ok_or_else(|| error("return before setting x0"));
                }
                Instruction::End => break,
                Instruction::Label(_) | Instruction::FunctionInfo { .. } => {
                    return Err(error("unsupported entry instruction"));
                }
            }
        }
        Err(error(format!(
            "function {}:{} did not return",
            module_name, function.name
        )))
    }
}

#[derive(Debug)]
struct Function<'a> {
    name: &'a str,
    arity: u32,
    start: usize,
}

fn function_at_label(module: &BeamModule, target: u32) -> Result<Function<'_>, BeamError> {
    let mut function = None;
    for (index, instruction) in module.instructions.iter().enumerate() {
        match instruction {
            Instruction::FunctionInfo {
                module: found_module,
                name,
                arity,
            } if found_module == &module.name => {
                function = Some(Function {
                    name,
                    arity: *arity,
                    start: index + 1,
                })
            }
            Instruction::Label(label) if *label == target => {
                return function
                    .take()
                    .map(|mut function| {
                        function.start = index + 1;
                        function
                    })
                    .ok_or_else(|| error(format!("entry label {target} has no function info")));
            }
            Instruction::FunctionInfo { .. } => function = None,
            _ => {}
        }
    }
    Err(error(format!(
        "entry label {target} not found in {}",
        module.name
    )))
}

fn call_arguments(registers: &[Option<Term>], arity: u32) -> Result<Vec<Term>, BeamError> {
    (0..arity)
        .map(|index| {
            registers
                .get(index as usize)
                .and_then(Clone::clone)
                .ok_or_else(|| error(format!("call argument x{index} is unset")))
        })
        .collect()
}

fn set_x0(value: Term, registers: &mut Vec<Option<Term>>) {
    if registers.is_empty() {
        registers.push(Some(value));
    } else {
        registers[0] = Some(value);
        registers.truncate(1);
    }
}

fn read_operand(
    operand: &Operand,
    registers: &[Option<Term>],
    stack: &[Option<Term>],
) -> Result<Term, BeamError> {
    match operand {
        Operand::Value(value) => Ok(value.clone()),
        Operand::X(index) => registers
            .get(*index as usize)
            .and_then(Clone::clone)
            .ok_or_else(|| error(format!("register x{index} is unset"))),
        Operand::Y(index) => stack
            .get(*index as usize)
            .and_then(Clone::clone)
            .ok_or_else(|| error(format!("stack register y{index} is unset"))),
    }
}

fn write_operand(
    operand: &Operand,
    value: Term,
    registers: &mut Vec<Option<Term>>,
    stack: &mut [Option<Term>],
) -> Result<(), BeamError> {
    match operand {
        Operand::X(index) => {
            let index = *index as usize;
            registers.resize(registers.len().max(index + 1), None);
            registers[index] = Some(value);
        }
        Operand::Y(index) => {
            let index = *index as usize;
            if index >= stack.len() {
                return Err(error(format!(
                    "register y{index} is outside allocated range"
                )));
            }
            stack[index] = Some(value);
        }
        Operand::Value(_) => return Err(error("invalid BEAM move destination")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        BeamProgram, call_arguments, function_at_label, read_operand, set_x0, write_operand,
    };
    use crate::beam::{
        code::{Instruction, Operand},
        module::BeamModule,
        term::Term,
    };

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
        }
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
        };
        assert!(
            program
                .execute_mfa("missing", "run", 0, &[])
                .unwrap_err()
                .to_string()
                .contains("unresolved external MFA missing:run/0")
        );
        assert!(
            program
                .execute_mfa("external", "missing", 0, &[])
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

    #[test]
    fn handles_register_values_and_reports_invalid_registers() {
        let mut registers = vec![];
        set_x0(Term::Integer(7), &mut registers);
        assert_eq!(
            read_operand(&Operand::X(0), &registers, &[]).unwrap(),
            Term::Integer(7)
        );
        assert!(read_operand(&Operand::X(1), &registers, &[]).is_err());
        assert!(read_operand(&Operand::Y(0), &registers, &[]).is_err());
        assert!(call_arguments(&[], 1).is_err());
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
}
