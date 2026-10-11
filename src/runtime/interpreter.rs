use super::{
    capability::{Policy, Restriction},
    host::output::IovecWrite,
    process::{DirtyWrite, Frame, PendingBif, ProcessState, ReceiveTimer, StackSlot, Start},
    scheduler::{PolicyEnd, Received, Scheduler, Turn},
    tty::{InitialMode, Ttys},
};
use crate::{
    beam::{
        BeamModule,
        code::{self, BinaryMatchCommand, BinarySegment, Instruction, Operand, TestKind},
    },
    error::{BeamError, error},
    term::{
        Atom, AtomTable, Fun, Map, MatchContext, Target, Term, print_term, segment_equals,
        segment_integer,
    },
};
use std::{borrow::Cow, cell::RefCell, collections::HashMap, rc::Rc};

mod bif;
mod bits;
mod code_table;
mod native;

pub(super) use code_table::CodeTable;

use bif::{BifResult, is_bif, is_function, is_heavy_bif, send};
use bits::BitWriter;

/// The largest number of arguments of `erlang:apply/3`. ERTS `apply` copies
/// at most `MAX_REG - 1` arguments to the X registers (`erl_vm.h:55`,
/// `beam_common.c:1501-1513`), so a longer list raises `system_limit`.
const APPLY_MAX_ARGUMENTS: usize = 1023;

/// The largest number of arguments of `erlang:apply/2`. The JIT stops at
/// `MAX_REG - 1` arguments, because the loader keeps the highest X register
/// (`erl_vm.h:55`, `jit/arm/instr_fun.cpp:290-297`), so a longer list
/// raises `system_limit`.
const APPLY_FUN_MAX_ARGUMENTS: usize = 1022;

/// The bits that ERTS copies for one reduction when it puts a binary in a
/// new binary (`ERL_BITS_PER_REDUCTION`, erl_bits.h:41).
const BITS_PER_REDUCTION: usize = 8 * 1024;

#[derive(Debug)]
pub struct BeamProgram {
    /// The start modules by name. Each run starts its code table with them
    /// (see [`CodeTable`]).
    modules: HashMap<String, Rc<BeamModule>>,
    /// The atom table of the loaded modules. ERTS has one atom table for
    /// each emulator, so each run starts its own table from a copy of it.
    atoms: AtomTable,
}

/// The issuer of the initial capabilities of a run.
#[derive(Clone, Copy)]
enum Issuer<'a> {
    /// No issuer: the launcher has no policy, so the run skips each
    /// capability check and issues no capability.
    Unchecked,
    /// A Rust policy of the host.
    Host(&'a dyn Policy),
    /// A policy module in BEAM code, which the policy process runs.
    Module {
        /// The name of the policy module.
        policy: &'a str,
        /// True in boot mode: the root process starts with the boot
        /// authority and enters the runtime with
        /// `pbeam_cap:enter_runtime/2`.
        boot: bool,
    },
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
    /// the switch point leaves its frame. A process that a signal ended in
    /// `loop_rec` also switches out this way, and it exits in its next turn.
    Yield,
    /// The process exited. `result` is the return value of a normal exit.
    Exit { reason: Term, result: Option<Term> },
    /// The process waits for its NIF call `write` on a dirty IO thread.
    /// `tail` tells whether the NIF call was a tail call.
    DirtyWrite { write: IovecWrite, tail: bool },
}

/// The class of an exception.
#[derive(Debug, Clone, Copy)]
enum ExceptionClass {
    /// Class `error`, as `erlang:nif_error/1` raises.
    Error,
    /// Class `exit`, as `erlang:exit/1` raises.
    Exit,
    /// Class `throw`, as `erlang:throw/1` raises.
    Throw,
}

impl ExceptionClass {
    /// Return the atom name of the class, as `try_case` puts it in `x0`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(ExceptionClass::Error.name(), "error");
    /// assert_eq!(ExceptionClass::Throw.name(), "throw");
    /// ```
    fn name(self) -> &'static str {
        match self {
            ExceptionClass::Error => "error",
            ExceptionClass::Exit => "exit",
            ExceptionClass::Throw => "throw",
        }
    }

    /// Return the class of the atom `error`, `exit`, or `throw`, and `None`
    /// for each other term.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(ExceptionClass::from_term(&Term::static_atom("exit")), Some(ExceptionClass::Exit)));
    /// assert!(ExceptionClass::from_term(&Term::static_atom("foo")).is_none());
    /// ```
    fn from_term(term: &Term) -> Option<Self> {
        match term {
            Term::Atom(atom) => match atom.name() {
                "error" => Some(ExceptionClass::Error),
                "exit" => Some(ExceptionClass::Exit),
                "throw" => Some(ExceptionClass::Throw),
                _ => None,
            },
            _ => None,
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
            if loaded.insert(name.clone(), Rc::new(module)).is_some() {
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
    /// (`erl_process.c:3452-3620`). When fd 0 has a read select, a port
    /// waits for a pipe, a dirty IO call runs, or a signal handler is
    /// installed, it waits for fd 0, for the pipes, for the completion of the
    /// call, and for the handled host signals until then (see
    /// `Scheduler::wait_events`). Before each scheduling step, it
    /// sends `{notify, Signal}` to `erl_signal_server` for each handled
    /// signal that arrived (see `Scheduler::check_signals`), and a process
    /// whose dirty IO call completed goes to the run queue (see
    /// `Scheduler::complete_dirty_writes`). The run ends at
    /// the next scheduling step after the root process exits, also when
    /// another process can run or a timer is active. A process that ends
    /// the root process with an exit signal continues until it waits,
    /// exits, or switches out. This is as
    /// `halt()` after `-eval` stops the ERTS node
    /// (`erlang.erl:3357-3358`) and as `init` halts after a boot process
    /// exit that is not `normal` (`init.erl:710-712`). A root process exit
    /// gives the error `root process exited with <Reason>`, with the reason
    /// as ERTS prints a term (`print_term`), for example
    /// `root process exited with {badarg,[]}`. A root process that
    /// waits when no process can run, no timer is active, fd 0 has no read
    /// select, no port waits for a pipe, no dirty IO call runs, and no
    /// signal handler is installed gives an error. When the run ends, it
    /// does not wait for a dirty IO call that runs, and a call in the queue
    /// does not run (see `DirtyIo`). A call of `erlang:halt/2` with a
    /// status code ends the run at once with the error
    /// [`BeamError::Halt`]. At the end of the
    /// run, also after an error, fd 0 gets its mode at the start of the run
    /// again when fds 0 and 1 are terminals, as at the exit of the ERTS
    /// emulator (see `InitialMode::reset`).
    ///
    /// The run has no policy, so it skips each capability check, as if each
    /// effect entry were `ambient`, and each process can do each operation,
    /// as on ERTS. No process gets a capability copy. The `{capabilities,
    /// Map}` spawn option still raises `badarg` for a `Map` that is not
    /// valid, but it does not reduce the child. `pbeam_cap:revoke/2`,
    /// `pbeam_cap:policy_revoke/2`, and `pbeam_cap:request/2` return `ok`
    /// after their argument checks and have no effect. The option is fixed for the run: no BIF can change
    /// it. Use [`BeamProgram::execute_with_policy`] or
    /// [`BeamProgram::execute_with_policy_module`] for a run with checks.
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
        self.execute_issued(module, function, arguments, Issuer::Unchecked)
    }

    /// Run an exported function as the root process, as
    /// [`BeamProgram::execute`] does, with the initial capabilities that
    /// `policy` gives the root process. An effect entry that the
    /// capabilities of the caller do not permit raises
    /// `error:capability_denied` before it has an effect (row "Capability
    /// checks" of `docs/erts-differences.md`). The column "Capability" of
    /// `docs/effect-inventory.md` lists the entries that check a capability
    /// now; the entries marked `ambient` have no check by decision. With
    /// [`AllowAll`](crate::AllowAll), each check passes, as a test of the
    /// checks needs. Before each
    /// scheduling step, the launcher revokes the initial capabilities of each
    /// kind that [`Policy::revocations`] gives.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let result = program.execute_with_policy("process_probe", "main", &[], &AllowAll)?;
    /// ```
    pub fn execute_with_policy(
        &self,
        module: &str,
        function: &str,
        arguments: &[Term],
        policy: &dyn Policy,
    ) -> Result<Term, BeamError> {
        self.execute_issued(module, function, arguments, Issuer::Host(policy))
    }

    /// Run an exported function as the root process, as
    /// [`BeamProgram::execute`] does, with the policy module `policy`, a
    /// loaded BEAM module in any BEAM language. The module exports
    /// `initial/1` and `grant/3`.
    ///
    /// Before the root process, the launcher makes the policy process, a
    /// trusted process outside each supervisor tree. It calls
    /// `policy:initial(Requester)` with the requester map of the root
    /// process, `#{pid => <0.0.0>, parent => none, initial_call => {Module,
    /// Function, Arity}}`. The value is a list of capability maps, each
    /// with the keys `kind`, `operations`, `delegable`, and the constraint
    /// field of the kind, with the values of the `{capabilities, Map}` spawn
    /// option. The root process gets these capabilities. A call of
    /// `pbeam_cap:request(Kind, Request)` in a process waits until the
    /// policy process calls `policy:grant(Kind, Request, Requester)` with
    /// the requester map of the process. A `{grant, Capability}` value adds
    /// the capability to the process through a grant signal, and the call
    /// returns `ok`. Each other value, an exception, and a call that uses
    /// more than 100000 reductions deny the request: the call raises
    /// `error:capability_denied`. The policy code runs with all
    /// capabilities.
    ///
    /// With `boot`, the run is in boot mode (row "Boot authority" of
    /// `docs/erts-differences.md`). The root process starts at
    /// once with the boot authority: each of its checks passes without
    /// copies, and each process that it spawns without `{capabilities,
    /// Map}` is a boot process too. A call of
    /// `pbeam_cap:enter_runtime(Module, Function)` in the root process
    /// calls `policy:initial(Requester)` with `#{pid => <0.0.0>, parent =>
    /// none, initial_call => {Module, Function, 0}}`, and the root process
    /// replaces its boot authority with the capabilities of the value, one
    /// way. A process that is not a boot process gets
    /// `error:capability_denied` for a message, a link, an exit signal, a
    /// group leader change, an inspection, or `erlang:unregister/1` that
    /// targets a boot process. A monitor of a boot process and a message to
    /// an alias of a boot process are permitted.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let result = program.execute_with_policy_module("policy_probe", "main", &[], "erl_policy", false)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`BeamProgram::execute`]. A policy module that is not
    /// loaded or that does not export `initial/1` or `grant/3` gives the
    /// error `missing BEAM module M` or `missing export M:F/A`. When the
    /// call of `initial/1` does not give a list of capability maps, raises
    /// an exception, uses more than 100000 reductions, or ends with the
    /// policy process, the run gives the error `the policy denied the root
    /// process: <cause>`, and the root process does not start. When the
    /// call waits and no process can run, the run gives the error `the
    /// policy process waits in initial/1`. In boot mode, the denial stops
    /// the run when the call of `pbeam_cap:enter_runtime/2` ends, and a
    /// call that waits gives the error of a root process that waits. A
    /// boot process that is not the root process gives a runtime error
    /// for `pbeam_cap:enter_runtime/2`.
    pub fn execute_with_policy_module(
        &self,
        module: &str,
        function: &str,
        arguments: &[Term],
        policy: &str,
        boot: bool,
    ) -> Result<Term, BeamError> {
        let issuer = Issuer::Module { policy, boot };
        self.execute_issued(module, function, arguments, issuer)
    }

    /// Run an exported function as the root process with the initial
    /// capabilities of `issuer`, as [`BeamProgram::execute_with_policy`] and
    /// [`BeamProgram::execute_with_policy_module`] describe.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let result = program.execute_issued("process_probe", "main", &[], Issuer::Host(&AllowAll))?;
    /// ```
    fn execute_issued(
        &self,
        module: &str,
        function: &str,
        arguments: &[Term],
        issuer: Issuer,
    ) -> Result<Term, BeamError> {
        let initial_mode = InitialMode::read();
        let mut scheduler = match issuer {
            Issuer::Unchecked => Scheduler::without_checks(self.atoms.clone()),
            Issuer::Host(_) | Issuer::Module { .. } => Scheduler::new(self.atoms.clone()),
        };
        scheduler.code = CodeTable::new(self.modules.clone());
        let run = self.run_root(&mut scheduler, module, function, arguments, issuer);
        initial_mode.reset(scheduler.resources.borrow_mut::<Ttys>().input_selected());
        run?;
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

    /// Run `Module:Function(Arguments...)` as the root process with the
    /// initial capabilities of `issuer` on `scheduler` until the run ends, as
    /// `BeamProgram::execute` describes. `scheduler` holds the code table of
    /// the run (see `CodeTable`). With no issuer, the root process
    /// gets no capability, and `scheduler` must skip the checks (see
    /// `Scheduler::without_checks`). With a Rust policy, the launcher
    /// revokes the initial capabilities of each kind that
    /// `Policy::revocations` gives before each scheduling step. With a
    /// policy module, the launcher makes the policy process first and the
    /// root process when `initial/1` returns. In boot mode, it makes both at
    /// once, the root process with the boot authority, and sends the value
    /// of `initial/1` to the root process (see `Scheduler::enter_runtime`).
    /// Before each scheduling step, it starts the call of the next request
    /// when the policy process waits (see `Scheduler::next_policy_request`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut scheduler = Scheduler::new(program.atoms.clone());
    /// scheduler.code = CodeTable::new(program.modules.clone());
    /// program.run_root(&mut scheduler, "process_probe", "main", &[], Issuer::Host(&AllowAll))?;
    /// assert!(scheduler.into_root_result().is_some());
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of `BeamProgram::execute`, except the root process
    /// results, and the errors of the policy module of
    /// `BeamProgram::execute_with_policy_module`.
    fn run_root(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        function: &str,
        arguments: &[Term],
        issuer: Issuer,
    ) -> Result<(), BeamError> {
        let arity =
            u32::try_from(arguments.len()).map_err(|_| error("arity exceeds BEAM limit"))?;
        let label = scheduler.code.entry_label(module, function, arity)?;
        let frame = scheduler.code.frame(module, label, arity)?;
        let mut root = match issuer {
            Issuer::Unchecked => {
                scheduler.start(frame, arguments.to_vec(), Vec::new())?;
                None
            }
            Issuer::Host(policy) => {
                scheduler.start(frame, arguments.to_vec(), policy.initial())?;
                None
            }
            Issuer::Module { policy, boot } => {
                scheduler.code.entry_label(policy, "grant", 3)?;
                let initial = scheduler.code.entry_label(policy, "initial", 1)?;
                let initial = scheduler.code.frame(policy, initial, 1)?;
                if boot {
                    scheduler.start_policy(initial, None)?;
                    scheduler.start_boot(frame, arguments.to_vec());
                    None
                } else {
                    let root = (module, function, arguments.len());
                    scheduler.start_policy(initial, Some(root))?;
                    Some(frame)
                }
            }
        };
        while !scheduler.root_exited() {
            if let Some(initial) = scheduler.take_policy_initial() {
                let capabilities = initial.map_err(|cause| {
                    error(format!("the policy denied the root process: {cause}"))
                })?;
                match root.take() {
                    Some(frame) => scheduler.start(frame, arguments.to_vec(), capabilities)?,
                    None => scheduler.enter_runtime(capabilities),
                }
            }
            let now = scheduler.timers.now();
            scheduler.expire_timers(now);
            scheduler.check_signals()?;
            match issuer {
                Issuer::Unchecked => {}
                Issuer::Host(policy) => scheduler.revoke_issued(&policy.revocations()),
                Issuer::Module { policy, .. } => {
                    if let Some((function, arguments)) = scheduler.next_policy_request() {
                        let arity = arguments.len() as u32;
                        let frame = scheduler.code.mfa_frame(policy, function, arity)?;
                        scheduler.begin_policy_call(frame, arguments);
                    }
                }
            }
            scheduler.complete_dirty_writes();
            match scheduler.schedule_next()? {
                // A message woke the policy process, but it has no call.
                Turn::Run if scheduler.policy_idle() => scheduler.park_current(),
                Turn::Run => {
                    let outcome = self.run(scheduler)?;
                    match policy_turn(scheduler, outcome) {
                        Outcome::Wait => scheduler.park_current(),
                        Outcome::Hibernate => scheduler.hibernate_current(),
                        Outcome::Yield => scheduler.yield_current(),
                        Outcome::Exit { reason, result } => {
                            scheduler.finish_current(reason, result)
                        }
                        Outcome::DirtyWrite { write, tail } => {
                            scheduler.start_dirty_write(write, tail)
                        }
                    }
                }
                Turn::Done => {}
                Turn::Idle => {
                    let wait = scheduler.timers.wait_time(scheduler.timers.now());
                    if !scheduler.wait_events(wait)? {
                        match wait {
                            Some(wait) => std::thread::sleep(wait),
                            None => break,
                        }
                    }
                }
            }
        }
        if root.is_some() {
            return Err(error("the policy process waits in initial/1"));
        }
        Ok(())
    }

    /// Run the current process until it waits for a message, calls
    /// `erlang:hibernate/0`, exits, or switches out. A process with an
    /// undefined start first calls `undefined_function` with its start MFA.
    /// A process with a pending BIF first finishes the BIF call through its
    /// export (see [`PendingBif`]).
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
        // A NIF call that ran on a dirty IO thread returns its result or
        // raises its exception, as `dirty_nif_finalizer` and
        // `dirty_nif_exception` do (`erl_nif.c:3386-3435`).
        if let Some(DirtyWrite {
            tail,
            written: Some(written),
            ..
        }) = scheduler.current_process_mut().dirty_write.take()
        {
            let result = native::dirty_write_result(&mut scheduler.atoms, written)?;
            if let Some(outcome) = self.finish_bif(scheduler, result, tail)? {
                return Ok(outcome);
            }
        }
        // A BIF that trapped to a BIF export continues through the export,
        // and then the process continues after the call.
        if let Some(PendingBif {
            module,
            function,
            arguments,
            tail,
        }) = scheduler.current_process_mut().pending_bif.take()
        {
            let result = self.bif_trap(scheduler, module, function, arguments)?;
            if let Some(outcome) = self.finish_bif(scheduler, result, tail)? {
                return Ok(outcome);
            }
        }
        let process = scheduler.current_process_mut();
        if process.frames.is_empty() {
            return normal_exit(process);
        }
        // A frame names its module, and the code of a module name does not
        // change in a run (see `CodeTable`). Thus the loop keeps the module
        // of the last instruction and looks up the code table only when the
        // active frame is in another module.
        let mut current = Rc::clone(scheduler.current_module()?);
        loop {
            let process = scheduler.current_process_mut();
            let frame = current_frame(process);
            if frame.module != current.name {
                current = Rc::clone(scheduler.current_module()?);
                continue;
            }
            let module: &BeamModule = &current;
            let Some(instruction) = module.instructions.get(frame.next_instruction) else {
                return Err(error(format!(
                    "function {}:{} did not return",
                    frame.module, frame.function
                )));
            };
            frame.next_instruction += 1;
            match instruction {
                // ERTS `move` copies the word of the source to the
                // destination (`emit_i_move`, jit/arm/instr_common.cpp:
                // 516-519). The BEAM validator lets it copy a match
                // context, and the two registers then name one value
                // (`assign_1`, beam_validator.erl:403-404, 2690-2693). The
                // copy of a match context term shares the match context.
                // `Terra.Input.Parser.do_parse/3` saves its match context
                // in a Y register with `move`.
                Instruction::Move(source, destination) => {
                    let value = read_any(process, source)?;
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
                    let frame = code_table::frame(module, *label, *arity)?;
                    process.frames.push(frame);
                }
                Instruction::CallOnly { arity, label } => {
                    check_local_call_arguments(&process.registers, *arity)?;
                    *current_frame(process) = code_table::frame(module, *label, *arity)?;
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
                    *current_frame(process) = code_table::frame(module, *label, *arity)?;
                }
                Instruction::TestArity {
                    fail_label,
                    value,
                    arity,
                } => {
                    let passed = matches!(
                        read_ref(process, value)?,
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
                    let result = is_function(read_ref(process, value)?, read_ref(process, arity)?);
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
                    let target = match read_ref(process, value)? {
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
                    let call = FunCall::new(read_ref(process, function)?, *arity);
                    if *tail {
                        // `i_call_fun_last` releases the frame before it
                        // tests the fun (`emit_i_call_fun2_last` in
                        // `jit/arm/instr_fun.cpp:425-462`).
                        release_frame(process, *deallocate)?;
                    }
                    if let Some(outcome) = self.call_fun(scheduler, call, *arity, *tail)? {
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
                    // A `bif` instruction with fail label 0 raises the
                    // exception of the BIF, as `handle_error` does in ERTS
                    // (`bif_instrs.tab`). Another fail label jumps to the
                    // label and raises nothing (`$FAIL($Fail)` in
                    // `i_bif.call`).
                    //
                    // The guard BIF `length/1` runs as `i_length`
                    // (`jit/arm/ops.tab:1329-1333`). At a trap, it keeps
                    // its trap state, switches out, and runs again when the
                    // process runs again (`jit/arm/instr_bif.cpp:253-272`).
                    let result = if let Some(state) = process.length_trap.take() {
                        self.bif_trap(scheduler, "erlang", "length", state)?
                    } else {
                        let values = arguments
                            .iter()
                            .map(|argument| read_any(process, argument))
                            .collect::<Result<Vec<_>, _>>()?;
                        match match_context_byte_size(bif_module, name, arguments, &values)? {
                            Some(size) => BifResult::Value(size),
                            None => self.bif(scheduler, bif_module, name, values)?,
                        }
                    };
                    let process = scheduler.current_process_mut();
                    match result {
                        BifResult::Value(value) => write(process, destination, value)?,
                        BifResult::TrapBif {
                            module: "erlang",
                            function: "length",
                            arguments,
                        } => {
                            process.length_trap = Some(arguments);
                            current_frame(process).next_instruction -= 1;
                            return Ok(Outcome::Yield);
                        }
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
                    // the handler. A normal return always sets x0. A throw
                    // gives its reason, an error gives `{'EXIT', {Reason,
                    // Stack}}`, and an exit gives `{'EXIT', Reason}` (emu
                    // `instrs.tab:1187-1212`).
                    if matches!(process.registers.first(), Some(None)) {
                        let class = read_ref(process, &Operand::X(3))?;
                        let throw = *class == Term::static_atom("throw");
                        let error_class = *class == Term::static_atom("error");
                        let mut reason = read(process, &Operand::X(1))?;
                        if !throw {
                            if error_class {
                                let (_, stack) =
                                    raw_stacktrace(read_ref(process, &Operand::X(2))?)?;
                                reason = Term::Tuple(vec![reason, stack]);
                            }
                            reason = Term::Tuple(vec![Term::static_atom("EXIT"), reason]);
                        }
                        write(process, &Operand::X(0), reason)?;
                    }
                }
                Instruction::Test {
                    kind,
                    fail_label,
                    value,
                } => {
                    let value = read_ref(process, value)?;
                    let passed = match kind {
                        TestKind::Atom => matches!(value, Term::Atom(_)),
                        TestKind::Pid => matches!(value, Term::Pid(_)),
                        TestKind::Port => matches!(value, Term::Port(_)),
                        TestKind::Reference => matches!(value, Term::Reference(_)),
                        TestKind::Tuple => matches!(value, Term::Tuple(_)),
                        // `is_list` and `is_nonempty_list` test the cons tag, so an
                        // improper list passes (emu `instrs.tab:879-891`; JIT
                        // `emit_is_list` and `emit_is_nonempty_list` in
                        // `jit/arm/instr_common.cpp:1118-1364`).
                        TestKind::List => value.is_list(),
                        TestKind::Nil => value.is_nil(),
                        TestKind::NonEmptyList => value.is_nonempty_list(),
                        TestKind::Binary => matches!(value, Term::Binary(_)),
                        // `is_bitstr` loads as `is_bitstring`, which passes for
                        // each boxed term with a bitstring header (emu
                        // `ops.tab:796-802`, `instrs.tab:945-949`; JIT
                        // `jit/arm/ops.tab:594-597`, `emit_is_bitstring` in
                        // `jit/arm/instr_common.cpp:1158-1184`;
                        // `erl_term.h:384-391`). Each PBeam bitstring is a
                        // binary.
                        TestKind::Bitstring => matches!(value, Term::Binary(_)),
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
                            matches!(value, Term::Atom(atom) if matches!(atom.name(), "true" | "false"))
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
                        // The destinations store the values, so they get copies.
                        let values: Vec<Term> = values.into_iter().cloned().collect();
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
                    if read_ref(process, left)? != read_ref(process, right)? {
                        jump(module, process, *fail_label)?;
                    }
                }
                // ERTS `is_ne` (emu instrs.tab:1115-1119, JIT
                // instr_common.cpp:2081-2105) runs `CMP_NE_ACTION`
                // (erl_utils.h:134-135) with `EQ_SPEC` in the arithmetic
                // order. As for `is_eq`, exact equality gives the ERTS
                // result while PBeam has no float term.
                Instruction::TestNotEqual {
                    fail_label,
                    left,
                    right,
                }
                | Instruction::TestExactNotEqual {
                    fail_label,
                    left,
                    right,
                } => {
                    if read_ref(process, left)? == read_ref(process, right)? {
                        jump(module, process, *fail_label)?;
                    }
                }
                Instruction::TestLess {
                    fail_label,
                    left,
                    right,
                } => {
                    if !read_ref(process, left)?
                        .compare(read_ref(process, right)?)?
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
                    if read_ref(process, left)?
                        .compare(read_ref(process, right)?)?
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
                        read_ref(process, value)?,
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
                // `bs_start_match4` with `no_fail` loads with the fail label
                // 0 (JIT jit/arm/ops.tab:1036-1037). The JIT then does not
                // test the source: the compiler gives it only a binary or a
                // match context (jit/arm/instr_bs.cpp:142-148), so each
                // other term is an invariant error.
                // ERTS writes a reused match context to the destination
                // (jit/arm/instr_bs.cpp:158-159, 187-188), so the two
                // registers share the match context and its position. The
                // copy of a match context term gives the same sharing.
                Instruction::BinaryMatchStart {
                    fail_label,
                    source,
                    destination,
                } => match read_any(process, source)? {
                    Term::Binary(ref bytes) => {
                        let context = MatchContext::new(bytes);
                        let context = Term::MatchContext(Rc::new(RefCell::new(context)));
                        write(process, destination, context)?;
                    }
                    context @ Term::MatchContext(_) => write(process, destination, context)?,
                    other if *fail_label == 0 => {
                        return Err(error(format!(
                            "invariant error: bs_start_match3 with no fail label got {other:?}, not a binary or a match context"
                        )));
                    }
                    _ => jump(module, process, *fail_label)?,
                },
                // On a 64-bit system, ERTS loads `bs_start_match4` with
                // `resume` as nothing when the source is the destination,
                // and else as `move` (JIT jit/arm/ops.tab:1041-1045). The
                // compiler gives it only a match context
                // (beam_ssa_codegen.erl:2300-2302). The copy shares the
                // match context, as for `move`.
                Instruction::BinaryMatchResume {
                    source,
                    destination,
                } => {
                    let context = any_match_context(process, source)?;
                    write(process, destination, Term::MatchContext(context))?;
                }
                // ERTS `bs_get_tail` writes the bits after the position as
                // a sub-bitstring and keeps the position
                // (`emit_bs_get_tail` and `emit_bs_get_tail_shared` in
                // jit/arm/instr_bs.cpp:407-454). PBeam has no bitstring
                // term (row "Binary matching" of docs/erts-differences.md).
                Instruction::BinaryGetTail {
                    context,
                    destination,
                } => {
                    let state = match_context(process, context)?;
                    let state = state.borrow();
                    let bits = state.remaining_bits();
                    if !bits.is_multiple_of(8) {
                        return Err(error(format!(
                            "unsupported bs_get_tail of {bits} bits: needs a bitstring term"
                        )));
                    }
                    write(process, destination, Term::Binary(state.tail()))?;
                }
                // ERTS `i_bs_get_binary2` checks the size
                // (`binary_field_size`), then reads the bits as a
                // sub-bitstring and moves the position, or jumps to `Fail`
                // when fewer bits remain (`emit_bs_get_binary` in
                // jit/arm/instr_bs.cpp:506-556; `erts_bs_get_binary_2` in
                // erl_bits.c:400-424). The loader fuses it with a
                // `bs_get_tail` after it (jit/arm/ops.tab:991-997), which
                // gives the same result.
                Instruction::BinaryGetBinary {
                    fail_label,
                    context,
                    size,
                    unit,
                    destination,
                } => {
                    let state = match_context(process, context)?;
                    let bytes = binary_field_size(read_ref(process, size)?, *unit)
                        .and_then(|bits| Some((bits, state.borrow_mut().read_bits(bits)?)));
                    match bytes {
                        Some((bits, bytes)) => {
                            if !bits.is_multiple_of(8) {
                                return Err(error(format!(
                                    "unsupported bs_get_binary2 of {bits} bits: needs a bitstring term"
                                )));
                            }
                            write(process, destination, Term::Binary(bytes))?;
                        }
                        None => jump(module, process, *fail_label)?,
                    }
                }
                // ERTS `i_bs_get_position` writes `sb->start` as a small
                // integer (emu ops.tab:1274-1277, bs_instrs.tab:1125-1129).
                Instruction::BinaryGetPosition {
                    context,
                    destination,
                } => {
                    let position = match_context(process, context)?.borrow().position();
                    write(process, destination, Term::Integer(position as i64))?;
                }
                // ERTS `bs_set_position` sets `sb->start` to the unsigned
                // value of the position (emu bs_instrs.tab:1120-1123). The
                // compiler gives it a position that `bs_get_position` made.
                Instruction::BinarySetPosition { context, position } => {
                    let state = any_match_context(process, context)?;
                    match read_ref(process, position)? {
                        Term::Integer(value) if let Ok(value) = usize::try_from(*value) => {
                            state.borrow_mut().set_position(value);
                        }
                        other => {
                            return Err(error(format!(
                                "invariant error: bs_set_position position {other:?} is not a position"
                            )));
                        }
                    }
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
                    let code_point = match_context(process, context)?.borrow_mut().get_utf8();
                    match code_point {
                        Some(code_point) => {
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
                    if !binary_match(process, context, commands)? {
                        jump(module, process, *fail_label)?;
                    }
                }
                // The ERTS JIT `i_bs_create_bin` first checks that the term
                // of each `binary` segment of size `all` is a bitstring, in
                // segment order, then checks the `append` segment, and then
                // builds the binary (JIT `emit_i_bs_create_bin` in
                // jit/arm/instr_bs.cpp:1442-1663, 1811-1864, 2000-2084). A
                // failed check jumps to `Fail`, or with `Fail` 0 raises
                // `badarg` with the `error_info` cause
                // `{Segment, binary, Reason, Value}` (`emit_bs_create_bin_error_shared`
                // in jit/arm/instr_bs.cpp:1002-1019;
                // `beam_jit_bs_construct_fail_info` in
                // jit/beam_jit_common.cpp:911-1010).
                Instruction::BinaryCreate {
                    fail_label,
                    destination,
                    segments,
                } => match create_binary(process, segments)? {
                    Ok(bytes) => write(process, destination, Term::Binary(bytes))?,
                    Err(_) if *fail_label != 0 => jump(module, process, *fail_label)?,
                    Err(failure) => {
                        if let Some(outcome) = raise_binary_failure(scheduler, module, failure)? {
                            return Ok(outcome);
                        }
                    }
                },
                // ERTS `fconv` converts a small integer, a bignum, or a float
                // to a float register, and raises `badarith` for each other
                // term (emu float_instrs.tab:38-52; JIT `emit_fconv` and
                // `emit_fconv_shared` in jit/arm/instr_float.cpp:96-135).
                // PBeam has no float register and no float or bignum term,
                // so an integer stops the run.
                Instruction::FloatConvert { source } => {
                    if let Term::Integer(_) = read_ref(process, source)? {
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
                    let value = read_ref(process, value)?;
                    let target = choices
                        .iter()
                        .find(|(choice, _)| choice == value)
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
                    let Term::Tuple(fields) = read_ref(process, tuple)? else {
                        return Err(error("BEAM tuple element source is not a tuple"));
                    };
                    let index = *index as usize;
                    let Some(field) = fields.get(index) else {
                        return Err(error(format!("BEAM tuple has no element {index}")));
                    };
                    // The destination stores the element, so it gets a copy.
                    let field = field.clone();
                    write(process, destination, field)?;
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
                    // `get_hd` reads only the head, so only the head is copied.
                    let list = read_ref(process, source)?;
                    let Some(head) = list.list_head() else {
                        return Err(error(format!(
                            "BEAM list source {list:?} is not a nonempty list"
                        )));
                    };
                    let head = head.clone();
                    write(process, destination, head)?;
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
                    let result = send(scheduler, &to, message)?;
                    if let Some(outcome) = self.finish_bif(scheduler, result, false)? {
                        return Ok(outcome);
                    }
                }
                Instruction::LoopReceive {
                    fail_label,
                    destination,
                } => {
                    // `emit_i_loop_rec_shared` switches out before the peek
                    // when the budget is 0 or less, and runs `loop_rec`
                    // again (`jit/arm/instr_msg.cpp:120-127`, `186-197`).
                    // At the end of the inner part, the receive helper
                    // fetches or handles signals (`jit/arm/instr_msg.cpp:131-168`).
                    // A process that the helper ends, or that used up its
                    // budget, switches out (`jit/arm/instr_msg.cpp:186-197`).
                    if process.fcalls <= 0 {
                        current_frame(process).next_instruction -= 1;
                        return Ok(Outcome::Yield);
                    }
                    if process.mailbox.current().is_none() {
                        match scheduler.receive_helper()? {
                            Received::Message => {}
                            Received::Empty => {
                                jump(module, scheduler.current_process_mut(), *fail_label)?;
                                continue;
                            }
                            Received::Yield => {
                                current_frame(scheduler.current_process_mut()).next_instruction -=
                                    1;
                                return Ok(Outcome::Yield);
                            }
                            Received::Exited => return Ok(Outcome::Yield),
                        }
                    }
                    let process = scheduler.current_process_mut();
                    let term = process.mailbox.current().cloned().expect("a message");
                    write(process, destination, term)?;
                }
                Instruction::LoopReceiveEnd(label) => {
                    // `emit_loop_rec_end` costs 1 and does not switch out
                    // (`jit/arm/instr_msg.cpp:258-268`).
                    if !process.mailbox.save_next() {
                        return Err(error("loop_rec_end without a current message"));
                    }
                    process.fcalls -= 1;
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
                        && let Some(outcome) = self.finish_bif(scheduler, result?, true)?
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
                Instruction::Yield => {
                    // `emit_i_yield` writes `true` to `x0` and switches the
                    // context with no reduction cost; the process keeps its
                    // frame (`jit/arm/instr_common.cpp:3121-3124`;
                    // `jit/arm/instr_call.cpp:30-36`).
                    write(process, &Operand::X(0), Term::static_atom("true"))?;
                    return Ok(Outcome::Yield);
                }
                // ERTS `build_stacktrace` changes the raw stack trace in
                // `x0` to a list (emu `instrs.tab:1247-1251`; JIT
                // `emit_build_stacktrace` in
                // `jit/arm/instr_common.cpp:3046-3056`). The compiler moves
                // it from `x2` first.
                Instruction::BuildStacktrace => {
                    let (_, stack) = raw_stacktrace(read_ref(process, &Operand::X(0))?)?;
                    write(process, &Operand::X(0), stack)?;
                }
                // ERTS `i_raise` raises the reason again with the class and
                // the stack trace of the raw stack trace (emu
                // `ops.tab:284-290`, `instrs.tab:1236-1245`; JIT
                // `emit_raise` and `emit_raise_shared` in
                // `jit/x86/instr_common.cpp:3138-3164`;
                // `erts_sanitize_freason` in `beam_common.c:1089-1097`).
                Instruction::Raise { trace, value } => {
                    let (class, stack) = raw_stacktrace(read_ref(process, trace)?)?;
                    let value = read(process, value)?;
                    if let Some(outcome) = raise_with_stacktrace(process, class, value, stack) {
                        return Ok(outcome);
                    }
                }
                // ERTS `badmatch`, `case_end`, `try_case_end`, and
                // `badrecord` raise class `error` with the reason
                // `{Tag, Value}` (emu `instrs.tab:1137-1162`, `1229-1234`;
                // `expand_error_value` in `beam_common.c:754-768`).
                Instruction::TaggedError { tag, value } => {
                    let reason = Term::Tuple(vec![Term::static_atom(tag), read(process, value)?]);
                    if let Some(outcome) = raise(process, ExceptionClass::Error, reason) {
                        return Ok(outcome);
                    }
                }
                // ERTS `if_end` raises class `error` with the reason
                // `if_clause` (emu `instrs.tab:1151-1155`;
                // `expand_error_value` in `beam_common.c:769-772`).
                Instruction::IfEnd => {
                    let reason = Term::static_atom("if_clause");
                    if let Some(outcome) = raise(process, ExceptionClass::Error, reason) {
                        return Ok(outcome);
                    }
                }
                // ERTS `raw_raise` raises `x1` with the class in `x0` and the
                // stack trace of the raw stack trace in `x2`. For a class
                // that is not `error`, `exit`, or `throw`, it writes
                // `badarg` to `x0` and continues (emu `instrs.tab:1253-1263`;
                // `raw_raise` in `beam_common.c:1148-1179`).
                Instruction::RawRaise => {
                    let (_, stack) = raw_stacktrace(read_ref(process, &Operand::X(2))?)?;
                    let class = ExceptionClass::from_term(read_ref(process, &Operand::X(0))?);
                    let value = read(process, &Operand::X(1))?;
                    match class {
                        Some(class) => {
                            if let Some(outcome) =
                                raise_with_stacktrace(process, class, value, stack)
                            {
                                return Ok(outcome);
                            }
                        }
                        None => write(process, &Operand::X(0), Term::static_atom("badarg"))?,
                    }
                }
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
            let result = native::load_nif(scheduler, &caller)?;
            return self.finish_bif(scheduler, result, false);
        }
        if is_bif(module, name, arity) {
            if !is_heavy_bif(module, name, arity) && light_bif_switch(process) {
                return Ok(Some(Outcome::Yield));
            }
            let arguments = call_arguments(&process.registers, arity)?;
            let result = self.bif(scheduler, module, name, arguments)?;
            return self.finish_bif(scheduler, result, false);
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
    /// `jit/arm/instr_bif.cpp:631-674`, `819-839`). A call of
    /// `erlang:yield/0` runs its body in `erlang.beam`, as in ERTS: the
    /// loader turns the `call_ext_only` of that body into
    /// `Instruction::Yield`. A function that a loaded module does not
    /// export goes to `undefined_function`. Return the outcome when the
    /// process stops.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `erlang:apply(erlang, self, [])` runs the `self/0` BIF.
    /// let outcome = program.call_mfa(&mut scheduler, "erlang", "apply", 3, false)?;
    /// assert!(outcome.is_none());
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
        let process = scheduler.current_process_mut();
        if is_bif(module, name, arity) {
            let arguments = call_arguments(&process.registers, arity)?;
            let result = self.bif(scheduler, module, name, arguments)?;
            return self.finish_bif(scheduler, result, tail);
        }
        match scheduler.code.export_frame(module, name, arity)? {
            Some(frame) => {
                let process = scheduler.current_process_mut();
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
        let handler = scheduler
            .code
            .export_frame(module, "$handle_undefined_function", 2)?;
        let process = scheduler.current_process_mut();
        let arguments = Term::list_from(call_arguments(&process.registers, arity)?);
        if let Some(handler) = handler {
            enter_frame(process, handler, tail);
            write(process, &Operand::X(0), function_atom)?;
            write(process, &Operand::X(1), arguments)?;
            return Ok(None);
        }
        let frame = Term::Tuple(vec![module_atom, function_atom, arguments, Term::nil()]);
        let undef = Term::static_atom("undef");
        Ok(raise_with_stacktrace(
            process,
            ExceptionClass::Error,
            undef,
            Term::list_from(vec![frame]),
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
            let Some((arguments, proper)) = list.take_list_elements() else {
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
        let mut list = read(process, &Operand::X(1))?;
        let Some((arguments, proper)) = list.take_list_elements() else {
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
        // The arguments replace the fun in x0, so the call reads the fun first.
        let call = FunCall::new(read_ref(process, &Operand::X(0))?, arity);
        for (index, argument) in (0..).zip(arguments) {
            write(process, &Operand::X(index), argument)?;
        }
        self.call_fun(scheduler, call, arity, tail)
    }

    /// Call the fun that `call` holds with the arguments in `x0` to `x(arity - 1)`, as ERTS
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
    /// assert!(program.call_fun(&mut scheduler, FunCall::new(&pair, 1), 1, false)?.is_none());
    /// // fun erlang:self/0 runs the `self/0` BIF.
    /// assert!(program.call_fun(&mut scheduler, FunCall::new(&self_fun, 0), 0, false)?.is_none());
    /// // An atom raises `error:{badfun, notfun}`.
    /// let notfun = Term::static_atom("notfun");
    /// assert!(program.call_fun(&mut scheduler, FunCall::new(&notfun, 0), 0, false)?.is_some());
    /// ```
    fn call_fun(
        &self,
        scheduler: &mut Scheduler,
        call: FunCall,
        arity: u32,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        let process = scheduler.current_process_mut();
        let (target, free) = match call {
            FunCall::Enter { target, free } => (target, free),
            FunCall::BadArity(fun) => {
                let arguments = call_arguments(&process.registers, arity)?;
                let call = Term::Tuple(vec![fun, Term::list_from(arguments)]);
                let reason = Term::Tuple(vec![Term::static_atom("badarity"), call]);
                return Ok(raise(process, ExceptionClass::Error, reason));
            }
            FunCall::BadFun(fun) => {
                let reason = Term::Tuple(vec![Term::static_atom("badfun"), fun]);
                return Ok(raise(process, ExceptionClass::Error, reason));
            }
        };
        let lambda = match &target {
            Target::Local(lambda) => lambda,
            Target::External {
                module, function, ..
            } if tail
                && (module.name(), function.name()) == ("erlang", "error")
                && matches!(arity, 1 | 2) =>
            {
                return self.tail_call_error(scheduler, arity);
            }
            Target::External {
                module, function, ..
            } => return self.call_mfa(scheduler, module.name(), function.name(), arity, tail),
        };
        let total_arity = arity + free.len() as u32;
        for (register, value) in (arity..).zip(free) {
            write(process, &Operand::X(register), value)?;
        }
        let frame = scheduler
            .code
            .frame(lambda.module().name(), lambda.label(), total_arity)?;
        enter_frame(scheduler.current_process_mut(), frame, tail);
        Ok(None)
    }

    /// Run a tail call of the fun `fun erlang:error/1` or
    /// `fun erlang:error/2`, with the arity `arity`. ERTS
    /// `i_call_fun_last` removes the frame before it calls the fun
    /// (`emit_i_call_fun2_last` in `jit/arm/instr_fun.cpp:425-455`). Only an
    /// apply puts the calling function back for an error BIF
    /// (`apply_bif_error_adjustment`, `beam_common.c:1345-1431`), so the
    /// first frame of the stack trace is the function of the caller frame.
    /// With no caller frame, the first frame is the initial call of the
    /// process with its arity, and the arguments are not in it
    /// (`build_stacktrace`, `beam_common.c:1245-1248`). The tail call has
    /// already released the Y registers of the frame, so the frame holds no
    /// catch tag. Return the outcome when the process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `tail(F) -> F(r, [a]).` as the start of a process.
    /// let outcome = program.tail_call_error(&mut scheduler, 2)?;
    /// // The process exits with {r, [{Module, tail, 1, []}]}.
    /// assert!(matches!(outcome, Some(Outcome::Exit { .. })));
    /// ```
    fn tail_call_error(
        &self,
        scheduler: &mut Scheduler,
        arity: u32,
    ) -> Result<Option<Outcome>, BeamError> {
        let process = scheduler.current_process_mut();
        process.frames.pop();
        if !process.frames.is_empty() {
            return self.call_mfa(scheduler, "erlang", "error", arity, false);
        }
        let reason = read(process, &Operand::X(0))?;
        let (module, function, arity) = process.initial_call.clone();
        let frame = Term::Tuple(vec![
            Term::Atom(scheduler.atoms.put(&module)?),
            Term::Atom(scheduler.atoms.put(&function)?),
            Term::Integer(arity as i64),
            Term::nil(),
        ]);
        let process = scheduler.current_process_mut();
        let stacktrace = Term::list_from(vec![frame]);
        Ok(raise_with_stacktrace(
            process,
            ExceptionClass::Error,
            reason,
            stacktrace,
        ))
    }

    /// Finish a BIF or NIF call with its result, as the ERTS `nif_bif.epilogue`
    /// does for BIFs and NIFs (`emu/bif_instrs.tab:604-636`; JIT
    /// `emit_bif_nif_epilogue` in `jit/arm/instr_bif.cpp:631-719`). A value
    /// goes to x0, and a tail call then returns from the frame with no
    /// reduction. A yield return writes its value to x0, does the return
    /// of a tail call, and switches out the process. An exception goes to
    /// the error handler. A trap writes its arguments to the X registers,
    /// enters the target function, and switches out the process. An exit ends the process, and no catch
    /// tag sees it. A trap to a BIF export keeps the export, its arguments,
    /// and `tail` in the process, and switches out the process: `run` then
    /// finishes the call through the export. Return the outcome when the
    /// process stops.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // A value in a body call stays in the frame.
    /// let outcome = program.finish_bif(&mut scheduler, BifResult::Value(Term::static_atom("ok")), false)?;
    /// assert!(outcome.is_none());
    /// // `exit(normal)` with no catch tag exits the process.
    /// let raise = BifResult::Raise { class: ExceptionClass::Exit, reason: Term::static_atom("normal") };
    /// assert!(matches!(program.finish_bif(&mut scheduler, raise, true)?, Some(Outcome::Exit { .. })));
    /// ```
    fn finish_bif(
        &self,
        scheduler: &mut Scheduler,
        result: BifResult,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        let process = scheduler.current_process_mut();
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
            BifResult::RaiseWithStacktrace {
                class,
                reason,
                stacktrace,
            } => Ok(raise_with_stacktrace(process, class, reason, stacktrace)),
            BifResult::Trap {
                module,
                function,
                arguments,
            } => {
                let arity = u32::try_from(arguments.len())
                    .map_err(|_| error("arity exceeds BEAM limit"))?;
                let frame = scheduler.code.mfa_frame(module, function, arity)?;
                let process = scheduler.current_process_mut();
                for (index, argument) in (0..).zip(arguments) {
                    write(process, &Operand::X(index), argument)?;
                }
                enter_frame(process, frame, tail);
                // A BIF trap always switches out the process
                // (`context_switch_simplified` in `jit/arm/instr_bif.cpp:525-536`,
                // `676-701`). The process enters the target when it runs
                // again, so the entry costs one reduction of the next turn.
                Ok(Some(Outcome::Yield))
            }
            BifResult::TrapBif {
                module,
                function,
                arguments,
            } => {
                process.pending_bif = Some(PendingBif {
                    module,
                    function,
                    arguments,
                    tail,
                });
                Ok(Some(Outcome::Yield))
            }
            BifResult::AwaitGrant => {
                process.pending_bif = Some(PendingBif {
                    module: "pbeam_cap",
                    function: "await_grant",
                    arguments: Vec::new(),
                    tail,
                });
                Ok(Some(Outcome::Wait))
            }
            BifResult::Exited(reason) => Ok(Some(Outcome::Exit {
                reason,
                result: None,
            })),
            BifResult::DirtyWrite(write) => Ok(Some(Outcome::DirtyWrite { write, tail })),
        }
    }
}

/// Return the outcome of a turn of the current process, `outcome`, after
/// the policy process ends its call of the policy code. When the current
/// process is the policy process and runs a call, each end of a turn after
/// more than `POLICY_REDUCTIONS` reductions of the call ends the call as an
/// overrun: a switch out for the budget, a wait in `receive`, a
/// hibernation, a dirty IO call, a return, and an exception. Thus a call
/// that waits often cannot pass the limit. Else a return or an exception
/// that the call does not catch ends the call. The policy process then
/// waits for the next request (see `Scheduler::end_policy_call`). An exit
/// signal that ends the policy process ends it as each other process.
///
/// # Examples
///
/// ```rust,ignore
/// let outcome = policy_turn(&mut scheduler, program.run(&mut scheduler)?);
/// ```
fn policy_turn(scheduler: &mut Scheduler, outcome: Outcome) -> Outcome {
    if !scheduler.policy_running() {
        return outcome;
    }
    let end = match outcome {
        Outcome::Exit { .. } if scheduler.current_process().exiting.is_some() => return outcome,
        _ if scheduler.policy_overrun() => PolicyEnd::Overrun,
        Outcome::Exit {
            result: Some(value),
            ..
        } => PolicyEnd::Returned(value),
        Outcome::Exit { reason, .. } => PolicyEnd::Raised(reason),
        outcome => return outcome,
    };
    scheduler.end_policy_call(end);
    Outcome::Wait
}

/// Enter `frame`: a tail call replaces the active frame, and a body call
/// pushes it.
///
/// # Examples
///
/// ```rust,ignore
/// enter_frame(&mut process, scheduler.code.mfa_frame("external", "effect", 1)?, false);
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
/// `ErlSpawnOpts`, and the PBeam option `{capabilities, Map}`.
#[derive(Default)]
struct SpawnOptions {
    /// Link the new process to the caller (`SPO_LINK`).
    link: bool,
    /// Monitor the new process and return `{Pid, Reference}` (`SPO_MONITOR`).
    monitor: bool,
    /// Start the new process with the `message_queue_data` flag `off_heap`
    /// (`SPO_OFF_HEAP_MSGQ`), else `on_heap`.
    off_heap_message_queue: bool,
    /// The restrictions of the capabilities of the new process, or `None` to
    /// give it a copy of each delegable capability of the caller.
    capabilities: Option<Vec<Restriction>>,
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
/// `unresolved external MFA M:F/A`. After these checks, a spawn that the
/// capabilities of the caller do not permit raises
/// `error:capability_denied` (see `Scheduler::spawn`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = create_process(&mut scheduler, &module, &function, &Term::nil(), SpawnOptions::default())?;
/// assert!(matches!(result, BifResult::Value(Term::Pid(_))));
/// let result = create_process(&mut scheduler, &module, &function, &Term::Integer(1), SpawnOptions::default())?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// let result = create_process(&mut scheduler, &Term::Integer(1), &function, &Term::nil(), SpawnOptions::default())?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
fn create_process(
    scheduler: &mut Scheduler,
    module: &Term,
    function: &Term,
    arguments: &Term,
    options: SpawnOptions,
) -> Result<BifResult, BeamError> {
    let (Term::Atom(module), Term::Atom(function), Some(arguments)) =
        (module, function, arguments.proper_list())
    else {
        return Ok(BifResult::badarg());
    };
    let arguments = arguments.to_vec();
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
        scheduler
            .code
            .export_frame(module.name(), function.name(), arity)?
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
        capabilities,
    } = options;
    let spawned = scheduler.spawn(
        start,
        arguments,
        monitor,
        link,
        off_heap_message_queue,
        capabilities.as_deref(),
    );
    let Ok((pid, reference)) = spawned? else {
        return BifResult::capability_denied(&mut scheduler.atoms);
    };
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
    read_ref(process, &Operand::X(0)).map_err(|_| error("return before setting x0"))?;
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
        .map_err(|_| error("return before setting x0"))?
        .clone();
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

/// Borrow the value of each key in the map of a `get_map_elements` or
/// `has_map_fields` instruction, or return `None` at the first key that the
/// map does not hold. The read copies neither the map nor a value.
///
/// # Examples
///
/// ```rust,ignore
/// // x0 holds #{output => raw}.
/// let output = Operand::Value(Term::static_atom("output"));
/// assert_eq!(map_values(&process, &Operand::X(0), [&output])?, Some(vec![&Term::static_atom("raw")]));
/// let input = Operand::Value(Term::static_atom("input"));
/// assert_eq!(map_values(&process, &Operand::X(0), [&output, &input])?, None);
/// ```
///
/// # Errors
///
/// Return a runtime error when the source is not a map. The compiler puts an
/// `is_map` test before each of these instructions.
fn map_values<'a>(
    process: &'a ProcessState,
    map: &'a Operand,
    keys: impl IntoIterator<Item = &'a Operand>,
) -> Result<Option<Vec<&'a Term>>, BeamError> {
    let Term::Map(map) = read_ref(process, map)? else {
        return Err(error("BEAM get_map_elements source is not a map"));
    };
    keys.into_iter()
        .map(|key| Ok(map.get(read_ref(process, key)?)))
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
    raise_with_stacktrace(process, class, reason, Term::nil())
}

/// Raise an exception in the process, as ERTS `handle_error` does: continue
/// at the nearest live catch tag, or return the exit outcome when no frame
/// holds one.
///
/// The scan starts at the newest frame and, in one frame, at the lowest Y
/// index, as ERTS `next_catch` scans up from the stack top. The frames above
/// the tag are dropped, and the tag becomes `[]`. The handler gets the
/// non-value in `x0`, the reason in `x1`, the raw stack trace in `x2`, and
/// the class atom in `x3` (`beam_common.c:541-544`). The raw stack trace
/// keeps the class and the stack trace, so that `raise` can raise the
/// exception again (see [`raw_stacktrace`]). An uncaught `error` exits the
/// process with `{Reason, Stacktrace}` (`beam_common.c:684-692`), an
/// uncaught `exit` exits it with `Reason`, and an uncaught `throw` becomes
/// the error `{nocatch, Reason}` (`beam_common.c:509-516`).
///
/// # Examples
///
/// ```rust,ignore
/// let frame = Term::Tuple(vec![module, function, Term::nil(), Term::nil()]);
/// let undef = Term::static_atom("undef");
/// let outcome = raise_with_stacktrace(&mut process, ExceptionClass::Error, undef, Term::list_from(vec![frame]));
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
            ExceptionClass::Throw => Term::Tuple(vec![
                Term::Tuple(vec![Term::static_atom("nocatch"), reason]),
                stacktrace,
            ]),
        };
        return Some(Outcome::Exit {
            reason,
            result: None,
        });
    };
    process.frames.truncate(depth + 1);
    let frame = current_frame(process);
    frame.stack[slot] = StackSlot::Term(Term::nil());
    frame.next_instruction = handler;
    process
        .registers
        .resize(process.registers.len().max(4), None);
    process.registers[0] = None;
    let class = Term::static_atom(class.name());
    process.registers[1] = Some(reason);
    process.registers[2] = Some(Term::Tuple(vec![class.clone(), stacktrace]));
    process.registers[3] = Some(class);
    None
}

/// Return the class and the stack trace of a raw stack trace, the value
/// that the handler of a `try` or a `catch` gets in `x2`.
///
/// ERTS makes the raw stack trace, a 3-tuple with the class in an internal
/// `StackTrace` struct, when it saves the stack trace of an exception
/// (`save_stacktrace`, `beam_common.c:894`, `1051`). PBeam makes the tuple
/// `{Class, Stacktrace}` instead. `[]` is the ERTS raw stack trace `NIL`:
/// `raise` raises class `error` for it, and `build_stacktrace` gives `[]`
/// (`beam_common.c:1079-1097`, `1219-1220`). ERTS then saves a new stack
/// trace for the error; PBeam records no frames, so the stack trace stays
/// `[]`. Compiled code gives a raw stack trace only to `raise` and
/// `build_stacktrace`.
///
/// # Examples
///
/// ```rust,ignore
/// let raw = Term::Tuple(vec![Term::static_atom("exit"), Term::nil()]);
/// assert!(matches!(raw_stacktrace(&raw)?, (ExceptionClass::Exit, stack) if stack.is_nil()));
/// assert!(matches!(raw_stacktrace(&Term::nil())?, (ExceptionClass::Error, _)));
/// ```
///
/// # Errors
///
/// Each other term gives a runtime invariant error. ERTS reads it as a
/// raw stack trace with no check, so its result is not defined.
fn raw_stacktrace(raw: &Term) -> Result<(ExceptionClass, Term), BeamError> {
    if let Term::Tuple(parts) = raw
        && let [class, stack] = parts.as_slice()
        && let Some(class) = ExceptionClass::from_term(class)
    {
        return Ok((class, stack.clone()));
    }
    if raw.is_nil() {
        return Ok((ExceptionClass::Error, raw.clone()));
    }
    Err(error(format!(
        "invariant error: {raw:?} is not a raw stack trace"
    )))
}

/// Clear the catch tag in a Y register of the active frame. The register
/// becomes `[]`, as ERTS `make_blank` writes NIL.
///
/// # Examples
///
/// ```rust,ignore
/// try_end(&mut process, 0)?;
/// assert_eq!(read(&process, &Operand::Y(0))?, Term::nil());
/// ```
fn try_end(process: &mut ProcessState, register: u32) -> Result<(), BeamError> {
    write(process, &Operand::Y(register), Term::nil())
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
/// let list = Term::list_from(vec![Term::Integer(1), Term::Integer(2)]);
/// assert_eq!(nonempty_list(list)?, (Term::Integer(1), Term::list_from(vec![Term::Integer(2)])));
/// let improper = Term::list_with_tail(vec![Term::Integer(1)], x());
/// assert_eq!(nonempty_list(improper)?, (Term::Integer(1), x()));
/// assert!(nonempty_list(Term::nil()).is_err());
/// ```
fn nonempty_list(list: Term) -> Result<(Term, Term), BeamError> {
    list.split_cons()
        .map_err(|list| error(format!("BEAM list source {list:?} is not a nonempty list")))
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

/// Read a copy of an operand from the process registers or the active
/// frame, for an instruction that stores the value in another place. A
/// match context is a runtime invariant error: only the binary-match
/// instructions read it, with `read_any` or `match_context`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read(&process, &Operand::Value(Term::Integer(1)))?, Term::Integer(1));
/// ```
fn read(process: &ProcessState, operand: &Operand) -> Result<Term, BeamError> {
    read_ref(process, operand).cloned()
}

/// Borrow an operand from the process registers or the active frame, for
/// an instruction that only reads the value. The reference points to the
/// literal or the register, so the read copies no term. A match context is
/// a runtime invariant error, as for `read`.
///
/// # Examples
///
/// ```rust,ignore
/// write(&mut process, &Operand::X(0), Term::Tuple(vec![Term::Integer(1)]))?;
/// assert!(matches!(read_ref(&process, &Operand::X(0))?, Term::Tuple(_)));
/// ```
fn read_ref<'a>(process: &'a ProcessState, operand: &'a Operand) -> Result<&'a Term, BeamError> {
    match read_any_ref(process, operand)? {
        Term::MatchContext(_) => Err(match_context_escape(operand)),
        value => Ok(value),
    }
}

/// Read a copy of an operand from the process registers or the active
/// frame, also when it is a match context.
///
/// # Examples
///
/// ```rust,ignore
/// let context = Rc::new(RefCell::new(MatchContext::new(b"a")));
/// write(&mut process, &Operand::X(0), Term::MatchContext(context))?;
/// assert!(matches!(read_any(&process, &Operand::X(0))?, Term::MatchContext(_)));
/// ```
fn read_any(process: &ProcessState, operand: &Operand) -> Result<Term, BeamError> {
    read_any_ref(process, operand).cloned()
}

/// Borrow an operand from the process registers or the active frame, also
/// when it is a match context.
///
/// # Examples
///
/// ```rust,ignore
/// let context = Rc::new(RefCell::new(MatchContext::new(b"a")));
/// write(&mut process, &Operand::X(0), Term::MatchContext(context))?;
/// assert!(matches!(read_any_ref(&process, &Operand::X(0))?, Term::MatchContext(_)));
/// ```
fn read_any_ref<'a>(
    process: &'a ProcessState,
    operand: &'a Operand,
) -> Result<&'a Term, BeamError> {
    let stack = process.frames.last().map_or(&[][..], |frame| &frame.stack);
    read_operand(operand, &process.registers, stack)
}

/// Read the match context of a binary-match instruction that reads its
/// position. A match context after a failed `=:=` command of `bs_match`
/// has no known position until `bs_set_position`, and gives a runtime
/// error (row "Binary matching" of docs/erts-differences.md). Return the
/// shared match context, so a change of its position is also in each
/// copy of it.
///
/// # Examples
///
/// ```rust,ignore
/// let context = Rc::new(RefCell::new(MatchContext::new(b"a")));
/// write(&mut process, &Operand::X(0), Term::MatchContext(context))?;
/// assert_eq!(match_context(&process, &Operand::X(0))?.borrow().position(), 0);
/// ```
fn match_context(
    process: &ProcessState,
    operand: &Operand,
) -> Result<Rc<RefCell<MatchContext>>, BeamError> {
    let context = any_match_context(process, operand)?;
    check_known_position(&context, operand)?;
    Ok(context)
}

/// Check that the position of the match context in `operand` is known.
/// After a failed `=:=` command of `bs_match`, a read gives a runtime
/// error (row "Binary matching" of docs/erts-differences.md).
///
/// # Examples
///
/// ```rust,ignore
/// let context = Rc::new(RefCell::new(MatchContext::new(b"a")));
/// assert!(check_known_position(&context, &Operand::X(0)).is_ok());
/// context.borrow_mut().forget_position();
/// assert!(check_known_position(&context, &Operand::X(0)).is_err());
/// ```
fn check_known_position(
    context: &RefCell<MatchContext>,
    operand: &Operand,
) -> Result<(), BeamError> {
    if context.borrow().position_known() {
        Ok(())
    } else {
        Err(error(format!(
            "unsupported read of the match context in {operand:?} after a failed bs_match =:= command"
        )))
    }
}

/// Return the `byte_size/1` result of a match context in `values`, the
/// argument values of the BIF `module:name`, or `None` for each other BIF
/// or argument. The BEAM validator lets `byte_size/1` get a match context
/// (`validate_bif_sources`, beam_validator.erl:1649-1654), and ERTS gives
/// the bits from its position to its end in bytes, rounded up
/// (`emit_bif_byte_size`, jit/arm/instr_guard_bifs.cpp:458-509; ops.tab:
/// 789-790). `Terra.Input.Parser.decode_codepoint/1` reads the size of its
/// match context with it.
///
/// # Errors
///
/// A match context in an argument of another BIF is a runtime invariant
/// error. A match context with no known position gives the error of
/// `match_context`.
///
/// # Examples
///
/// ```rust,ignore
/// let context = Rc::new(RefCell::new(MatchContext::new(b"abc")));
/// context.borrow_mut().skip(4);
/// let values = [Term::MatchContext(context)];
/// let size = match_context_byte_size("erlang", "byte_size", &[Operand::X(0)], &values)?;
/// assert_eq!(size, Some(Term::Integer(3)));
/// assert!(match_context_byte_size("erlang", "hd", &[Operand::X(0)], &values).is_err());
/// ```
fn match_context_byte_size(
    module: &str,
    name: &str,
    arguments: &[Operand],
    values: &[Term],
) -> Result<Option<Term>, BeamError> {
    if let (("erlang", "byte_size"), [Term::MatchContext(context)]) = ((module, name), values) {
        check_known_position(context, &arguments[0])?;
        let bits = context.borrow().remaining_bits();
        return Ok(Some(Term::Integer(bits.div_ceil(8) as i64)));
    }
    match values
        .iter()
        .position(|value| matches!(value, Term::MatchContext(_)))
    {
        Some(index) => Err(match_context_escape(&arguments[index])),
        None => Ok(None),
    }
}

/// Read the shared match context of a binary-match instruction, also when
/// its position is not known. Each other term is a runtime invariant
/// error: the BEAM validator gives these instructions only a match
/// context.
///
/// # Examples
///
/// ```rust,ignore
/// let context = Rc::new(RefCell::new(MatchContext::new(b"a")));
/// write(&mut process, &Operand::X(0), Term::MatchContext(context))?;
/// assert_eq!(any_match_context(&process, &Operand::X(0))?.borrow().position(), 0);
/// assert!(any_match_context(&process, &Operand::Value(Term::Integer(1))).is_err());
/// ```
fn any_match_context(
    process: &ProcessState,
    operand: &Operand,
) -> Result<Rc<RefCell<MatchContext>>, BeamError> {
    match &read_any(process, operand)? {
        Term::MatchContext(context) => Ok(Rc::clone(context)),
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

/// The parts of a fun term that `call_fun` uses. The call reads them
/// before it writes the argument registers or releases the frame, because
/// the fun can be in one of these registers. Only the free variables, which
/// the call stores in X registers, and the entry are copies. A term that
/// the call cannot enter is copied for the exception reason only.
///
/// # Examples
///
/// ```rust,ignore
/// let call = FunCall::new(&Term::static_atom("notfun"), 0);
/// assert!(matches!(call, FunCall::BadFun(Term::Atom(_))));
/// ```
enum FunCall {
    /// A fun of the called arity: its entry and its free variables.
    Enter { target: Target, free: Vec<Term> },
    /// A fun of another arity, for the reason `{badarity, {Fun, Arguments}}`.
    BadArity(Term),
    /// A term that is not a fun, for the reason `{badfun, Term}`.
    BadFun(Term),
}

impl FunCall {
    /// Read the parts of `fun` that a call with `arity` arguments uses, as
    /// ERTS `call_fun` tests them (`beam_common.c:1605-1731`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let call = FunCall::new(&Term::Fun(Fun::new(lambda, vec![tag])), 1);
    /// assert!(matches!(call, FunCall::Enter { .. }));
    /// ```
    fn new(fun: &Term, arity: u32) -> Self {
        match fun {
            Term::Fun(fun) if fun.target().arity() == arity => FunCall::Enter {
                target: fun.target().clone(),
                free: fun.free().to_vec(),
            },
            Term::Fun(_) => FunCall::BadArity(fun.clone()),
            _ => FunCall::BadFun(fun.clone()),
        }
    }
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
/// let context = Rc::new(RefCell::new(MatchContext::new(b"a")));
/// let registers = vec![Some(Term::MatchContext(context))];
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
/// let context = Rc::new(RefCell::new(MatchContext::new(b"a")));
/// let registers = vec![Some(Term::MatchContext(context))];
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

/// Return the size in bits of a `bs_get_binary2` segment of `size` units
/// of `unit` bits, or `None` when the instruction fails. As the ERTS JIT
/// field size check, a size that is not a non-negative integer fails
/// (`emit_bs_get_field_size` in jit/arm/instr_bs.cpp:38-116). That check
/// also fails a register size from 2^48 and a literal size above
/// `MAX_SMALL / unit`. The compiler gives a unit from 1 to 256
/// (`type_to_record` in erl_bits.erl:93), so such a size is always more
/// bits than a binary has, and `erts_bs_get_binary_2` then fails
/// (erl_bits.c:400-424). Thus `MatchContext::read_bits` gives the same
/// result for it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(binary_field_size(&Term::Integer(3), 8), Some(24));
/// assert_eq!(binary_field_size(&Term::Integer(-1), 8), None);
/// assert_eq!(binary_field_size(&Term::static_atom("all"), 8), None);
/// assert_eq!(binary_field_size(&Term::Integer(i64::MAX), 8), None);
/// ```
fn binary_field_size(size: &Term, unit: u64) -> Option<u64> {
    match size {
        Term::Integer(size) => u64::try_from(*size).ok()?.checked_mul(unit),
        _ => None,
    }
}

/// Return the runtime invariant error of a match context in `operand` at an
/// instruction that is not a binary match, `move`, or a local call. The
/// BEAM validator also lets `swap` exchange a match context
/// (beam_validator.erl:405-407), which PBeam does not implement yet (row
/// "Binary matching" of docs/erts-differences.md).
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

/// Run the commands of a `bs_match` instruction in order on the match
/// context in `context`, as the ERTS JIT does (`gen.bs_match`,
/// jit/arm/generators.tab:532-604; `emit_i_bs_match_test_heap`,
/// jit/arm/instr_bs.cpp:2916-3205). Return `false` at the first command
/// that fails. The register keeps the position after each command that
/// moves it.
///
/// - `ensure_at_least` tests that `Size` bits remain and that the bits
///   after them are a multiple of `Unit`. `ensure_exactly` tests that
///   exactly `Size` bits remain.
/// - `integer` reads an integer (`segment_integer`), and `binary` reads a
///   binary. `skip` moves the position. `get_tail` writes the bits after
///   the position and keeps the position. These commands do not fail:
///   the compiler puts an `ensure_at_least` or `ensure_exactly` for their
///   bits first (beam_ssa_codegen.erl:2759-2868).
/// - `=:=` fails when the bits are not its value. The position is then
///   not known (`MatchContext::forget_position`).
///
/// # Errors
///
/// A command that reads more bits than remain gives a runtime invariant
/// error. An integer outside the 64-bit signed range needs a bignum, and
/// a `binary` or `get_tail` of a size that is not a multiple of 8 bits
/// needs a bitstring term: each gives a runtime error (rows "Integer
/// range" and "Binary matching" of docs/erts-differences.md).
///
/// # Examples
///
/// ```rust,ignore
/// let context = Rc::new(RefCell::new(MatchContext::new(&[7, 8])));
/// write(&mut process, &Operand::X(0), Term::MatchContext(context))?;
/// let unit = NonZeroU64::new(8).unwrap();
/// let byte = BinaryMatchCommand::Integer { size: 8, little: false, signed: false, destination: Operand::X(1) };
/// assert!(binary_match(&mut process, &Operand::X(0), &[BinaryMatchCommand::EnsureAtLeast { size: 8, unit }, byte])?);
/// assert_eq!(read(&process, &Operand::X(1))?, Term::Integer(7));
/// assert!(!binary_match(&mut process, &Operand::X(0), &[BinaryMatchCommand::EnsureExactly { size: 0 }])?);
/// ```
fn binary_match(
    process: &mut ProcessState,
    context: &Operand,
    commands: &[BinaryMatchCommand],
) -> Result<bool, BeamError> {
    let context = match_context(process, context)?;
    let mut state = context.borrow_mut();
    // A binary of `size` bits, or a runtime error for a bitstring.
    let binary_size = |command: &str, size: u64| {
        if size.is_multiple_of(8) {
            Ok(())
        } else {
            Err(error(format!(
                "unsupported bs_match {command} of {size} bits: needs a bitstring term"
            )))
        }
    };
    for command in commands {
        let remaining = state.remaining_bits() as u64;
        let too_few_bits = |size: &u64| {
            error(format!(
                "invariant error: a bs_match command needs {size} bits, but {remaining} bits remain"
            ))
        };
        match command {
            BinaryMatchCommand::EnsureAtLeast { size, unit } => {
                if remaining < *size || !(remaining - size).is_multiple_of(unit.get()) {
                    return Ok(false);
                }
            }
            BinaryMatchCommand::EnsureExactly { size } => {
                if remaining != *size {
                    return Ok(false);
                }
            }
            BinaryMatchCommand::Integer {
                size,
                little,
                signed,
                destination,
            } => {
                let bits = state.read_bits(*size).ok_or_else(|| too_few_bits(size))?;
                let value =
                    segment_integer(&bits, *size as usize, *little, *signed).ok_or_else(|| {
                        error(
                            "unsupported bs_match integer outside the 64-bit range: needs a bignum",
                        )
                    })?;
                write(process, destination, Term::Integer(value))?;
            }
            BinaryMatchCommand::Binary { size, destination } => {
                binary_size("binary", *size)?;
                let bytes = state.read_bits(*size).ok_or_else(|| too_few_bits(size))?;
                write(process, destination, Term::Binary(bytes))?;
            }
            BinaryMatchCommand::Equal { size, value } => {
                let bits = state.read_bits(*size).ok_or_else(|| too_few_bits(size))?;
                let equal = segment_equals(&bits, *size as usize, *value);
                if !equal {
                    state.forget_position();
                    return Ok(false);
                }
            }
            BinaryMatchCommand::GetTail { destination } => {
                binary_size("get_tail", remaining)?;
                write(process, destination, Term::Binary(state.tail()))?;
            }
            BinaryMatchCommand::Skip { size } => {
                state.skip(*size).ok_or_else(|| too_few_bits(size))?;
            }
        }
    }
    Ok(true)
}

/// The failure of a `bs_create_bin` segment, which ERTS gives in the
/// `error_info` cause `{segment, operation, info, value}`
/// (`beam_jit_bs_construct_fail_info`, jit/beam_jit_common.cpp:911-1010).
#[derive(Debug, PartialEq)]
struct BinarySegmentFailure {
    /// The segment number of the compiler.
    segment: u64,
    /// The segment operation: `binary`, `integer`, or `utf8`.
    operation: &'static str,
    /// The failed check: `type`, `unit`, or `size`.
    info: &'static str,
    /// The term that failed the check.
    value: Term,
    /// The exception reason: `badarg`, or `system_limit` for a size that
    /// is too large.
    reason: &'static str,
}

impl BinarySegmentFailure {
    /// Return the `badarg` failure of a check of `operation`.
    fn badarg(segment: u64, operation: &'static str, info: &'static str, value: Term) -> Self {
        BinarySegmentFailure {
            segment,
            operation,
            info,
            value,
            reason: "badarg",
        }
    }
}

/// A segment of `bs_create_bin` after the size pass: the bytes of a
/// `string` or `binary` segment and the reductions of their copy, an
/// `integer` segment with its term and its size in bits, or the code point
/// of a `utf8` segment.
enum BinaryPart<'a> {
    Bytes(Cow<'a, [u8]>, usize),
    Integer {
        segment: u64,
        value: Term,
        bits: u64,
        little: bool,
        /// True when the size is a literal from 0 to 2^56 - 1, as ERTS
        /// computes its size at load time (jit/arm/instr_bs.cpp:1493-1505).
        fixed: bool,
    },
    Utf8(char),
}

impl BinaryPart<'_> {
    /// Return the `type` failure of an `integer` part whose term is not an
    /// integer.
    fn type_failure(&self) -> Option<BinarySegmentFailure> {
        match self {
            BinaryPart::Integer { segment, value, .. } if !matches!(value, Term::Integer(_)) => {
                Some(BinarySegmentFailure::badarg(
                    *segment,
                    "integer",
                    "type",
                    value.clone(),
                ))
            }
            _ => None,
        }
    }

    /// Return true when the part writes at least one bit.
    fn has_bits(&self) -> bool {
        match self {
            BinaryPart::Bytes(bytes, _) => !bytes.is_empty(),
            BinaryPart::Integer { bits, .. } => *bits > 0,
            BinaryPart::Utf8(_) => true,
        }
    }
}

/// Return the size in bits of an `integer` segment, or the exception
/// reason of its failure, as the ERTS JIT size pass checks it
/// (jit/arm/instr_bs.cpp:1493-1505, 1639-1689). A `fixed` size, a literal
/// from 0 to 2^56 - 1, needs no check. Each other size must be a small
/// integer that is not negative, else it fails with `system_limit` for a
/// positive bignum and with `badarg` for each other term
/// (`BSC_REASON_DEPENDS`, jit/beam_jit_common.cpp:968-975). For a unit
/// other than 1, a size of 2^52 or more fails with `system_limit`, as the
/// arm64 JIT checks it (jit/arm/instr_bs.cpp:1685). The x86_64 JIT fails
/// only on an overflow of the size times the unit
/// (jit/x86/instr_bs.cpp:1942-1943) and then stops at the allocation; row
/// "Binary construction" of `docs/erts-differences.md` records it.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(integer_segment_bits(false, &Term::Integer(3), 8), Ok(24));
/// assert_eq!(integer_segment_bits(false, &Term::Integer(-1), 1), Err("badarg"));
/// assert_eq!(integer_segment_bits(false, &Term::Integer(1 << 52), 8), Err("system_limit"));
/// ```
fn integer_segment_bits(fixed: bool, size: &Term, unit: u64) -> Result<u64, &'static str> {
    match (size.small_integer(), size) {
        (Some(bits), _) if bits >= 0 && (fixed || unit == 1 || bits >> 52 == 0) => {
            Ok((bits as u64).saturating_mul(unit))
        }
        (Some(bits), _) if bits >= 0 => Err("system_limit"),
        (None, Term::Integer(bits)) if *bits >= 0 => Err("system_limit"),
        _ => Err("badarg"),
    }
}

/// Return the failure of the `integer` part that ERTS reports when the
/// part at `first` is the first part in segment order whose term is not an
/// integer. The JIT construct pass joins the adjacent `integer` segments of
/// a fixed size from 1 to 64 bits and one byte order into one group, up to
/// 64 bits, and a little-endian group joins a segment only after a whole
/// number of bytes (`beam_jit_bsc_combine_segments`,
/// jit/beam_jit_bs.cpp:162-232). It checks the segments of a
/// little-endian group in reverse order (`push_group`,
/// jit/beam_jit_bs.cpp:106-123), so the last bad term of that group fails
/// first.
///
/// # Examples
///
/// ```rust,ignore
/// // <<A:8/little, B:8/little>> with A = a and B = b fails at B.
/// assert_eq!(integer_type_failure(&parts, 0).segment, 2);
/// ```
fn integer_type_failure(parts: &[BinaryPart], first: usize) -> BinarySegmentFailure {
    let mut group = 0..0;
    let mut group_little = false;
    let mut group_bits = 0;
    for (index, part) in parts.iter().enumerate() {
        let member = match part {
            BinaryPart::Integer {
                bits,
                little,
                fixed: true,
                ..
            } if (1..=64).contains(bits) => Some((*bits, *little)),
            _ => None,
        };
        match member {
            Some((bits, little))
                if !group.is_empty()
                    && little == group_little
                    && group_bits + bits <= 64
                    && (!little || group_bits % 8 == 0) =>
            {
                group.end = index + 1;
                group_bits += bits;
            }
            _ if index > first => break,
            Some((bits, little)) => {
                group = index..index + 1;
                group_little = little;
                group_bits = bits;
            }
            None => group = index + 1..index + 1,
        }
    }
    let end = if group_little && group.contains(&first) {
        group.end
    } else {
        first + 1
    };
    (first..end)
        .rev()
        .find_map(|index| parts[index].type_failure())
        .expect("the part at `first` has a term that is not an integer")
}

/// Build the binary of the segments of a `bs_create_bin` instruction, as
/// the ERTS JIT `i_bs_create_bin` does (JIT `emit_i_bs_create_bin` in
/// jit/arm/instr_bs.cpp:1442-2490). It has three passes, and the first
/// failed check gives the failure.
///
/// The size pass reads the segments in order (jit/arm/instr_bs.cpp:
/// 1579-1808). A `binary` segment of size `all` with a term that is not a
/// binary fails with `type`. An `integer` segment checks its size (see
/// `integer_segment_bits`). A `utf8` segment with a term that is not a
/// code point from 0 to 16#10FFFF outside 16#D800 to 16#DFFF fails with
/// `type`.
///
/// Then the JIT checks the `append` segment, which is the first segment
/// (jit/arm/instr_bs.cpp:1811-1864), with `erts_bs_append_checked`
/// (erl_bits.c:1724-1925). A term that is not a binary fails with the
/// reason `type`, and a binary whose size in bits is not a multiple of a
/// unit larger than 1 fails with the reason `unit`. When the other
/// segments add no bits, the result is the binary of the `append` segment
/// and the append uses no reduction, but the construct pass still checks
/// the term of each `integer` segment of size 0. Else the result starts with a copy of
/// that binary, which uses one reduction for each 8192 bits, as the ERTS
/// path for a binary that is not writable does (erl_bits.c:1855-1924).
/// PBeam has no writable binary (row "Binary construction" of
/// `docs/erts-differences.md`).
///
/// The construct pass writes the segments (jit/arm/instr_bs.cpp:
/// 1994-2486). A `string` segment copies its bytes (`erts_bs_put_string`,
/// erl_bits.c:1641-1654). A `binary` segment copies its binary and uses
/// one reduction for each 8192 bits (`beam_jit_bs_put_binary_all`,
/// jit/beam_jit_common.cpp:1043-1057). An `integer` segment with a term
/// that is not an integer fails with `type`, in the order of
/// `integer_type_failure`; else it writes the integer with no reduction
/// (`erts_bs_put_integer_be` and `erts_bs_put_integer_le`, erl_bits.c:
/// 926-1167). A `utf8` segment writes the UTF-8 bytes of its code point
/// (`emit_construct_utf8`). PBeam has no bitstring that is not a binary,
/// so the unit 8 test of `erts_bs_put_binary_all` always passes.
///
/// # Examples
///
/// ```rust,ignore
/// write(&mut process, &Operand::X(0), Term::Binary(b"3".to_vec()))?;
/// let segments = [
///     BinarySegment::String(b"count: ".to_vec()),
///     BinarySegment::BinaryAll { segment: 2, source: Operand::X(0) },
/// ];
/// assert_eq!(create_binary(&mut process, &segments)?, Ok(b"count: 3".to_vec()));
/// write(&mut process, &Operand::X(0), Term::static_atom("a"))?;
/// assert_eq!(
///     create_binary(&mut process, &segments)?,
///     Err(BinarySegmentFailure::badarg(2, "binary", "type", Term::static_atom("a")))
/// );
///
/// // <<X:16/little>> with X = 16#1234
/// write(&mut process, &Operand::X(0), Term::Integer(0x1234))?;
/// let segments = [BinarySegment::Integer {
///     segment: 1,
///     unit: 1,
///     little: true,
///     source: Operand::X(0),
///     size: Operand::Value(Term::Integer(16)),
/// }];
/// assert_eq!(create_binary(&mut process, &segments)?, Ok(vec![0x34, 0x12]));
/// ```
///
/// # Errors
///
/// A segment of a type, size, or unit that PBeam does not run yet is a
/// runtime error. A result whose size in bits is not a multiple of 8 is a
/// runtime error, because PBeam has no bitstring term. A match context in
/// a `binary` segment is a runtime invariant error.
fn create_binary(
    process: &mut ProcessState,
    segments: &[BinarySegment],
) -> Result<Result<Vec<u8>, BinarySegmentFailure>, BeamError> {
    let mut parts = Vec::with_capacity(segments.len());
    let mut append = None;
    for segment in segments {
        match segment {
            BinarySegment::String(text) => {
                parts.push(BinaryPart::Bytes(Cow::Borrowed(text.as_slice()), 0));
            }
            BinarySegment::BinaryAll { segment, source } => {
                let mut value = read(process, source)?;
                let Term::Binary(bytes) = &mut value else {
                    return Ok(Err(BinarySegmentFailure::badarg(
                        *segment, "binary", "type", value,
                    )));
                };
                let reductions = bytes.len() * 8 / BITS_PER_REDUCTION;
                parts.push(BinaryPart::Bytes(
                    Cow::Owned(std::mem::take(bytes)),
                    reductions,
                ));
            }
            BinarySegment::Append {
                segment,
                unit,
                source,
            } => append = Some((*segment, *unit, read(process, source)?)),
            BinarySegment::Integer {
                segment,
                unit,
                little,
                source,
                size,
            } => {
                // ERTS computes the size of a literal from 0 to 2^56 - 1 at
                // load time with no check (jit/arm/instr_bs.cpp:1493-1505).
                let fixed = matches!(size, Operand::Value(Term::Integer(literal)) if (0..1 << 56).contains(literal));
                let size = read(process, size)?;
                let bits = match integer_segment_bits(fixed, &size, *unit) {
                    Ok(bits) => bits,
                    Err(reason) => {
                        return Ok(Err(BinarySegmentFailure {
                            reason,
                            ..BinarySegmentFailure::badarg(*segment, "integer", "size", size)
                        }));
                    }
                };
                parts.push(BinaryPart::Integer {
                    segment: *segment,
                    value: read(process, source)?,
                    bits,
                    little: *little,
                    fixed,
                });
            }
            BinarySegment::Utf8 { segment, source } => {
                let value = read(process, source)?;
                let code_point = match &value {
                    Term::Integer(code) => u32::try_from(*code).ok().and_then(char::from_u32),
                    _ => None,
                };
                let Some(code_point) = code_point else {
                    return Ok(Err(BinarySegmentFailure::badarg(
                        *segment, "utf8", "type", value,
                    )));
                };
                parts.push(BinaryPart::Utf8(code_point));
            }
            BinarySegment::Unsupported(kind) => {
                return Err(error(format!(
                    "unsupported bs_create_bin segment {}",
                    kind.name()
                )));
            }
        }
    }
    let mut writer = BitWriter::default();
    if let Some((segment, unit, mut value)) = append {
        let Term::Binary(start) = &mut value else {
            return Ok(Err(BinarySegmentFailure::badarg(
                segment, "binary", "type", value,
            )));
        };
        if unit > 1 && !(start.len() as u64 * 8).is_multiple_of(unit) {
            return Ok(Err(BinarySegmentFailure::badarg(
                segment, "binary", "unit", value,
            )));
        }
        if !parts.iter().any(BinaryPart::has_bits) {
            // The construct pass still checks the term of each `integer`
            // segment of size 0 (erl_bits.c:934-936).
            if let Some(index) = parts.iter().position(|part| part.type_failure().is_some()) {
                return Ok(Err(integer_type_failure(&parts, index)));
            }
            return Ok(Ok(std::mem::take(start)));
        }
        let reductions = start.len() * 8 / BITS_PER_REDUCTION;
        process.bump_reductions(i32::try_from(reductions).unwrap_or(i32::MAX));
        writer.push_bytes(start);
    }
    for (index, part) in parts.iter().enumerate() {
        match part {
            BinaryPart::Bytes(bytes, reductions) => {
                writer.push_bytes(bytes);
                process.bump_reductions(i32::try_from(*reductions).unwrap_or(i32::MAX));
            }
            BinaryPart::Integer {
                value: Term::Integer(value),
                bits,
                little,
                ..
            } => writer.push_integer(*value, *bits, *little),
            BinaryPart::Integer { .. } => return Ok(Err(integer_type_failure(&parts, index))),
            BinaryPart::Utf8(code_point) => {
                writer.push_bytes(code_point.encode_utf8(&mut [0; 4]).as_bytes());
            }
        }
    }
    writer.into_bytes().map(Ok).map_err(|bits| {
        error(format!(
            "unsupported bs_create_bin result: a bitstring of {bits} bits, which is not a binary"
        ))
    })
}

/// Raise the exception of a failed `bs_create_bin` segment in the current
/// process, as ERTS `emit_bs_create_bin_error_shared` does
/// (jit/arm/instr_bs.cpp:1002-1019). The class is `error`, the reason is
/// the `reason` of `failure` (`badarg` or `system_limit`), and the
/// `error_info` cause is `{Segment, Operation, Info, Value}`. The `binary`
/// and `append` segments give the operation `binary` (`beam_jit_bsc_init`,
/// jit/beam_jit_bs.cpp:55-74). The stack trace is the frame of the
/// function that holds the instruction, with the ERTS `error_info`
/// (`beam_jit_bs_construct_fail_info` in jit/beam_jit_common.cpp:911-1010).
/// PBeam keeps no line table and no caller frames, so the frame has no
/// `file` or `line` and is the only frame (row "Exception stack trace" of
/// `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// // `prefix(X) -> <<"count: ", X/binary>>.` with `X = a` and no catch
/// // exits with {badarg, [{bs_create_bin_probe, prefix, 1, [{error_info,
/// // #{cause => {2, binary, type, a}, function => format_bs_fail,
/// // module => erl_erts_errors}}]}]}.
/// let failure = BinarySegmentFailure::badarg(2, "binary", "type", Term::static_atom("a"));
/// let outcome = raise_binary_failure(scheduler, &module, failure)?;
/// assert!(matches!(outcome, Some(Outcome::Exit { result: None, .. })));
/// ```
///
/// # Errors
///
/// An instruction with no `func_info` before it, or a module or function
/// name that the atom table does not hold, is a runtime invariant error.
fn raise_binary_failure(
    scheduler: &mut Scheduler,
    module: &BeamModule,
    failure: BinarySegmentFailure,
) -> Result<Option<Outcome>, BeamError> {
    let (module_atom, function_atom, arity) = current_function(scheduler, module)?;
    let cause = Term::Tuple(vec![
        Term::Integer(failure.segment as i64),
        Term::static_atom(failure.operation),
        Term::static_atom(failure.info),
        failure.value,
    ]);
    // ponytail: each map form holds atom keys, so `from_pairs` cannot fail.
    let error_info = Map::from_pairs(vec![
        (Term::static_atom("cause"), cause),
        (
            Term::static_atom("function"),
            Term::static_atom("format_bs_fail"),
        ),
        (
            Term::static_atom("module"),
            Term::static_atom("erl_erts_errors"),
        ),
    ])
    .expect("atom keys fit each map form");
    let location = Term::Tuple(vec![Term::static_atom("error_info"), Term::Map(error_info)]);
    let frame = Term::Tuple(vec![
        module_atom,
        function_atom,
        Term::Integer(arity.into()),
        Term::list_from(vec![location]),
    ]);
    Ok(raise_with_stacktrace(
        scheduler.current_process_mut(),
        ExceptionClass::Error,
        Term::static_atom(failure.reason),
        Term::list_from(vec![frame]),
    ))
}

/// Return the module atom, the function atom, and the arity of the function
/// that holds the last instruction that the active frame ran, from the
/// `func_info` before it. ERTS finds the function of a code address in the
/// same way for the first frame of a stack trace (`erts_lookup_function_info`
/// in `build_stacktrace`, `beam_common.c:1229-1236`).
///
/// # Examples
///
/// ```rust,ignore
/// // In `prefix/1` of `bs_create_bin_probe`.
/// let (module, function, arity) = current_function(scheduler, &module)?;
/// assert_eq!(arity, 1);
/// ```
///
/// # Errors
///
/// An instruction with no `func_info` before it, or a module or function
/// name that the atom table does not hold, is a runtime invariant error.
fn current_function(
    scheduler: &mut Scheduler,
    module: &BeamModule,
) -> Result<(Term, Term, u32), BeamError> {
    let index = current_frame(scheduler.current_process_mut()).next_instruction - 1;
    let (module_name, name, arity) = module.instructions[..index]
        .iter()
        .rev()
        .find_map(|instruction| match instruction {
            Instruction::FunctionInfo {
                module,
                name,
                arity,
            } => Some((module, name, *arity)),
            _ => None,
        })
        .ok_or_else(|| error("invariant error: the instruction has no func_info before it"))?;
    let atom = |name: &str| {
        scheduler
            .atoms
            .get(name)
            .map(Term::Atom)
            .ok_or_else(|| error(format!("invariant error: atom table does not hold {name}")))
    };
    Ok((atom(module_name)?, atom(name)?, arity))
}

/// Borrow a literal, an X register, or a Y register. The reference points
/// to the literal of the instruction or to the register, so the read
/// copies no term.
///
/// # Examples
///
/// ```rust,ignore
/// let registers = vec![Some(Term::Integer(7))];
/// assert_eq!(read_operand(&Operand::X(0), &registers, &[])?, &Term::Integer(7));
/// assert!(read_operand(&Operand::Y(0), &registers, &[]).is_err());
/// ```
fn read_operand<'a>(
    operand: &'a Operand,
    registers: &'a [Option<Term>],
    stack: &'a [StackSlot],
) -> Result<&'a Term, BeamError> {
    match operand {
        Operand::Value(value) => Ok(value),
        Operand::UnsupportedLiteral(index) => {
            Err(error(format!("unsupported BEAM literal index {index}")))
        }
        Operand::UnsupportedInteger => Err(error(
            "unsupported BEAM integer operand outside the 64-bit range: needs a bignum",
        )),
        Operand::X(index) => registers
            .get(*index as usize)
            .and_then(Option::as_ref)
            .ok_or_else(|| error(format!("register x{index} is unset"))),
        Operand::Y(index) => match stack.get(*index as usize) {
            Some(StackSlot::Term(value)) => Ok(value),
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
#[path = "../../tests/runtime/interpreter.rs"]
mod tests;
