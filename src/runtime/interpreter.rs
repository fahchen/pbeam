use super::{
    process::{Frame, ProcessState, ReceiveTimer, StackSlot, Start},
    scheduler::Scheduler,
    unicode::{characters_to_code_points, characters_to_utf8},
};
use crate::{
    beam::{
        BeamModule,
        code::{self, BinaryMatchCommand, Instruction, Operand, TestKind},
    },
    error::{BeamError, error},
    term::{Atom, AtomTable, Fun, Map, MatchContext, Target, Term, print_term},
};
use std::collections::HashMap;

mod bif;
mod native;

use bif::{BifResult, INSTRUCTION_CALLS, is_bif, is_function, send};

/// The largest number of arguments of `erlang:apply/3`. ERTS `apply` copies
/// at most `MAX_REG - 1` arguments to the X registers (`erl_vm.h:55`,
/// `beam_common.c:1501-1513`), so a longer list raises `system_limit`.
const APPLY_MAX_ARGUMENTS: usize = 1023;

/// The largest number of arguments of `erlang:apply/2`. The JIT stops at
/// `MAX_REG - 1` arguments, because the loader keeps the highest X register
/// (`erl_vm.h:55`, `jit/arm/instr_fun.cpp:290-297`), so a longer list
/// raises `system_limit`.
const APPLY_FUN_MAX_ARGUMENTS: usize = 1022;

#[derive(Debug)]
pub struct BeamProgram {
    /// The loaded modules by name.
    modules: HashMap<String, BeamModule>,
    /// The atom table of the loaded modules. ERTS has one atom table for
    /// each emulator, so each run starts its own table from a copy of it.
    atoms: AtomTable,
}

/// Why a process stopped running on the scheduler.
#[derive(Debug)]
enum Outcome {
    /// The process waits for a new message.
    Wait,
    /// The process called `erlang:hibernate/0`. It continues at its next
    /// instruction when the scheduler runs it again.
    Hibernate,
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

    /// Return the atom of a name when the atom table of the loaded modules
    /// holds it. Use it to build terms that compare with the terms of the
    /// program. The table does not hold the atoms that a run makes, for
    /// example with `erlang:list_to_atom/1`.
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
    /// spawns run on the same cooperative scheduler. A process runs until it
    /// waits, calls `erlang:hibernate/0`, or exits. Before it takes the
    /// next process, the scheduler reads the run clock and expires each due
    /// timer, as ERTS bumps the timers in `check_activities_to_run`
    /// (`erl_process.c:9797-9810`). When no process can run, it sleeps until
    /// the first timer end, as `scheduler_wait` sleeps until the next
    /// timeout (`erl_process.c:3452-3620`). The run ends at the next
    /// scheduling step after the root process exits, also when another
    /// process can run or a timer is active. A process that ends the root
    /// process with an exit signal continues until it waits or exits. This
    /// is as `halt()` after `-eval` stops the ERTS node
    /// (`erlang.erl:3357-3358`) and as `init` halts after a boot process
    /// exit that is not `normal` (`init.erl:710-712`). A root process exit
    /// gives the error `root process exited with <Reason>`, with the reason
    /// as ERTS prints a term (`print_term`), for example
    /// `root process exited with {badarg,[]}`. A root process that
    /// waits when no process can run and no timer is active gives an error.
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
        let arity =
            u32::try_from(arguments.len()).map_err(|_| error("arity exceeds BEAM limit"))?;
        let label = self.entry_label(module, function, arity)?;
        let mut scheduler = Scheduler::new(self.atoms.clone());
        scheduler.start(self.frame(module, label, arity)?, arguments.to_vec())?;
        while !scheduler.root_exited() {
            let now = scheduler.timers.now();
            scheduler.expire_timers(now);
            if scheduler.schedule_next().is_some() {
                match self.run(&mut scheduler)? {
                    Outcome::Wait => scheduler.park_current(),
                    Outcome::Hibernate => scheduler.hibernate_current(),
                    Outcome::Exit { reason, result } => {
                        scheduler.finish_current(reason, result)?;
                    }
                }
            } else if let Some(wait) = scheduler.timers.wait_time(scheduler.timers.now()) {
                std::thread::sleep(wait);
            } else {
                break;
            }
        }
        match scheduler.into_root_result() {
            Some(Ok(result)) => Ok(result),
            Some(Err(reason)) => {
                // `erlang:halt/1` writes each byte of the `%T` text as one code
                // point (`bif.c:4372-4387`), so the text is valid UTF-8.
                let text: String = print_term(&reason).into_iter().map(char::from).collect();
                Err(error(format!("root process exited with {text}")))
            }
            None => Err(error("root process waits for a message that cannot arrive")),
        }
    }

    /// Return the entry label of an exported function that a run starts
    /// from.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let label = program.entry_label("process_probe", "main", 0)?;
    /// assert!(program.entry_label("process_probe", "missing", 0).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// A module that is not loaded is the error `missing BEAM module M`. A
    /// function that the module does not export is the error
    /// `missing export M:F/A`.
    fn entry_label(&self, module: &str, function: &str, arity: u32) -> Result<u32, BeamError> {
        let loaded = self
            .modules
            .get(module)
            .ok_or_else(|| error(format!("missing BEAM module {module}")))?;
        loaded
            .exports
            .get(&(function.to_owned(), arity))
            .copied()
            .ok_or_else(|| error(format!("missing export {module}:{function}/{arity}")))
    }

    /// Create the frame for an exported function of a loaded module. Return
    /// `None` when the module does not export the function: ERTS then runs
    /// the export entry of the function, which calls the error handler
    /// (`call_error_handler` in `beam_common.c:1294-1342`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(program.export_frame("undef_handler", "$handle_undefined_function", 2)?.is_some());
    /// assert!(program.export_frame("undef_target", "missing", 1)?.is_none());
    /// assert!(program.export_frame("nomod", "missing", 1).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// A module that is not loaded is the runtime error
    /// `unresolved external MFA M:F/A`. ERTS loads it from the code path.
    fn export_frame(
        &self,
        module: &str,
        function: &str,
        arity: u32,
    ) -> Result<Option<Frame>, BeamError> {
        let loaded = self.modules.get(module).ok_or_else(|| {
            error(format!(
                "unresolved external MFA {module}:{function}/{arity}"
            ))
        })?;
        loaded
            .exports
            .get(&(function.to_owned(), arity))
            .map(|label| self.frame(module, *label, arity))
            .transpose()
    }

    /// Create the frame for an exported function, such as the target of a
    /// BIF trap.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let frame = program.mfa_frame("external", "effect", 1)?;
    /// ```
    ///
    /// # Errors
    ///
    /// A module that is not loaded or a function that the module does not
    /// export is the runtime error `unresolved external MFA M:F/A`.
    fn mfa_frame(&self, module: &str, function: &str, arity: u32) -> Result<Frame, BeamError> {
        self.export_frame(module, function, arity)?.ok_or_else(|| {
            error(format!(
                "unresolved external MFA {module}:{function}/{arity}"
            ))
        })
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

    /// Run the current process until it waits for a message, calls
    /// `erlang:hibernate/0`, or exits. A process with an undefined start
    /// first calls `undefined_function` with its start MFA.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(program.run(&mut scheduler)?, Outcome::Wait));
    /// // `spawn(undef_target, missing, [p])` exits with
    /// // `{undef, [{undef_target, missing, [p], []}]}`.
    /// assert!(matches!(program.run(&mut scheduler)?, Outcome::Exit { result: None, .. }));
    /// ```
    fn run(&self, scheduler: &mut Scheduler) -> Result<Outcome, BeamError> {
        // ERTS starts each process with `i_apply_only` of its MFA
        // (`erl_process.c:12616`; JIT `jit/beam_jit_main.cpp:258`). Its
        // `apply` raises `system_limit` for more than `APPLY_MAX_ARGUMENTS`
        // arguments, and else a function that the module does not export
        // goes to its export entry when the process first runs. The arity is
        // the number of argument registers.
        let process = scheduler.current_process_mut();
        if let Some((module, function)) = process.undefined_start.take() {
            let outcome = if process.registers.len() > APPLY_MAX_ARGUMENTS {
                let system_limit = Term::static_atom("system_limit");
                raise(process, ExceptionClass::Error, system_limit)
            } else {
                let arity = process.registers.len() as u32;
                self.undefined_function(scheduler, module.name(), function.name(), arity, false)?
            };
            if let Some(outcome) = outcome {
                return Ok(outcome);
            }
        }
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
                    check_local_call_arguments(&process.registers, *arity)?;
                    let frame = self.frame(&module.name, *label, *arity)?;
                    process.frames.push(frame);
                }
                Instruction::CallOnly { arity, label } => {
                    check_local_call_arguments(&process.registers, *arity)?;
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
                    check_local_call_arguments(&process.registers, *arity)?;
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
                    if *tail {
                        // `apply_last` releases the frame before it tests
                        // the module and the function name
                        // (`emit_apply_last` in `jit/arm/instr_call.cpp:307-316`).
                        release_frame(process, *deallocate)?;
                    }
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
                    if let Some(outcome) =
                        self.call_mfa(scheduler, module.name(), name.name(), *arity, *tail)?
                    {
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
                Instruction::CallFun {
                    arity,
                    function,
                    tail,
                    deallocate,
                } => {
                    let fun = read(process, function)?;
                    if *tail {
                        // `i_call_fun_last` releases the frame before it
                        // tests the fun (`emit_i_call_fun2_last` in
                        // `jit/arm/instr_fun.cpp:425-462`).
                        release_frame(process, *deallocate)?;
                    }
                    if let Some(outcome) = self.call_fun(scheduler, &fun, *arity, *tail)? {
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
                        // BIF that does not trap or end the caller.
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
                        // `is_list` and `is_nonempty_list` test the cons tag, so an
                        // improper list passes (emu `instrs.tab:879-891`; JIT
                        // `emit_is_list` and `emit_is_nonempty_list` in
                        // `jit/arm/instr_common.cpp:1118-1364`).
                        TestKind::List => matches!(value, Term::List(_) | Term::ImproperList(..)),
                        TestKind::Nil => matches!(&value, Term::List(values) if values.is_empty()),
                        TestKind::NonEmptyList => {
                            matches!(
                                &value,
                                Term::List(values) if !values.is_empty()
                            ) || matches!(value, Term::ImproperList(..))
                        }
                        TestKind::Binary => matches!(value, Term::Binary(_)),
                        TestKind::Map => matches!(value, Term::Map(_)),
                        TestKind::Function => matches!(value, Term::Fun(_)),
                        TestKind::Integer => matches!(value, Term::Integer(_)),
                        // `is_float` passes only for a boxed term with the
                        // `HEADER_FLONUM` header (emu `instrs.tab:951-955`,
                        // `erl_term.h:484-485`; JIT `emit_is_float` in
                        // `jit/arm/instr_common.cpp:1229-1245`). PBeam has no
                        // float term, so the test fails for each term.
                        TestKind::Float => false,
                        TestKind::Boolean => {
                            matches!(&value, Term::Atom(atom) if matches!(atom.name(), "true" | "false"))
                        }
                    };
                    if !passed {
                        jump(module, process, *fail_label)?;
                    }
                }
                Instruction::GetMapElements {
                    fail_label,
                    map,
                    pairs,
                } => match map_values(process, map, pairs.iter().map(|(key, _)| key))? {
                    Some(values) => {
                        for ((_, destination), value) in pairs.iter().zip(values) {
                            write(process, destination, value)?;
                        }
                    }
                    None => jump(module, process, *fail_label)?,
                },
                // ERTS loads `has_map_fields` as `get_map_elements` with
                // scratch destinations (`generators.tab:230-253`).
                Instruction::HasMapFields {
                    fail_label,
                    map,
                    keys,
                } => {
                    if map_values(process, map, keys)?.is_none() {
                        jump(module, process, *fail_label)?;
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
                Instruction::PutMapExact {
                    fail_label,
                    map,
                    destination,
                    pairs,
                } => {
                    let map = read(process, map)?;
                    let pairs = pairs
                        .iter()
                        .map(|(key, value)| Ok((read(process, key)?, read(process, value)?)))
                        .collect::<Result<Vec<_>, BeamError>>()?;
                    // ERTS `update_map_exact` jumps to a fail label that is
                    // not 0, and else raises the exception
                    // (`jit/arm/instr_map.cpp:678-719`).
                    match update_map_exact(map, pairs)? {
                        Ok(map) => write(process, destination, map)?,
                        Err(_) if *fail_label != 0 => jump(module, process, *fail_label)?,
                        Err(reason) => {
                            if let Some(outcome) = raise(process, ExceptionClass::Error, reason) {
                                return Ok(outcome);
                            }
                        }
                    }
                }
                Instruction::Swap(left, right) => {
                    let left_value = read(process, left)?;
                    let right_value = read(process, right)?;
                    write(process, left, right_value)?;
                    write(process, right, left_value)?;
                }
                // ERTS `is_eq` (emu instrs.tab:1109-1113, JIT
                // instr_common.cpp:2055-2079) runs `CMP_EQ_ACTION`
                // (erl_utils.h:132-133). `EQ_SPEC` (erl_utils.h:137-147) and
                // `erts_cmp_compound(A, B, 0, 1)` (utils.c:1658) compare in
                // the arithmetic order, which differs from exact equality
                // only for a float (`1 == 1.0` is true). PBeam has no float
                // term, so exact equality gives the ERTS result. The
                // float-term task must change this arm.
                Instruction::TestEqual {
                    fail_label,
                    left,
                    right,
                }
                | Instruction::TestExactEqual {
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
                Instruction::TestLess {
                    fail_label,
                    left,
                    right,
                } => {
                    if !read(process, left)?
                        .compare(&read(process, right)?)?
                        .is_lt()
                    {
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
                // ERTS `i_bs_start_match3` jumps to `Fail` for each term
                // that is not a bitstring. It reuses a match context, and
                // makes one for another bitstring, and writes it to `Dst`
                // (emu bs_instrs.tab:1074-1118 and 1199-1238; JIT
                // `emit_i_bs_start_match3` in jit/arm/instr_bs.cpp:134-189;
                // `erts_bs_start_match_3` in erl_bits.c:81-105). A literal
                // source loads as `jump Fail` (emu ops.tab:1266-1267; JIT
                // jit/arm/ops.tab:1054-1055), which gives the same result.
                // The compiler gives a real fail label
                // (beam_ssa_codegen.erl:1666-1673), so the label is never 0.
                // ERTS writes a reused match context to the destination, so
                // the two registers share one position. PBeam keeps the
                // position in each register, so it reuses a match context
                // only in its own register and else stops (row "Binary
                // matching" of docs/erts-differences.md).
                // `prim_tty:is_usascii/1` reuses the match context in `x0`
                // with the destination `x0`.
                Instruction::BinaryMatchStart {
                    fail_label,
                    source,
                    destination,
                } => match &read_any(process, source)? {
                    Term::Binary(bytes) => {
                        let context = Term::MatchContext(MatchContext::new(bytes));
                        write(process, destination, context)?;
                    }
                    Term::MatchContext(_) => {
                        if !matches!(
                            (source, destination),
                            (Operand::X(from), Operand::X(to)) | (Operand::Y(from), Operand::Y(to))
                                if from == to
                        ) {
                            return Err(error(format!(
                                "unsupported bs_start_match3 that copies the match context in {source:?} to {destination:?}"
                            )));
                        }
                    }
                    _ => jump(module, process, *fail_label)?,
                },
                // ERTS `i_bs_get_position` writes `sb->start` as a small
                // integer (emu ops.tab:1274-1277, bs_instrs.tab:1125-1129).
                Instruction::BinaryGetPosition {
                    context,
                    destination,
                } => {
                    let position = match_context(process, context)?.position();
                    write(process, destination, Term::Integer(position as i64))?;
                }
                // ERTS `bs_set_position` sets `sb->start` to the unsigned
                // value of the position (emu bs_instrs.tab:1120-1123). The
                // compiler gives it a position that `bs_get_position` made.
                Instruction::BinarySetPosition { context, position } => {
                    let mut state = match_context(process, context)?;
                    match read(process, position)? {
                        Term::Integer(value) if let Ok(value) = usize::try_from(value) => {
                            state.set_position(value);
                        }
                        other => {
                            return Err(error(format!(
                                "invariant error: bs_set_position position {other:?} is not a position"
                            )));
                        }
                    }
                    write(process, context, Term::MatchContext(state))?;
                }
                // ERTS `i_bs_get_utf8` reads a byte below 128 directly and
                // else calls `erts_bs_get_utf8`, which fails for each
                // invalid form (emu ops.tab:1286-1287, bs_instrs.tab:958-981;
                // erl_bits.c:2128-2221). Both paths give the same result.
                Instruction::BinaryGetUtf8 {
                    fail_label,
                    context,
                    destination,
                } => {
                    let mut state = match_context(process, context)?;
                    match state.get_utf8() {
                        Some(code_point) => {
                            write(process, context, Term::MatchContext(state))?;
                            write(process, destination, Term::Integer(code_point.into()))?;
                        }
                        None => jump(module, process, *fail_label)?,
                    }
                }
                Instruction::BinaryMatch {
                    fail_label,
                    context,
                    commands,
                } => {
                    if !binary_match(&match_context(process, context)?, commands)? {
                        jump(module, process, *fail_label)?;
                    }
                }
                // ERTS `fconv` converts a small integer, a bignum, or a float
                // to a float register, and raises `badarith` for each other
                // term (emu float_instrs.tab:38-52; JIT `emit_fconv` and
                // `emit_fconv_shared` in jit/arm/instr_float.cpp:96-135).
                // PBeam has no float register and no float or bignum term,
                // so an integer stops the run.
                Instruction::FloatConvert { source } => {
                    if let Term::Integer(_) = read(process, source)? {
                        return Err(error("unsupported BEAM opcode 97"));
                    }
                    let badarith = Term::static_atom("badarith");
                    if let Some(outcome) = raise(process, ExceptionClass::Error, badarith) {
                        return Ok(outcome);
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
                    // ERTS `put_list` stores any tail in the new cons cell, so a
                    // tail that is not a list gives an improper list (emu
                    // `instrs.tab:732-737`; JIT `emit_put_list` in
                    // `jit/arm/instr_common.cpp:707-721`).
                    let head = read(process, head)?;
                    let tail = read(process, tail)?;
                    write(process, destination, Term::list_with_tail(vec![head], tail))?;
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
                    let (first, rest) = nonempty_list(read(process, source)?)?;
                    write(process, head, first)?;
                    write(process, tail, rest)?;
                }
                Instruction::GetHead {
                    source,
                    destination,
                } => {
                    let (first, _) = nonempty_list(read(process, source)?)?;
                    write(process, destination, first)?;
                }
                Instruction::GetTail {
                    source,
                    destination,
                } => {
                    let (_, rest) = nonempty_list(read(process, source)?)?;
                    write(process, destination, rest)?;
                }
                Instruction::Send => {
                    // ERTS `send` calls `erl_send` and writes its result to
                    // x0, or raises (`emu/bif_instrs.tab:425-454`; JIT
                    // `emit_send` in `jit/arm/instr_bif.cpp:612-625`).
                    let to = read(process, &Operand::X(0))?;
                    let message = read(process, &Operand::X(1))?;
                    let result = send(scheduler, &to, message);
                    if let Some(outcome) =
                        self.finish_bif(scheduler.current_process_mut(), result, false)?
                    {
                        return Ok(outcome);
                    }
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
                    scheduler.cancel_receive_timer();
                }
                Instruction::Wait(label) => {
                    jump(module, process, *label)?;
                    return Ok(Outcome::Wait);
                }
                Instruction::WaitTimeout { label, source } => {
                    // ERTS `beam_jit_wait_timeout` reads the timeout only
                    // when the process has no receive timer and no `F_TIMO`
                    // (`jit/beam_jit_common.cpp:1208-1240`). A value that is
                    // not valid moves the save pointer to the first message
                    // and raises `timeout_value` (`error.h:154`;
                    // `jit/arm/instr_msg.cpp:312-338`).
                    if process.receive_timer == ReceiveTimer::None {
                        let timeout = read(process, source)?;
                        if timeout == Term::Integer(0) {
                            continue;
                        }
                        if timeout == Term::static_atom("infinity") {
                            process.receive_timer = ReceiveTimer::Timo;
                        } else {
                            let resume = current_frame(process).next_instruction;
                            let now = scheduler.timers.now();
                            if !scheduler.set_receive_timer(now, &timeout, resume) {
                                let process = scheduler.current_process_mut();
                                process.mailbox.set_save_first();
                                let reason = Term::static_atom("timeout_value");
                                if let Some(outcome) = raise(process, ExceptionClass::Error, reason)
                                {
                                    return Ok(outcome);
                                }
                                continue;
                            }
                        }
                    }
                    jump(module, scheduler.current_process_mut(), *label)?;
                    return Ok(Outcome::Wait);
                }
                Instruction::Timeout => {
                    // ERTS `beam_jit_timeout` clears `F_TIMO`
                    // (`jit/beam_jit_common.cpp:1242-1251`).
                    process.receive_timer = ReceiveTimer::None;
                    process.mailbox.set_save_first();
                }
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
                    if let Some(result) = native::call(scheduler, &module.name, name, &arguments)
                        && let Some(outcome) =
                            self.finish_bif(scheduler.current_process_mut(), result?, true)?
                    {
                        return Ok(outcome);
                    }
                }
                Instruction::Hibernate => {
                    // `emit_i_hibernate` writes `ok` to `x0` and switches
                    // the context; the process keeps its frame
                    // (`jit/arm/instr_trace.cpp:222-238`).
                    write(process, &Operand::X(0), Term::static_atom("ok"))?;
                    return Ok(Outcome::Hibernate);
                }
                // ERTS `build_stacktrace` changes the raw stack trace in
                // `x0` to a list (emu `instrs.tab:1247-1251`; JIT
                // `emit_build_stacktrace` in
                // `jit/arm/instr_common.cpp:3046-3056`). The compiler moves
                // it from `x2` first. PBeam keeps the list in `x2`, so `x0`
                // already holds it.
                Instruction::BuildStacktrace => {}
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
    /// `call_ext` does. A tail call first checks that the frame has
    /// `deallocate` Y registers, then replaces the frame. A tail call of
    /// `erlang:apply/2` or `erlang:apply/3` releases the Y registers before
    /// the apply, as the ERTS loader makes `i_apply_fun_last` and
    /// `i_apply_last` (`jit/arm/ops.tab:649-657`), so an exception of the
    /// apply does not see the catch tags of the frame. A tail call of a BIF
    /// keeps the frame until the BIF returns, as `call_light_bif`,
    /// `deallocate`, and `return` do (`jit/arm/ops.tab:711-712`). A call of
    /// `erlang:load_nif/2` runs as the `i_load_nif` instruction with the
    /// module of the current frame, and a tail call keeps the frame until
    /// the instruction finishes (`jit/arm/ops.tab:615-623`,
    /// `emu/ops.tab:854-862`). Return the outcome when the process exits.
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
        if tail && matches!((module, name, arity), ("erlang", "apply", 2 | 3)) {
            release_frame(process, deallocate)?;
        } else if tail && deallocate as usize != current_frame(process).stack.len() {
            return Err(error("invalid BEAM tail-call stack size"));
        }
        if (module, name, arity) == ("erlang", "load_nif", 2) {
            let caller = current_frame(process).module.clone();
            let result = native::load_nif(self, scheduler, &caller)?;
            return self.finish_bif(scheduler.current_process_mut(), result, tail);
        }
        self.call_mfa(scheduler, module, name, arity, tail)
    }

    /// Call `module:name/arity` with the arguments in the X registers. The
    /// calls of `erlang:apply/2` and `erlang:apply/3` run as the ERTS
    /// `i_apply_fun` and `i_apply` instructions. A BIF runs before the module
    /// code, as in ERTS. A function that a loaded module does not export
    /// goes to `undefined_function`. Return the outcome when the process
    /// exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `erlang:apply(erlang, self, [])` runs the `self/0` BIF.
    /// let outcome = program.call_mfa(&mut scheduler, "erlang", "apply", 3, false)?;
    /// assert!(outcome.is_none());
    /// // `erlang:yield/0` runs as an instruction in ERTS.
    /// assert!(program.call_mfa(&mut scheduler, "erlang", "yield", 0, false).is_err());
    /// ```
    fn call_mfa(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        name: &str,
        arity: u32,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        match (module, name, arity) {
            ("erlang", "apply", 2) => return self.apply_fun(scheduler, tail),
            ("erlang", "apply", 3) => return self.apply_mfa(scheduler, tail),
            _ => {}
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
        match self.export_frame(module, name, arity)? {
            Some(frame) => {
                check_remote_call_arguments(&process.registers, arity)?;
                enter_frame(process, frame, tail);
                Ok(None)
            }
            None => self.undefined_function(scheduler, module, name, arity, tail),
        }
    }

    /// Call a function that a loaded module does not export, with the
    /// arguments in `x0` to `x(arity - 1)`. ERTS goes to the export entry of
    /// the function, which calls `error_handler:undefined_function/3` with
    /// the module, the function, and the argument list `Args`
    /// (`call_error_handler` in `beam_common.c:1294-1342`; JIT
    /// `emit_export_trampoline` in `jit/arm/beam_asm_global.cpp:164-232`).
    /// For a loaded module, `undefined_function/3` calls
    /// `call_undefined_function_handler/3` (`error_handler.erl:85-98`).
    /// PBeam does the result of these functions here and does not run
    /// `error_handler.beam` (see "Undefined functions" in
    /// `docs/erts-differences.md`):
    ///
    /// - When the module exports `'$handle_undefined_function'/2`, enter it
    ///   with `x0 = Function` and `x1 = Args`, as the tail call
    ///   `Module:Handler(Func, Args)` does (`error_handler.erl:206-213`). A
    ///   tail call replaces the active frame.
    /// - Else raise `error:undef` with the stack trace
    ///   `[{Module, Function, Args, []}]`, as `crash/3` and `crash/1` do
    ///   (`error_handler.erl:165-178`). ERTS puts the caller frames after
    ///   this frame. PBeam records no frames.
    ///
    /// Return the outcome when the process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `undef_handler:missing(m)` enters the handler with x0 = missing, x1 = [m].
    /// assert!(program.undefined_function(&mut scheduler, "undef_handler", "missing", 1, false)?.is_none());
    /// // `undef_target:missing(a)` with no catch tag exits with
    /// // `{undef, [{undef_target, missing, [a], []}]}`.
    /// let outcome = program.undefined_function(&mut scheduler, "undef_target", "missing", 1, false)?;
    /// assert!(matches!(outcome, Some(Outcome::Exit { result: None, .. })));
    /// ```
    ///
    /// # Errors
    ///
    /// A module or function name that the atom table of the run does not
    /// hold is a runtime error. Each name comes from an import or an atom
    /// term of the program, so the table holds it.
    fn undefined_function(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        function: &str,
        arity: u32,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        let atom = |name: &str| {
            scheduler
                .atoms
                .get(name)
                .map(Term::Atom)
                .ok_or_else(|| error(format!("atom table does not hold {name}")))
        };
        let (module_atom, function_atom) = (atom(module)?, atom(function)?);
        let process = scheduler.current_process_mut();
        let arguments = Term::List(call_arguments(&process.registers, arity)?);
        if let Some(handler) = self.export_frame(module, "$handle_undefined_function", 2)? {
            enter_frame(process, handler, tail);
            write(process, &Operand::X(0), function_atom)?;
            write(process, &Operand::X(1), arguments)?;
            return Ok(None);
        }
        let frame = Term::Tuple(vec![
            module_atom,
            function_atom,
            arguments,
            Term::List(Vec::new()),
        ]);
        let undef = Term::static_atom("undef");
        Ok(raise_with_stacktrace(
            process,
            ExceptionClass::Error,
            undef,
            Term::List(vec![frame]),
        ))
    }

    /// Run `erlang:apply(Module, Function, Arguments)` with the arguments in
    /// `x0` to `x2`, as ERTS `apply` (`beam_common.c:1434-1526`) does for the
    /// `i_apply` instructions. The function and the module must be atoms,
    /// and `Arguments` must be a list, else the call raises `error:badarg`.
    /// An apply of `erlang:apply/3` continues with its inner call. A list of
    /// 1024 or more elements raises `error:system_limit`, also when the list
    /// is improper, because ERTS counts the elements before it reads the
    /// tail. Then an improper list raises `error:badarg`. Else the elements
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
        let process = scheduler.current_process_mut();
        let mut call = call_arguments(&process.registers, 3)?;
        let (module, name, arguments, proper) = loop {
            // Each wrong form raises the same `badarg`, so one pattern checks
            // the function, the module, and the list.
            let [Term::Atom(module), Term::Atom(name), list] = &mut call[..] else {
                let badarg = Term::static_atom("badarg");
                return Ok(raise(process, ExceptionClass::Error, badarg));
            };
            let Some((arguments, proper)) = take_list_elements(list) else {
                let badarg = Term::static_atom("badarg");
                return Ok(raise(process, ExceptionClass::Error, badarg));
            };
            if !proper
                || (module.name(), name.name()) != ("erlang", "apply")
                || arguments.len() != 3
            {
                break (module.clone(), name.clone(), arguments, proper);
            }
            call = arguments;
        };
        if arguments.len() > APPLY_MAX_ARGUMENTS {
            let system_limit = Term::static_atom("system_limit");
            return Ok(raise(process, ExceptionClass::Error, system_limit));
        }
        if !proper {
            let badarg = Term::static_atom("badarg");
            return Ok(raise(process, ExceptionClass::Error, badarg));
        }
        let arity = arguments.len() as u32;
        for (index, argument) in (0..).zip(arguments) {
            write(process, &Operand::X(index), argument)?;
        }
        self.call_mfa(scheduler, module.name(), name.name(), arity, tail)
    }

    /// Run `erlang:apply(Fun, Arguments)` with the arguments in `x0` and
    /// `x1`, as the JIT `emit_apply_fun_shared` (`jit/arm/instr_fun.cpp:253-328`)
    /// does for the `i_apply_fun` instructions. `Arguments` must be a list,
    /// else the call raises `error:badarg`. A list of 1023 or more elements
    /// raises `error:system_limit`, also when the list is improper, because
    /// the JIT counts the elements before it reads the tail. Then an improper
    /// list raises `error:badarg`. Else the elements go to `x0` onwards, and
    /// the call continues as `call_fun` does, so a term that is not a fun
    /// raises `badfun` only after these checks. Return the outcome when the
    /// process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // x0 = fun(X) -> {a, X} end, x1 = [x] enters the lambda with x0 = x.
    /// assert!(program.apply_fun(&mut scheduler, false)?.is_none());
    /// // x0 = notfun, x1 = notalist raises `error:badarg`.
    /// assert!(matches!(program.apply_fun(&mut scheduler, true)?, Some(Outcome::Exit { .. })));
    /// ```
    fn apply_fun(
        &self,
        scheduler: &mut Scheduler,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        let process = scheduler.current_process_mut();
        let fun = read(process, &Operand::X(0))?;
        let mut list = read(process, &Operand::X(1))?;
        let Some((arguments, proper)) = take_list_elements(&mut list) else {
            let badarg = Term::static_atom("badarg");
            return Ok(raise(process, ExceptionClass::Error, badarg));
        };
        if arguments.len() > APPLY_FUN_MAX_ARGUMENTS {
            let system_limit = Term::static_atom("system_limit");
            return Ok(raise(process, ExceptionClass::Error, system_limit));
        }
        if !proper {
            let badarg = Term::static_atom("badarg");
            return Ok(raise(process, ExceptionClass::Error, badarg));
        }
        let arity = arguments.len() as u32;
        for (index, argument) in (0..).zip(arguments) {
            write(process, &Operand::X(index), argument)?;
        }
        self.call_fun(scheduler, &fun, arity, tail)
    }

    /// Call `fun` with the arguments in `x0` to `x(arity - 1)`, as ERTS
    /// `call_fun` (`beam_common.c:1605-1731`) does for `call_fun` and
    /// `call_fun2`. A term that is not a fun raises `error:{badfun, Term}`. A
    /// fun of another arity raises `error:{badarity, {Fun, Arguments}}`,
    /// where `Arguments` lists `x0` to `x(arity - 1)`. Else a local fun puts
    /// its free variables in `x(arity)` onwards and enters the code of the
    /// lambda, and an external fun calls its function as `call_ext` does, so
    /// a BIF runs as a BIF. A tail call replaces the active frame. Return the
    /// outcome when the process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // x0 = one and fun(X) -> {Tag, X} end enters the lambda with x1 = Tag.
    /// assert!(program.call_fun(&mut scheduler, &pair, 1, false)?.is_none());
    /// // fun erlang:self/0 runs the `self/0` BIF.
    /// assert!(program.call_fun(&mut scheduler, &self_fun, 0, false)?.is_none());
    /// // An atom raises `error:{badfun, notfun}`.
    /// assert!(program.call_fun(&mut scheduler, &Term::static_atom("notfun"), 0, false)?.is_some());
    /// ```
    fn call_fun(
        &self,
        scheduler: &mut Scheduler,
        fun: &Term,
        arity: u32,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        let process = scheduler.current_process_mut();
        let fun = match fun {
            Term::Fun(fun) if fun.target().arity() == arity => fun,
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
        let lambda = match fun.target() {
            Target::Local(lambda) => lambda,
            Target::External {
                module, function, ..
            } => return self.call_mfa(scheduler, module.name(), function.name(), arity, tail),
        };
        for (register, value) in (arity..).zip(fun.free()) {
            write(process, &Operand::X(register), value.clone())?;
        }
        let total_arity = arity + fun.free().len() as u32;
        let frame = self.frame(lambda.module().name(), lambda.label(), total_arity)?;
        enter_frame(process, frame, tail);
        Ok(None)
    }

    /// Finish a BIF or NIF call with its result, as the ERTS `nif_bif.epilogue`
    /// does for BIFs and NIFs (`emu/bif_instrs.tab:604-636`; JIT
    /// `emit_bif_nif_epilogue` in `jit/arm/instr_bif.cpp:631-719`). A value
    /// goes to x0, and a tail call then returns from the frame. An exception
    /// goes to the error handler. A trap writes its arguments to the X
    /// registers and enters the target function. An exit ends the process,
    /// and no catch tag sees it. Return the outcome when the process exits.
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
            BifResult::Exited(reason) => Ok(Some(Outcome::Exit {
                reason,
                result: None,
            })),
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

/// Take the elements of a list term for an apply, and tell whether the list
/// is proper. `[]` gives no elements. Return `None` for a term that is not a
/// list. The elements of an improper list come before its tail, so a caller
/// can count them first, as ERTS `apply` does.
///
/// # Examples
///
/// ```rust,ignore
/// let mut list = Term::ImproperList(vec![Term::Integer(1)], Box::new(Term::static_atom("x")));
/// assert_eq!(take_list_elements(&mut list), Some((vec![Term::Integer(1)], false)));
/// assert_eq!(take_list_elements(&mut Term::List(vec![])), Some((vec![], true)));
/// assert_eq!(take_list_elements(&mut Term::static_atom("x")), None);
/// ```
fn take_list_elements(list: &mut Term) -> Option<(Vec<Term>, bool)> {
    match list {
        Term::List(elements) => Some((std::mem::take(elements), true)),
        Term::ImproperList(elements, _) => Some((std::mem::take(elements), false)),
        _ => None,
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
    /// Start the new process with the `message_queue_data` flag `off_heap`
    /// (`SPO_OFF_HEAP_MSGQ`), else `on_heap`.
    off_heap_message_queue: bool,
}

/// Start a process that runs `module:function` with the elements of
/// `arguments`, as ERTS `erl_create_process` does. Return the pid, or
/// `{Pid, Reference}` when `options.monitor` is set. A `module` or
/// `function` that is not an atom, and `arguments` that are not a proper
/// list, raise `badarg` (`erl_process.c:12490-12492`). A function that a
/// loaded module does not export starts the process anyway: the process
/// calls `undefined_function` when it first runs. More than 1023 arguments
/// also start the process, which then raises `error:system_limit`, as the
/// `apply` of the ERTS `i_apply_only` start does (`beam_common.c:1501-1513`).
/// Else a module that is not loaded is the runtime error
/// `unresolved external MFA M:F/A`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = create_process(&program, &mut scheduler, &module, &function, &Term::List(vec![]), SpawnOptions::default())?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// let result = create_process(&program, &mut scheduler, &module, &function, &Term::Integer(1), SpawnOptions::default())?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// let result = create_process(&program, &mut scheduler, &Term::Integer(1), &function, &Term::List(vec![]), SpawnOptions::default())?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
fn create_process(
    program: &BeamProgram,
    scheduler: &mut Scheduler,
    module: &Term,
    function: &Term,
    arguments: &Term,
    options: SpawnOptions,
) -> Result<BifResult, BeamError> {
    let (Term::Atom(module), Term::Atom(function), Term::List(arguments)) =
        (module, function, arguments)
    else {
        return Ok(BifResult::badarg());
    };
    let arguments = arguments.clone();
    let arity = u32::try_from(arguments.len()).map_err(|_| error("arity exceeds BEAM limit"))?;
    // A function that the loaded module does not export gives an undefined
    // start, which goes to `undefined_function` when the process first
    // runs, as the `i_apply_only` start of ERTS goes to its export entry.
    // More than `APPLY_MAX_ARGUMENTS` arguments also give an undefined start,
    // which raises `system_limit` when the process first runs. The `apply`
    // of ERTS checks this limit before it looks up the export, so the module
    // can be not loaded.
    let export = if arguments.len() > APPLY_MAX_ARGUMENTS {
        None
    } else {
        program.export_frame(module.name(), function.name(), arity)?
    };
    let start = match export {
        Some(frame) => Start::Entry(frame),
        None => Start::Undefined {
            module: module.clone(),
            function: function.clone(),
        },
    };
    let SpawnOptions {
        link,
        monitor,
        off_heap_message_queue,
    } = options;
    let (pid, reference) =
        scheduler.spawn(start, arguments, monitor, link, off_heap_message_queue)?;
    Ok(BifResult::Value(match reference {
        Some(reference) => Term::Tuple(vec![Term::Pid(pid), Term::Reference(reference)]),
        None => Term::Pid(pid),
    }))
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

/// Release the Y registers of the active frame before a tail call, as the
/// ERTS `deallocate` does. The frame must have `deallocate` Y registers.
///
/// # Examples
///
/// ```rust,ignore
/// // The frame has one Y register.
/// release_frame(&mut process, 1)?;
/// assert!(current_frame(&mut process).stack.is_empty());
/// assert!(release_frame(&mut process, 1).is_err());
/// ```
fn release_frame(process: &mut ProcessState, deallocate: u32) -> Result<(), BeamError> {
    let stack = &mut current_frame(process).stack;
    if stack.len() != deallocate as usize {
        return Err(error("invalid BEAM tail-call stack size"));
    }
    stack.clear();
    Ok(())
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

/// Replace the value of each existing key of a map, as ERTS
/// `erts_gc_update_map_exact` does (`beam_common.c:2203-2345`). The pairs go
/// in the flatmap key order, as the ERTS loader sorts them, so the first key
/// that the map does not hold is the first missing key in that order. Return
/// the new map, or the exception reason `{badmap, Map}` for a term that is
/// not a map and `{badkey, Key}` for a missing key. A key of a type that a
/// PBeam map cannot hold, for example a tuple, is a missing key.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(Term::static_atom("output"), Term::static_atom("raw"))])?);
/// let cooked = vec![(Term::static_atom("output"), Term::static_atom("cooked"))];
/// assert!(matches!(update_map_exact(map, cooked.clone())?, Ok(Term::Map(_))));
/// let badmap = update_map_exact(Term::static_atom("foo"), cooked)?.unwrap_err();
/// assert_eq!(badmap, Term::Tuple(vec![Term::static_atom("badmap"), Term::static_atom("foo")]));
/// ```
///
/// # Errors
///
/// Return a runtime error when the key order depends on two different
/// references (`Map::sort_pairs`).
fn update_map_exact(
    mut map: Term,
    mut pairs: Vec<(Term, Term)>,
) -> Result<Result<Term, Term>, BeamError> {
    let Term::Map(entries) = &mut map else {
        return Ok(Err(Term::Tuple(vec![Term::static_atom("badmap"), map])));
    };
    Map::sort_pairs(&mut pairs)?;
    for (key, value) in pairs {
        let Some(existing) = entries.get_mut(&key) else {
            return Ok(Err(Term::Tuple(vec![Term::static_atom("badkey"), key])));
        };
        *existing = value;
    }
    Ok(Ok(map))
}

/// Read the value of each key in the map of a `get_map_elements` or
/// `has_map_fields` instruction, or return `None` at the first key that the
/// map does not hold.
///
/// # Examples
///
/// ```rust,ignore
/// // x0 holds #{output => raw}.
/// let output = Operand::Value(Term::static_atom("output"));
/// assert_eq!(map_values(&process, &Operand::X(0), [&output])?, Some(vec![Term::static_atom("raw")]));
/// let input = Operand::Value(Term::static_atom("input"));
/// assert_eq!(map_values(&process, &Operand::X(0), [&output, &input])?, None);
/// ```
///
/// # Errors
///
/// Return a runtime error when the source is not a map. The compiler puts an
/// `is_map` test before each of these instructions.
fn map_values<'a>(
    process: &ProcessState,
    map: &Operand,
    keys: impl IntoIterator<Item = &'a Operand>,
) -> Result<Option<Vec<Term>>, BeamError> {
    let map = read(process, map)?;
    let Term::Map(map) = &map else {
        return Err(error("BEAM get_map_elements source is not a map"));
    };
    keys.into_iter()
        .map(|key| Ok(map.get(&read(process, key)?).cloned()))
        .collect()
}

/// Raise an exception with the stack trace `[]` in the process, as
/// `raise_with_stacktrace` does.
///
/// # Examples
///
/// ```rust,ignore
/// let outcome = raise(&mut process, ExceptionClass::Exit, Term::static_atom("badarg"));
/// assert!(matches!(outcome, Some(Outcome::Exit { result: None, .. })));
/// ```
fn raise(process: &mut ProcessState, class: ExceptionClass, reason: Term) -> Option<Outcome> {
    raise_with_stacktrace(process, class, reason, Term::List(Vec::new()))
}

/// Raise an exception in the process, as ERTS `handle_error` does: continue
/// at the nearest live catch tag, or return the exit outcome when no frame
/// holds one.
///
/// The scan starts at the newest frame and, in one frame, at the lowest Y
/// index, as ERTS `next_catch` scans up from the stack top. The frames above
/// the tag are dropped, and the tag becomes `[]`. The handler gets the
/// non-value in `x0`, the reason in `x1`, the stack trace in `x2`, and the
/// class atom in `x3` (`beam_common.c:543`). ERTS keeps a raw stack trace
/// in `x2` that `build_stacktrace` and `catch_end` change to a list
/// (`beam_common.c:730-735`, `1204-1292`). PBeam keeps the list. An
/// uncaught `error` exits the process with `{Reason, Stacktrace}`
/// (`beam_common.c:684-692`), and an uncaught `exit` exits it with
/// `Reason`.
///
/// # Examples
///
/// ```rust,ignore
/// let frame = Term::Tuple(vec![module, function, Term::List(vec![]), Term::List(vec![])]);
/// let undef = Term::static_atom("undef");
/// let outcome = raise_with_stacktrace(&mut process, ExceptionClass::Error, undef, Term::List(vec![frame]));
/// // With no catch tag, the exit reason is `{undef, [Frame]}`.
/// assert!(matches!(outcome, Some(Outcome::Exit { result: None, .. })));
/// ```
fn raise_with_stacktrace(
    process: &mut ProcessState,
    class: ExceptionClass,
    reason: Term,
    stacktrace: Term,
) -> Option<Outcome> {
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
            ExceptionClass::Error => Term::Tuple(vec![reason, stacktrace]),
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
    process.registers[2] = Some(stacktrace);
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

/// Return the head and the tail of a nonempty proper or improper list,
/// which `get_list`, `get_hd`, and `get_tl` read as the `CAR` and the `CDR`
/// of the first cons cell (emu `instrs.tab:389-407`; JIT
/// `jit/arm/instr_common.cpp:274-318`). The BEAM validator makes sure that
/// the source is a nonempty list.
///
/// # Examples
///
/// ```rust,ignore
/// let x = || Term::static_atom("x");
/// let list = Term::List(vec![Term::Integer(1), Term::Integer(2)]);
/// assert_eq!(nonempty_list(list)?, (Term::Integer(1), Term::List(vec![Term::Integer(2)])));
/// let improper = Term::ImproperList(vec![Term::Integer(1)], Box::new(x()));
/// assert_eq!(nonempty_list(improper)?, (Term::Integer(1), x()));
/// assert!(nonempty_list(Term::List(vec![])).is_err());
/// ```
fn nonempty_list(mut list: Term) -> Result<(Term, Term), BeamError> {
    let (mut values, tail) = match &mut list {
        Term::List(values) if !values.is_empty() => {
            (std::mem::take(values), Term::List(Vec::new()))
        }
        Term::ImproperList(values, tail) => (
            std::mem::take(values),
            std::mem::replace(&mut **tail, Term::List(Vec::new())),
        ),
        _ => {
            return Err(error(format!(
                "BEAM list source {list:?} is not a nonempty list"
            )));
        }
    };
    // ponytail: the tail is a copy of the rest, O(n) with the `Vec` list model of #71.
    let head = values.remove(0);
    Ok((head, Term::list_with_tail(values, tail)))
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

/// Read an operand from the process registers or the active frame. A
/// match context is a runtime invariant error: only the binary-match
/// instructions read it, with `read_any` or `match_context`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read(&process, &Operand::Value(Term::Integer(1)))?, Term::Integer(1));
/// ```
fn read(process: &ProcessState, operand: &Operand) -> Result<Term, BeamError> {
    match read_any(process, operand)? {
        Term::MatchContext(_) => Err(match_context_escape(operand)),
        value => Ok(value),
    }
}

/// Read an operand from the process registers or the active frame, also
/// when it is a match context.
///
/// # Examples
///
/// ```rust,ignore
/// write(&mut process, &Operand::X(0), Term::MatchContext(MatchContext::new(b"a")))?;
/// assert!(matches!(read_any(&process, &Operand::X(0))?, Term::MatchContext(_)));
/// ```
fn read_any(process: &ProcessState, operand: &Operand) -> Result<Term, BeamError> {
    let stack = process.frames.last().map_or(&[][..], |frame| &frame.stack);
    read_operand(operand, &process.registers, stack)
}

/// Read the match context of a binary-match instruction. Each other term
/// is a runtime invariant error: the BEAM validator gives these
/// instructions only a match context.
///
/// # Examples
///
/// ```rust,ignore
/// write(&mut process, &Operand::X(0), Term::MatchContext(MatchContext::new(b"a")))?;
/// assert_eq!(match_context(&process, &Operand::X(0))?.position(), 0);
/// assert!(match_context(&process, &Operand::Value(Term::Integer(1))).is_err());
/// ```
fn match_context(process: &ProcessState, operand: &Operand) -> Result<MatchContext, BeamError> {
    match &read_any(process, operand)? {
        Term::MatchContext(context) => Ok(context.clone()),
        other => Err(error(format!(
            "invariant error: {operand:?} holds {other:?}, not a match context"
        ))),
    }
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
        .map(
            |index| match registers.get(index as usize).and_then(Clone::clone) {
                Some(Term::MatchContext(_)) => Err(match_context_escape(&Operand::X(index))),
                Some(value) => Ok(value),
                None => Err(error(format!("call argument x{index} is unset"))),
            },
        )
        .collect()
}

/// Check that the X registers of the arguments of a local call are set. A
/// local call keeps each argument in its register. Thus a match context
/// can go to the callee, as the `call_only` of `prim_tty:is_usascii/1`
/// gives its match context in `x0` to the next `bs_start_match3`.
///
/// # Examples
///
/// ```rust,ignore
/// let registers = vec![Some(Term::MatchContext(MatchContext::new(b"a")))];
/// assert!(check_local_call_arguments(&registers, 1).is_ok());
/// assert!(check_local_call_arguments(&registers, 2).is_err());
/// ```
fn check_local_call_arguments(registers: &[Option<Term>], arity: u32) -> Result<(), BeamError> {
    match (0..arity).find(|&index| !matches!(registers.get(index as usize), Some(Some(_)))) {
        Some(index) => Err(error(format!("call argument x{index} is unset"))),
        None => Ok(()),
    }
}

/// Check that the X registers of the arguments of an external call to a
/// loaded function are set and hold no match context. The BEAM validator
/// gives an external call no match context (`verify_remote_args_1` and
/// `get_term_type`, beam_validator.erl:1944-1951, 2903-2908).
///
/// # Examples
///
/// ```rust,ignore
/// let registers = vec![Some(Term::MatchContext(MatchContext::new(b"a")))];
/// assert!(check_remote_call_arguments(&registers, 1).is_err());
/// assert!(check_remote_call_arguments(&[Some(Term::Integer(1))], 1).is_ok());
/// ```
fn check_remote_call_arguments(registers: &[Option<Term>], arity: u32) -> Result<(), BeamError> {
    check_local_call_arguments(registers, arity)?;
    match (0..arity).find(|&index| matches!(registers[index as usize], Some(Term::MatchContext(_))))
    {
        Some(index) => Err(match_context_escape(&Operand::X(index))),
        None => Ok(()),
    }
}

/// Return the runtime invariant error of a match context in `operand` at an
/// instruction that is not a binary match or a local call. The BEAM
/// validator also lets `move` and `swap` copy a match context
/// (beam_validator.erl:403-407, 2690-2693), which PBeam does not implement
/// yet (row "Binary matching" of docs/erts-differences.md).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(
///     match_context_escape(&Operand::X(0)).to_string(),
///     "invariant error: the match context in X(0) reached an instruction that is not a binary match"
/// );
/// ```
fn match_context_escape(operand: &Operand) -> BeamError {
    error(format!(
        "invariant error: the match context in {operand:?} reached an instruction that is not a binary match"
    ))
}

/// Run the commands of a `bs_match` instruction in order on a match
/// context, as the instructions that the ERTS loader makes from them do
/// (`gen.bs_match`, emu generators.tab:1102-1430). Return `false` at the
/// first command that fails. `ensure_at_least` loads as `i_bs_ensure_bits`,
/// `i_bs_ensure_bits_unit`, `i_bs_test_unit8`, or `i_bs_test_unit`
/// (generators.tab:1164-1209; bs_instrs.tab:760-772, 1260-1293), which all
/// test that `Size` bits remain and that the bits after them are a multiple
/// of `Unit`. `ensure_exactly` loads as `bs_test_tail2` (generators.tab:
/// 1210-1220; ops.tab:1220-1222; bs_instrs.tab:746-758), which tests that
/// exactly `Size` bits remain. Each other command gives a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let context = MatchContext::new(b"a");
/// let unit = NonZeroU64::new(8).unwrap();
/// assert!(binary_match(&context, &[BinaryMatchCommand::EnsureAtLeast { size: 0, unit }])?);
/// assert!(!binary_match(&context, &[BinaryMatchCommand::EnsureExactly { size: 0 }])?);
/// ```
fn binary_match(
    context: &MatchContext,
    commands: &[BinaryMatchCommand],
) -> Result<bool, BeamError> {
    let remaining = context.remaining_bits() as u64;
    for command in commands {
        let passed = match command {
            BinaryMatchCommand::EnsureAtLeast { size, unit } => {
                remaining >= *size && (remaining - size).is_multiple_of(unit.get())
            }
            BinaryMatchCommand::EnsureExactly { size } => remaining == *size,
            BinaryMatchCommand::Unsupported(name) => {
                return Err(error(format!(
                    "unsupported bs_match command {}",
                    name.name()
                )));
            }
        };
        if !passed {
            return Ok(false);
        }
    }
    Ok(true)
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
        Operand::UnsupportedInteger => Err(error(
            "unsupported BEAM integer operand outside the 64-bit range: needs a bignum",
        )),
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
        Operand::Value(_) | Operand::UnsupportedInteger => {
            return Err(error("invalid BEAM move destination"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, process::Command};

    use super::bif::{BIFS, INSTRUCTION_CALLS};
    use super::{
        BeamProgram, Outcome, Scheduler, call_arguments, function_at_label, read_operand,
        write_operand,
    };
    use crate::{
        BeamModule, Map, Term,
        beam::code::{self, Instruction, Operand, TestKind},
        term::{Atom, AtomTable, Lambda, test_atom},
    };

    /// The runtime error of a root process that exits with an uncaught
    /// `error:badarg`. PBeam gives the stack trace `[]`.
    const BADARG_EXIT: &str = "root process exited with {badarg,[]}";

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
    fn calls_an_external_fun_of_a_missing_function_as_call_ext() {
        // A call of an external fun goes to the export entry of its
        // function, as `call_ext` does (`beam_common.c:1605-1731`). ERTS 29.1.1
        // raises `undef` for `(erlang:make_fun(missing, run, 0))()`; PBeam
        // gives the runtime error of the row "Exception raise sites".
        let program = make_program(
            vec![
                Instruction::Move(atom("missing"), Operand::X(0)),
                Instruction::Move(atom("run"), Operand::X(1)),
                Instruction::Move(Operand::Value(Term::Integer(0)), Operand::X(2)),
                erlang_call("make_fun", 3, false),
                Instruction::CallFun {
                    arity: 0,
                    function: Operand::X(0),
                    tail: false,
                    deallocate: 0,
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
            "unresolved external MFA missing:run/0"
        );
    }

    #[test]
    fn replaces_the_caller_frame_for_a_fun_call_in_a_tail_position() {
        // `fun_probe:tail_wait/1` ends with `call_fun 0`, `deallocate 1`, and
        // `return`, which ERTS runs as `i_call_fun_last`
        // (`jit/arm/ops.tab:868`). The lambda replaces the frame of
        // `tail_wait/1`, so the process that waits in the lambda has one
        // frame, and a loop of tail fun calls uses constant stack, as in ERTS.
        let program =
            BeamProgram::load(&[include_bytes!("../../fixtures/funs/fun_probe.beam")]).unwrap();
        let label = program.modules["fun_probe"].exports[&("tail_wait".to_owned(), 1)];
        let mut scheduler = Scheduler::new(program.atoms.clone());
        let frame = program.frame("fun_probe", label, 1).unwrap();
        scheduler.start(frame, vec![Term::Integer(1)]).unwrap();
        scheduler.schedule_next().unwrap();
        assert!(matches!(
            program.run(&mut scheduler).unwrap(),
            Outcome::Wait
        ));
        let frames = &scheduler.current_process_mut().frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].function, "-tail_wait/1-fun-0-");
    }

    #[test]
    fn requires_the_frame_size_for_a_fun_call_in_a_tail_position() {
        // `i_call_fun_last` releases exactly the Y registers of the frame.
        let program = make_program(
            vec![
                Instruction::Move(atom("notfun"), Operand::X(0)),
                Instruction::CallFun {
                    arity: 0,
                    function: Operand::X(0),
                    tail: true,
                    deallocate: 1,
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
            "invalid BEAM tail-call stack size"
        );
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
                    tail: false,
                    deallocate: 0,
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
            "root process exited with {{badfun,notfun},[]}"
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
            .contains("root process exited with boom")
        );
        assert!(
            error(vec![
                Instruction::Move(atom("undef"), Operand::X(0)),
                erlang_call("nif_error", 1, false),
            ])
            .contains("root process exited with {undef,[]}")
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
        // ERTS 29.1.1: spawn_link(M, F, 1) raises badarg.
        assert_eq!(error(spawn_link_child(Term::Integer(1))), BADARG_EXIT);
        // `erlang:is_alive/0` is not a BIF: it runs the code of the `erlang`
        // module, which this program does not load.
        assert!(
            error(vec![erlang_call("is_alive", 0, false)])
                .contains("unresolved external MFA erlang:is_alive/0")
        );
        // The ERTS loader turns this call into an instruction. Its body in
        // `erlang.beam` calls itself, so it must not run.
        assert!(
            error(vec![erlang_call("yield", 0, false)])
                .contains("unsupported instruction call erlang:yield/0")
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
    fn rejects_invalid_tuple_access_receives_and_bifs() {
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
    fn tests_map_keys_with_has_map_fields_and_writes_no_register() {
        let map = Operand::Value(Term::Map(
            Map::from_pairs(vec![
                (test_atom("input"), test_atom("cooked")),
                (Term::Integer(1), test_atom("one")),
            ])
            .unwrap(),
        ));
        let run = |keys: Vec<Operand>, map: Operand| {
            make_program(
                vec![
                    Instruction::Move(Operand::Value(Term::Integer(1)), Operand::X(1)),
                    Instruction::Move(atom("none"), Operand::X(0)),
                    Instruction::HasMapFields {
                        fail_label: 3,
                        map,
                        keys,
                    },
                    Instruction::Return,
                    Instruction::Label(3),
                    Instruction::Move(atom("missing"), Operand::X(0)),
                    Instruction::Return,
                ],
                0,
            )
            .execute("fixture", "main", &[])
        };
        // All keys present: x0 keeps its value.
        assert_eq!(
            run(vec![atom("input"), Operand::X(1)], map.clone()).unwrap(),
            test_atom("none")
        );
        assert_eq!(
            run(vec![atom("input"), atom("output")], map).unwrap(),
            test_atom("missing")
        );
        assert!(
            run(vec![atom("input")], atom("raw"))
                .unwrap_err()
                .to_string()
                .contains("get_map_elements source is not a map")
        );
    }

    #[test]
    fn raises_or_jumps_when_put_map_exact_gets_no_map_or_no_key() {
        // erlc puts an `is_map` test before each `put_map_exact`, so compiled
        // code does not reach this case. ERTS still raises `{badmap, Map}`
        // with fail label 0 and jumps to another fail label
        // (`beam_common.c:2231-2234`).
        let run = |fail_label, map| {
            make_program(
                vec![
                    Instruction::PutMapExact {
                        fail_label,
                        map,
                        destination: Operand::X(0),
                        pairs: vec![(atom("output"), atom("cooked"))],
                    },
                    Instruction::Return,
                    Instruction::Label(3),
                    Instruction::Move(atom("failed"), Operand::X(0)),
                    Instruction::Return,
                ],
                0,
            )
            .execute("fixture", "main", &[])
        };
        assert_eq!(
            run(0, atom("foo")).unwrap_err().to_string(),
            "root process exited with {{badmap,foo},[]}"
        );
        assert_eq!(run(3, atom("foo")).unwrap(), test_atom("failed"));
        // A key of a type that a PBeam map cannot hold is a missing key.
        let tuple_key = |fail_label| {
            make_program(
                vec![
                    Instruction::PutMapExact {
                        fail_label,
                        map: Operand::Value(Term::Map(Map::new())),
                        destination: Operand::X(0),
                        pairs: vec![(Operand::Value(Term::Tuple(vec![])), atom("cooked"))],
                    },
                    Instruction::Return,
                    Instruction::Label(3),
                    Instruction::Move(atom("failed"), Operand::X(0)),
                    Instruction::Return,
                ],
                0,
            )
            .execute("fixture", "main", &[])
        };
        assert_eq!(
            tuple_key(0).unwrap_err().to_string(),
            "root process exited with {{badkey,{}},[]}"
        );
        assert_eq!(tuple_key(3).unwrap(), test_atom("failed"));
        // Two different references have no PBeam order.
        let references = make_program(
            vec![Instruction::PutMapExact {
                fail_label: 0,
                map: Operand::Value(Term::Map(Map::new())),
                destination: Operand::X(0),
                pairs: vec![
                    (
                        Operand::Value(Term::Reference(crate::Reference(1))),
                        atom("a"),
                    ),
                    (
                        Operand::Value(Term::Reference(crate::Reference(2))),
                        atom("b"),
                    ),
                ],
            }],
            0,
        )
        .execute("fixture", "main", &[]);
        assert!(
            references
                .unwrap_err()
                .to_string()
                .contains("unsupported term order of two references")
        );
    }

    #[test]
    fn reports_a_maps_merge_result_key_that_the_map_form_does_not_support() {
        // A flatmap accepts the integer 2^59, and a hashmap does not yet.
        let pairs = |keys: std::ops::RangeInclusive<i64>| {
            let pairs = keys.map(|key| (Term::Integer(key), Term::Integer(0)));
            Term::Map(Map::from_pairs(pairs.collect()).unwrap())
        };
        let merge = make_program(
            vec![
                Instruction::Move(Operand::Value(pairs(1 << 59..=1 << 59)), Operand::X(0)),
                Instruction::Move(Operand::Value(pairs(1..=32)), Operand::X(1)),
                Instruction::CallExternal {
                    arity: 2,
                    module: "maps".into(),
                    name: "merge".into(),
                    tail: true,
                    deallocate: 0,
                },
            ],
            0,
        )
        .execute("fixture", "main", &[]);
        assert_eq!(
            merge.unwrap_err().to_string(),
            "unsupported map key Integer(576460752303423488)"
        );
    }

    #[test]
    fn reports_a_maps_from_list_key_that_the_map_form_does_not_support() {
        // ERTS accepts a tuple key; a PBeam map does not hold one yet.
        let pair = Term::Tuple(vec![Term::Tuple(vec![]), Term::Integer(0)]);
        let from_list = make_program(
            vec![
                Instruction::Move(Operand::Value(Term::List(vec![pair])), Operand::X(0)),
                Instruction::CallExternal {
                    arity: 1,
                    module: "maps".into(),
                    name: "from_list".into(),
                    tail: true,
                    deallocate: 0,
                },
            ],
            0,
        )
        .execute("fixture", "main", &[]);
        assert_eq!(
            from_list.unwrap_err().to_string(),
            "unsupported map key Tuple([])"
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
                    name: "keysearch".into(),
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
                exports: HashMap::from([(("keysearch".into(), 3), 1)]),
                instructions: vec![
                    Instruction::FunctionInfo {
                        module: "lists".into(),
                        name: "keysearch".into(),
                        arity: 3,
                    },
                    Instruction::Label(1),
                    Instruction::Move(atom("stub"), Operand::X(0)),
                    Instruction::Return,
                ],
            },
        );
        // `lists:keysearch/3` is in the ERTS 29.1.1 `bif.tab`. ERTS runs the
        // BIF, so the BEAM stub body of the loaded `lists` module must not run.
        assert_eq!(
            program
                .execute("fixture", "main", &[])
                .unwrap_err()
                .to_string(),
            "unresolved external MFA lists:keysearch/3"
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
        // yield/0 as builtin.
        for mfa in INSTRUCTION_CALLS {
            assert!(builtin.contains(&mfa));
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
        // ERTS 29.1.1 raises badarg for a term that is not chardata.
        assert_eq!(
            convert(Term::Integer(1), "unicode")
                .unwrap_err()
                .to_string(),
            BADARG_EXIT
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
    fn builds_a_proper_and_an_improper_list() {
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

        // ERTS: [10 | 0] is the improper list [10|0].
        let improper = make_program(
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
        assert_eq!(
            improper.execute("fixture", "main", &[]).unwrap(),
            Term::ImproperList(vec![Term::Integer(10)], Box::new(Term::Integer(0)))
        );
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
        // ERTS: lst([a|x]) is {a,x}, lst([a,b|x]) is {a,[b|x]}, and
        // hd1([x|y]) is x.
        let improper = |names: &[&str], tail: &str| {
            Term::ImproperList(
                names.iter().map(|name| test_atom(name)).collect(),
                Box::new(test_atom(tail)),
            )
        };
        assert_eq!(
            get_list(improper(&["a"], "x")).unwrap(),
            Term::Tuple(vec![test_atom("a"), test_atom("x")])
        );
        assert_eq!(
            get_list(improper(&["a", "b"], "x")).unwrap(),
            Term::Tuple(vec![test_atom("a"), improper(&["b"], "x")])
        );
        assert_eq!(get_head(improper(&["x"], "y")).unwrap(), test_atom("x"));
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
        // ERTS: nil([a|x]) is nonempty.
        assert_eq!(classify(improper(&["a"], "x")), test_atom("nonempty"));
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

    /// ERTS 29.1.1 `beam_jit_wait_timeout` (`jit/beam_jit_common.cpp:1208-1240`):
    /// `infinity` waits as `wait` does, so the `DOWN` message of a monitored
    /// child continues the receive at its label. `0` continues at once at
    /// `timeout`. A value that is not valid and that no `try` catches exits
    /// the process with `{timeout_value, Stack}`.
    #[test]
    fn waits_or_continues_at_a_receive_timeout_and_raises_timeout_value() {
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
                Instruction::RemoveMessage,
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
        assert_eq!(wait(Term::Integer(0)).unwrap(), test_atom("timeout"));
        assert_eq!(
            wait(test_atom("foo")).unwrap_err().to_string(),
            "root process exited with {timeout_value,[]}"
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

    /// PBeam lets a match context reach only the binary-match instructions
    /// and local calls. It gives a runtime invariant error for each other
    /// use, and for a binary-match instruction that gets another term.
    #[test]
    fn gives_an_invariant_error_for_a_match_context_outside_binary_matching() {
        let run = |rest: Vec<Instruction>| {
            let mut body = vec![
                Instruction::Move(Operand::Value(Term::Binary(b"a".to_vec())), Operand::X(0)),
                Instruction::BinaryMatchStart {
                    fail_label: 1,
                    source: Operand::X(0),
                    destination: Operand::X(0),
                },
            ];
            body.extend(rest);
            // `child/1` does not read its argument.
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
                Instruction::Move(atom("ok"), Operand::X(0)),
                Instruction::Return,
            ]);
            program
                .execute("fixture", "main", &[])
                .unwrap_err()
                .to_string()
        };
        let escape = "invariant error: the match context in X(0) reached an instruction that \
                      is not a binary match";
        assert_eq!(
            run(vec![Instruction::Move(Operand::X(0), Operand::X(1))]),
            escape
        );
        assert_eq!(run(vec![erlang_call("hd", 1, false)]), escape);
        // An external call to a loaded function also checks its arguments.
        assert_eq!(
            run(vec![Instruction::CallExternal {
                arity: 1,
                module: "fixture".into(),
                name: "child".into(),
                tail: false,
                deallocate: 0,
            }]),
            escape
        );
        // A reuse in another register would share the position in ERTS.
        assert_eq!(
            run(vec![Instruction::BinaryMatchStart {
                fail_label: 1,
                source: Operand::X(0),
                destination: Operand::X(1),
            }]),
            "unsupported bs_start_match3 that copies the match context in X(0) to X(1)"
        );
        assert_eq!(
            run(vec![Instruction::BinarySetPosition {
                context: Operand::X(0),
                position: atom("a"),
            }]),
            "invariant error: bs_set_position position Atom(\"a\") is not a position"
        );
        assert_eq!(
            run(vec![Instruction::BinaryGetUtf8 {
                fail_label: 1,
                context: atom("a"),
                destination: Operand::X(1),
            }]),
            "invariant error: Value(Atom(\"a\")) holds Atom(\"a\"), not a match context"
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
    fn selects_tuple_arity_and_rejects_call_last_stack_size() {
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
    }

    #[test]
    fn puts_a_new_atom_into_the_atom_table_of_the_run() {
        // ERTS has one atom table for each emulator, and
        // `erlang:list_to_atom/1` gives a new atom the next index
        // (`erts_atom_put`). A run starts with a copy of the table of the
        // loaded modules, so `pbeam_alpha` gets index 1151 after
        // `pbeam_zeta` (1150). The table of the loaded modules does not
        // change, so each run gives the same index.
        let mut program = make_program(
            vec![
                Instruction::Move(
                    Operand::Value(Term::List(
                        "pbeam_alpha"
                            .bytes()
                            .map(|byte| Term::Integer(i64::from(byte)))
                            .collect(),
                    )),
                    Operand::X(0),
                ),
                erlang_call("list_to_atom", 1, true),
            ],
            0,
        );
        assert_eq!(program.atoms.put("pbeam_zeta").unwrap().index(), 1150);
        for _ in 0..2 {
            assert!(matches!(
                program.execute("fixture", "main", &[]).unwrap(),
                Term::Atom(ref alpha) if alpha.name() == "pbeam_alpha" && alpha.index() == 1151
            ));
        }
        assert_eq!(program.atom("pbeam_alpha"), None);
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
                .contains("root process exited with {undef,[]}")
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
        // monitor(process, Child), then demonitor(Ref, [flush]) before the
        // child runs: the monitor is still active, so ERTS returns true
        // without flushing.
        let mut body = spawn_link_child(Term::List(vec![test_atom("normal")]));
        body.pop();
        body.push(erlang_call("spawn", 3, false));
        body.extend([
            Instruction::Move(Operand::X(0), Operand::X(1)),
            Instruction::Move(atom("process"), Operand::X(0)),
            erlang_call("monitor", 2, false),
            Instruction::Move(
                Operand::Value(Term::List(vec![test_atom("flush")])),
                Operand::X(1),
            ),
            erlang_call("demonitor", 2, true),
        ]);
        assert_eq!(run_with_child(body).unwrap(), test_atom("true"));
    }

    /// Run `main/0` with the functions `zero/0`, `id/3`, and `last/2` of
    /// the probe that gives the ERTS values of the `apply` tests:
    /// `zero() -> zero.`, `id(A, B, C) -> {A, B, C}.`, and
    /// `last(M, F) -> M:F(bye).`, which compiles to `{apply_last,1,0}`.
    fn run_with_apply_targets(body: Vec<Instruction>) -> Result<Term, crate::BeamError> {
        apply_targets_program(body).execute("fixture", "main", &[])
    }

    /// Return the program of `run_with_apply_targets`.
    fn apply_targets_program(body: Vec<Instruction>) -> BeamProgram {
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
        program
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
        // calls `erlang:apply/2`. ERTS 29.1.1 raises `badarg` for
        // `apply(erlang, apply, [fixture, id])`, because `id` is not a list.
        let mut body = apply_of(
            test_atom("erlang"),
            test_atom("apply"),
            Term::List(vec![test_atom("fixture"), test_atom("id")]),
        );
        body.push(erlang_call("apply", 3, true));
        assert_eq!(
            run_with_apply_targets(body).unwrap_err().to_string(),
            BADARG_EXIT
        );
        // `apply` copies at most 1023 arguments to the X registers. ERTS
        // raises `undef` for `apply(fixture, missing, L1023)`, because the
        // loaded module `fixture` does not export `missing/1023`, and raises
        // `system_limit` for a list of 1024 elements.
        let mut body = apply_of(
            test_atom("fixture"),
            test_atom("missing"),
            Term::List(vec![test_atom("x"); 1023]),
        );
        body.push(erlang_call("apply", 3, true));
        let mut program = apply_targets_program(body);
        // The atoms of the `undef` frame come from the atom table of the run.
        assert_eq!(
            program
                .execute("fixture", "main", &[])
                .unwrap_err()
                .to_string(),
            "atom table does not hold fixture"
        );
        for name in ["fixture", "missing"] {
            program.atoms.put(name).unwrap();
        }
        assert_eq!(
            program
                .execute("fixture", "main", &[])
                .unwrap_err()
                .to_string(),
            format!(
                "root process exited with {{undef,[{{fixture,missing,[{}],[]}}]}}",
                ["x"; 1023].join(",")
            )
        );
        let mut body = apply_of(
            test_atom("fixture"),
            test_atom("missing"),
            Term::List(vec![test_atom("x"); 1024]),
        );
        body.push(erlang_call("apply", 3, true));
        assert_eq!(
            run_with_apply_targets(body).unwrap_err().to_string(),
            "root process exited with {system_limit,[]}"
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
            append(integers(&[1]), test_atom("x")).unwrap(),
            Term::ImproperList(vec![Term::Integer(1)], Box::new(test_atom("x")))
        );
        for right in [Term::List(Vec::new()), test_atom("x")] {
            assert_eq!(
                append(test_atom("a"), right).unwrap_err().to_string(),
                BADARG_EXIT
            );
        }
    }

    #[test]
    fn reverses_a_list_onto_a_tail_that_is_not_a_list() {
        // ERTS 29.1.1: lists:reverse([a], x) is the improper list [a|x].
        let reverse = call_ext_with(
            "lists",
            "reverse",
            vec![Term::List(vec![test_atom("a")]), test_atom("x")],
        );
        assert_eq!(
            reverse.unwrap(),
            Term::ImproperList(vec![test_atom("a")], Box::new(test_atom("x")))
        );
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
    fn rejects_is_ge_and_is_lt_on_two_different_references() {
        // ERTS orders two references by the scheduler thread that made each
        // one; this is the "Term order" row in `docs/erts-differences.md`.
        let reference = |number| Operand::Value(Term::Reference(crate::Reference(number)));
        let is_ge = Instruction::TestGreaterEqual {
            fail_label: 3,
            left: reference(1),
            right: reference(2),
        };
        let is_lt = Instruction::TestLess {
            fail_label: 3,
            left: reference(1),
            right: reference(2),
        };
        for test in [is_ge, is_lt] {
            let body = vec![test, Instruction::Label(3), Instruction::Return];
            assert_eq!(
                make_program(body, 0)
                    .execute("fixture", "main", &[])
                    .unwrap_err()
                    .to_string(),
                "unsupported term order of two references Reference(Reference(1)) and \
                 Reference(Reference(2))"
            );
        }
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
