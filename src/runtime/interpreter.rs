use super::{
    process::{Frame, ProcessState, StackSlot},
    scheduler::Scheduler,
    unicode::characters_to_utf8,
};
use crate::{
    beam::{
        BeamModule,
        code::{self, Instruction, Operand, TestKind},
    },
    error::{BeamError, error},
    term::{Atom, AtomTable, Term},
};
use std::collections::HashMap;

#[derive(Debug)]
pub struct BeamProgram {
    /// The loaded modules by name.
    modules: HashMap<String, BeamModule>,
    /// The atom table of the program. ERTS has one atom table for each
    /// emulator, so each program has its own table.
    atoms: AtomTable,
}

/// Why a process stopped running on the scheduler.
#[derive(Debug)]
enum Outcome {
    /// The process waits for a new message.
    Wait,
    /// The process exited. `result` is the return value of a normal exit.
    Exit { reason: Term, result: Option<Term> },
}

/// The class of an exception. The `throw` class is not implemented yet.
#[derive(Debug, Clone, Copy)]
enum ExceptionClass {
    /// Class `error`, as `erlang:nif_error/1` raises.
    Error,
    /// Class `exit`, as `erlang:exit/1` raises.
    Exit,
}

impl ExceptionClass {
    /// Return the atom name of the class, as `try_case` puts it in `x0`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ExceptionClass::Error.name(), "error");
    /// assert_eq!(ExceptionClass::Exit.name(), "exit");
    /// ```
    fn name(self) -> &'static str {
        match self {
            ExceptionClass::Error => "error",
            ExceptionClass::Exit => "exit",
        }
    }
}

/// What a BIF call does to the calling process.
#[derive(Debug)]
enum BifResult {
    /// The BIF returns this value.
    Value(Term),
    /// The BIF raises an exception in the calling process.
    Raise {
        /// Class of the exception.
        class: ExceptionClass,
        /// Reason of the exception, without a stack trace.
        reason: Term,
    },
    /// The BIF continues in BEAM code, as `BIF_TRAP` does in ERTS: the call
    /// becomes a call to this exported function, and its return value is the
    /// value of the BIF call.
    Trap {
        /// Module of the trap target, for example `erts_internal`.
        module: &'static str,
        /// Exported function of the trap target.
        function: &'static str,
        /// Arguments for the trap target, in `x0` onwards.
        arguments: Vec<Term>,
    },
}

impl BifResult {
    /// Return the value of a BIF called through a `bif` instruction with
    /// fail label 0. An exception or a trap from such a BIF is not supported.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = BifResult::Value(Term::Integer(1)).into_bif_value("erlang", "self", 0)?;
    /// assert_eq!(value, Term::Integer(1));
    /// let raise = BifResult::Raise { class: ExceptionClass::Exit, reason: Term::static_atom("badarg") };
    /// assert!(raise.into_bif_value("erlang", "node", 1).is_err());
    /// ```
    fn into_bif_value(self, module: &str, name: &str, arity: usize) -> Result<Term, BeamError> {
        match self {
            BifResult::Value(value) => Ok(value),
            // ponytail: no BIF that a `bif` instruction reaches raises or traps; add it when one does.
            other => Err(error(format!(
                "unsupported {module}:{name}/{arity} result {other:?}"
            ))),
        }
    }
}

impl BeamProgram {
    /// Load BEAM files into one program. Module names must be unique. The
    /// atom table starts with the 1150 static atoms. Each module, in the
    /// given order, puts its `AtU8` atoms and then its `LitT` atoms into it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let program = BeamProgram::load(&[CALLS, EXTERNAL])?;
    /// assert!(BeamProgram::load(&[CALLS, CALLS]).is_err());
    /// ```
    pub fn load(modules: &[&[u8]]) -> Result<Self, BeamError> {
        let mut atoms = AtomTable::new();
        let mut loaded = HashMap::new();
        for bytes in modules {
            let module = code::parse(bytes, &mut atoms)?;
            let name = module.name.clone();
            if loaded.insert(name.clone(), module).is_some() {
                return Err(error(format!("duplicate BEAM module {name}")));
            }
        }
        Ok(Self {
            modules: loaded,
            atoms,
        })
    }

    /// Return the atom of a name when the atom table of the program holds
    /// it. Use it to build terms that compare with the terms of the program.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let program = BeamProgram::load(&[ATOM_MAP_PROBE])?;
    /// assert!(program.atom("pbeam_zeta").is_some());
    /// assert_eq!(program.atom("pbeam_unknown"), None);
    /// ```
    pub fn atom(&self, name: &str) -> Option<Atom> {
        self.atoms.get(name)
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
        while scheduler.schedule_next().is_some() {
            match self.run(&mut scheduler)? {
                Outcome::Wait => scheduler.park_current(),
                Outcome::Exit { reason, result } => scheduler.finish_current(reason, result)?,
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
            next_instruction: function.start,
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

    /// Run the current process until it waits for a message or exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(program.run(&mut scheduler)?, Outcome::Wait));
    /// ```
    fn run(&self, scheduler: &mut Scheduler) -> Result<Outcome, BeamError> {
        loop {
            let process = scheduler.current_process_mut();
            let frame = current_frame(process);
            let module = self.module(&frame.module)?;
            let Some(instruction) = module.instructions.get(frame.next_instruction) else {
                return Err(error(format!(
                    "function {}:{} did not return",
                    frame.module, frame.function
                )));
            };
            frame.next_instruction += 1;
            match instruction {
                Instruction::Move(source, destination) => {
                    let value = read(process, source)?;
                    write(process, destination, value)?;
                }
                Instruction::Allocate { stack_size } => {
                    current_frame(process)
                        .stack
                        .resize_with(*stack_size as usize, || StackSlot::Empty);
                }
                Instruction::Deallocate { stack_size } => {
                    let stack = &mut current_frame(process).stack;
                    if stack.len() != *stack_size as usize {
                        return Err(error("invalid BEAM stack size"));
                    }
                    stack.clear();
                }
                Instruction::Trim { count, remaining } => {
                    let stack = &mut current_frame(process).stack;
                    if stack.len() != *count as usize + *remaining as usize {
                        return Err(error("invalid BEAM stack size"));
                    }
                    stack.drain(..*count as usize);
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
                Instruction::CallLast {
                    arity,
                    label,
                    deallocate,
                } => {
                    if *deallocate as usize != current_frame(process).stack.len() {
                        return Err(error("invalid BEAM tail-call stack size"));
                    }
                    call_arguments(&process.registers, *arity)?;
                    *current_frame(process) = self.frame(&module.name, *label, *arity)?;
                }
                Instruction::TestArity {
                    fail_label,
                    value,
                    arity,
                } => {
                    let passed = matches!(
                        &read(process, value)?,
                        Term::Tuple(elements) if elements.len() == *arity as usize
                    );
                    if !passed {
                        jump(module, process, *fail_label)?;
                    }
                }
                Instruction::SelectTupleArity {
                    value,
                    fail_label,
                    choices,
                } => {
                    let target = match &read(process, value)? {
                        Term::Tuple(elements) => choices
                            .iter()
                            .find(|(arity, _)| *arity as usize == elements.len())
                            .map_or(*fail_label, |(_, label)| *label),
                        _ => *fail_label,
                    };
                    jump(module, process, target)?;
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
                    if is_bif(module, name, *arity) {
                        let (target, arguments) =
                            match self.bif(scheduler, module, name, arguments)? {
                                BifResult::Value(value) => {
                                    let process = scheduler.current_process_mut();
                                    write(process, &Operand::X(0), value)?;
                                    if *tail && let Some(outcome) = return_from_frame(process)? {
                                        return Ok(outcome);
                                    }
                                    continue;
                                }
                                BifResult::Raise { class, reason } => {
                                    let process = scheduler.current_process_mut();
                                    if let Some(outcome) = raise(process, class, reason) {
                                        return Ok(outcome);
                                    }
                                    continue;
                                }
                                BifResult::Trap {
                                    module,
                                    function,
                                    arguments,
                                } => ((module, function), arguments),
                            };
                        let process = scheduler.current_process_mut();
                        let arity = u32::try_from(arguments.len())
                            .map_err(|_| error("arity exceeds BEAM limit"))?;
                        for (index, argument) in (0..).zip(arguments) {
                            write(process, &Operand::X(index), argument)?;
                        }
                        let frame = self.mfa_frame(target.0, target.1, arity)?;
                        if *tail {
                            *current_frame(process) = frame;
                        } else {
                            process.frames.push(frame);
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
                Instruction::Bif {
                    module,
                    name,
                    arguments,
                    destination,
                } => {
                    if !is_bif(module, name, arguments.len() as u32) {
                        return Err(error(format!(
                            "unsupported BIF {module}:{name}/{}",
                            arguments.len()
                        )));
                    }
                    let values = arguments
                        .iter()
                        .map(|argument| read(process, argument))
                        .collect::<Result<Vec<_>, _>>()?;
                    let result = self.bif(scheduler, module, name, values)?;
                    let value = result.into_bif_value(module, name, arguments.len())?;
                    write(scheduler.current_process_mut(), destination, value)?;
                }
                Instruction::Jump(label) => jump(module, process, *label)?,
                Instruction::Catch { register, label } => {
                    // ERTS also counts `c_p->catches` here. PBeam finds a live
                    // tag with a frame scan in `raise` and needs no counter.
                    let handler = label_index(module, *label)?;
                    let slot = current_frame(process)
                        .stack
                        .get_mut(*register as usize)
                        .ok_or_else(|| {
                            error(format!("register y{register} is outside allocated range"))
                        })?;
                    *slot = StackSlot::Catch(handler);
                }
                Instruction::TryEnd { register } => try_end(process, *register)?,
                Instruction::TryCase { register } => {
                    try_end(process, *register)?;
                    let class = read(process, &Operand::X(3))?;
                    write(process, &Operand::X(0), class)?;
                }
                Instruction::CatchEnd { register } => {
                    try_end(process, *register)?;
                    // An unset x0 is the ERTS non-value: an exception reached
                    // the handler. A normal return always sets x0.
                    if matches!(process.registers.first(), Some(None)) {
                        if read(process, &Operand::X(3))? == Term::static_atom("error") {
                            let reason = read(process, &Operand::X(1))?;
                            let stack = read(process, &Operand::X(2))?;
                            write(process, &Operand::X(1), Term::Tuple(vec![reason, stack]))?;
                        }
                        let reason = read(process, &Operand::X(1))?;
                        let caught = Term::Tuple(vec![Term::static_atom("EXIT"), reason]);
                        write(process, &Operand::X(0), caught)?;
                    }
                }
                Instruction::Test {
                    kind,
                    fail_label,
                    value,
                } => {
                    let value = read(process, value)?;
                    let passed = match kind {
                        TestKind::Atom => matches!(value, Term::Atom(_)),
                        TestKind::Pid => matches!(value, Term::Pid(_)),
                        TestKind::Reference => matches!(value, Term::Reference(_)),
                        TestKind::Tuple => matches!(value, Term::Tuple(_)),
                        TestKind::List => matches!(value, Term::List(_)),
                        TestKind::Binary => matches!(value, Term::Binary(_)),
                        TestKind::Map => matches!(value, Term::Map(_)),
                    };
                    if !passed {
                        jump(module, process, *fail_label)?;
                    }
                }
                Instruction::GetMapElements {
                    fail_label,
                    map,
                    pairs,
                } => {
                    let map = read(process, map)?;
                    let Term::Map(map) = &map else {
                        return Err(error("BEAM get_map_elements source is not a map"));
                    };
                    let mut found = Vec::with_capacity(pairs.len());
                    for (key, destination) in pairs {
                        let Some(value) = map.get(&read(process, key)?) else {
                            break;
                        };
                        found.push((destination, value.clone()));
                    }
                    if found.len() < pairs.len() {
                        jump(module, process, *fail_label)?;
                    } else {
                        for (destination, value) in found {
                            write(process, destination, value)?;
                        }
                    }
                }
                Instruction::PutMapAssoc {
                    map,
                    destination,
                    pairs,
                } => {
                    let mut map = read(process, map)?;
                    let Term::Map(entries) = &mut map else {
                        // ponytail: ERTS raises error {badmap, Map}; see "Exception raise sites" in docs/erts-differences.md.
                        return Err(error("unsupported BEAM put_map_assoc source: not a map"));
                    };
                    // PBeam inserts the pairs one by one. ERTS merges a flatmap source and builds a
                    // hashmap in one step above 32 keys. The result is the same, because the flatmap
                    // order and the HAMT shape depend only on the set of keys.
                    for (key, value) in pairs {
                        entries
                            .insert(read(process, key)?, read(process, value)?)
                            .map_err(|key| error(format!("unsupported map key {key:?}")))?;
                    }
                    write(process, destination, map)?;
                }
                Instruction::Swap(left, right) => {
                    let left_value = read(process, left)?;
                    let right_value = read(process, right)?;
                    write(process, left, right_value)?;
                    write(process, right, left_value)?;
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
                        &read(process, value)?,
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
                    let mut tail = read(process, tail)?;
                    let Term::List(values) = &mut tail else {
                        return Err(error("unsupported BEAM list tail"));
                    };
                    let mut values = std::mem::take(values);
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
                    let mut tuple = read(process, tuple)?;
                    let Term::Tuple(fields) = &mut tuple else {
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
                    scheduler.deliver(target, message.clone());
                    write(scheduler.current_process_mut(), &Operand::X(0), message)?;
                }
                Instruction::LoopReceive {
                    fail_label,
                    destination,
                } => match process.mailbox.current().cloned() {
                    Some(term) => write(process, destination, term)?,
                    None => jump(module, process, *fail_label)?,
                },
                Instruction::LoopReceiveEnd(label) => {
                    if !process.mailbox.save_next() {
                        return Err(error("loop_rec_end without a current message"));
                    }
                    jump(module, process, *label)?;
                }
                Instruction::RemoveMessage => {
                    if !process.mailbox.remove_current() {
                        return Err(error("remove_message without a current message"));
                    }
                }
                Instruction::Wait(label) => {
                    jump(module, process, *label)?;
                    return Ok(Outcome::Wait);
                }
                Instruction::Timeout => process.mailbox.set_save_first(),
                Instruction::ReceiveMarkerReserve(marker) => {
                    let id = process.mailbox.reserve_marker();
                    write(process, marker, id)?;
                }
                Instruction::ReceiveMarkerBind { marker, reference } => {
                    let (marker, reference) = (read(process, marker)?, read(process, reference)?);
                    process.mailbox.bind_marker(&marker, &reference);
                }
                Instruction::ReceiveMarkerUse(reference) => {
                    let reference = read(process, reference)?;
                    process.mailbox.use_marker(&reference);
                }
                Instruction::ReceiveMarkerClear(reference) => {
                    let reference = read(process, reference)?;
                    process.mailbox.clear_marker(&reference);
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

    /// Run a BIF that the observed process path calls. The result
    /// is a value, an exception, or a trap into BEAM code.
    ///
    /// BIFs are native functions listed in the ERTS `bif.tab`, mostly in the
    /// `erlang` module. The compiler emits guard BIFs such as `self/0` as
    /// `bif` instructions and other BIFs as `call_ext` instructions. Both
    /// forms come here.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = program.bif(&mut scheduler, "erlang", "make_ref", vec![])?;
    /// assert!(matches!(value, BifResult::Value(Term::Reference(_))));
    /// ```
    fn bif(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        name: &str,
        arguments: Vec<Term>,
    ) -> Result<BifResult, BeamError> {
        let value = match (module, name, arguments.as_slice()) {
            ("erlang", "process_flag", [Term::Atom(flag), Term::Atom(value)])
                if flag.name() == "trap_exit"
                    && (value.name() == "true" || value.name() == "false") =>
            {
                let process = scheduler.current_process_mut();
                let old = Term::static_atom(&process.trap_exit.to_string());
                process.trap_exit = value.name() == "true";
                old
            }
            ("erlang", "self", []) => Term::Pid(scheduler.current_process_mut().id),
            // PBeam is a local node that is not distributed, so every pid is
            // local and its node is `nonode@nohost`, as on ERTS (`dist.c`).
            ("erlang", "node", [] | [Term::Pid(_)]) => Term::static_atom("nonode@nohost"),
            ("erlang", "node", [argument]) => {
                return Err(error(format!(
                    "unsupported erlang:node/1 argument {argument:?}: not a pid"
                )));
            }
            ("erlang", "make_ref", []) => Term::Reference(scheduler.make_ref()?),
            ("erlang", "link", [Term::Pid(target)]) => scheduler.link(*target)?,
            ("erlang", "group_leader", []) => {
                Term::Pid(scheduler.current_process_mut().group_leader)
            }
            ("erlang", "group_leader", [Term::Pid(leader), Term::Pid(target)]) => {
                scheduler.set_group_leader(*target, *leader)?;
                Term::static_atom("true")
            }
            ("erlang", "monitor", [Term::Atom(kind), Term::Pid(target)])
                if kind.name() == "process" =>
            {
                Term::Reference(scheduler.monitor(*target)?)
            }
            ("erlang", "demonitor", [Term::Reference(reference), Term::List(options)]) => {
                let flush = match options.as_slice() {
                    [] => false,
                    [Term::Atom(option)] if option.name() == "flush" => true,
                    _ => return Err(error(format!("unsupported demonitor options {options:?}"))),
                };
                // ERTS `demonitor_2` (bif.c) flushes only when no active
                // monitor was removed: the DOWN message can then be queued.
                let removed = scheduler.demonitor(*reference);
                if removed || !flush {
                    Term::static_atom("true")
                } else {
                    let arguments = vec![
                        Term::Reference(*reference),
                        Term::static_atom("false"),
                        Term::static_atom("true"),
                    ];
                    let module = "erts_internal";
                    let function = "flush_monitor_messages";
                    return Ok(BifResult::Trap {
                        module,
                        function,
                        arguments,
                    });
                }
            }
            ("erlang", "exit", [reason]) => {
                return Ok(BifResult::Raise {
                    class: ExceptionClass::Exit,
                    reason: reason.clone(),
                });
            }
            ("erlang", "nif_error", [reason]) => {
                return Ok(BifResult::Raise {
                    class: ExceptionClass::Error,
                    reason: reason.clone(),
                });
            }
            (
                "erlang",
                "spawn" | "spawn_monitor" | "spawn_link",
                [Term::Atom(spawn_module), Term::Atom(function), list],
            ) => {
                let Term::List(arguments) = list else {
                    return Err(error("spawn arguments are not a proper list"));
                };
                let arguments = arguments.clone();
                let arity = u32::try_from(arguments.len())
                    .map_err(|_| error("arity exceeds BEAM limit"))?;
                let frame = self.mfa_frame(spawn_module.name(), function.name(), arity)?;
                let monitor = name == "spawn_monitor";
                let link = name == "spawn_link";
                let (pid, reference) = scheduler.spawn(frame, arguments, monitor, link)?;
                match reference {
                    Some(reference) => {
                        Term::Tuple(vec![Term::Pid(pid), Term::Reference(reference)])
                    }
                    None => Term::Pid(pid),
                }
            }
            ("maps", "keys", [Term::Map(map)]) => {
                Term::List(map.iter().map(|(key, _)| key.clone()).collect())
            }
            // ponytail: ERTS raises error {badmap, Map}; see "Exception raise sites" in docs/erts-differences.md.
            ("maps", "keys", [_]) => {
                return Err(error("unsupported maps:keys/1 argument: not a map"));
            }
            ("unicode", "characters_to_binary", [chardata, Term::Atom(encoding)])
                if encoding.name() == "unicode" =>
            {
                Term::Binary(characters_to_utf8(chardata)?)
            }
            // ponytail: ERTS also converts `utf8`, `latin1`, and other encodings; see "Chardata conversion" in docs/erts-differences.md.
            ("unicode", "characters_to_binary", [_, encoding]) => {
                return Err(error(format!(
                    "unsupported unicode:characters_to_binary/2 encoding {encoding:?}"
                )));
            }
            _ => {
                return Err(error(format!(
                    "unresolved external MFA {module}:{name}/{}",
                    arguments.len()
                )));
            }
        };
        Ok(BifResult::Value(value))
    }
}

/// The BIFs in modules other than `erlang`, as `(module, name, arity)`.
/// `erlang:is_builtin/3` returns `true` for each entry on the pinned ERTS
/// 29.1.1. ERTS binds each import of a BIF to the BIF when it loads a
/// module, so the BIF runs even when a loaded module exports the same MFA.
const BIFS: &[(&str, &str, u32)] = &[("maps", "keys", 1), ("unicode", "characters_to_binary", 2)];

/// Return whether an external call or a `bif` instruction goes to a BIF. An
/// entry of `BIFS` runs before the module code, as in ERTS. Every `erlang`
/// call goes to the BIF dispatch, which reports an unresolved MFA.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_bif("erlang", "self", 0));
/// assert!(is_bif("maps", "keys", 1));
/// assert!(!is_bif("maps", "keys", 2));
/// assert!(!is_bif("io", "put_chars", 2));
/// ```
fn is_bif(module: &str, name: &str, arity: u32) -> bool {
    module == "erlang" || BIFS.contains(&(module, name, arity))
}

/// Return the active frame of a running process.
///
/// # Examples
///
/// ```rust,ignore
/// current_frame(&mut process).next_instruction += 1;
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
        reason: Term::static_atom("normal"),
        result: Some(result),
    }))
}

/// Raise an exception in the process, as ERTS `handle_error` does: continue
/// at the nearest live catch tag, or return the exit outcome when no frame
/// holds one.
///
/// The scan starts at the newest frame and, in one frame, at the lowest Y
/// index, as ERTS `next_catch` scans up from the stack top. The frames above
/// the tag are dropped, and the tag becomes `[]`. The handler gets the
/// non-value in `x0`, the reason in `x1`, the stack trace `[]` in `x2`, and
/// the class atom in `x3`. An uncaught `error` exits the process with
/// `{Reason, []}`, and an uncaught `exit` exits it with `Reason`.
///
/// # Examples
///
/// ```rust,ignore
/// let outcome = raise(&mut process, ExceptionClass::Exit, Term::static_atom("badarg"));
/// assert!(matches!(outcome, Some(Outcome::Exit { result: None, .. })));
/// ```
fn raise(process: &mut ProcessState, class: ExceptionClass, reason: Term) -> Option<Outcome> {
    let tag = process
        .frames
        .iter()
        .enumerate()
        .rev()
        .find_map(|(depth, frame)| {
            frame
                .stack
                .iter()
                .enumerate()
                .find_map(|(slot, value)| match value {
                    StackSlot::Catch(handler) => Some((depth, slot, *handler)),
                    _ => None,
                })
        });
    let Some((depth, slot, handler)) = tag else {
        let reason = match class {
            ExceptionClass::Error => Term::Tuple(vec![reason, Term::List(Vec::new())]),
            ExceptionClass::Exit => reason,
        };
        return Some(Outcome::Exit {
            reason,
            result: None,
        });
    };
    process.frames.truncate(depth + 1);
    let frame = current_frame(process);
    frame.stack[slot] = StackSlot::Term(Term::List(Vec::new()));
    frame.next_instruction = handler;
    process
        .registers
        .resize(process.registers.len().max(4), None);
    process.registers[0] = None;
    process.registers[1] = Some(reason);
    process.registers[2] = Some(Term::List(Vec::new()));
    process.registers[3] = Some(Term::static_atom(class.name()));
    None
}

/// Clear the catch tag in a Y register of the active frame. The register
/// becomes `[]`, as ERTS `make_blank` writes NIL.
///
/// # Examples
///
/// ```rust,ignore
/// try_end(&mut process, 0)?;
/// assert_eq!(read(&process, &Operand::Y(0))?, Term::List(Vec::new()));
/// ```
fn try_end(process: &mut ProcessState, register: u32) -> Result<(), BeamError> {
    write(process, &Operand::Y(register), Term::List(Vec::new()))
}

/// Continue execution after a label in the active module.
///
/// # Examples
///
/// ```rust,ignore
/// jump(module, &mut process, 3)?;
/// ```
fn jump(module: &BeamModule, process: &mut ProcessState, label: u32) -> Result<(), BeamError> {
    current_frame(process).next_instruction = label_index(module, label)?;
    Ok(())
}

/// Return the index of the first instruction after a label in a module.
///
/// # Examples
///
/// ```rust,ignore
/// // [FunctionInfo, Label(1), Label(2), Return]
/// assert_eq!(label_index(&module, 2)?, 3);
/// assert!(label_index(&module, 9).is_err());
/// ```
fn label_index(module: &BeamModule, label: u32) -> Result<usize, BeamError> {
    module
        .instructions
        .iter()
        .position(|instruction| matches!(instruction, Instruction::Label(found) if *found == label))
        .map(|index| index + 1)
        .ok_or_else(|| error(format!("missing BEAM label {label}")))
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

/// Find the function whose entry label is `target`. The function starts
/// after the label.
///
/// # Examples
///
/// ```rust,ignore
/// let function = function_at_label(&module, 2)?;
/// assert_eq!((function.name, function.arity), ("main", 0));
/// ```
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

/// Copy the first `arity` X registers as call arguments. Each register
/// must be set.
///
/// # Examples
///
/// ```rust,ignore
/// let registers = vec![Some(Term::Integer(1))];
/// assert_eq!(call_arguments(&registers, 1)?, vec![Term::Integer(1)]);
/// assert!(call_arguments(&registers, 2).is_err());
/// ```
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

/// Read a literal, an X register, or a Y register.
///
/// # Examples
///
/// ```rust,ignore
/// let registers = vec![Some(Term::Integer(7))];
/// assert_eq!(read_operand(&Operand::X(0), &registers, &[])?, Term::Integer(7));
/// assert!(read_operand(&Operand::Y(0), &registers, &[]).is_err());
/// ```
fn read_operand(
    operand: &Operand,
    registers: &[Option<Term>],
    stack: &[StackSlot],
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
        Operand::Y(index) => match stack.get(*index as usize) {
            Some(StackSlot::Term(value)) => Ok(value.clone()),
            // The BEAM validator never lets an instruction read a catch tag.
            Some(StackSlot::Catch(_)) => {
                Err(error(format!("stack register y{index} holds a catch tag")))
            }
            Some(StackSlot::Empty) | None => {
                Err(error(format!("stack register y{index} is unset")))
            }
        },
    }
}

/// Write a value to an X register or an allocated Y register. X registers
/// grow as necessary. A literal destination is an error.
///
/// # Examples
///
/// ```rust,ignore
/// let mut registers = vec![];
/// write_operand(&Operand::X(1), Term::Integer(7), &mut registers, &mut [])?;
/// assert_eq!(registers, vec![None, Some(Term::Integer(7))]);
/// ```
fn write_operand(
    operand: &Operand,
    value: Term,
    registers: &mut Vec<Option<Term>>,
    stack: &mut [StackSlot],
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
            stack[index] = StackSlot::Term(value);
        }
        Operand::Value(_) => return Err(error("invalid BEAM move destination")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, process::Command};

    use super::{
        BeamProgram, BifResult, ExceptionClass, call_arguments, function_at_label, read_operand,
        write_operand,
    };
    use crate::{
        BeamModule, Map, Term,
        beam::code::{self, Instruction, Operand},
        term::{AtomTable, test_atom},
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
            atoms: AtomTable::new(),
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
        Operand::Value(test_atom(name))
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
        assert!(
            error(vec![
                Instruction::Move(atom("undef"), Operand::X(0)),
                erlang_call("nif_error", 1, false),
            ])
            .contains("root process exited with Tuple([Atom(\"undef\"), List([])])")
        );
        let mut body = spawn_link_child(Term::List(vec![Term::Integer(1)]));
        body.extend(wait_forever());
        assert!(error(body).contains("unsupported exit signal"));
        let mut body = spawn_link_child(Term::List(vec![test_atom("normal")]));
        body.extend(wait_forever());
        assert!(error(body).contains("waits for a message that cannot arrive"));
        let mut body = spawn_link_child(Term::List(vec![Term::Integer(1)]));
        body.push(Instruction::Return);
        assert!(matches!(run_with_child(body), Ok(Term::Pid(_))));
        assert!(error(spawn_link_child(Term::Integer(1))).contains("not a proper list"));
        assert!(error(vec![erlang_call("is_alive", 0, false)]).contains("erlang:is_alive/0"));
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
            Term::Tuple(vec![test_atom("a")])
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
        assert!(error(vec![Instruction::LoopReceiveEnd(1)]).contains("loop_rec_end"));
        assert!(error(vec![Instruction::RemoveMessage]).contains("remove_message"));
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
            error(vec![Instruction::Bif {
                module: "erlang".into(),
                name: "is_alive".into(),
                arguments: Vec::new(),
                destination: Operand::X(0),
            }])
            .contains("unresolved external MFA erlang:is_alive/0")
        );
        assert!(
            error(vec![Instruction::Bif {
                module: "lists".into(),
                name: "node".into(),
                arguments: Vec::new(),
                destination: Operand::X(0),
            }])
            .contains("unsupported BIF lists:node/0")
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
                Instruction::Move(Operand::Value(test_atom("pid")), Operand::X(0)),
                Instruction::Return,
                Instruction::Label(3),
                Instruction::Move(Operand::Value(test_atom("other")), Operand::X(0)),
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
            test_atom("pid")
        );
        assert_eq!(
            program
                .execute("fixture", "main", &[Term::Integer(1)])
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
                    Operand::Value(Term::Tuple(vec![])),
                    Operand::Value(Term::Integer(1))
                )],
            }])
            .contains("unsupported map key Tuple([])")
        );
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
                        tail: true,
                        deallocate: 0,
                    },
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
            Term::List(vec![Term::Integer(1), Term::Integer(2), a])
        );
        // ERTS 29.1.1: maps:keys(#{true => V, abs_path => V, 1 => V}) is [1,true,abs_path].
        assert_eq!(
            keys(map(vec![atom("abs_path"), atom("true"), Term::Integer(1)])).unwrap(),
            Term::List(vec![Term::Integer(1), atom("true"), atom("abs_path")])
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
                    tail: true,
                    deallocate: 0,
                },
            ],
            0,
        );
        program.modules.insert(
            "maps".into(),
            BeamModule {
                name: "maps".into(),
                exports: HashMap::from([(("keys".into(), 1), 1)]),
                instructions: vec![
                    Instruction::FunctionInfo {
                        module: "maps".into(),
                        name: "keys".into(),
                        arity: 1,
                    },
                    Instruction::Label(1),
                    Instruction::Move(atom("module_code"), Operand::X(0)),
                    Instruction::Return,
                ],
            },
        );
        // The BIF returns the keys of the empty map. The module code would
        // return `module_code`.
        assert_eq!(
            program.execute("fixture", "main", &[]).unwrap(),
            Term::List(Vec::new())
        );
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
                        tail: true,
                        deallocate: 0,
                    },
                ],
                0,
            )
            .execute("fixture", "main", &[])
        };
        let chardata = || Term::List(vec![Term::Binary(b"Hello".to_vec()), Term::Integer(10)]);
        assert_eq!(
            convert(chardata(), "unicode").unwrap(),
            Term::Binary(b"Hello\n".to_vec())
        );
        assert_eq!(
            convert(Term::Integer(1), "unicode")
                .unwrap_err()
                .to_string(),
            "unsupported chardata Integer(1): not a binary or a list"
        );
        assert_eq!(
            convert(chardata(), "utf8").unwrap_err().to_string(),
            "unsupported unicode:characters_to_binary/2 encoding Atom(\"utf8\")"
        );
    }

    #[test]
    fn builds_a_list_from_nil_and_rejects_an_unsupported_tail() {
        let list = make_program(
            vec![
                Instruction::PutList {
                    head: Operand::Value(Term::Integer(10)),
                    tail: Operand::Value(Term::List(Vec::new())),
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
        // One atom table for the three modules, as `BeamProgram::load` makes it.
        let mut atoms = AtomTable::new();
        let mut load = |bytes: &[u8]| code::parse(bytes, &mut atoms).unwrap();
        let mut modules = HashMap::from([
            ("Elixir.Hello".into(), load(&hello)),
            ("Elixir.HelloVariant".into(), load(&variant)),
            ("Elixir.IO".into(), load(&io)),
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
        let program = BeamProgram { modules, atoms };

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
            atoms: AtomTable::new(),
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

    #[test]
    fn selects_tuple_arity_and_rejects_call_last_stack_size_and_demonitor_options() {
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
            select(Term::Tuple(vec![Term::Integer(1)])),
            test_atom("one")
        );
        assert_eq!(select(Term::Tuple(vec![])), test_atom("other"));
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
        assert!(
            error(vec![
                erlang_call("make_ref", 0, false),
                Instruction::Move(
                    Operand::Value(Term::List(vec![test_atom("info")])),
                    Operand::X(1),
                ),
                erlang_call("demonitor", 2, false),
            ])
            .contains("unsupported demonitor options")
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
            Term::Tuple(vec![
                Term::static_atom("EXIT"),
                Term::Tuple(vec![Term::static_atom("undef"), Term::List(Vec::new())]),
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
                .contains("root process exited with Tuple([Atom(\"undef\"), List([])])")
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
    fn rejects_an_exception_from_a_bif_instruction() {
        assert_eq!(
            BifResult::Value(Term::Integer(1))
                .into_bif_value("erlang", "self", 0)
                .unwrap(),
            Term::Integer(1)
        );
        assert!(
            BifResult::Raise {
                class: ExceptionClass::Error,
                reason: test_atom("badarg"),
            }
            .into_bif_value("erlang", "node", 1)
            .unwrap_err()
            .to_string()
            .contains("unsupported erlang:node/1 result")
        );
    }

    #[test]
    fn returns_the_local_node_for_node_0_and_a_pid() {
        let node = |arguments: Vec<Operand>| {
            make_program(
                vec![
                    Instruction::Bif {
                        module: "erlang".into(),
                        name: "self".into(),
                        arguments: Vec::new(),
                        destination: Operand::X(1),
                    },
                    Instruction::Bif {
                        module: "erlang".into(),
                        name: "node".into(),
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
            node(Vec::new()).unwrap(),
            Term::static_atom("nonode@nohost")
        );
        assert_eq!(
            node(vec![Operand::X(1)]).unwrap(),
            Term::static_atom("nonode@nohost")
        );
        assert_eq!(
            node(vec![atom("foo")]).unwrap_err().to_string(),
            "unsupported erlang:node/1 argument Atom(\"foo\"): not a pid"
        );
    }

    #[test]
    fn traps_a_tail_called_demonitor_flush_into_erts_internal() {
        let mut program = make_program(
            vec![
                erlang_call("make_ref", 0, false),
                Instruction::Move(
                    Operand::Value(Term::List(vec![test_atom("flush")])),
                    Operand::X(1),
                ),
                erlang_call("demonitor", 2, true),
            ],
            0,
        );
        program.modules.insert(
            "erts_internal".into(),
            BeamModule {
                name: "erts_internal".into(),
                exports: HashMap::from([(("flush_monitor_messages".into(), 3), 2)]),
                instructions: vec![
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
            },
        );
        assert_eq!(
            program.execute("fixture", "main", &[]).unwrap(),
            test_atom("true")
        );
    }

    #[test]
    fn removes_an_active_monitor_without_a_trap() {
        // monitor(process, self()), then demonitor(Ref, [flush]): the monitor
        // is still active, so ERTS returns true without flushing.
        let program = make_program(
            vec![
                Instruction::Bif {
                    module: "erlang".into(),
                    name: "self".into(),
                    arguments: Vec::new(),
                    destination: Operand::X(1),
                },
                Instruction::Move(atom("process"), Operand::X(0)),
                erlang_call("monitor", 2, false),
                Instruction::Move(
                    Operand::Value(Term::List(vec![test_atom("flush")])),
                    Operand::X(1),
                ),
                erlang_call("demonitor", 2, true),
            ],
            0,
        );
        assert_eq!(
            program.execute("fixture", "main", &[]).unwrap(),
            test_atom("true")
        );
    }
}
