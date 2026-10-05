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
    term::{Atom, AtomTable, Fun, Term},
};
use std::collections::HashMap;

mod bif;
mod native;

use bif::{BifResult, INSTRUCTION_CALLS, is_bif, is_function};

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
                Instruction::TestFunctionArity {
                    fail_label,
                    value,
                    arity,
                } => {
                    let result = is_function(&read(process, value)?, &read(process, arity)?);
                    // The `badarg` of the check fails the test and raises
                    // nothing.
                    let passed = matches!(
                        result,
                        BifResult::Value(value) if value == Term::static_atom("true")
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
                    if let Some(outcome) =
                        self.call_external(scheduler, module, name, *arity, *tail, *deallocate)?
                    {
                        return Ok(outcome);
                    }
                }
                Instruction::Apply {
                    arity,
                    tail,
                    deallocate,
                } => {
                    let module = read(process, &Operand::X(*arity))?;
                    let name = read(process, &Operand::X(arity.saturating_add(1)))?;
                    // ERTS `fixed_apply` (`beam_common.c`) raises `badarg`
                    // when the module or the function name is not an atom.
                    let (Term::Atom(module), Term::Atom(name)) = (&module, &name) else {
                        if let Some(outcome) =
                            raise(process, ExceptionClass::Error, Term::static_atom("badarg"))
                        {
                            return Ok(outcome);
                        }
                        continue;
                    };
                    if let Some(outcome) = self.call_external(
                        scheduler,
                        module.name(),
                        name.name(),
                        *arity,
                        *tail,
                        *deallocate,
                    )? {
                        return Ok(outcome);
                    }
                }
                Instruction::MakeFun {
                    lambda,
                    free,
                    destination,
                } => {
                    let free = free
                        .iter()
                        .map(|operand| read(process, operand))
                        .collect::<Result<_, _>>()?;
                    let fun = Term::Fun(Fun::new(lambda.clone(), free));
                    write(process, destination, fun)?;
                }
                Instruction::CallFun { arity, function } => {
                    let fun = read(process, function)?;
                    if let Some(outcome) = self.call_fun(process, &fun, *arity)? {
                        return Ok(outcome);
                    }
                }
                Instruction::Bif {
                    fail_label,
                    module: bif_module,
                    name,
                    arguments,
                    destination,
                } => {
                    if !is_bif(bif_module, name, arguments.len() as u32) {
                        return Err(error(format!(
                            "unsupported BIF {bif_module}:{name}/{}",
                            arguments.len()
                        )));
                    }
                    let values = arguments
                        .iter()
                        .map(|argument| read(process, argument))
                        .collect::<Result<Vec<_>, _>>()?;
                    // A `bif` instruction with fail label 0 raises the
                    // exception of the BIF, as `handle_error` does in ERTS
                    // (`bif_instrs.tab`). Another fail label jumps to the
                    // label and raises nothing (`$FAIL($Fail)` in
                    // `i_bif.call`).
                    let result = self.bif(scheduler, bif_module, name, values)?;
                    let process = scheduler.current_process_mut();
                    match result {
                        BifResult::Value(value) => write(process, destination, value)?,
                        BifResult::Raise { .. } if *fail_label != 0 => {
                            jump(module, process, *fail_label)?
                        }
                        BifResult::Raise { class, reason } => {
                            if let Some(outcome) = raise(process, class, reason) {
                                return Ok(outcome);
                            }
                        }
                        // The compiler emits a `bif` instruction only for a
                        // BIF that does not trap.
                        trap => {
                            return Err(error(format!(
                                "unsupported {bif_module}:{name}/{} result {trap:?}",
                                arguments.len()
                            )));
                        }
                    }
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
                        TestKind::Nil => matches!(&value, Term::List(values) if values.is_empty()),
                        TestKind::NonEmptyList => {
                            matches!(&value, Term::List(values) if !values.is_empty())
                        }
                        TestKind::Binary => matches!(value, Term::Binary(_)),
                        TestKind::Map => matches!(value, Term::Map(_)),
                        TestKind::Function => matches!(value, Term::Fun(_)),
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
                Instruction::TestExactNotEqual {
                    fail_label,
                    left,
                    right,
                } => {
                    if read(process, left)? == read(process, right)? {
                        jump(module, process, *fail_label)?;
                    }
                }
                Instruction::TestGreaterEqual {
                    fail_label,
                    left,
                    right,
                } => {
                    if read(process, left)?
                        .compare(&read(process, right)?)?
                        .is_lt()
                    {
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
                Instruction::UpdateRecord {
                    source,
                    destination,
                    updates,
                } => {
                    let mut record = read(process, source)?;
                    let Term::Tuple(fields) = &mut record else {
                        return Err(error("BEAM update_record source is not a tuple"));
                    };
                    // ERTS reads each value before it writes the destination, which can be a value register.
                    for (position, value) in updates {
                        let value = read(process, value)?;
                        let field = (*position as usize)
                            .checked_sub(1)
                            .and_then(|index| fields.get_mut(index))
                            .ok_or_else(|| {
                                error(format!("BEAM update_record has no position {position}"))
                            })?;
                        *field = value;
                    }
                    write(process, destination, record)?;
                }
                Instruction::GetList { source, head, tail } => {
                    let mut values = nonempty_list(read(process, source)?)?;
                    // ponytail: the tail is a copy of the rest, O(n) with the `Vec` list model of #71.
                    let first = values.remove(0);
                    write(process, head, first)?;
                    write(process, tail, Term::List(values))?;
                }
                Instruction::GetHead {
                    source,
                    destination,
                } => {
                    let mut values = nonempty_list(read(process, source)?)?;
                    write(process, destination, values.swap_remove(0))?;
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
                Instruction::WaitTimeout { label, source } => {
                    let timeout = read(process, source)?;
                    if timeout != Term::static_atom("infinity") {
                        // ponytail: PBeam has no timers; see "Receive timeout" in docs/erts-differences.md.
                        return Err(error(format!("unsupported receive timeout {timeout:?}")));
                    }
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
                Instruction::NifStart { name, arity } => {
                    // ERTS `erlang:load_nif/2` writes the NIF call over this
                    // instruction, so each entry to the function runs the
                    // NIF and returns its result to the caller
                    // (`beam/erl_nif.c:4922-4928`, `5050-5098`). A function
                    // without a binding runs its BEAM stub body.
                    let arguments = call_arguments(&process.registers, *arity)?;
                    if let Some(result) = native::call(&module.name, name, &arguments)
                        && let Some(outcome) = self.finish_bif(process, result, true)?
                    {
                        return Ok(outcome);
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

    /// Call `module:name/arity` with the arguments in the X registers, as
    /// `call_ext` and `apply` do. A tail call first checks that the frame
    /// has `deallocate` Y registers, then replaces the frame. Return the
    /// outcome when the process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `erlang:exit/1` in a tail call with no catch tag exits the process.
    /// let outcome = program.call_external(&mut scheduler, "erlang", "exit", 1, true, 0)?;
    /// assert!(matches!(outcome, Some(Outcome::Exit { result: None, .. })));
    /// // A tail call from a frame with one Y register must deallocate it.
    /// assert!(program.call_external(&mut scheduler, "erlang", "self", 0, true, 0).is_err());
    /// ```
    fn call_external(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        name: &str,
        arity: u32,
        tail: bool,
        deallocate: u32,
    ) -> Result<Option<Outcome>, BeamError> {
        let process = scheduler.current_process_mut();
        if tail && deallocate as usize != current_frame(process).stack.len() {
            return Err(error("invalid BEAM tail-call stack size"));
        }
        self.call_mfa(scheduler, module, name, arity, tail)
    }

    /// Call `module:name/arity` with the arguments in the X registers. The
    /// call of `erlang:apply/3` runs as the ERTS `i_apply` instruction. A BIF
    /// runs before the module code, as in ERTS. Return the outcome when the
    /// process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `erlang:apply(erlang, self, [])` runs the `self/0` BIF.
    /// let outcome = program.call_mfa(&mut scheduler, "erlang", "apply", 3, false)?;
    /// assert!(outcome.is_none());
    /// // `erlang:apply/2` runs as an instruction in ERTS.
    /// assert!(program.call_mfa(&mut scheduler, "erlang", "apply", 2, false).is_err());
    /// ```
    fn call_mfa(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        name: &str,
        arity: u32,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        if (module, name, arity) == ("erlang", "apply", 3) {
            return self.apply_mfa(scheduler, tail);
        }
        if INSTRUCTION_CALLS.contains(&(module, name, arity)) {
            // ponytail: ERTS runs these calls as instructions; see "Instruction calls" in docs/erts-differences.md.
            return Err(error(format!(
                "unsupported instruction call {module}:{name}/{arity}"
            )));
        }
        let process = scheduler.current_process_mut();
        if is_bif(module, name, arity) {
            let arguments = call_arguments(&process.registers, arity)?;
            let result = self.bif(scheduler, module, name, arguments)?;
            return self.finish_bif(scheduler.current_process_mut(), result, tail);
        }
        enter_frame(process, self.mfa_frame(module, name, arity)?, tail);
        Ok(None)
    }

    /// Run `erlang:apply(Module, Function, Arguments)` with the arguments in
    /// `x0` to `x2`, as ERTS `apply` (`beam_common.c:1434-1526`) does for the
    /// `i_apply` instructions. The function and the module must be atoms,
    /// and `Arguments` must be a list, else the call raises `error:badarg`.
    /// An apply of `erlang:apply/3` continues with its inner call. A list of
    /// 1024 or more elements raises `error:system_limit`. Then the elements
    /// go to `x0` onwards, and the call continues as `call_ext` does. Return
    /// the outcome when the process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // x0 = erlang, x1 = apply, x2 = [fixture, id, [w]] calls fixture:id(w).
    /// assert!(program.apply_mfa(&mut scheduler, false)?.is_none());
    /// // x0 = 1 raises `error:badarg`.
    /// assert!(matches!(program.apply_mfa(&mut scheduler, true)?, Some(Outcome::Exit { .. })));
    /// ```
    fn apply_mfa(
        &self,
        scheduler: &mut Scheduler,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        /// ERTS `apply` copies at most `MAX_REG - 1` arguments to the X
        /// registers (`erl_vm.h:55`, `beam_common.c:1501-1513`).
        const MAX_ARGUMENTS: usize = 1023;
        let process = scheduler.current_process_mut();
        let mut call = call_arguments(&process.registers, 3)?;
        let (module, name, arguments) = loop {
            // Each wrong form raises the same `badarg`, so one pattern checks
            // the function, the module, and the list.
            let [Term::Atom(module), Term::Atom(name), Term::List(arguments)] = &mut call[..]
            else {
                let badarg = Term::static_atom("badarg");
                return Ok(raise(process, ExceptionClass::Error, badarg));
            };
            let arguments = std::mem::take(arguments);
            if (module.name(), name.name()) != ("erlang", "apply") || arguments.len() != 3 {
                break (module.clone(), name.clone(), arguments);
            }
            call = arguments;
        };
        if arguments.len() > MAX_ARGUMENTS {
            let system_limit = Term::static_atom("system_limit");
            return Ok(raise(process, ExceptionClass::Error, system_limit));
        }
        let arity = arguments.len() as u32;
        for (index, argument) in (0..).zip(arguments) {
            write(process, &Operand::X(index), argument)?;
        }
        self.call_mfa(scheduler, module.name(), name.name(), arity, tail)
    }

    /// Call `fun` with the arguments in `x0` to `x(arity - 1)`, as ERTS
    /// `call_fun` (`beam_common.c:1605-1731`) does for `call_fun` and
    /// `call_fun2`. A term that is not a fun raises `error:{badfun, Term}`. A
    /// fun of another arity raises `error:{badarity, {Fun, Arguments}}`,
    /// where `Arguments` lists `x0` to `x(arity - 1)`. Else the free
    /// variables go to `x(arity)` onwards, and the call enters the code of
    /// the lambda. Return the outcome when the process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // x0 = one and fun(X) -> {Tag, X} end enters the lambda with x1 = Tag.
    /// assert!(program.call_fun(process, &pair, 1)?.is_none());
    /// // An atom raises `error:{badfun, notfun}`.
    /// assert!(program.call_fun(process, &Term::static_atom("notfun"), 0)?.is_some());
    /// ```
    fn call_fun(
        &self,
        process: &mut ProcessState,
        fun: &Term,
        arity: u32,
    ) -> Result<Option<Outcome>, BeamError> {
        let fun = match fun {
            Term::Fun(local) if local.lambda().arity() == arity => local,
            Term::Fun(_) => {
                let arguments = call_arguments(&process.registers, arity)?;
                let call = Term::Tuple(vec![fun.clone(), Term::List(arguments)]);
                let reason = Term::Tuple(vec![Term::static_atom("badarity"), call]);
                return Ok(raise(process, ExceptionClass::Error, reason));
            }
            _ => {
                let reason = Term::Tuple(vec![Term::static_atom("badfun"), fun.clone()]);
                return Ok(raise(process, ExceptionClass::Error, reason));
            }
        };
        for (register, value) in (arity..).zip(fun.free()) {
            write(process, &Operand::X(register), value.clone())?;
        }
        let lambda = fun.lambda();
        let total_arity = arity + fun.free().len() as u32;
        let frame = self.frame(lambda.module().name(), lambda.label(), total_arity)?;
        enter_frame(process, frame, false);
        Ok(None)
    }

    /// Finish a BIF or NIF call with its result, as the ERTS `nif_bif.epilogue`
    /// does for BIFs and NIFs (`emu/bif_instrs.tab:604-636`; JIT
    /// `emit_bif_nif_epilogue` in `jit/arm/instr_bif.cpp:631-719`). A value
    /// goes to x0, and a tail call then returns from the frame. An exception
    /// goes to the error handler. A trap writes its arguments to the X
    /// registers and enters the target function. Return the outcome when the
    /// process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // A value in a body call stays in the frame.
    /// let outcome = program.finish_bif(process, BifResult::Value(Term::static_atom("ok")), false)?;
    /// assert!(outcome.is_none());
    /// // `exit(normal)` with no catch tag exits the process.
    /// let raise = BifResult::Raise { class: ExceptionClass::Exit, reason: Term::static_atom("normal") };
    /// assert!(matches!(program.finish_bif(process, raise, true)?, Some(Outcome::Exit { .. })));
    /// ```
    fn finish_bif(
        &self,
        process: &mut ProcessState,
        result: BifResult,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        match result {
            BifResult::Value(value) => {
                write(process, &Operand::X(0), value)?;
                if tail {
                    return_from_frame(process)
                } else {
                    Ok(None)
                }
            }
            BifResult::Raise { class, reason } => Ok(raise(process, class, reason)),
            BifResult::Trap {
                module,
                function,
                arguments,
            } => {
                let arity = u32::try_from(arguments.len())
                    .map_err(|_| error("arity exceeds BEAM limit"))?;
                for (index, argument) in (0..).zip(arguments) {
                    write(process, &Operand::X(index), argument)?;
                }
                enter_frame(process, self.mfa_frame(module, function, arity)?, tail);
                Ok(None)
            }
        }
    }
}

/// Enter `frame`: a tail call replaces the active frame, and a body call
/// pushes it.
///
/// # Examples
///
/// ```rust,ignore
/// enter_frame(&mut process, program.mfa_frame("external", "effect", 1)?, false);
/// assert_eq!(process.frames.len(), 2);
/// ```
fn enter_frame(process: &mut ProcessState, frame: Frame, tail: bool) {
    if tail {
        *current_frame(process) = frame;
    } else {
        process.frames.push(frame);
    }
}

/// The spawn options that this runtime supports, as the flags of ERTS
/// `ErlSpawnOpts`.
#[derive(Default)]
struct SpawnOptions {
    /// Link the new process to the caller (`SPO_LINK`).
    link: bool,
    /// Monitor the new process and return `{Pid, Reference}` (`SPO_MONITOR`).
    monitor: bool,
}

/// Start a process that runs `module:function` with the elements of
/// `arguments`, as ERTS `erl_create_process` does. Return the pid, or
/// `{Pid, Reference}` when `options.monitor` is set.
///
/// # Examples
///
/// ```rust,ignore
/// let result = create_process(&program, &mut scheduler, &module, &function, &Term::List(vec![]), SpawnOptions::default())?;
/// assert!(matches!(result, Term::Pid(_)));
/// assert!(create_process(&program, &mut scheduler, &module, &function, &Term::Integer(1), SpawnOptions::default()).is_err());
/// ```
fn create_process(
    program: &BeamProgram,
    scheduler: &mut Scheduler,
    module: &Atom,
    function: &Atom,
    arguments: &Term,
    options: SpawnOptions,
) -> Result<Term, BeamError> {
    let Term::List(arguments) = arguments else {
        return Err(error("spawn arguments are not a proper list"));
    };
    let arguments = arguments.clone();
    let arity = u32::try_from(arguments.len()).map_err(|_| error("arity exceeds BEAM limit"))?;
    let frame = program.mfa_frame(module.name(), function.name(), arity)?;
    let (pid, reference) = scheduler.spawn(frame, arguments, options.monitor, options.link)?;
    Ok(match reference {
        Some(reference) => Term::Tuple(vec![Term::Pid(pid), Term::Reference(reference)]),
        None => Term::Pid(pid),
    })
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

/// Return the elements of a nonempty list, which `get_list` and `get_hd`
/// read. The BEAM validator makes sure that the source is a nonempty list.
///
/// # Examples
///
/// ```rust,ignore
/// let list = Term::List(vec![Term::Integer(1), Term::Integer(2)]);
/// assert_eq!(nonempty_list(list)?, vec![Term::Integer(1), Term::Integer(2)]);
/// assert!(nonempty_list(Term::List(vec![])).is_err());
/// ```
fn nonempty_list(mut list: Term) -> Result<Vec<Term>, BeamError> {
    match &mut list {
        Term::List(values) if !values.is_empty() => Ok(std::mem::take(values)),
        _ => Err(error(format!(
            "BEAM list source {list:?} is not a nonempty list"
        ))),
    }
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

    use super::bif::{BIFS, INSTRUCTION_CALLS};
    use super::{BeamProgram, call_arguments, function_at_label, read_operand, write_operand};
    use crate::{
        BeamModule, Map, Term,
        beam::code::{self, Instruction, Operand, TestKind},
        term::{Atom, AtomTable, Lambda, test_atom},
    };

    /// The runtime error of a root process that exits with an uncaught
    /// `error:badarg`. PBeam gives the stack trace `[]`.
    const BADARG_EXIT: &str = "root process exited with Tuple([Atom(\"badarg\"), List([])])";

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
        assert_eq!(run(Term::Integer(1)), test_atom("pass"));
        for arity in [Term::Integer(0), Term::Integer(-1), test_atom("a")] {
            assert_eq!(run(arity), test_atom("fail"));
        }
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
            "root process exited with Tuple([Tuple([Atom(\"badfun\"), Atom(\"notfun\")]), List([])])"
        );
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
        // `erlang:is_alive/0` is not a BIF: it runs the code of the `erlang`
        // module, which this program does not load.
        assert!(
            error(vec![erlang_call("is_alive", 0, false)])
                .contains("unresolved external MFA erlang:is_alive/0")
        );
        // The ERTS loader turns these calls into instructions. Their bodies in
        // `erlang.beam` call themselves, so they must not run.
        assert!(
            error(vec![erlang_call("apply", 2, false)])
                .contains("unsupported instruction call erlang:apply/2")
        );
        assert!(
            error(vec![erlang_call("hibernate", 0, false)])
                .contains("unsupported instruction call erlang:hibernate/0")
        );
    }

    #[test]
    fn spawns_with_link_and_monitor_options_and_rejects_other_options() {
        let spawn_opt_body = |reason: Term, options: Vec<Term>| {
            let mut body = spawn_link_child(Term::List(vec![reason]));
            body.pop();
            body.push(Instruction::Move(
                Operand::Value(Term::List(options)),
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
        assert!(
            wait_for_abnormal_child(vec![test_atom("link")])
                .unwrap_err()
                .to_string()
                .contains("unsupported exit signal")
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
                if matches!(elements.as_slice(), [Term::Pid(_), Term::Reference(_)])
        ));
        assert!(matches!(spawn_opt(vec![]), Ok(Term::Pid(_))));
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
                fail_label: 0,
                module: "erlang".into(),
                name: "date".into(),
                arguments: Vec::new(),
                destination: Operand::X(0),
            }])
            .contains("unresolved external MFA erlang:date/0")
        );
        assert!(
            error(vec![Instruction::Bif {
                fail_label: 0,
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
    fn never_runs_the_stub_of_a_table_bif_that_pbeam_does_not_implement() {
        let mut program = make_program(
            vec![
                Instruction::Move(atom("a"), Operand::X(0)),
                Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(1)),
                Instruction::Move(Operand::Value(Term::List(Vec::new())), Operand::X(2)),
                Instruction::CallExternal {
                    arity: 3,
                    module: "lists".into(),
                    name: "keymember".into(),
                    tail: true,
                    deallocate: 0,
                },
            ],
            0,
        );
        program.modules.insert(
            "lists".into(),
            BeamModule {
                name: "lists".into(),
                exports: HashMap::from([(("keymember".into(), 3), 1)]),
                instructions: vec![
                    Instruction::FunctionInfo {
                        module: "lists".into(),
                        name: "keymember".into(),
                        arity: 3,
                    },
                    Instruction::Label(1),
                    Instruction::Move(atom("stub"), Operand::X(0)),
                    Instruction::Return,
                ],
            },
        );
        // `lists:keymember/3` is in the ERTS 29.1.1 `bif.tab`. ERTS runs the
        // BIF, so the BEAM stub body of the loaded `lists` module must not run.
        assert_eq!(
            program
                .execute("fixture", "main", &[])
                .unwrap_err()
                .to_string(),
            "unresolved external MFA lists:keymember/3"
        );
        // `is_bif` finds an entry with a binary search.
        // Strictly increasing: sorted for a binary search, with no duplicate entry.
        assert!(BIFS.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// The BIF table holds the BIFs of the pinned ERTS. `erlang:is_builtin/3`
    /// returns `true` for each table entry and for no other exported function
    /// of the OTP 29.1.1 installation, except `erlang:apply/2`,
    /// `erlang:apply/3`, and `erlang:yield/0`. `erts_is_builtin`
    /// (`beam_common.c`) marks these three, but they are not `bif.tab` entries:
    /// the loader turns a call to them into the `i_apply_fun`, `i_apply`, and
    /// `i_yield` instructions (`ops.tab`).
    #[test]
    fn bif_table_equals_erts_is_builtin() {
        let output = Command::new("erl")
            .args([
                "-noshell",
                "-eval",
                concat!(
                    "[io:format(\"~ts\\t~ts\\t~b~n\", [M, F, A]) ",
                    "|| P <- filelib:wildcard(filename:join([code:root_dir(), \"lib\", \"*\", \"ebin\", \"*.beam\"])), ",
                    "{ok, {M, [{exports, E}]}} <- [beam_lib:chunks(P, [exports])], ",
                    "{F, A} <- E, erlang:is_builtin(M, F, A)], halt()."
                ),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let mut builtin: Vec<(&str, &str, u32)> = stdout
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
        builtin.sort_unstable();
        builtin.dedup();
        // `erts_is_builtin` (`beam_common.c`) marks apply/2, apply/3, and
        // yield/0 as builtin, but not hibernate/0.
        for mfa in INSTRUCTION_CALLS {
            assert_eq!(builtin.contains(&mfa), mfa.1 != "hibernate");
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
        // ERTS 29.1.1: unicode:characters_to_binary([<<"Hello">>, 10], utf8)
        // is <<"Hello\n">>.
        assert_eq!(
            convert(chardata(), "utf8").unwrap(),
            Term::Binary(b"Hello\n".to_vec())
        );
        assert_eq!(
            convert(chardata(), "latin1").unwrap_err().to_string(),
            "unsupported unicode:characters_to_binary/2 encoding Atom(\"latin1\")"
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

    /// The values come from a pinned OTP 29.1.1 run of `lst/1`, `hd1/1`, and
    /// `nil/1` in a module compiled with `erlc +no_line_info`:
    /// `lst([H | T]) -> {H, T}.`, `hd1([H | _]) -> H.`, and
    /// `nil([]) -> nil; nil([_ | _]) -> nonempty; nil(_) -> other.`
    #[test]
    fn reads_and_tests_lists_as_pinned_erts() {
        let list = |names: &[&str]| Term::List(names.iter().map(|name| test_atom(name)).collect());
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
            get_list(list(&["a", "b", "c"])).unwrap(),
            Term::Tuple(vec![test_atom("a"), list(&["b", "c"])])
        );
        assert_eq!(
            get_list(list(&["a"])).unwrap(),
            Term::Tuple(vec![test_atom("a"), list(&[])])
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
        assert_eq!(get_head(list(&["x", "y"])).unwrap(), test_atom("x"));
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
        assert_eq!(classify(list(&[])), test_atom("nil"));
        assert_eq!(classify(list(&["a"])), test_atom("nonempty"));
        assert_eq!(classify(test_atom("a")), test_atom("other"));
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
            Operand::Value(Term::Tuple(vec![
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
            run(Operand::X(0), vec![(3, Operand::X(1)), (4, Operand::X(2))]).unwrap(),
            Term::Tuple(vec![
                test_atom("r"),
                Term::Integer(1),
                Term::Integer(9),
                Term::Integer(1),
            ])
        );
        // The destination x1 is also a value register. The update uses the
        // value of x1 before the write, as the ERTS `SCRATCH_X_REG` rewrite does.
        assert_eq!(
            run(Operand::X(1), vec![(2, Operand::X(2)), (3, Operand::X(1))]).unwrap(),
            Term::Tuple(vec![
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

    /// A `wait_timeout` with `infinity` waits as `wait` does: the `DOWN`
    /// message of a monitored child continues the receive at its label.
    #[test]
    fn waits_for_a_message_with_an_infinity_timeout_and_rejects_other_timeouts() {
        let wait = |timeout: Term| {
            let mut body = spawn_link_child(Term::List(vec![Term::Integer(1)]));
            body.pop();
            body.extend([
                Instruction::Move(
                    Operand::Value(Term::List(vec![test_atom("monitor")])),
                    Operand::X(3),
                ),
                erlang_call("spawn_opt", 4, false),
                Instruction::Label(5),
                Instruction::LoopReceive {
                    fail_label: 6,
                    destination: Operand::X(0),
                },
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
        // ERTS 29.1.1: `receive M -> M after T -> timeout end` with T = 0
        // returns timeout at once. PBeam has no timers yet.
        assert_eq!(
            wait(Term::Integer(0)).unwrap_err().to_string(),
            "unsupported receive timeout Integer(0)"
        );
        assert_eq!(
            wait(Term::Integer(100)).unwrap_err().to_string(),
            "unsupported receive timeout Integer(100)"
        );
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
    fn returns_the_iovec_of_a_binary_and_of_a_list() {
        // On pinned ERTS, `erlang:iolist_to_iovec(<<"Hello">>)` returns
        // `[<<"Hello">>]`, and `[1, [<<2>>]]` returns `[<<1,2>>]` through the
        // iolist engine (`iolist_to_iovec_engine` in `erl_iolist.c`).
        let run = |argument| {
            make_program(
                vec![
                    Instruction::Move(Operand::Value(argument), Operand::X(0)),
                    erlang_call("iolist_to_iovec", 1, true),
                ],
                0,
            )
            .execute("fixture", "main", &[])
            .unwrap()
        };
        let hello = Term::Binary(b"Hello".to_vec());
        assert_eq!(run(hello.clone()), Term::List(vec![hello]));
        assert_eq!(
            run(Term::List(vec![
                Term::Integer(1),
                Term::List(vec![Term::Binary(vec![2])]),
            ])),
            Term::List(vec![Term::Binary(vec![1, 2])])
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
    fn raises_the_exception_of_a_bif_instruction_and_rejects_a_trap() {
        let element = |index: i64| {
            make_program(
                vec![
                    Instruction::Allocate { stack_size: 1 },
                    Instruction::Catch {
                        register: 0,
                        label: 3,
                    },
                    element_bif(index, Operand::Value(Term::Tuple(vec![])), Operand::X(0)),
                    Instruction::Label(3),
                    Instruction::CatchEnd { register: 0 },
                    Instruction::Deallocate { stack_size: 1 },
                    Instruction::Return,
                ],
                0,
            )
            .execute("fixture", "main", &[])
        };
        // ERTS 29.1.1: catch element(1, {}) is {'EXIT', {badarg, Stack}}.
        // PBeam gives the stack trace [].
        assert_eq!(
            element(1).unwrap(),
            Term::Tuple(vec![
                test_atom("EXIT"),
                Term::Tuple(vec![test_atom("badarg"), Term::List(Vec::new())]),
            ])
        );
        // Without a catch, the exception exits the process.
        let body = vec![
            element_bif(1, Operand::Value(Term::Tuple(vec![])), Operand::X(0)),
            Instruction::Return,
        ];
        assert_eq!(
            make_program(body, 0)
                .execute("fixture", "main", &[])
                .unwrap_err()
                .to_string(),
            BADARG_EXIT
        );
        // demonitor(Ref, [flush]) of a reference with no monitor traps. No
        // compiler emits it as a `bif` instruction.
        let body = vec![
            erlang_call("make_ref", 0, false),
            Instruction::Bif {
                fail_label: 0,
                module: "erlang".into(),
                name: "demonitor".into(),
                arguments: vec![
                    Operand::X(0),
                    Operand::Value(Term::List(vec![test_atom("flush")])),
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
        let node = |arguments: Vec<Operand>| {
            make_program(
                vec![
                    Instruction::Bif {
                        fail_label: 0,
                        module: "erlang".into(),
                        name: "self".into(),
                        arguments: Vec::new(),
                        destination: Operand::X(1),
                    },
                    Instruction::Bif {
                        fail_label: 0,
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
                    fail_label: 0,
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

    /// Run `main/0` with the functions `zero/0`, `id/3`, and `last/2` of
    /// the probe that gives the ERTS values of the `apply` tests:
    /// `zero() -> zero.`, `id(A, B, C) -> {A, B, C}.`, and
    /// `last(M, F) -> M:F(bye).`, which compiles to `{apply_last,1,0}`.
    fn run_with_apply_targets(body: Vec<Instruction>) -> Result<Term, crate::BeamError> {
        let mut program = make_program(body, 0);
        let module = program.modules.get_mut("fixture").unwrap();
        module.exports.extend([
            (("zero".into(), 0), 11),
            (("id".into(), 3), 12),
            (("last".into(), 2), 13),
        ]);
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
        module.instructions.extend(function("zero", 0, 11));
        module.instructions.extend([
            Instruction::Move(atom("zero"), Operand::X(0)),
            Instruction::Return,
        ]);
        module.instructions.extend(function("id", 3, 12));
        module.instructions.extend([
            Instruction::PutTuple {
                elements: vec![Operand::X(0), Operand::X(1), Operand::X(2)],
                destination: Operand::X(0),
            },
            Instruction::Return,
        ]);
        module.instructions.extend(function("last", 2, 13));
        module.instructions.extend([
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
        program.execute("fixture", "main", &[])
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
            Term::Tuple(vec![test_atom("a"), test_atom("b"), test_atom("c")])
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
                    label: 13
                },
                Instruction::Label(3),
                Instruction::CatchEnd { register: 0 },
                Instruction::Return,
            ])
            .unwrap(),
            Term::Tuple(vec![test_atom("EXIT"), test_atom("bye")])
        );
    }

    #[test]
    fn applies_erlang_apply_3_as_the_i_apply_instruction() {
        let xyz = || Term::List(vec![test_atom("x"), test_atom("y"), test_atom("z")]);
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
        let tuple = Term::Tuple(vec![test_atom("x"), test_atom("y"), test_atom("z")]);
        assert_eq!(run_with_apply_targets(body).unwrap(), tuple);
        // `erlang:apply(erlang, apply, [fixture, id, [x, y, z]])` as a tail
        // `call_ext` runs the inner call.
        let mut body = apply_of(
            test_atom("erlang"),
            test_atom("apply"),
            Term::List(vec![test_atom("fixture"), test_atom("id"), xyz()]),
        );
        body.push(erlang_call("apply", 3, true));
        assert_eq!(run_with_apply_targets(body).unwrap(), tuple);
        // An apply of `erlang:apply` with a list that is not three elements
        // calls `erlang:apply/2`, which runs as an instruction in ERTS.
        let mut body = apply_of(
            test_atom("erlang"),
            test_atom("apply"),
            Term::List(vec![test_atom("fixture"), test_atom("id")]),
        );
        body.push(erlang_call("apply", 3, true));
        assert_eq!(
            run_with_apply_targets(body).unwrap_err().to_string(),
            "unsupported instruction call erlang:apply/2"
        );
        // `apply` copies at most 1023 arguments to the X registers. ERTS gives
        // `undef` for `apply(nomod, nofun, L1023)` and raises `system_limit`
        // for a list of 1024 elements.
        let mut body = apply_of(
            test_atom("fixture"),
            test_atom("missing"),
            Term::List(vec![test_atom("x"); 1023]),
        );
        body.push(erlang_call("apply", 3, true));
        assert_eq!(
            run_with_apply_targets(body).unwrap_err().to_string(),
            "unresolved external MFA fixture:missing/1023"
        );
        let mut body = apply_of(
            test_atom("fixture"),
            test_atom("missing"),
            Term::List(vec![test_atom("x"); 1024]),
        );
        body.push(erlang_call("apply", 3, true));
        assert_eq!(
            run_with_apply_targets(body).unwrap_err().to_string(),
            "root process exited with Tuple([Atom(\"system_limit\"), List([])])"
        );
    }

    #[test]
    fn rejects_an_apply_with_a_stack_mismatch_and_raises_badarg() {
        let error = |body| run_with_apply_targets(body).unwrap_err().to_string();
        // ERTS 29.1.1 raises `badarg` for `M:F()` with `1` and `f`, or with
        // `m` and `1`.
        assert_eq!(
            error(apply_body(0, Operand::Value(Term::Integer(1)), atom("f"))),
            BADARG_EXIT
        );
        assert_eq!(
            error(apply_body(0, atom("m"), Operand::Value(Term::Integer(1)))),
            BADARG_EXIT
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
    fn call_ext_with(
        module: &str,
        name: &str,
        arguments: Vec<Term>,
    ) -> Result<Term, crate::BeamError> {
        let arity = arguments.len() as u32;
        let mut body: Vec<Instruction> = arguments
            .into_iter()
            .zip(0..)
            .map(|(argument, register)| {
                Instruction::Move(Operand::Value(argument), Operand::X(register))
            })
            .collect();
        body.push(Instruction::CallExternal {
            arity,
            module: module.into(),
            name: name.into(),
            tail: true,
            deallocate: 0,
        });
        make_program(body, 0).execute("fixture", "main", &[])
    }

    fn get_bif(key: &str, destination: Operand) -> Instruction {
        Instruction::Bif {
            fail_label: 0,
            module: "erlang".into(),
            name: "get".into(),
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
                .unwrap(),
            Term::Tuple(vec![
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
            Instruction::Move(Operand::Value(Term::List(Vec::new())), Operand::X(2)),
            Instruction::Move(
                Operand::Value(Term::List(vec![test_atom("monitor")])),
                Operand::X(3),
            ),
            erlang_call("spawn_opt", 4, false),
        ];
        body.extend(wait_forever());
        let mut program = make_program(body, 0);
        let module = program.modules.get_mut("fixture").unwrap();
        module.exports.insert(("fresh".into(), 0), 11);
        module.instructions.extend([
            Instruction::FunctionInfo {
                module: "fixture".into(),
                name: "fresh".into(),
                arity: 0,
            },
            Instruction::Label(11),
            get_bif("k", Operand::X(0)),
            erlang_call("exit", 1, true),
        ]);
        // ERTS 29.1.1: after put(k, parent), a spawned process gets
        // undefined for get(k).
        assert!(matches!(
            program.execute("fixture", "main", &[]),
            Ok(Term::Tuple(ref elements))
                if elements.first() == Some(&test_atom("DOWN"))
                    && elements.last() == Some(&Term::static_atom("undefined"))
        ));
    }

    #[test]
    fn appends_lists_as_pinned_erts() {
        let integers =
            |values: &[i64]| Term::List(values.iter().copied().map(Term::Integer).collect());
        let append = |left: Term, right: Term| call_ext_with("erlang", "++", vec![left, right]);
        // ERTS 29.1.1: [] ++ x is x, and [1,2] ++ [3] is [1,2,3].
        assert_eq!(
            append(Term::List(Vec::new()), test_atom("x")).unwrap(),
            test_atom("x")
        );
        assert_eq!(
            append(integers(&[1, 2]), integers(&[3])).unwrap(),
            integers(&[1, 2, 3])
        );
        // ERTS 29.1.1: [1] ++ x is the improper list [1|x], and a ++ []
        // and a ++ x raise badarg.
        assert_eq!(
            append(integers(&[1]), test_atom("x"))
                .unwrap_err()
                .to_string(),
            "unsupported erlang:++/2 tail Atom(\"x\"): not a list"
        );
        for right in [Term::List(Vec::new()), test_atom("x")] {
            assert_eq!(
                append(test_atom("a"), right).unwrap_err().to_string(),
                BADARG_EXIT
            );
        }
    }

    #[test]
    fn finds_key_tuples_as_pinned_erts() {
        let tuple = |elements: Vec<Term>| Term::Tuple(elements);
        let keyfind = |key: Term, position: Term, list: Vec<Term>| {
            call_ext_with("lists", "keyfind", vec![key, position, Term::List(list)])
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
        let one = Term::List(vec![tuple(vec![a.clone()])]);
        for position in [
            Term::Integer(0),
            Term::Integer(-1),
            test_atom("x"),
            Term::Integer(1 << 59),
        ] {
            assert_eq!(badarg(a.clone(), position, one.clone()), BADARG_EXIT);
        }
        for list in [a.clone(), tuple(vec![a.clone()])] {
            assert_eq!(badarg(a.clone(), Term::Integer(1), list), BADARG_EXIT);
        }
    }

    #[test]
    fn finds_list_members_as_pinned_erts() {
        let member =
            |element: Term, list: Term| call_ext_with("lists", "member", vec![element, list]);
        // ERTS 29.1.1: lists:member(a, []) is false, and lists:member(1, [1])
        // is true.
        assert_eq!(
            member(test_atom("a"), Term::List(Vec::new())).unwrap(),
            Term::static_atom("false")
        );
        assert_eq!(
            member(Term::Integer(1), Term::List(vec![Term::Integer(1)])).unwrap(),
            Term::static_atom("true")
        );
        // ERTS 29.1.1: lists:member(a, x) raises badarg.
        assert_eq!(
            member(test_atom("a"), test_atom("x"))
                .unwrap_err()
                .to_string(),
            BADARG_EXIT
        );
    }

    fn element_bif(index: i64, tuple: Operand, destination: Operand) -> Instruction {
        Instruction::Bif {
            fail_label: 0,
            module: "erlang".into(),
            name: "element".into(),
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
                Operand::Value(Term::Tuple(vec![a.clone(), b.clone(), c])),
                Operand::X(0),
            ),
            element_bif(2, Operand::X(0), Operand::X(0)),
            Instruction::Return,
        ];
        assert_eq!(
            make_program(body, 0)
                .execute("fixture", "main", &[])
                .unwrap(),
            b
        );
        let element =
            |index: Term, tuple: Term| call_ext_with("erlang", "element", vec![index, tuple]);
        let one = Term::Tuple(vec![a.clone()]);
        // ERTS 29.1.1: element(1, {a}) is a.
        assert_eq!(element(Term::Integer(1), one.clone()).unwrap(), a);
        // ERTS 29.1.1 raises badarg for element(0, {a}), element(2, {a}),
        // element(-1, {a}), element(a, {a}), and element(1, [a]).
        for (index, tuple) in [
            (Term::Integer(0), one.clone()),
            (Term::Integer(2), one.clone()),
            (Term::Integer(-1), one.clone()),
            (a.clone(), one),
            (Term::Integer(1), Term::List(vec![a])),
        ] {
            assert_eq!(element(index, tuple).unwrap_err().to_string(), BADARG_EXIT);
        }
    }

    #[test]
    fn reads_the_tuple_size_as_pinned_erts() {
        let tuple_size = |term: Term| call_ext_with("erlang", "tuple_size", vec![term]);
        // ERTS 29.1.1: tuple_size({}) is 0, and tuple_size(a) raises badarg.
        assert_eq!(tuple_size(Term::Tuple(vec![])).unwrap(), Term::Integer(0));
        assert_eq!(
            tuple_size(test_atom("a")).unwrap_err().to_string(),
            BADARG_EXIT
        );
    }

    #[test]
    fn rejects_is_ge_on_two_different_references() {
        // ERTS orders two references by the scheduler thread that made each
        // one; this is the "Term order" row in `docs/erts-differences.md`.
        let reference = |number| Operand::Value(Term::Reference(crate::Reference(number)));
        let body = vec![
            Instruction::TestGreaterEqual {
                fail_label: 3,
                left: reference(1),
                right: reference(2),
            },
            Instruction::Label(3),
            Instruction::Return,
        ];
        assert_eq!(
            make_program(body, 0)
                .execute("fixture", "main", &[])
                .unwrap_err()
                .to_string(),
            "unsupported term order of two references Reference(Reference(1)) and \
             Reference(Reference(2))"
        );
    }

    #[test]
    fn reads_an_empty_current_stacktrace_of_the_calling_process() {
        // `STACKTRACE()` in the pinned `gen_statem` is
        // `element(2, erlang:process_info(self(), current_stacktrace))`.
        let body = vec![
            Instruction::Bif {
                fail_label: 0,
                module: "erlang".into(),
                name: "self".into(),
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
        let stacktrace = Term::Tuple(vec![
            test_atom("current_stacktrace"),
            Term::List(Vec::new()),
        ]);
        assert_eq!(
            make_program(body, 0)
                .execute("fixture", "main", &[])
                .unwrap(),
            Term::Tuple(vec![stacktrace, Term::List(Vec::new())])
        );
        let process_info = |process: Term, item: Term| {
            call_ext_with("erlang", "process_info", vec![process, item])
        };
        let root = || Term::Pid(crate::ProcessId(0));
        let dictionary = || Term::Tuple(vec![test_atom("dictionary"), test_atom("k")]);
        // ERTS 29.1.1 checks the item before the process. It raises badarg for
        // process_info(a, current_stacktrace), process_info(a, bogus_item),
        // process_info(self(), bogus_item), process_info(self(), 5),
        // process_info(self(), [registered_name, bogus]), and
        // process_info(self(), {bogus, k}).
        for (process, item) in [
            (test_atom("a"), test_atom("current_stacktrace")),
            (test_atom("a"), test_atom("bogus_item")),
            (Term::Pid(crate::ProcessId(99)), test_atom("bogus_item")),
            (root(), test_atom("bogus_item")),
            (root(), Term::Integer(5)),
            (
                root(),
                Term::List(vec![test_atom("registered_name"), test_atom("bogus")]),
            ),
            (
                root(),
                Term::Tuple(vec![test_atom("bogus"), test_atom("k")]),
            ),
        ] {
            assert_eq!(
                process_info(process, item).unwrap_err().to_string(),
                BADARG_EXIT
            );
        }
        // ERTS 29.1.1: a process that is not alive gives undefined for each
        // valid item, also a list of items (`erl_bif_info.c:1384-1386`).
        for item in [
            test_atom("current_stacktrace"),
            Term::List(vec![test_atom("registered_name")]),
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
            (root(), Term::List(vec![test_atom("registered_name")])),
            (root(), test_atom("status")),
        ] {
            assert_eq!(
                process_info(process.clone(), item.clone())
                    .unwrap_err()
                    .to_string(),
                format!("unsupported erlang:process_info/2 arguments {process:?}, {item:?}")
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
            Term::List(Vec::new())
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
                Operand::Value(Term::List(vec![test_atom("normal")])),
                Operand::X(2),
            ),
            erlang_call("spawn", 3, false),
            Instruction::Move(atom("current_stacktrace"), Operand::X(1)),
            erlang_call("process_info", 2, true),
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
        for (name, id) in [
            (Term::Integer(1), root),
            (test_atom("a"), test_atom("x")),
            (test_atom("a"), Term::Pid(crate::ProcessId(99))),
        ] {
            assert_eq!(register(name, id).unwrap_err().to_string(), BADARG_EXIT);
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
        for arguments in [
            vec![Term::Integer(1), self_name.clone(), Term::Integer(0)],
            vec![erlang.clone(), Term::Integer(1), Term::Integer(0)],
            vec![erlang.clone(), self_name.clone(), test_atom("x")],
            vec![erlang.clone(), self_name.clone(), Term::Integer(1 << 59)],
            vec![erlang, self_name, Term::Integer(-(1 << 59) - 1)],
        ] {
            assert_eq!(
                call_ext_with("erlang", "function_exported", arguments)
                    .unwrap_err()
                    .to_string(),
                BADARG_EXIT
            );
        }
    }
}
