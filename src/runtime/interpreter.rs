use super::process::{Frame, ProcessState, Scheduler};
use crate::{
    beam::{
        BeamModule,
        code::{Instruction, Operand, TestKind},
    },
    error::{BeamError, error},
    term::Term,
};
use std::collections::HashMap;

#[derive(Debug)]
pub struct BeamProgram {
    modules: HashMap<String, BeamModule>,
}

/// Why a process stopped running on the scheduler.
#[derive(Debug)]
enum Outcome {
    /// The process waits for a new message.
    Wait,
    /// The process exited. `result` is the return value of a normal exit.
    Exit { reason: Term, result: Option<Term> },
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

    /// Run an exported function as the root process. Processes that it
    /// spawns run on the same cooperative scheduler until the root exits or
    /// no process can run.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let result = program.execute("process_probe", "main", &[])?;
    /// ```
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
        let mut scheduler = Scheduler::default();
        scheduler.start(self.frame(module, label, arity)?, arguments.to_vec())?;
        while let Some(mut process) = scheduler.take_next() {
            match self.run(&mut scheduler, &mut process)? {
                Outcome::Wait => scheduler.park(process),
                Outcome::Exit { reason, result } => scheduler.finish(process, reason, result)?,
            }
        }
        match scheduler.into_root_result() {
            Some(Ok(result)) => Ok(result),
            Some(Err(reason)) => Err(error(format!("root process exited with {reason:?}"))),
            None => Err(error("root process waits for a message that cannot arrive")),
        }
    }

    /// Create the frame for an exported function.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let frame = program.mfa_frame("external", "effect", 1)?;
    /// ```
    fn mfa_frame(&self, module: &str, function: &str, arity: u32) -> Result<Frame, BeamError> {
        let label = self
            .modules
            .get(module)
            .and_then(|loaded| loaded.exports.get(&(function.to_owned(), arity)))
            .ok_or_else(|| {
                error(format!(
                    "unresolved external MFA {module}:{function}/{arity}"
                ))
            })?;
        self.frame(module, *label, arity)
    }

    /// Create the frame for the function at an entry label.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let frame = program.frame("fixture", 2, 0)?;
    /// assert_eq!(frame.function, "main");
    /// ```
    fn frame(&self, module_name: &str, label: u32, arity: u32) -> Result<Frame, BeamError> {
        let module = self.module(module_name)?;
        let function = function_at_label(module, label)?;
        if function.arity != arity {
            return Err(error(format!(
                "call arity does not match {module_name}:{}",
                function.name
            )));
        }
        Ok(Frame {
            module: module_name.to_owned(),
            function: function.name.to_owned(),
            pc: function.start,
            stack: Vec::new(),
        })
    }

    /// Return a loaded module by name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(program.module("fixture")?.name, "fixture");
    /// ```
    fn module(&self, name: &str) -> Result<&BeamModule, BeamError> {
        self.modules
            .get(name)
            .ok_or_else(|| error(format!("missing BEAM module {name}")))
    }

    /// Run one process until it waits for a message or exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(program.run(&mut scheduler, &mut process)?, Outcome::Wait));
    /// ```
    fn run(
        &self,
        scheduler: &mut Scheduler,
        process: &mut ProcessState,
    ) -> Result<Outcome, BeamError> {
        loop {
            let frame = current_frame(process);
            let module = self.module(&frame.module)?;
            let Some(instruction) = module.instructions.get(frame.pc) else {
                return Err(error(format!(
                    "function {}:{} did not return",
                    frame.module, frame.function
                )));
            };
            frame.pc += 1;
            match instruction {
                Instruction::Move(source, destination) => {
                    let value = read(process, source)?;
                    write(process, destination, value)?;
                }
                Instruction::Allocate { stack_size } => {
                    current_frame(process)
                        .stack
                        .resize(*stack_size as usize, None);
                }
                Instruction::Deallocate { stack_size } => {
                    let stack = &mut current_frame(process).stack;
                    if stack.len() != *stack_size as usize {
                        return Err(error("invalid BEAM stack size"));
                    }
                    stack.clear();
                }
                Instruction::Call { arity, label } => {
                    call_arguments(&process.registers, *arity)?;
                    let frame = self.frame(&module.name, *label, *arity)?;
                    process.frames.push(frame);
                }
                Instruction::CallOnly { arity, label } => {
                    call_arguments(&process.registers, *arity)?;
                    *current_frame(process) = self.frame(&module.name, *label, *arity)?;
                }
                Instruction::CallExternal {
                    arity,
                    module,
                    name,
                    tail,
                    deallocate,
                } => {
                    if *tail && *deallocate as usize != current_frame(process).stack.len() {
                        return Err(error("invalid BEAM tail-call stack size"));
                    }
                    let arguments = call_arguments(&process.registers, *arity)?;
                    if module == "erlang" {
                        let value = match self.bif(scheduler, process, name, arguments)? {
                            Ok(value) => value,
                            Err(reason) => {
                                return Ok(Outcome::Exit {
                                    reason,
                                    result: None,
                                });
                            }
                        };
                        write(process, &Operand::X(0), value)?;
                        if *tail && let Some(outcome) = return_from_frame(process)? {
                            return Ok(outcome);
                        }
                        continue;
                    }
                    let frame = self.mfa_frame(module, name, *arity)?;
                    if *tail {
                        *current_frame(process) = frame;
                    } else {
                        process.frames.push(frame);
                    }
                }
                Instruction::Bif0 {
                    module,
                    name,
                    destination,
                } => {
                    if module != "erlang" || name != "self" {
                        return Err(error(format!("unsupported BIF {module}:{name}/0")));
                    }
                    write(process, destination, Term::Pid(process.id))?;
                }
                Instruction::Jump(label) => jump(module, process, *label)?,
                Instruction::Test {
                    kind,
                    fail_label,
                    value,
                } => {
                    let value = read(process, value)?;
                    let passed = match kind {
                        TestKind::Atom => matches!(value, Term::Atom(_)),
                        TestKind::Pid => matches!(value, Term::Pid(_)),
                        TestKind::List => matches!(value, Term::List(_)),
                        TestKind::Binary => matches!(value, Term::Binary(_)),
                    };
                    if !passed {
                        jump(module, process, *fail_label)?;
                    }
                }
                Instruction::TestExactEqual {
                    fail_label,
                    left,
                    right,
                } => {
                    if read(process, left)? != read(process, right)? {
                        jump(module, process, *fail_label)?;
                    }
                }
                Instruction::TestTaggedTuple {
                    fail_label,
                    value,
                    arity,
                    tag,
                } => {
                    let passed = matches!(
                        read(process, value)?,
                        Term::Tuple(fields)
                            if fields.len() == *arity as usize
                                && fields.first() == Some(tag)
                    );
                    if !passed {
                        jump(module, process, *fail_label)?;
                    }
                }
                Instruction::SelectValue {
                    value,
                    fail_label,
                    choices,
                } => {
                    let value = read(process, value)?;
                    let target = choices
                        .iter()
                        .find(|(choice, _)| choice == &value)
                        .map_or(*fail_label, |(_, label)| *label);
                    jump(module, process, target)?;
                }
                Instruction::PutList {
                    head,
                    tail,
                    destination,
                } => {
                    let head = read(process, head)?;
                    let mut values = match read(process, tail)? {
                        Term::List(values) => values,
                        Term::Atom(atom) if atom == "nil" => Vec::new(),
                        _ => return Err(error("unsupported BEAM list tail")),
                    };
                    values.insert(0, head);
                    write(process, destination, Term::List(values))?;
                }
                Instruction::PutTuple {
                    elements,
                    destination,
                } => {
                    let elements = elements
                        .iter()
                        .map(|element| read(process, element))
                        .collect::<Result<_, _>>()?;
                    write(process, destination, Term::Tuple(elements))?;
                }
                Instruction::GetTupleElement {
                    tuple,
                    index,
                    destination,
                } => {
                    let Term::Tuple(mut fields) = read(process, tuple)? else {
                        return Err(error("BEAM tuple element source is not a tuple"));
                    };
                    let index = *index as usize;
                    if index >= fields.len() {
                        return Err(error(format!("BEAM tuple has no element {index}")));
                    }
                    write(process, destination, fields.swap_remove(index))?;
                }
                Instruction::Send => {
                    let Term::Pid(target) = read(process, &Operand::X(0))? else {
                        return Err(error("unsupported BEAM send target"));
                    };
                    let message = read(process, &Operand::X(1))?;
                    scheduler.deliver(process, target, message.clone())?;
                    write(process, &Operand::X(0), message)?;
                }
                Instruction::LoopReceive {
                    fail_label,
                    destination,
                } => {
                    let message = process
                        .mailbox
                        .first_at_or_after(process.receive_cursor)
                        .map(|message| (message.sequence, message.term.clone()));
                    match message {
                        Some((sequence, term)) => {
                            process.current_message = Some(sequence);
                            write(process, destination, term)?;
                        }
                        None => jump(module, process, *fail_label)?,
                    }
                }
                Instruction::LoopReceiveEnd(label) => {
                    let sequence = process
                        .current_message
                        .take()
                        .ok_or_else(|| error("loop_rec_end without a current message"))?;
                    process.receive_cursor = sequence + 1;
                    jump(module, process, *label)?;
                }
                Instruction::RemoveMessage => {
                    let sequence = process
                        .current_message
                        .take()
                        .ok_or_else(|| error("remove_message without a current message"))?;
                    process.mailbox.remove(sequence);
                    process.receive_cursor = 0;
                }
                Instruction::Wait(label) => {
                    jump(module, process, *label)?;
                    return Ok(Outcome::Wait);
                }
                Instruction::Timeout => {
                    process.current_message = None;
                    process.receive_cursor = 0;
                }
                Instruction::ReceiveMarkerReserve(marker) => {
                    let reference = scheduler.make_ref()?;
                    let position = process.mailbox.next_sequence();
                    process.receive_markers.insert(reference, position);
                    write(process, marker, Term::Reference(reference))?;
                }
                Instruction::ReceiveMarkerBind { marker, reference } => {
                    if let (Term::Reference(marker), Term::Reference(reference)) =
                        (read(process, marker)?, read(process, reference)?)
                        && let Some(position) = process.receive_markers.remove(&marker)
                    {
                        process.receive_markers.insert(reference, position);
                    }
                }
                Instruction::ReceiveMarkerUse(reference) => {
                    if let Term::Reference(reference) = read(process, reference)?
                        && let Some(position) = process.receive_markers.get(&reference)
                    {
                        process.receive_cursor = *position;
                    }
                }
                Instruction::ReceiveMarkerClear(reference) => {
                    if let Term::Reference(reference) = read(process, reference)? {
                        process.receive_markers.remove(&reference);
                    }
                }
                Instruction::Noop | Instruction::Label(_) => {}
                Instruction::Return => {
                    if let Some(outcome) = return_from_frame(process)? {
                        return Ok(outcome);
                    }
                }
                Instruction::End => {
                    let frame = current_frame(process);
                    return Err(error(format!(
                        "function {}:{} did not return",
                        frame.module, frame.function
                    )));
                }
                Instruction::Unsupported(opcode) => {
                    return Err(error(format!("unsupported BEAM opcode {opcode}")));
                }
                Instruction::FunctionInfo {
                    module,
                    name,
                    arity,
                } => return Err(error(format!("function clause {module}:{name}/{arity}"))),
            }
        }
    }

    /// Run an `erlang` BIF that the observed process path calls. `Err` holds
    /// the exit reason when the BIF exits the process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = program.bif(&mut scheduler, &mut process, "make_ref", vec![])?;
    /// assert!(matches!(value, Ok(Term::Reference(_))));
    /// ```
    fn bif(
        &self,
        scheduler: &mut Scheduler,
        process: &mut ProcessState,
        name: &str,
        arguments: Vec<Term>,
    ) -> Result<Result<Term, Term>, BeamError> {
        let value = match (name, arguments.as_slice()) {
            ("process_flag", [Term::Atom(flag), Term::Atom(value)])
                if flag == "trap_exit" && (value == "true" || value == "false") =>
            {
                let old = Term::Atom(process.trap_exit.to_string());
                process.trap_exit = value == "true";
                old
            }
            ("make_ref", []) => Term::Reference(scheduler.make_ref()?),
            ("exit", [reason]) => return Ok(Err(reason.clone())),
            ("spawn_monitor" | "spawn_link", [Term::Atom(module), Term::Atom(function), list]) => {
                let arguments = match list {
                    Term::List(values) => values.clone(),
                    Term::Atom(atom) if atom == "nil" => Vec::new(),
                    _ => return Err(error("spawn arguments are not a proper list")),
                };
                let arity = u32::try_from(arguments.len())
                    .map_err(|_| error("arity exceeds BEAM limit"))?;
                let frame = self.mfa_frame(module, function, arity)?;
                let monitor = name == "spawn_monitor";
                let (pid, reference) =
                    scheduler.spawn(process, frame, arguments, monitor, !monitor)?;
                match reference {
                    Some(reference) => {
                        Term::Tuple(vec![Term::Pid(pid), Term::Reference(reference)])
                    }
                    None => Term::Pid(pid),
                }
            }
            _ => {
                return Err(error(format!(
                    "unresolved external MFA erlang:{name}/{}",
                    arguments.len()
                )));
            }
        };
        Ok(Ok(value))
    }
}

/// Return the active frame of a running process.
///
/// # Examples
///
/// ```rust,ignore
/// current_frame(&mut process).pc += 1;
/// ```
fn current_frame(process: &mut ProcessState) -> &mut Frame {
    // ponytail: the run loop exits when the last frame returns, so a frame is always active.
    process
        .frames
        .last_mut()
        .expect("a running process has an active frame")
}

/// Pop the active frame. Return the exit outcome when it was the last frame.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(return_from_frame(&mut process)?.is_some());
/// ```
fn return_from_frame(process: &mut ProcessState) -> Result<Option<Outcome>, BeamError> {
    let result = read(process, &Operand::X(0)).map_err(|_| error("return before setting x0"))?;
    process.frames.pop();
    if !process.frames.is_empty() {
        return Ok(None);
    }
    Ok(Some(Outcome::Exit {
        reason: Term::Atom("normal".into()),
        result: Some(result),
    }))
}

/// Continue execution after a label in the active module.
///
/// # Examples
///
/// ```rust,ignore
/// jump(module, &mut process, 3)?;
/// ```
fn jump(module: &BeamModule, process: &mut ProcessState, label: u32) -> Result<(), BeamError> {
    current_frame(process).pc = module
        .instructions
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Label(found) if *found == label))
        .map(|index| index + 1)
        .ok_or_else(|| error(format!("missing BEAM label {label}")))?;
    Ok(())
}

/// Read an operand from the process registers or the active frame.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read(&process, &Operand::Value(Term::Integer(1)))?, Term::Integer(1));
/// ```
fn read(process: &ProcessState, operand: &Operand) -> Result<Term, BeamError> {
    let stack = process.frames.last().map_or(&[][..], |frame| &frame.stack);
    read_operand(operand, &process.registers, stack)
}

/// Write a value to an X register or a Y register of the active frame.
///
/// # Examples
///
/// ```rust,ignore
/// write(&mut process, &Operand::X(0), Term::Integer(1))?;
/// ```
fn write(process: &mut ProcessState, operand: &Operand, value: Term) -> Result<(), BeamError> {
    let stack = &mut process
        .frames
        .last_mut()
        .expect("a running process has an active frame")
        .stack;
    write_operand(operand, value, &mut process.registers, stack)
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

fn read_operand(
    operand: &Operand,
    registers: &[Option<Term>],
    stack: &[Option<Term>],
) -> Result<Term, BeamError> {
    match operand {
        Operand::Value(value) => Ok(value.clone()),
        Operand::UnsupportedLiteral(index) => {
            Err(error(format!("unsupported BEAM literal index {index}")))
        }
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
        Operand::UnsupportedLiteral(index) => {
            return Err(error(format!("unsupported BEAM literal index {index}")));
        }
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
    use std::{collections::HashMap, path::PathBuf, process::Command};

    use super::{BeamProgram, call_arguments, function_at_label, read_operand, write_operand};
    use crate::{
        BeamModule, Term,
        beam::code::{Instruction, Operand},
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
        Operand::Value(Term::Atom(name.into()))
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
    fn reports_process_exits_and_unsupported_exit_signals() {
        let error = |body| run_with_child(body).unwrap_err().to_string();
        assert!(
            error(vec![
                Instruction::Move(atom("boom"), Operand::X(0)),
                erlang_call("exit", 1, false),
            ])
            .contains("root process exited with Atom(\"boom\")")
        );
        let mut body = spawn_link_child(Term::List(vec![Term::Integer(1)]));
        body.extend(wait_forever());
        assert!(error(body).contains("unsupported exit signal"));
        let mut body = spawn_link_child(Term::List(vec![Term::Atom("normal".into())]));
        body.extend(wait_forever());
        assert!(error(body).contains("waits for a message that cannot arrive"));
        let mut body = spawn_link_child(Term::List(vec![Term::Integer(1)]));
        body.push(Instruction::Return);
        assert!(matches!(run_with_child(body), Ok(Term::Pid(_))));
        assert!(error(spawn_link_child(Term::Integer(1))).contains("not a proper list"));
        assert!(error(vec![erlang_call("node", 0, false)]).contains("erlang:node/0"));
    }

    #[test]
    fn returns_a_tail_called_bif_value_and_drops_messages_to_missing_processes() {
        assert!(matches!(
            run_with_child(vec![erlang_call("make_ref", 0, true)]),
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
            Term::Atom("lost".into())
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
                    tag: Term::Atom("a".into()),
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
            Term::Tuple(vec![Term::Atom("a".into())])
        );
    }

    #[test]
    fn rejects_invalid_tuple_access_sends_and_bifs() {
        let error = |body| run_with_child(body).unwrap_err().to_string();
        let element = |tuple, index| Instruction::GetTupleElement {
            tuple: Operand::Value(tuple),
            index,
            destination: Operand::X(0),
        };
        assert!(error(vec![element(Term::Integer(1), 0)]).contains("not a tuple"));
        assert!(error(vec![element(Term::Tuple(vec![]), 0)]).contains("no element 0"));
        assert!(
            error(vec![
                Instruction::Move(atom("nobody"), Operand::X(0)),
                Instruction::Send
            ])
            .contains("send target")
        );
        assert!(
            error(vec![Instruction::Bif0 {
                module: "erlang".into(),
                name: "node".into(),
                destination: Operand::X(0),
            }])
            .contains("unsupported BIF erlang:node/0")
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
                Instruction::Move(Operand::Value(Term::Atom("pid".into())), Operand::X(0)),
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Move(Operand::Value(Term::Atom("other".into())), Operand::X(0)),
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
            Term::Atom("pid".into())
        );
        assert_eq!(
            program
                .execute("fixture", "main", &[Term::Integer(1)])
                .unwrap(),
            Term::Atom("other".into())
        );
    }

    #[test]
    fn builds_a_list_from_nil_and_rejects_an_unsupported_tail() {
        let list = make_program(
            vec![
                Instruction::PutList {
                    head: Operand::Value(Term::Integer(10)),
                    tail: Operand::Value(Term::Atom("nil".into())),
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

        let invalid_tail = make_program(
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
        assert!(invalid_tail.execute("fixture", "main", &[]).is_err());
    }

    #[test]
    fn executes_compiled_io_puts_and_passes_chardata_to_beam_io() {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/hello/_build/test/lib/hello_reference/ebin");
        let hello = std::fs::read(fixture_dir.join("Elixir.Hello.beam")).unwrap();
        let variant = std::fs::read(fixture_dir.join("Elixir.HelloVariant.beam")).unwrap();
        let io_path = Command::new("elixir")
            .args(["-e", "IO.write(:code.which(IO))"])
            .output()
            .unwrap();
        assert!(io_path.status.success());
        let io = std::fs::read(String::from_utf8(io_path.stdout).unwrap()).unwrap();
        let mut modules = HashMap::from([
            ("Elixir.Hello".into(), BeamModule::load(&hello).unwrap()),
            (
                "Elixir.HelloVariant".into(),
                BeamModule::load(&variant).unwrap(),
            ),
            ("Elixir.IO".into(), BeamModule::load(&io).unwrap()),
        ]);
        modules.insert(
            "io".into(),
            BeamModule {
                name: "io".into(),
                exports: HashMap::from([(("put_chars".into(), 2), 1)]),
                instructions: vec![
                    Instruction::FunctionInfo {
                        module: "io".into(),
                        name: "put_chars".into(),
                        arity: 2,
                    },
                    Instruction::Label(1),
                    Instruction::Move(Operand::X(1), Operand::X(0)),
                    Instruction::Return,
                ],
            },
        );
        let program = BeamProgram { modules };

        assert_eq!(
            program.execute("Elixir.Hello", "main", &[]).unwrap(),
            Term::List(vec![
                Term::Binary(b"Hello, world!".to_vec()),
                Term::Integer(10),
            ])
        );
        assert_eq!(
            program.execute("Elixir.HelloVariant", "main", &[]).unwrap(),
            Term::List(vec![
                Term::Binary(b"Another greeting.".to_vec()),
                Term::Integer(10),
            ])
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
}
