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

use bif::{BifResult, INSTRUCTION_CALLS, is_bif, is_function, is_heavy_bif, send};

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
    /// The process switches out at an ERTS switch point, for example at the
    /// end of its reduction budget. It stays runnable and continues where
    /// the switch point leaves its frame.
    Yield,
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
    /// spawns run on the same scheduler. A process runs until it waits,
    /// calls `erlang:hibernate/0`, exits, or switches out at an ERTS switch
    /// point, for example at the end of its reduction budget of
    /// `CONTEXT_REDS`. A process that switches out goes to the end of the
    /// run queue. Before it takes the next process, the scheduler reads the
    /// run clock and expires each due timer, as ERTS bumps the timers in
    /// `check_activities_to_run` (`erl_process.c:9797-9810`). When no
    /// process can run, it sleeps until the first timer end, as
    /// `scheduler_wait` sleeps until the next timeout
    /// (`erl_process.c:3452-3620`). The run ends at the next scheduling step
    /// after the root process exits, also when another process can run or a
    /// timer is active. A process that ends the root process with an exit
    /// signal continues until it waits, exits, or switches out. This is as
    /// `halt()` after `-eval` stops the ERTS node
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
                    Outcome::Yield => scheduler.yield_current(),
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
    /// `erlang:hibernate/0`, exits, or switches out. A process with an
    /// undefined start first calls `undefined_function` with its start MFA.
    /// A process that switched out at the `return` of its last frame has no
    /// frame. It exits now with the value in `x0`, as ERTS continues at the
    /// normal exit (`beam_normal_exit`).
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
        let process = scheduler.current_process_mut();
        if process.frames.is_empty() {
            return normal_exit(process);
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
                    // ERTS `send` calls `erl_send` as a light BIF and writes
                    // its result to x0, or raises (`emu/bif_instrs.tab:425-454`;
                    // JIT `emit_send` in `jit/arm/instr_bif.cpp:612-625`).
                    if light_bif_switch(process) {
                        return Ok(Outcome::Yield);
                    }
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
                Instruction::TestYield => {
                    // `emit_i_test_yield` (`jit/arm/instr_common.cpp:3097-3119`).
                    process.fcalls -= 1;
                    if process.fcalls <= 0 {
                        return Ok(Outcome::Yield);
                    }
                }
                Instruction::Return => {
                    // `emit_dispatch_return` (`jit/arm/instr_call.cpp:49-54`)
                    // switches after the return, so the process continues at
                    // the caller, or at the normal exit.
                    process.fcalls -= 1;
                    let exit = return_from_frame(process)?;
                    if process.fcalls < 0 {
                        return Ok(Outcome::Yield);
                    }
                    if let Some(outcome) = exit {
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
    /// apply does not see the catch tags of the frame.
    ///
    /// A BIF runs as a body call also in a tail call, and the process then
    /// continues at the `Deallocate` and the `Return` that the loader puts
    /// after a tail call, as `call_light_bif Bif | deallocate D | return`
    /// and `i_call_ext Bif | deallocate D | return` do
    /// (`jit/arm/ops.tab:687-714`). A light BIF, which is not in
    /// `HEAVY_BIFS`, first costs one reduction. When the budget is then 0
    /// or less, the process switches out before the BIF and runs this
    /// instruction again (`emit_call_light_bif_shared` in
    /// `jit/arm/instr_bif.cpp:399-400`, `583-592`). A heavy BIF goes through
    /// its export, which costs no reduction (`emit_call_bif_shared` in
    /// `jit/arm/instr_bif.cpp:721-789`). A call of `erlang:load_nif/2` runs
    /// as the `i_load_nif` instruction with the module of the current frame
    /// (`jit/arm/ops.tab:615-623`, `emu/ops.tab:854-862`). Return the
    /// outcome when the process stops.
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
            return self.finish_bif(scheduler.current_process_mut(), result, false);
        }
        if is_bif(module, name, arity) {
            if !is_heavy_bif(module, name, arity) && light_bif_switch(process) {
                return Ok(Some(Outcome::Yield));
            }
            let arguments = call_arguments(&process.registers, arity)?;
            let result = self.bif(scheduler, module, name, arguments)?;
            return self.finish_bif(scheduler.current_process_mut(), result, false);
        }
        self.call_mfa(scheduler, module, name, arity, tail)
    }

    /// Call `module:name/arity` through its export with the arguments in
    /// the X registers, as `apply`, a fun call, and `call_ext` of Erlang
    /// code do. The calls of `erlang:apply/2` and `erlang:apply/3` run as
    /// the ERTS `i_apply_fun` and `i_apply` instructions. A BIF runs before
    /// the module code, as in ERTS. Through its export, a BIF costs no
    /// reduction, and a tail call of it returns to the caller with no
    /// reduction (`emit_call_bif_mfa` and `emit_bif_nif_epilogue` in
    /// `jit/arm/instr_bif.cpp:631-674`, `819-839`). A function that a loaded
    /// module does not export goes to `undefined_function`. Return the
    /// outcome when the process stops.
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
    /// goes to x0, and a tail call then returns from the frame with no
    /// reduction. A yield return writes its value to x0, does the return
    /// of a tail call, and switches out the process. An exception goes to
    /// the error handler. A trap writes its arguments to the X registers,
    /// enters the target function, and switches out the process. An exit ends the process, and no catch
    /// tag sees it. Return the outcome when the process stops.
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
            BifResult::YieldReturn(value) => {
                write(process, &Operand::X(0), value)?;
                if tail {
                    // The process exits at its next run when it returned
                    // from its last frame.
                    return_from_frame(process)?;
                }
                Ok(Some(Outcome::Yield))
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
                // A BIF trap always switches out the process
                // (`context_switch_simplified` in `jit/arm/instr_bif.cpp:525-536`,
                // `676-701`). The process enters the target when it runs
                // again, so the entry costs one reduction of the next turn.
                Ok(Some(Outcome::Yield))
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
    read(process, &Operand::X(0)).map_err(|_| error("return before setting x0"))?;
    process.frames.pop();
    if !process.frames.is_empty() {
        return Ok(None);
    }
    normal_exit(process).map(Some)
}

/// Return the normal exit of a process that returned from its last frame,
/// with the value in `x0` as its result.
///
/// # Examples
///
/// ```rust,ignore
/// write(&mut process, &Operand::X(0), Term::static_atom("ok"))?;
/// assert!(matches!(normal_exit(&process)?, Outcome::Exit { result: Some(_), .. }));
/// ```
fn normal_exit(process: &ProcessState) -> Result<Outcome, BeamError> {
    let result = read_operand(&Operand::X(0), &process.registers, &[])
        .map_err(|_| error("return before setting x0"))?;
    Ok(Outcome::Exit {
        reason: Term::static_atom("normal"),
        result: Some(result),
    })
}

/// Use one reduction for a light BIF, as `emit_call_light_bif_shared` does
/// before the BIF (`jit/arm/instr_bif.cpp:399-400`). Return true when the
/// budget is then 0 or less: the process must switch out before the BIF,
/// and it runs the instruction again when it runs again
/// (`jit/arm/instr_bif.cpp:583-592`). The instruction then costs one
/// reduction again.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 2;
/// assert!(!light_bif_switch(&mut process));
/// assert!(light_bif_switch(&mut process));
/// ```
fn light_bif_switch(process: &mut ProcessState) -> bool {
    process.fcalls -= 1;
    if process.fcalls > 0 {
        return false;
    }
    current_frame(process).next_instruction -= 1;
    true
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
mod tests;
