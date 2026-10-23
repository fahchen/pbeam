use super::{
    Job,
    capability::{Policy, Restriction},
    dirty::{Output, Work},
    process::{
        BifCall, DirtyCall, Frame, PendingBif, PendingTrap, ProcessState, ReceiveTimer, StackSlot,
        Start, StartFlags,
    },
    scheduler::{PolicyEnd, Received, Scheduler, Turn, node::Node},
    tty::{InitialMode, Ttys},
};
use crate::{
    beam::{
        BeamModule, Import, ImportKind, NameId,
        code::{
            self, BinaryMatchCommand, BinarySegment, FloatOperation, Instruction, LineTable,
            Operand, SelectValueChoices, TestKind, TestOperand,
        },
    },
    error::{BeamError, error},
    term::{
        Atom, AtomTable, Binary, Fun, HostTerm, Insert, Map, MatchState, Target, Term, debug_term,
        low_64_bits, print_term, segment_equals, segment_integer,
    },
};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::{
    borrow::{Borrow, Cow},
    cell::Cell,
    cmp::Ordering,
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
};

pub(crate) mod bif;
mod bits;
mod code_table;
mod native;

pub(crate) use native::has_static_library;
pub use native::{NifGap, nif_manifest_gaps};

pub(super) use bif::BifResult;
pub(super) use code_table::{CodeSnapshot, CodeTable, LoadedModule};

use code_table::{ExportTarget, ImportTargets};

use bif::{
    BifArguments, BifId, MAX_BORROWED_ARGUMENTS, OwnedBifArguments, dispatch_leaf,
    error_info_location, is_function, is_leaf, is_pbeam_bif, mfa, resolve, send,
};
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

/// The module of the error handler of a process that does not set one, as
/// the atom `error_handler` of `erts_proc_get_error_handler` when the
/// process-specific data holds no handler (`erl_process.h:2420-2430`).
const DEFAULT_ERROR_HANDLER: &str = "error_handler";

#[derive(Debug)]
pub struct BeamProgram {
    /// The start modules by name. Each run starts its code table with them
    /// (see [`CodeTable`]).
    modules: HashMap<String, Arc<BeamModule>>,
    /// The atom table of the loaded modules. ERTS has one atom table for
    /// each emulator, so each run starts its own table from a copy of it.
    /// The `Arc` lets a [`HostTerm`] of this program share the table.
    atoms: Arc<AtomTable>,
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
    /// The process switches out to run the work of its NIF call on a
    /// thread of the dirty pool of `job`. `entry` is the module, the name,
    /// and the arguments of the NIF.
    Dirty {
        job: Job,
        work: Work,
        entry: (String, String, Vec<Term>),
    },
}

/// The class of an exception.
#[derive(Debug, Clone, Copy)]
pub(in crate::runtime) enum ExceptionClass {
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
            Term::Atom(atom) => {
                if atom.is_static("error") {
                    Some(ExceptionClass::Error)
                } else if atom.is_static("exit") {
                    Some(ExceptionClass::Exit)
                } else if atom.is_static("throw") {
                    Some(ExceptionClass::Throw)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Force the next scheduler thread creation to fail, for a test of the
/// start path.
#[cfg(test)]
pub(super) static FAIL_SCHEDULER_SPAWN: AtomicBool = AtomicBool::new(false);

/// A one-use gate that releases the scheduler threads after each created
/// its root. `abort` releases them when a scheduler thread cannot start, so
/// no thread waits without end on a barrier that will not open.
struct StartGate {
    state: Mutex<StartState>,
    ready: Condvar,
}

/// The state of [`StartGate`].
struct StartState {
    /// The number of threads that created their root.
    arrived: usize,
    /// The number of threads of the run.
    expected: usize,
    /// True when a scheduler thread cannot start.
    aborted: bool,
}

impl StartGate {
    /// Return a closed gate for `expected` scheduler threads.
    fn new(expected: usize) -> Self {
        Self {
            state: Mutex::new(StartState {
                arrived: 0,
                expected,
                aborted: false,
            }),
            ready: Condvar::new(),
        }
    }

    /// Record that one scheduler thread created its root.
    fn arrive(&self) {
        let mut state = self.state.lock().expect("the start gate is not poisoned");
        state.arrived += 1;
        self.ready.notify_all();
    }

    /// Wait until each scheduler thread created its root. Return false when
    /// the run was aborted.
    fn wait(&self) -> bool {
        let mut state = self.state.lock().expect("the start gate is not poisoned");
        while state.arrived < state.expected && !state.aborted {
            state = self
                .ready
                .wait(state)
                .expect("the start gate is not poisoned");
        }
        !state.aborted
    }

    /// Release each waiting scheduler thread after a thread cannot start.
    fn abort(&self) {
        let mut state = self.state.lock().expect("the start gate is not poisoned");
        state.aborted = true;
        self.ready.notify_all();
    }
}

/// Abort the start gate when a scheduler thread ends before the gate opens,
/// also on a panic. Disarm it after the gate opened.
struct StartGuard<'a>(Option<&'a StartGate>);

impl Drop for StartGuard<'_> {
    fn drop(&mut self) {
        if let Some(gate) = self.0 {
            gate.abort();
        }
    }
}

/// Wait for input in a run with scheduler threads, or decide that a root
/// waits for a message that cannot arrive. A scheduler with a timer, a dirty
/// call, or an input source waits for it, as the one-thread run does.
/// Another scheduler waits in the idle state of the node until a delivery
/// or an event wakes it. The scheduler that makes each scheduler of the
/// node idle while no delivery waits in an inbox gives the error (see
/// `Node::enter_idle`): then each live process waits, and no source can
/// send to it.
fn idle_step(scheduler: &mut Scheduler, node: &Node) -> Result<(), BeamError> {
    // A scheduler with no runnable process takes work from another
    // scheduler while it waits (`try_steal_task`, `erl_process.c:4640`):
    // the mark lets a scheduler with more work give it some at its next
    // step (`Scheduler::serve_steal`), which ends the wait with a delivery.
    let index = scheduler.index();
    node.set_hungry(index, true);
    let waited = idle_wait(scheduler, node);
    node.set_hungry(index, false);
    // Each process that a steal gave this scheduler is in the inbox now, and
    // it enters the slots before a timer, an OS signal, or a dirty
    // completion of the next loop step looks for it.
    scheduler.drain_inbox();
    waited
}

/// Wait in [`idle_step`] for the next timer, event, or delivery.
fn idle_wait(scheduler: &mut Scheduler, node: &Node) -> Result<(), BeamError> {
    let wait = scheduler.timers.wait_time(scheduler.timers.now());
    if scheduler.timers.active() || scheduler.has_external_sources() {
        scheduler.wait_events(Some(wait))?;
        scheduler.timers.now();
        return Ok(());
    }
    if node.enter_idle() {
        return Err(error("root process waits for a message that cannot arrive"));
    }
    // The scheduler stays counted as idle for the wait, also when the check
    // found a delivery. A sender writes the wake byte after it puts its
    // delivery into the inbox, and the run loop takes the inbox at the next
    // scheduling step after each wait, so a delivery in the inbox always
    // has a wake byte that ends this wait.
    // The wait watches the wake socket of the scheduler, which each delivery
    // to its inbox writes, and ends at the next time correction check at the
    // latest. Without a port or a select of fd 0, the wait handles no event,
    // so the scheduler changes no state while the node counts it as idle.
    let waited = scheduler.wait_events(Some(wait));
    node.leave_idle();
    // A scheduler reads the clock after each wake, so a timer that is due
    // at the wake expires before the next process runs
    // (`erl_process.c:3568`, `3605`, `3613-3614`).
    scheduler.timers.now();
    waited?;
    Ok(())
}

impl BeamProgram {
    /// Load BEAM files into one program. Module names must be unique. A
    /// module with the name of a preloaded module of ERTS keeps no line
    /// table, as ERTS strips it from the preloaded modules. The
    /// atom table starts with the 1150 static atoms and the preloaded
    /// atoms. Each module, in the
    /// given order, puts its `AtU8` atoms and then its `LitT` atoms into it.
    /// A module with an `-on_load` function is the error `the start module
    /// M has an -on_load function, which PBeam runs only for a module that
    /// loads in the run`. The ERTS boot runs the function of each such
    /// module that the boot script loads (`init.erl:1381-1403`, `1857-1880`),
    /// and the start set of PBeam has no boot script.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let program = BeamProgram::load(&[CALLS, EXTERNAL])?;
    /// assert!(BeamProgram::load(&[CALLS, CALLS]).is_err());
    /// ```
    pub fn load(modules: &[&[u8]]) -> Result<Self, BeamError> {
        let atoms = AtomTable::new();
        let mut loaded = HashMap::new();
        for bytes in modules {
            let mut module = code::parse(bytes, &atoms)?;
            // ERTS builds each preloaded module into the emulator without
            // its `Line` chunk (`beam_strip`, `make_preload:143-157`), so
            // each frame of such a module has the location `[]`.
            if bif::PRELOADED.contains(&module.name.as_str()) {
                module.lines = LineTable::default();
            }
            let name = module.name.clone();
            if module.on_load.is_some() {
                return Err(error(format!(
                    "the start module {name} has an -on_load function, which PBeam runs only for a module that loads in the run"
                )));
            }
            if loaded.insert(name.clone(), Arc::new(module)).is_some() {
                return Err(error(format!("duplicate BEAM module {name}")));
            }
        }
        Ok(Self {
            modules: loaded,
            atoms: Arc::new(atoms),
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

    /// Return the atom table of this program. A caller passes it as the
    /// owning table of a term that it builds from [`BeamProgram::atom`], for
    /// example to [`Map::from_pairs`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let program = BeamProgram::load(&[ATOM_MAP_PROBE])?;
    /// assert_eq!(program.atoms().name(506), Some("ok"));
    /// ```
    pub fn atoms(&self) -> &AtomTable {
        &self.atoms
    }

    /// Check each native declaration of the loaded modules against the
    /// bindings of the registered extension libraries. A declaration without
    /// a binding is an error that names the MFA. Call it before a run starts
    /// the application, so a missing implementation fails before the run
    /// executes the declaration.
    ///
    /// A run that skips this check still stops at the call: the stub raises
    /// `:pbeam_native_only` when no binding matches
    /// (`docs/portable-beam-spec.md`, section "Elixir declaration").
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let program = BeamProgram::load(&[NATIVE_HASH])?;
    /// register_library(&NATIVE_HASH_LIBRARY).unwrap();
    /// assert!(program.validate_native_declarations().is_ok());
    /// ```
    pub fn validate_native_declarations(&self) -> Result<(), BeamError> {
        let mut names: Vec<&String> = self.modules.keys().collect();
        names.sort();
        for name in names {
            let module = &self.modules[name];
            for (function, arity) in module.native_declarations() {
                if super::extension::binding(name, function, *arity).is_none() {
                    return Err(error(format!(
                        "native declaration {name}:{function}/{arity} has no registered implementation"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Pair `term` with the atom table of this program. Pass the result as
    /// an argument of [`BeamProgram::execute`],
    /// [`BeamProgram::execute_with_policy`], or
    /// [`BeamProgram::execute_with_policy_module`]. A copy of the result is
    /// cheap: the term copy is O(1), and the table is shared. The result of
    /// a run is already a `HostTerm` that carries the table of the run, so
    /// use this function for a term that the caller builds with
    /// [`BeamProgram::atom`].
    ///
    /// # Precondition
    ///
    /// Every atom of `term` must come from this program, through
    /// [`BeamProgram::atom`], or be a static atom, whose index is the same
    /// in every table. A term of another table enters through
    /// `HostTerm::import_into` first. This function reads no atom of the
    /// term, because a bare atom term holds an index and no table identity,
    /// so it cannot prove that the term belongs to the program. A foreign
    /// index is a caller error, and no run-time check detects it. When the
    /// program table gives that index to a different name, the host term
    /// reads that other name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let argument = program.host_term(Term::Atom(program.atom("ok").unwrap()));
    /// let applied = program.execute("process_probe", "apply", &[argument])?;
    /// ```
    pub fn host_term(&self, term: Term) -> HostTerm {
        HostTerm {
            term,
            atoms: Arc::clone(&self.atoms),
        }
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
    /// waits for a pipe, a dirty NIF call runs, or a signal handler is
    /// installed, it waits for fd 0, for the pipes, for the completion of the
    /// call, and for the handled host signals until then (see
    /// `Scheduler::wait_events`). Before each scheduling step, it
    /// sends `{notify, Signal}` to `erl_signal_server` for each handled
    /// signal that arrived (see `Scheduler::check_signals`), and a process
    /// whose dirty NIF call completed goes to the run queue (see
    /// `Scheduler::complete_dirty_calls`). The run ends at
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
    /// select, no port waits for a pipe, no dirty NIF call runs, and no
    /// signal handler is installed gives an error. When the run ends, it
    /// does not wait for a dirty NIF call that runs, and a call in the queue
    /// does not run (see `DirtyCalls`). A call of `erlang:halt/2` with a
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
        arguments: &[HostTerm],
    ) -> Result<HostTerm, BeamError> {
        self.execute_issued(
            module,
            function,
            arguments,
            Issuer::Unchecked,
            &mut None,
            false,
        )
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
        arguments: &[HostTerm],
        policy: &dyn Policy,
    ) -> Result<HostTerm, BeamError> {
        self.execute_issued(
            module,
            function,
            arguments,
            Issuer::Host(policy),
            &mut None,
            false,
        )
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
        arguments: &[HostTerm],
        policy: &str,
        boot: bool,
    ) -> Result<HostTerm, BeamError> {
        let issuer = Issuer::Module { policy, boot };
        self.execute_issued(module, function, arguments, issuer, &mut None, false)
    }

    /// Load the preloaded modules of the pinned ERTS, which
    /// `erlang:pre_loaded/0` names, from the file `<Module>.beam` of
    /// `directory` for each module, for example the `ebin` directory of
    /// `erts`. ERTS builds these modules into the emulator and loads them in
    /// the order of `PRELOAD_BEAM` and `ESOCK_PRELOAD_BEAM` before the first
    /// process (`erts/emulator/Makefile.in:804-828`, `erl_init.c:459-484`),
    /// so a boot script loads none of them. The program loads them in the
    /// same order, for [`BeamProgram::boot`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let program = BeamProgram::load_preloaded(Path::new("/otp/lib/erts-17.1/ebin"))?;
    /// program.boot(&arguments)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The error `cannot read <Path>: <Cause>` for a file that cannot be
    /// read, and the errors of [`BeamProgram::load`].
    pub fn load_preloaded(directory: &std::path::Path) -> Result<Self, BeamError> {
        let modules = bif::PRELOADED
            .iter()
            .map(|module| {
                let path = directory.join(format!("{module}.beam"));
                std::fs::read(&path)
                    .map_err(|cause| error(format!("cannot read {}: {cause}", path.display())))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::load(&modules.iter().map(Vec::as_slice).collect::<Vec<_>>())
    }

    /// Boot the node: run the pinned `erl_init:start(init, Arguments)` as
    /// the first process, as ERTS `erl_first_process_otp` does
    /// (`erl_init.c:2540`, `355-399`). `Arguments` is the list of
    /// `arguments`, each as a binary of its bytes, as the boot arguments of
    /// the emulator. `erl_init:start/2` runs `init:boot(Arguments)`
    /// (`erl_init.erl:35-50`, `62-69`), so the first process registers the
    /// name `init`, reads the boot file of its `-boot` flag, and evaluates
    /// the boot script in a boot process (`init.erl:586-596`,
    /// `1192-1358`). The run has no policy, as [`BeamProgram::execute`] has
    /// none.
    ///
    /// The `init` process is the root process of the run and a system
    /// process, as in ERTS (`erl_init.c:333-353`). It does not exit while
    /// the node runs (`init.erl:683-797`), so the run ends with a halt:
    /// `erlang:halt/0,1,2` gives [`BeamError::Halt`]. The exit of a system
    /// process, also of `init`, stops the run with the runtime error `System
    /// process <Pid> terminated: <Reason>`, where ERTS stops the emulator
    /// with this text (`erl_process.c:14186-14188`). The other ends of a run
    /// are those of [`BeamProgram::execute`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let halted = program.boot(&[b"-root", root, b"-bindir", bindir, b"-boot", b"start"]);
    /// assert_eq!(halted.unwrap_err(), BeamError::Halt(0));
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`BeamProgram::execute`]. A program without the
    /// `erl_init` module gives the error `missing BEAM module erl_init`.
    pub fn boot(&self, arguments: &[&[u8]]) -> Result<HostTerm, BeamError> {
        let arguments = arguments
            .iter()
            .map(|argument| Term::binary(argument.to_vec()));
        let start = [
            self.host_term(Term::static_atom("init")),
            self.host_term(Term::list_from(arguments)),
        ];
        self.execute_issued(
            "erl_init",
            "start",
            &start,
            Issuer::Unchecked,
            &mut None,
            true,
        )
    }

    /// Run one root process on each of `roots.len()` scheduler threads.
    ///
    /// Each root is `Module:Function/0`. The schedulers share one node: the
    /// process owners, the registered names, the identifier counters, and
    /// the inbox of each scheduler for the messages and signals that cross a
    /// thread boundary. Terms are shared and immutable, and one process runs
    /// on one scheduler thread only. A root is an ordinary process with a
    /// root mark. The node counts the roots, and the run ends after the exit
    /// of the last root completes; the threads join before this function
    /// returns. The node collects the result of each root. The results keep
    /// the order of `roots`, and each result is the return value of its
    /// root.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let results = program.execute_on_schedulers(&[("ping_pong", "ping"), ("ping_pong", "pong")])?;
    /// assert_eq!(results.len(), 2);
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`BeamProgram::execute`] for a root that exits with an
    /// error or waits for a message that cannot arrive, the error of an
    /// empty list of roots, and the error of a scheduler that cannot be
    /// created.
    pub fn execute_on_schedulers(
        &self,
        roots: &[(&str, &str)],
    ) -> Result<Vec<HostTerm>, BeamError> {
        if roots.is_empty() {
            return Err(error("a multi-scheduler run needs at least one root"));
        }
        let initial_mode = InitialMode::read();
        let (node, readers) = Node::new(roots.len())
            .map_err(|cause| error(format!("cannot create the scheduler node: {cause}")))?;
        let gate = Arc::new(StartGate::new(roots.len()));
        // The schedulers share one copy of the program table, so an atom
        // that one scheduler creates has one index on every scheduler.
        let atoms = Arc::new((*self.atoms).clone());
        let mut spawn_error = None;
        let results = std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(roots.len());
            let atoms = &atoms;
            for (index, ((module, function), reader)) in roots.iter().zip(readers).enumerate() {
                let node = Arc::clone(&node);
                let thread_gate = Arc::clone(&gate);
                let body = move || -> Result<_, (BeamError, bool)> {
                    // The guard aborts the gate if the scheduler cannot be
                    // created, so a peer never waits for a thread that
                    // panicked before it arrived.
                    let mut start = StartGuard(Some(&thread_gate));
                    let mut scheduler =
                        Scheduler::with_shared(Arc::clone(atoms), Arc::clone(&node), index, reader);
                    scheduler.code = CodeTable::new(self.modules.clone());
                    // The reset of the terminal mode needs the state of
                    // each scheduler after the run.
                    let run =
                        self.run_multi_root(&mut scheduler, index, module, function, &mut start);
                    let selected = scheduler.resources.borrow_mut::<Ttys>().input_selected();
                    match run {
                        Ok(()) => Ok(selected),
                        Err(cause) => {
                            node.abort();
                            Err((cause, selected))
                        }
                    }
                };
                let force_failure = {
                    #[cfg(test)]
                    {
                        FAIL_SCHEDULER_SPAWN.swap(false, AtomicOrdering::SeqCst)
                    }
                    #[cfg(not(test))]
                    {
                        false
                    }
                };
                let spawned = if force_failure {
                    Err(std::io::Error::other(
                        "a test forces a scheduler thread failure",
                    ))
                } else {
                    std::thread::Builder::new().spawn_scoped(scope, body)
                };
                match spawned {
                    Ok(handle) => handles.push(handle),
                    Err(cause) => {
                        spawn_error =
                            Some(error(format!("cannot create a scheduler thread: {cause}")));
                        gate.abort();
                        break;
                    }
                }
            }
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .expect("a scheduler thread does not panic in a multi-scheduler run")
                })
                .collect::<Vec<_>>()
        });
        if let Some(cause) = spawn_error {
            return Err(cause);
        }
        // Restore the terminal mode of the run, also after a failed root.
        let selected = results.iter().any(|result| match result {
            Ok(selected) | Err((_, selected)) => *selected,
        });
        initial_mode.reset(selected);
        // Each root result is a term of the shared table of the run.
        let roots = node.take_root_results();
        // A failed root aborts the run. Report its error before the wait
        // error or the missing result of a root that the abort stopped.
        for root in &roots {
            if let Some(Err(reason)) = root {
                return Err(Self::root_error(reason, &atoms));
            }
        }
        for result in results {
            result.map_err(|(cause, _)| cause)?;
        }
        let mut results_out = Vec::with_capacity(roots.len());
        for root in roots {
            let root = root.expect("a root that did not exit reports its error");
            let term = root.expect("a failed root returns above");
            results_out.push(HostTerm {
                term,
                atoms: Arc::clone(&atoms),
            });
        }
        Ok(results_out)
    }

    /// Return the error of a root process that exited with `reason`.
    fn root_error(reason: &Term, atoms: &AtomTable) -> BeamError {
        let text: String = print_term(reason, atoms)
            .into_iter()
            .map(char::from)
            .collect();
        error(format!("root process exited with {text}"))
    }

    /// Apply the outcome of one process turn, as each run loop does.
    fn apply_outcome(&self, scheduler: &mut Scheduler, outcome: Outcome) -> Result<(), BeamError> {
        match outcome {
            Outcome::Wait => scheduler.park_current(),
            Outcome::Hibernate => scheduler.hibernate_current(),
            Outcome::Yield => scheduler.yield_current(),
            Outcome::Exit { reason, result } => scheduler.finish_current(reason, result)?,
            Outcome::Dirty { job, work, entry } => scheduler.start_dirty_call(job, work, entry)?,
        }
        Ok(())
    }

    /// Start the root of the slot `slot` of a multi-scheduler run, and run
    /// the scheduler until the run ends: after the exit of the last root,
    /// or after a root or a scheduler failed.
    fn run_multi_root(
        &self,
        scheduler: &mut Scheduler,
        slot: usize,
        module: &str,
        function: &str,
        start: &mut StartGuard<'_>,
    ) -> Result<(), BeamError> {
        let node = Arc::clone(scheduler.node());
        let label = scheduler.code.entry_label(module, function, 0)?;
        let frame = scheduler.code.frame(module, label, 0)?;
        let id = scheduler.next_pid()?;
        scheduler.start_with_id(id, slot, frame, Vec::new(), Vec::new())?;
        // Wait until each scheduler created its root, so an idle root cannot
        // decide that its message cannot arrive before another root started.
        let gate = start.0.expect("the start gate is armed");
        gate.arrive();
        if !gate.wait() {
            return Err(error("the multi-scheduler run was cancelled"));
        }
        start.0 = None;
        // After a root exits, its scheduler keeps running its other
        // processes, because a process of another scheduler can still depend
        // on them until the last root exits.
        while !node.is_shutdown() {
            let now = scheduler.timers.timer_time();
            scheduler.expire_timers(now);
            scheduler.check_signals()?;
            scheduler.complete_dirty_calls();
            match scheduler.schedule_next()? {
                Turn::Run => {
                    let outcome = self.run(scheduler)?;
                    self.apply_outcome(scheduler, outcome)?;
                }
                Turn::Done => {}
                Turn::Idle => idle_step(scheduler, &node)?,
            }
        }
        Ok(())
    }

    /// Run an exported function as the root process, as
    /// [`BeamProgram::execute`] does, and call `sample` with the live bytes
    /// of the guest containers before each scheduling step. The bytes are
    /// the accounting of the runtime containers: the element array of a
    /// tuple, the cells of a list, the forms and pairs of a map, the
    /// backing of a binary, the free variables of a fun, the frames and
    /// slots of a process, the mailbox, and the dictionary, each counted
    /// once for each allocation (see `Scheduler::live_bytes`). The run has
    /// no policy, as [`BeamProgram::execute`] has none.
    ///
    /// The sample runs outside the interpreter loop, at a scheduling step,
    /// so the accounting changes no term behavior and no measured hot path.
    /// A sample is the state at a scheduling step, and the run samples no
    /// state inside a turn.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut peak = 0;
    /// let result = program.execute_sampled("terra_probe", "main", &[], &mut |live| peak = peak.max(live))?;
    /// assert!(peak > 0);
    /// ```
    pub fn execute_sampled(
        &self,
        module: &str,
        function: &str,
        arguments: &[HostTerm],
        sample: &mut dyn FnMut(usize),
    ) -> Result<HostTerm, BeamError> {
        let mut sample = Some(sample);
        self.execute_issued(
            module,
            function,
            arguments,
            Issuer::Unchecked,
            &mut sample,
            false,
        )
    }

    /// Run an exported function as the root process with the initial
    /// capabilities of `issuer`, as [`BeamProgram::execute_with_policy`] and
    /// [`BeamProgram::execute_with_policy_module`] describe. With
    /// `system_root`, the root process is a system process, as the `init`
    /// process of [`BeamProgram::boot`] is.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let result = program.execute_issued("process_probe", "main", &[], Issuer::Host(&AllowAll), &mut None, false)?;
    /// ```
    fn execute_issued(
        &self,
        module: &str,
        function: &str,
        arguments: &[HostTerm],
        issuer: Issuer,
        sample: &mut Option<&mut dyn FnMut(usize)>,
        system_root: bool,
    ) -> Result<HostTerm, BeamError> {
        let initial_mode = InitialMode::read();
        let mut scheduler = match issuer {
            Issuer::Unchecked => Scheduler::without_checks((*self.atoms).clone()),
            Issuer::Host(_) | Issuer::Module { .. } => Scheduler::new((*self.atoms).clone()),
        };
        if system_root {
            scheduler.mark_root_system();
        }
        scheduler.code = CodeTable::new(self.modules.clone());
        // Each argument of another table is imported into the table of the
        // run. `run_root` then runs with terms of one table.
        let imported = arguments
            .iter()
            .map(|argument| argument.import_into(&scheduler.atoms))
            .collect::<Result<Vec<Term>, BeamError>>();
        let run = match imported {
            Ok(arguments) => {
                self.run_root(&mut scheduler, module, function, &arguments, issuer, sample)
            }
            Err(import_error) => Err(import_error),
        };
        initial_mode.reset(scheduler.resources.borrow_mut::<Ttys>().input_selected());
        run?;
        // The node holds the result of the root. The table of the run holds
        // each name that the run created, so the host result can name every
        // atom of the root result.
        let root = scheduler.node().take_root_results().pop().flatten();
        let atoms = scheduler.into_atoms();
        match root {
            Some(Ok(result)) => Ok(HostTerm {
                term: result,
                atoms,
            }),
            Some(Err(reason)) => {
                // `erlang:halt/1` writes each byte of the `%T` text as one code
                // point (`bif.c:4372-4387`), so the text is valid UTF-8.
                let text: String = print_term(&reason, &atoms)
                    .into_iter()
                    .map(char::from)
                    .collect();
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
    /// let mut scheduler = Scheduler::new((*program.atoms).clone());
    /// scheduler.code = CodeTable::new(program.modules.clone());
    /// program.run_root(&mut scheduler, "process_probe", "main", &[], Issuer::Host(&AllowAll))?;
    /// assert!(scheduler.node().take_root_results()[0].is_some());
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
        sample: &mut Option<&mut dyn FnMut(usize)>,
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
        sample_live(scheduler, sample);
        // The node ends the run after the exit of the root completes.
        while !scheduler.node().is_shutdown() {
            if let Some(initial) = scheduler.take_policy_initial() {
                let capabilities = initial.map_err(|cause| {
                    error(format!("the policy denied the root process: {cause}"))
                })?;
                match root.take() {
                    Some(frame) => scheduler.start(frame, arguments.to_vec(), capabilities)?,
                    None => scheduler.enter_runtime(capabilities),
                }
            }
            let now = scheduler.timers.timer_time();
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
            scheduler.complete_dirty_calls();
            match scheduler.schedule_next()? {
                // A message woke the policy process, but it has no call.
                Turn::Run if scheduler.policy_idle() => scheduler.park_current(),
                Turn::Run => {
                    let outcome = self.run(scheduler)?;
                    let outcome = policy_turn(scheduler, outcome);
                    self.apply_outcome(scheduler, outcome)?;
                }
                Turn::Done => {}
                Turn::Idle => {
                    // The wait ends at the next time correction check, but
                    // that check alone does not keep a run alive.
                    let wait = scheduler.timers.wait_time(scheduler.timers.now());
                    if !scheduler.wait_events(Some(wait))? {
                        if !scheduler.timers.active() {
                            break;
                        }
                        std::thread::sleep(wait);
                    }
                    scheduler.timers.now();
                }
            }
            sample_live(scheduler, sample);
        }
        sample_live(scheduler, sample);
        if root.is_some() {
            return Err(error("the policy process waits in initial/1"));
        }
        Ok(())
    }

    /// Run the current process until it waits for a message, calls
    /// `erlang:hibernate/0`, exits, or switches out. A process with an
    /// undefined start first calls its start MFA through `call_mfa`.
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
        // arguments, and else a BIF or a function that the module does not
        // export goes to its export entry when the process first runs. The
        // arity is the number of argument registers.
        let process = scheduler.current_process_mut();
        if let Some((module, function)) = process.undefined_start.take() {
            let outcome = if process.registers.len() > APPLY_MAX_ARGUMENTS {
                // ERTS `apply` raises `system_limit` at the 1024th element
                // of the argument list, before it looks up the export
                // (`beam_common.c:1503-1513`). The registers then hold the
                // module, the function, and the list
                // (`beam_common.c:1455-1459`), so the stack trace is the one
                // frame `{erlang, apply, [Module, Function, Arguments],
                // [{error_info, ...}]}`.
                let arity = process.registers.len() as u32;
                let arguments = call_arguments(&process.registers, arity)?;
                let call = vec![
                    Term::Atom(module),
                    Term::Atom(function),
                    Term::list_from(arguments),
                ];
                self.apply_error(scheduler, call, "system_limit", true)?
            } else {
                let arity = process.registers.len() as u32;
                // The start of a process is a tail call of the MFA from the
                // initial function of ERTS (`i_apply_only`,
                // `erl_process.c:12616`), which goes to the export entry of
                // the MFA (`beam_common.c:1559-1565`). PBeam starts the
                // process with no frame, so a BIF or the error handler runs
                // as a body call of the first frame. The result of the BIF,
                // or the `return` of the handler, then ends the process.
                self.call_mfa(scheduler, &module, &function, arity, false)?
            };
            if let Some(outcome) = outcome {
                return Ok(outcome);
            }
        }
        // A NIF call that ran on a dirty thread returns its result or raises
        // its exception, as `dirty_nif_finalizer` and `dirty_nif_exception`
        // do (`erl_nif.c:3386-3435`). The exception has the MFA and the
        // arguments of the NIF first in its stack trace
        // (`erl_nif.c:3431-3433`).
        if let Some(DirtyCall {
            entry: (module, name, arguments),
            output: Some(output),
            ..
        }) = scheduler.current_process_mut().dirty_call.take()
        {
            let result = match output {
                Output::Written(written) => native::dirty_write_result(&scheduler.atoms, written)?,
                Output::Native(result) => native::to_bif_result(result?),
                Output::Host(finish) => finish.run(scheduler)?,
            };
            let outcome = match result {
                BifResult::Raise { class, reason } => {
                    let entry = (module.as_str(), name.as_str(), arguments, Term::nil());
                    raise_at_entry(scheduler, entry, class, reason)?
                }
                result => self.finish_bif(scheduler, result, true)?,
            };
            if let Some(outcome) = outcome {
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
            origin,
        }) = scheduler.current_process_mut().pending_bif.take()
        {
            // The `lists` BIFs trap to their own export with the rest of
            // the list in the registers, and an error after the trap keeps
            // these registers (`BIF_TRAP2` and `BIF_ERROR`,
            // `erl_bif_lists.c:1129-1139`, `1191`, `1240-1243`). The other
            // BIFs put the original arguments back
            // (`ERTS_BIF_ERROR_TRAPPED1`, `bif.h:242-249`).
            let registers = (module == "lists").then(|| arguments.clone());
            let result = self.bif_trap(scheduler, module, function, arguments)?;
            let result = match origin {
                Some(call)
                    if matches!(
                        result,
                        BifResult::Raise { .. } | BifResult::ErrorWithCause { .. }
                    ) =>
                {
                    let BifCall {
                        module,
                        name,
                        arguments,
                        skip,
                    } = call;
                    let arguments = registers.unwrap_or(arguments);
                    bif_error(scheduler, &module, &name, arguments, result, skip)?
                }
                // The BIF traps again and keeps the call.
                Some(call) if matches!(result, BifResult::TrapBif { .. }) => {
                    scheduler.current_process_mut().trap_origin = Some(call);
                    result
                }
                _ => result,
            };
            if let Some(outcome) = self.finish_bif(scheduler, result, tail)? {
                return Ok(outcome);
            }
        }
        // A BIF trap whose target had no loaded export resolves the target
        // in this turn, after the switch out of the trap
        // (`context_switch_simplified` in `jit/arm/instr_bif.cpp:525-536`,
        // `676-701`).
        if let Some(PendingTrap {
            module,
            function,
            arity,
            tail,
        }) = scheduler.current_process_mut().pending_trap.take()
            && let Some(outcome) = self.resume_trap(scheduler, module, function, arity, tail)?
        {
            return Ok(outcome);
        }
        let process = scheduler.current_process_mut();
        if process.frames.is_empty() {
            return normal_exit(process);
        }
        // A frame names its module, and the code of a module name does not
        // change in a run (see `CodeTable`). Thus the cursor keeps the code
        // of the active frame, and each exit that can change the active
        // module refreshes it. The refresh compares the integer identity of
        // the two modules, so a call, a return, and an exception each pay
        // that compare and no name compare. The loop reads the active frame
        // only where an instruction needs it, and not on each iteration. The
        // test profile asserts that the refresh keeps the two modules equal,
        // so a path that changes the active module without a refresh fails a
        // test instead of running another module's code in release.
        let mut cursor = Cursor::new(scheduler)?;
        loop {
            let process = scheduler.current_process_mut();
            debug_assert_eq!(
                current_frame(process).module_id,
                cursor.module.id,
                "a frame change must refresh the cursor"
            );
            let module: &BeamModule = &cursor.module;
            let Some(instruction) = module.instructions.get(cursor.position) else {
                let frame = current_frame(process);
                return Err(error(format!(
                    "function {}:{} did not return",
                    module.name,
                    module.tables.name(frame.function)
                )));
            };
            cursor.position += 1;
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
                Instruction::Call {
                    arity,
                    function,
                    target,
                } => {
                    check_local_call_arguments(&process.registers, *arity)?;
                    cursor.commit(process);
                    let frame = code_table::frame_at(module, *function, *target, *arity)?;
                    cursor.position = frame.next_instruction;
                    process.frames.push(frame);
                }
                Instruction::CallOnly {
                    arity,
                    function,
                    target,
                } => {
                    check_local_call_arguments(&process.registers, *arity)?;
                    let frame = code_table::frame_at(module, *function, *target, *arity)?;
                    cursor.position = frame.next_instruction;
                    *current_frame(process) = frame;
                }
                Instruction::CallLast {
                    arity,
                    function,
                    target,
                    deallocate,
                } => {
                    if *deallocate as usize != current_frame(process).stack.len() {
                        return Err(error("invalid BEAM tail-call stack size"));
                    }
                    check_local_call_arguments(&process.registers, *arity)?;
                    let frame = code_table::frame_at(module, *function, *target, *arity)?;
                    cursor.position = frame.next_instruction;
                    *current_frame(process) = frame;
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
                        cursor.position = label_position(*fail_label)?;
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
                        cursor.position = label_position(*fail_label)?;
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
                    cursor.position = label_position(target)?;
                }
                Instruction::CallExternal {
                    import,
                    tail,
                    deallocate,
                } => {
                    cursor.commit(process);
                    // A tail form has Y registers to release; a body call
                    // has none, and the parser writes 0 for it.
                    let deallocate = tail.then_some(*deallocate);
                    if let Some(outcome) =
                        self.call_external(scheduler, module, &cursor.imports, *import, deallocate)?
                    {
                        return Ok(outcome);
                    }
                    cursor.refresh(scheduler)?;
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
                    cursor.commit(process);
                    let module = read(process, &Operand::X(*arity))?;
                    let name = read(process, &Operand::X(arity.saturating_add(1)))?;
                    // ERTS `fixed_apply` (`beam_common.c`) raises `badarg`
                    // when the module or the function name is not an atom.
                    // It raises with the BIF frame of `erlang:apply/3` and
                    // the arguments `[Module, Function, Args]`
                    // (`fixed_apply`, `beam_common.c:1529-1551`).
                    let (Term::Atom(module), Term::Atom(name)) = (&module, &name) else {
                        let arguments = call_arguments(&process.registers, *arity)?;
                        let call = vec![module, name, Term::list_from(arguments)];
                        let raised =
                            bif_error(scheduler, "erlang", "apply", call, BifResult::badarg(), 0)?;
                        if let Some(outcome) = self.finish_bif(scheduler, raised, false)? {
                            return Ok(outcome);
                        }
                        cursor.refresh(scheduler)?;
                        continue;
                    };
                    if let Some(outcome) = self.call_mfa(scheduler, module, name, *arity, *tail)? {
                        return Ok(outcome);
                    }
                    cursor.refresh(scheduler)?;
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
                    let fun = Term::Fun(Fun::new((**lambda).clone(), free));
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
                    cursor.commit(process);
                    if let Some(outcome) = self.call_fun(scheduler, call, *arity, *tail)? {
                        return Ok(outcome);
                    }
                    cursor.refresh(scheduler)?;
                }
                Instruction::Bif {
                    fail_label,
                    bif,
                    arguments,
                    destination,
                } => {
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
                    // The argument `Vec<Term>` of a `bif` instruction is
                    // one of the two allocations that remain on the call
                    // path (step 3 of the plan). The `bif` path therefore
                    // has two argument lanes. A leaf identity of [`is_leaf`]
                    // reads its arguments into a borrowed [`BifArguments`]
                    // and runs [`dispatch_leaf`], which takes no scheduler,
                    // so the borrow of the process stays valid. Every other
                    // identity copies its arguments into an
                    // [`OwnedBifArguments`] and runs the owned dispatch, as
                    // before. The loader gives a `bif` instruction at most
                    // `MAX_BORROWED_ARGUMENTS` arguments, so each lane
                    // covers every `bif` instruction.
                    //
                    // `erlang:'+'/2` and `erlang:'-'/2` of two small
                    // integers run here, with no BIF call, as
                    // `emit_i_plus` and `emit_i_minus` add or subtract two
                    // small integers inline and call the BIF for each other
                    // case (`jit/arm/instr_arith.cpp:145-205`, `:334-392`).
                    // No `length/1` trap state can be pending at these two
                    // identities: the trap runs its own instruction again
                    // before any other instruction of the process.
                    if let Some(value) = small_integer_arith(process, *bif, arguments) {
                        write(process, destination, value)?;
                        continue;
                    }
                    let result = if let Some(state) = process.length_trap.take() {
                        self.bif_trap(scheduler, "erlang", "length", state)?
                    } else if is_leaf(*bif) {
                        // The leaf read borrows the process, so the commit
                        // of the advanced position runs first. The commit
                        // writes the same position in either order, and
                        // the read errors leave the run.
                        cursor.commit(process);
                        let values = read_bif_arguments(process, arguments)?;
                        // No leaf identity is `bit_size/1` or
                        // `byte_size/1`, so the size case of
                        // `match_context_size` cannot apply. The call
                        // still reports a match context in a leaf
                        // argument, as the owned lane does. The assertion holds the two facts together,
                        // so an identity that joins the leaf set without
                        // this case fails a test.
                        let size =
                            values.with(|terms| match_context_size(*bif, arguments, terms))?;
                        debug_assert_eq!(size, None);
                        let mut reductions = 0;
                        let result =
                            values.with(|terms| dispatch_leaf(*bif, terms, &mut reductions))?;
                        scheduler.current_process_mut().bump_reductions(reductions);
                        result
                    } else {
                        let values = read_owned_bif_arguments(process, arguments)?;
                        cursor.commit(process);
                        let size =
                            values.with(|terms| match_context_size(*bif, arguments, terms))?;
                        match size {
                            Some(size) => BifResult::Value(size),
                            None => values.with(|terms| self.bif_slice(scheduler, *bif, terms))?,
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
                            cursor.position -= 1;
                            cursor.commit(process);
                            return Ok(Outcome::Yield);
                        }
                        BifResult::Raise { .. } | BifResult::ErrorWithCause { .. }
                            if *fail_label != 0 =>
                        {
                            cursor.position = label_position(*fail_label)?
                        }
                        raised @ (BifResult::Raise { .. } | BifResult::ErrorWithCause { .. }) => {
                            // The BIF wrote no destination, so the operands
                            // still hold the arguments of the call, also the
                            // list of a `length/1` trap
                            // (`save_stacktrace`, `beam_common.c:1005`).
                            let values = arguments
                                .iter()
                                .map(|argument| read(process, argument))
                                .collect::<Result<_, _>>()?;
                            let (module_name, name, _) = mfa(*bif);
                            let raised =
                                bif_error(scheduler, module_name, name, values, raised, 0)?;
                            if let Some(outcome) = self.finish_bif(scheduler, raised, false)? {
                                return Ok(outcome);
                            }
                            cursor.refresh(scheduler)?;
                        }
                        // The compiler emits a `bif` instruction only for a
                        // BIF that does not trap or end the caller.
                        trap => {
                            let (module, name, arity) = mfa(*bif);
                            return Err(error(format!(
                                "unsupported {module}:{name}/{arity} result {trap:?}"
                            )));
                        }
                    }
                }
                Instruction::Jump(label) => cursor.position = label_position(*label)?,
                Instruction::Catch { register, label } => {
                    // ERTS also counts `c_p->catches` here. PBeam finds a live
                    // tag with a frame scan in `raise` and needs no counter.
                    let handler = label_position(*label)?;
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
                    let (process, atoms) = scheduler.current_process_and_atoms();
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
                                    raw_stacktrace(read_ref(process, &Operand::X(2))?, atoms)?;
                                reason = Term::tuple(vec![reason, stack]);
                            }
                            reason = Term::tuple(vec![Term::static_atom("EXIT"), reason]);
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
                        // `is_binary` fails for a bitstring with tail bits
                        // (emu `instrs.tab:939-943`; JIT `emit_is_binary` in
                        // `jit/arm/instr_common.cpp:1186-1224`).
                        TestKind::Binary => value.is_binary(),
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
                        TestKind::Integer => value.is_integer(),
                        // `is_float` passes only for a boxed term with the
                        // `HEADER_FLONUM` header (emu `instrs.tab:951-955`,
                        // `erl_term.h:484-485`; JIT `emit_is_float` in
                        // `jit/arm/instr_common.cpp:1229-1245`).
                        TestKind::Float => matches!(value, Term::Float(_)),
                        // `is_number` passes for a small integer, a bignum,
                        // and a float (emu `instrs.tab:999-1003`:
                        // `is_not_integer(Src) && is_not_float(Src)`; JIT
                        // `emit_is_number` in
                        // `jit/arm/instr_common.cpp:1403-1444`).
                        TestKind::Number => value.is_integer() || matches!(value, Term::Float(_)),
                        TestKind::Boolean => {
                            matches!(value, Term::Atom(atom) if atom.is_one_of(&["true", "false"]))
                        }
                        // `is_any_native_record` passes only for a boxed term
                        // with the `_TAG_HEADER_RECORD` header (emu
                        // `instrs.tab:1323-1327`; JIT
                        // `emit_is_any_native_record` in
                        // `jit/arm/instr_records.cpp:30-45`;
                        // `erl_term.h:139`, `167`). PBeam has no native record
                        // term, so the test fails for each term.
                        TestKind::AnyNativeRecord => false,
                    };
                    if !passed {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                Instruction::GetMapElements {
                    fail_label,
                    map,
                    pairs,
                } => {
                    let Term::Map(map) = read_ref(process, map)? else {
                        return Err(error("BEAM get_map_elements source is not a map"));
                    };
                    // A destination can be the source register, so the arm
                    // keeps one copy of the map handle.
                    let map = map.clone();
                    if !get_map_elements(process, &map, pairs)? {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                // ERTS loads `has_map_fields` as `get_map_elements` with
                // scratch destinations (`generators.tab:230-253`).
                Instruction::HasMapFields {
                    fail_label,
                    map,
                    keys,
                } => {
                    if map_values(process, map, keys)?.is_none() {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                Instruction::PutMapAssoc {
                    map,
                    destination,
                    pairs,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let pairs = pairs
                        .iter()
                        .map(|(key, value)| Ok((read(process, key)?, read(process, value)?)))
                        .collect::<Result<Vec<_>, BeamError>>()?;
                    // PBeam checks every key before the first change and
                    // inserts the pairs. ERTS merges a flatmap source and
                    // builds a hashmap in one step above 32 keys
                    // (`erts_maps_put`, `erl_map.c:2105-2237`). The result
                    // is the same, because the flatmap order and the HAMT
                    // shape depend only on the set of keys.
                    match update_owned(process, map, destination, |entries| {
                        entries
                            .update(
                                pairs
                                    .into_iter()
                                    .map(|(key, value)| (Insert::Add, key, value))
                                    .collect(),
                                atoms,
                            )
                            .map_err(|key| {
                                error(format!("unsupported map key {}", debug_term(&key, atoms)))
                            })?;
                        Ok(None)
                    })? {
                        Update::Written => {}
                        // `put_map_assoc` has no fail label and no exception
                        // of its own. The compiler puts an `is_map` test and
                        // an `erlang:error({badmap, Map})` call before it, so
                        // compiled code gives it only a map. ERTS asserts a
                        // map and has no exception for another term
                        // (`beam_common.c:1981-1985`).
                        Update::Missing(_) | Update::Reason(_) => {
                            return Err(error("unsupported BEAM put_map_assoc source: not a map"));
                        }
                    }
                }
                Instruction::PutMapExact {
                    fail_label,
                    map,
                    destination,
                    pairs,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let pairs = pairs
                        .iter()
                        .map(|(key, value)| Ok((read(process, key)?, read(process, value)?)))
                        .collect::<Result<Vec<_>, BeamError>>()?;
                    // ERTS `update_map_exact` jumps to a fail label that is
                    // not 0, and else raises the exception
                    // (`jit/arm/instr_map.cpp:687-716`). It writes no
                    // register before the update succeeds, so the source
                    // register keeps its value on a failure
                    // (`beam_common.c:2231-2253`, `erts_gc_update_map_exact`).
                    match update_owned(process, map, destination, |entries| {
                        update_map_exact(entries, pairs, atoms)
                    })? {
                        Update::Written => {}
                        Update::Missing(value) => {
                            // ERTS `erts_gc_update_map_exact` sets
                            // `{badmap, Map}` (`beam_common.c:2224-2235`).
                            let reason = Term::tuple(vec![Term::static_atom("badmap"), value]);
                            if *fail_label != 0 {
                                cursor.position = label_position(*fail_label)?;
                            } else if let Some(outcome) =
                                raise(process, ExceptionClass::Error, reason)
                            {
                                return Ok(outcome);
                            } else {
                                cursor.refresh(scheduler)?;
                            }
                        }
                        Update::Reason(_) if *fail_label != 0 => {
                            cursor.position = label_position(*fail_label)?
                        }
                        Update::Reason(reason) => {
                            if let Some(outcome) = raise(process, ExceptionClass::Error, reason) {
                                return Ok(outcome);
                            }
                            cursor.refresh(scheduler)?;
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
                // only for a float (`1 == 1.0` is true, and
                // `0.0 == -0.0` is true).
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
                    let (left, right) = (read_ref(process, left)?, read_ref(process, right)?);
                    let arith = matches!(instruction, Instruction::TestEqual { .. });
                    if !(if arith {
                        left.arith_eq(right)
                    } else {
                        left == right
                    }) {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                // The loader selects the classified operand form of
                // `test is_eq_exact` when both operands are a register or an
                // immediate and at least one is a register. The run loop then
                // reads a checked register slot or the stored term, and it
                // matches no `Operand`.
                Instruction::TestExactEqualOperands {
                    fail_label,
                    left,
                    right,
                } => {
                    if read_test_operand(process, left)? != read_test_operand(process, right)? {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                // ERTS `is_ne` (emu instrs.tab:1115-1119, JIT
                // instr_common.cpp:2081-2105) runs `CMP_NE_ACTION`
                // (erl_utils.h:134-135) with `EQ_SPEC` in the arithmetic
                // order, as `is_eq` does.
                Instruction::TestNotEqual {
                    fail_label,
                    left,
                    right,
                } => {
                    if read_ref(process, left)?.arith_eq(read_ref(process, right)?) {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                Instruction::TestExactNotEqual {
                    fail_label,
                    left,
                    right,
                } => {
                    if read_ref(process, left)? == read_ref(process, right)? {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                Instruction::TestLess {
                    fail_label,
                    left,
                    right,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    if !read_ref(process, left)?
                        .compare(read_ref(process, right)?, atoms)?
                        .is_lt()
                    {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                Instruction::TestGreaterEqual {
                    fail_label,
                    left,
                    right,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    if read_ref(process, left)?
                        .compare(read_ref(process, right)?, atoms)?
                        .is_lt()
                    {
                        cursor.position = label_position(*fail_label)?;
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
                        cursor.position = label_position(*fail_label)?;
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
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    match read_any(process, source)? {
                        Term::Binary(ref bytes) => {
                            let state = MatchState::new(bytes.clone());
                            write(process, destination, Term::MatchContext(Arc::new(state)))?;
                        }
                        context @ Term::MatchContext(_) => {
                            write(process, destination, context)?;
                        }
                        other if *fail_label == 0 => {
                            return Err(error(format!(
                                "invariant error: bs_start_match3 with no fail label got {}, not a binary or a match context",
                                debug_term(&other, atoms)
                            )));
                        }
                        _ => cursor.position = label_position(*fail_label)?,
                    }
                }
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
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let context = any_match_context(process, source, atoms)?;
                    write(process, destination, Term::MatchContext(context))?;
                }
                // ERTS `bs_get_tail` writes the bits after the position as
                // a sub-bitstring and keeps the position
                // (`emit_bs_get_tail` and `emit_bs_get_tail_shared` in
                // jit/arm/instr_bs.cpp:407-454).
                Instruction::BinaryGetTail {
                    context,
                    destination,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let state = match_context(process, context, atoms)?;
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
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let state = match_context(process, context, atoms)?;
                    // ERTS writes a sub-bitstring that refers to the
                    // original binary and moves the position past the
                    // segment (`erts_bs_get_binary_2`, `erl_bits.c:400-424`),
                    // so the view needs no copy. The position moves before
                    // the size check, as the `Fail` jump does.
                    let bytes = binary_field_size(read_ref(process, size)?, *unit)
                        .and_then(|bits| Some((bits, state.advance(bits)?)));
                    match bytes {
                        Some((bits, ())) => {
                            let position = state.position();
                            let bits = state.bits_at(position - bits as usize, position);
                            write(process, destination, Term::Binary(bits))?;
                        }
                        None => cursor.position = label_position(*fail_label)?,
                    }
                }
                // ERTS `i_bs_get_float2` checks the size as
                // `i_bs_get_binary2` does, then reads the float or jumps to
                // `Fail` (`emit_i_bs_get_float2` in
                // jit/arm/instr_bs.cpp:578-610; `erts_bs_get_float_2` in
                // erl_bits.c:549-611). A size that is not a small
                // non-negative integer, such as a literal bignum, jumps to
                // `Fail` (`emit_bs_get_field_size`, jit/arm/
                // instr_bs.cpp:58-86), as `binary_field_size` gives `None`.
                Instruction::BinaryGetFloat {
                    fail_label,
                    context,
                    size,
                    unit,
                    little,
                    destination,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let state = match_context(process, context, atoms)?;
                    let value = binary_field_size(read_ref(process, size)?, *unit)
                        .and_then(|bits| state.get_float(bits, *little));
                    match value {
                        Some(value) => write(process, destination, Term::Float(value))?,
                        None => cursor.position = label_position(*fail_label)?,
                    }
                }
                // ERTS `i_bs_get_integer2` checks the size, then reads the
                // integer of that size and moves the position, or jumps to
                // `Fail` when fewer bits remain
                // (`emit_bs_get_integer2`, jit/arm/instr_bs.cpp:312-358;
                // `erts_bs_get_integer_2`, erl_bits.c:170-398). A size that
                // is not a small non-negative integer, such as a literal
                // bignum, jumps to `Fail` (`emit_bs_get_field_size`,
                // jit/arm/instr_bs.cpp:58-86), as `binary_field_size` gives
                // `None`. ERTS loads a small fixed size below 64 bits as a
                // `bs_match` integer command, which reads the same bits.
                Instruction::BinaryGetInteger {
                    fail_label,
                    context,
                    size,
                    unit,
                    little,
                    signed,
                    destination,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let state = match_context(process, context, atoms)?;
                    let bits = binary_field_size(read_ref(process, size)?, *unit)
                        .and_then(|bits| state.read_bits(bits).map(|read| (bits as usize, read)));
                    match bits {
                        Some((bits, read)) => {
                            let value = segment_integer(&read, bits, *little, *signed).ok_or_else(
                                || {
                                    error(
                                        "unsupported bs_get_integer2 integer outside the 64-bit \
                                         range: needs a bignum",
                                    )
                                },
                            )?;
                            write(process, destination, Term::Integer(value))?;
                        }
                        None => cursor.position = label_position(*fail_label)?,
                    }
                }
                // ERTS `i_bs_get_position` writes `sb->start` as a small
                // integer (emu ops.tab:1274-1277, bs_instrs.tab:1125-1129).
                Instruction::BinaryGetPosition {
                    context,
                    destination,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let position = match_context(process, context, atoms)?.position();
                    write(process, destination, Term::Integer(position as i64))?;
                }
                // ERTS `bs_set_position` sets `sb->start` to the unsigned
                // value of the position (emu bs_instrs.tab:1120-1123). The
                // compiler gives it a position that `bs_get_position` made.
                Instruction::BinarySetPosition { context, position } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let state = any_match_context(process, context, atoms)?;
                    match read_ref(process, position)? {
                        Term::Integer(value) if let Ok(value) = usize::try_from(*value) => {
                            state.set_position(value);
                        }
                        other => {
                            return Err(error(format!(
                                "invariant error: bs_set_position position {} is not a position",
                                debug_term(other, atoms)
                            )));
                        }
                    }
                }
                // ERTS `i_bs_skip_bits2` checks the size as
                // `i_bs_get_binary2` does, and then moves the position, or
                // jumps to `Fail` when the size is not a small non-negative
                // integer, when `size * unit` overflows, and when fewer
                // bits remain (emu bs_instrs.tab:260-283; JIT
                // `emit_i_bs_skip_bits2`, jit/arm/instr_bs.cpp:474-500).
                // `binary_field_size` gives the same two size failures.
                Instruction::BinarySkipBits {
                    fail_label,
                    context,
                    size,
                    unit,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let state = match_context(process, context, atoms)?;
                    let skipped = binary_field_size(read_ref(process, size)?, *unit)
                        .and_then(|bits| state.advance(bits));
                    if skipped.is_none() {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                // ERTS `i_bs_get_utf8` reads a byte below 128 directly and
                // else calls `erts_bs_get_utf8`, which fails for each
                // invalid form (emu ops.tab:1286-1287, bs_instrs.tab:958-981;
                // erl_bits.c:2128-2221). Both paths give the same result.
                // `bs_skip_utf8` loads as the same instruction without a
                // destination (emu ops.tab:1289), so it reads the character
                // and drops it.
                Instruction::BinaryGetUtf8 {
                    fail_label,
                    context,
                    destination,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let code_point = match_context(process, context, atoms)?.get_utf8();
                    match code_point {
                        Some(code_point) => {
                            if let Some(destination) = destination {
                                write(process, destination, Term::Integer(code_point.into()))?;
                            }
                        }
                        None => cursor.position = label_position(*fail_label)?,
                    }
                }
                // ERTS `i_bs_get_utf16` and `i_bs_skip_utf16` call
                // `erts_bs_get_utf16` and jump to `Fail` when it gives no
                // value (jit/arm/instr_bs.cpp:905-934). ERTS loads
                // `bs_get_utf32` and `bs_skip_utf32` as a 32-bit
                // `bs_get_integer2` and `i_bs_validate_unicode_retract`
                // (jit/arm/ops.tab:1086-1095). That integer read writes `Dst`
                // before the check fails, but the compiler defines `Dst` in
                // the instruction, so no code reads it at `Fail`.
                Instruction::BinaryGetUtf16 {
                    fail_label,
                    context,
                    little,
                    destination,
                }
                | Instruction::BinaryGetUtf32 {
                    fail_label,
                    context,
                    little,
                    destination,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let state = match_context(process, context, atoms)?;
                    let code_point = if matches!(instruction, Instruction::BinaryGetUtf16 { .. }) {
                        state.get_utf16(*little)
                    } else {
                        state.get_utf32(*little)
                    };
                    match (code_point, destination) {
                        (Some(code_point), Some(destination)) => {
                            write(process, destination, Term::Integer(code_point.into()))?;
                        }
                        (Some(_), None) => {}
                        (None, _) => cursor.position = label_position(*fail_label)?,
                    }
                }
                Instruction::BinaryMatch {
                    fail_label,
                    context,
                    commands,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    if !binary_match(process, context, commands, atoms)? {
                        cursor.position = label_position(*fail_label)?;
                    }
                }
                // ERTS `i_bs_match_string` compares the bits of the segment
                // with the string and jumps to `Fail` when fewer bits
                // remain or when the bits differ, in both cases with no
                // move of the position (emu bs_instrs.tab:1004-1027; JIT
                // `emit_i_bs_match_string`, jit/arm/instr_bs.cpp:191-234).
                Instruction::BinaryMatchString {
                    fail_label,
                    context,
                    bits,
                    text,
                } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let state = match_context(process, context, atoms)?;
                    if !state.match_string(*bits, text) {
                        cursor.position = label_position(*fail_label)?;
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
                // ERTS `bs_init_writable` reads a size hint in `x0` and
                // writes a new writable binary with no byte to `x0`
                // (`erts_bs_init_writable`, erl_bits.c:2020-2049; JIT
                // jit/arm/instr_bs.cpp:985-1000). A new binary has one owner,
                // so a `private_append` can write in its array.
                Instruction::BinaryInitWritable => {
                    write(process, &Operand::X(0), Term::binary(Vec::new()))?;
                }
                Instruction::BinaryCreate {
                    fail_label,
                    destination,
                    segments,
                } => {
                    let result = {
                        let (process, atoms) = scheduler.current_process_and_atoms();
                        create_binary(atoms, process, segments)?
                    };
                    match result {
                        Ok(term) => {
                            let process = scheduler.current_process_mut();
                            write(process, destination, term)?;
                        }
                        Err(_) if *fail_label != 0 => {
                            cursor.position = label_position(*fail_label)?;
                        }
                        Err(failure) => {
                            let process = scheduler.current_process_mut();
                            cursor.commit(process);
                            if let Some(outcome) = raise_binary_failure(scheduler, module, failure)?
                            {
                                return Ok(outcome);
                            }
                            cursor.refresh(scheduler)?;
                        }
                    }
                }
                // ERTS `fconv` converts a small integer, a bignum, or a float
                // to a float register, and raises `badarith` for each other
                // term (emu float_instrs.tab:38-52; JIT `emit_fconv` and
                // `emit_fconv_shared` in jit/arm/instr_float.cpp:96-135). An
                // integer converts to the nearest float, as the C cast does.
                Instruction::FloatConvert { source, register } => {
                    // A bignum that `big_to_double` cannot convert raises
                    // `badarith` too (`emit_fconv_shared`).
                    let value = read_ref(process, source)?.number_float();
                    if let Some(value) = value {
                        write_float_register(process, *register, value);
                    } else {
                        let badarith = Term::static_atom("badarith");
                        if let Some(outcome) = raise(process, ExceptionClass::Error, badarith) {
                            return Ok(outcome);
                        }
                        cursor.refresh(scheduler)?;
                    }
                }
                // `fload` reads a float term (JIT `emit_fload`,
                // jit/arm/instr_float.cpp:71-79). The compiler moves only a
                // float, which beam_validator checks.
                Instruction::FloatLoad { source, register } => {
                    let value = read_ref(process, source)?
                        .float()
                        .ok_or_else(|| error("fmove of a term that is not a float"))?;
                    write_float_register(process, *register, value);
                }
                // `fstore` makes a float term (JIT `emit_fstore`,
                // jit/arm/instr_float.cpp:81-94).
                Instruction::FloatStore {
                    register,
                    destination,
                } => {
                    let value = read_float_register(process, *register)?;
                    write(process, destination, Term::Float(value))?;
                }
                // A result that is not finite raises `badarith`
                // (`emit_check_float_error`, jit/arm/instr_float.cpp:33-53).
                Instruction::FloatArith {
                    operation,
                    left,
                    right,
                    destination,
                } => {
                    let left = read_float_register(process, *left)?;
                    let right = read_float_register(process, *right)?;
                    let value = match operation {
                        FloatOperation::Add => left + right,
                        FloatOperation::Subtract => left - right,
                        FloatOperation::Multiply => left * right,
                        FloatOperation::Divide => left / right,
                    };
                    if value.is_finite() {
                        write_float_register(process, *destination, value);
                    } else {
                        let badarith = Term::static_atom("badarith");
                        if let Some(outcome) = raise(process, ExceptionClass::Error, badarith) {
                            return Ok(outcome);
                        }
                        cursor.refresh(scheduler)?;
                    }
                }
                // The negation of a finite float is finite, so `fnegate`
                // has no error (`emit_i_fnegate`,
                // jit/arm/instr_float.cpp:222-234).
                Instruction::FloatNegate {
                    source,
                    destination,
                } => {
                    let value = -read_float_register(process, *source)?;
                    write_float_register(process, *destination, value);
                }
                Instruction::SelectValue {
                    value,
                    fail_label,
                    choices,
                } => {
                    // The loader prepared the choices: a small list keeps the
                    // encoded order, and a large list is sorted by the exact
                    // term order. The linear arm is the walk that PBeam always
                    // ran. The ordered arm binary-searches the sorted list,
                    // as ERTS `select_val_bins.select` does over its sorted
                    // values (`emu/select_instrs.tab:57-88`, OTP 29.1.1).
                    let target = match choices {
                        SelectValueChoices::Linear(choices) => {
                            let value = read_ref(process, value)?;
                            choices
                                .iter()
                                .find(|(choice, _)| choice == value)
                                .map_or(*fail_label, |(_, label)| *label)
                        }
                        SelectValueChoices::Ordered(choices) => {
                            let (process, atoms) = scheduler.current_process_and_atoms();
                            let value = read_ref(process, value)?;
                            select_value_ordered(value, choices, atoms)?.unwrap_or(*fail_label)
                        }
                    };
                    cursor.position = label_position(target)?;
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
                    write(process, destination, Term::cons(head, tail))?;
                }
                Instruction::PutTuple {
                    elements,
                    destination,
                } => {
                    let elements = elements
                        .iter()
                        .map(|element| read(process, element))
                        .collect::<Result<_, _>>()?;
                    write(process, destination, Term::tuple(elements))?;
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
                    // `read` gives the source a copy that shares the element array, so the source
                    // holds the array too, and `Arc::make_mut` copies the array before each write.
                    // Thus the source keeps its tuple, as for the ERTS `copy` hint:
                    // `i_update_record_copy` copies the whole record (emu `instrs.tab:785-802`;
                    // JIT `jit/arm/instr_common.cpp:893-998`). The `inplace` hint overwrites the
                    // record only when the compiler proves that it is a fresh young tuple
                    // (emu `instrs.tab:804-834`), so both hints give the same result.
                    let fields = std::sync::Arc::make_mut(fields.arc_mut());
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
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let (first, rest) = nonempty_list(read_ref(process, source)?, atoms)?;
                    let (first, rest) = (first.clone(), rest.clone());
                    write(process, head, first)?;
                    write(process, tail, rest)?;
                }
                Instruction::GetHead {
                    source,
                    destination,
                } => {
                    // `get_hd` reads only the head, so only the head is copied.
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let list = read_ref(process, source)?;
                    let Some(head) = list.list_head() else {
                        return Err(error(format!(
                            "BEAM list source {} is not a nonempty list",
                            debug_term(list, atoms)
                        )));
                    };
                    let head = head.clone();
                    write(process, destination, head)?;
                }
                Instruction::GetTail {
                    source,
                    destination,
                } => {
                    // `get_tl` reads only the tail, so only the tail is copied.
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let (_, rest) = nonempty_list(read_ref(process, source)?, atoms)?;
                    let rest = rest.clone();
                    write(process, destination, rest)?;
                }
                Instruction::Send => {
                    // ERTS `send` calls `erl_send` as a light BIF and writes
                    // its result to x0, or raises (`emu/bif_instrs.tab:425-454`;
                    // JIT `emit_send` in `jit/arm/instr_bif.cpp:612-625`).
                    cursor.commit(process);
                    if light_bif_switch(process) {
                        return Ok(Outcome::Yield);
                    }
                    let to = read(process, &Operand::X(0))?;
                    let message = read(process, &Operand::X(1))?;
                    let result = send(scheduler, &to, message)?;
                    if let Some(outcome) = self.finish_bif(scheduler, result, false)? {
                        return Ok(outcome);
                    }
                    cursor.refresh(scheduler)?;
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
                        cursor.position -= 1;
                        cursor.commit(process);
                        return Ok(Outcome::Yield);
                    }
                    if process.mailbox.current().is_none() {
                        cursor.commit(process);
                        match scheduler.receive_helper()? {
                            Received::Message => {}
                            Received::Empty => {
                                cursor.position = label_position(*fail_label)?;
                                continue;
                            }
                            Received::Yield => {
                                cursor.position -= 1;
                                cursor.commit(scheduler.current_process_mut());
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
                    cursor.position = label_position(*label)?;
                }
                Instruction::RemoveMessage => {
                    if !process.mailbox.remove_current() {
                        return Err(error("remove_message without a current message"));
                    }
                    scheduler.cancel_receive_timer();
                }
                Instruction::Wait(label) => {
                    cursor.position = label_position(*label)?;
                    cursor.commit(process);
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
                            let resume = cursor.position;
                            let now = scheduler.timers.now();
                            if !scheduler.set_receive_timer(now, &timeout, resume) {
                                let process = scheduler.current_process_mut();
                                process.mailbox.set_save_first();
                                let reason = Term::static_atom("timeout_value");
                                if let Some(outcome) = raise(process, ExceptionClass::Error, reason)
                                {
                                    return Ok(outcome);
                                }
                                cursor.refresh(scheduler)?;
                                continue;
                            }
                        }
                    }
                    cursor.position = label_position(*label)?;
                    cursor.commit(scheduler.current_process_mut());
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
                    // without a binding runs its BEAM stub body. An
                    // exception of the NIF has the frame of the function
                    // with its arguments first, as ERTS gives it.
                    let arguments = call_arguments(&process.registers, *arity)?;
                    cursor.commit(process);
                    // A load of another scheduler that returned patches
                    // the entry for each later call
                    // (`beam/erl_nif.c:5043-5098`).
                    scheduler.sync_code();
                    if let Some((job, result)) = native::call(scheduler, module, name, &arguments) {
                        let outcome = match result? {
                            BifResult::Raise { class, reason } => raise_at_entry(
                                scheduler,
                                (&module.name, name, arguments, Term::nil()),
                                class,
                                reason,
                            ),
                            // The caller switches out, and its next turn
                            // after the completion returns the result.
                            BifResult::Dirty(work) => Ok(Some(Outcome::Dirty {
                                job,
                                work,
                                entry: (module.name.clone(), name.clone(), arguments),
                            })),
                            result => self.finish_bif(scheduler, result, true),
                        };
                        if let Some(outcome) = outcome? {
                            return Ok(outcome);
                        }
                    }
                    cursor.refresh(scheduler)?;
                }
                Instruction::Hibernate => {
                    // `emit_i_hibernate` writes `ok` to `x0` and switches
                    // the context; the process keeps its frame
                    // (`jit/arm/instr_trace.cpp:222-238`).
                    write(process, &Operand::X(0), Term::static_atom("ok"))?;
                    cursor.commit(process);
                    return Ok(Outcome::Hibernate);
                }
                Instruction::Yield => {
                    // `emit_i_yield` writes `true` to `x0` and switches the
                    // context with no reduction cost; the process keeps its
                    // frame (`jit/arm/instr_common.cpp:3121-3124`;
                    // `jit/arm/instr_call.cpp:30-36`).
                    write(process, &Operand::X(0), Term::static_atom("true"))?;
                    cursor.commit(process);
                    return Ok(Outcome::Yield);
                }
                // ERTS `build_stacktrace` changes the raw stack trace in
                // `x0` to a list (emu `instrs.tab:1247-1251`; JIT
                // `emit_build_stacktrace` in
                // `jit/arm/instr_common.cpp:3046-3056`). The compiler moves
                // it from `x2` first.
                Instruction::BuildStacktrace => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let (_, stack) = raw_stacktrace(read_ref(process, &Operand::X(0))?, atoms)?;
                    write(process, &Operand::X(0), stack)?;
                }
                // ERTS `i_raise` raises the reason again with the class and
                // the stack trace of the raw stack trace (emu
                // `ops.tab:284-290`, `instrs.tab:1236-1245`; JIT
                // `emit_raise` and `emit_raise_shared` in
                // `jit/x86/instr_common.cpp:3138-3164`;
                // `erts_sanitize_freason` in `beam_common.c:1089-1097`).
                Instruction::Raise { trace, value } => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let (class, stack) = raw_stacktrace(read_ref(process, trace)?, atoms)?;
                    let value = read(process, value)?;
                    if let Some(outcome) = raise_with_stacktrace(process, class, value, stack) {
                        return Ok(outcome);
                    }
                    cursor.refresh(scheduler)?;
                }
                // ERTS `badmatch`, `case_end`, `try_case_end`, and
                // `badrecord` raise class `error` with the reason
                // `{Tag, Value}` (emu `instrs.tab:1137-1162`, `1229-1234`;
                // `expand_error_value` in `beam_common.c:754-768`).
                Instruction::TaggedError { tag, value } => {
                    let reason = Term::tuple(vec![Term::static_atom(tag), read(process, value)?]);
                    cursor.commit(process);
                    if let Some(outcome) = raise_in_function(scheduler, reason)? {
                        return Ok(outcome);
                    }
                    cursor.refresh(scheduler)?;
                }
                // ERTS `if_end` raises class `error` with the reason
                // `if_clause` (emu `instrs.tab:1151-1155`;
                // `expand_error_value` in `beam_common.c:769-772`).
                Instruction::IfEnd => {
                    let reason = Term::static_atom("if_clause");
                    cursor.commit(process);
                    if let Some(outcome) = raise_in_function(scheduler, reason)? {
                        return Ok(outcome);
                    }
                    cursor.refresh(scheduler)?;
                }
                // ERTS `raw_raise` raises `x1` with the class in `x0` and the
                // stack trace of the raw stack trace in `x2`. For a class
                // that is not `error`, `exit`, or `throw`, it writes
                // `badarg` to `x0` and continues (emu `instrs.tab:1253-1263`;
                // `raw_raise` in `beam_common.c:1148-1179`).
                Instruction::RawRaise => {
                    let (process, atoms) = scheduler.current_process_and_atoms();
                    let (_, stack) = raw_stacktrace(read_ref(process, &Operand::X(2))?, atoms)?;
                    let class = ExceptionClass::from_term(read_ref(process, &Operand::X(0))?);
                    let value = read(process, &Operand::X(1))?;
                    match class {
                        Some(class) => {
                            if let Some(outcome) =
                                raise_with_stacktrace(process, class, value, stack)
                            {
                                return Ok(outcome);
                            }
                            cursor.refresh(scheduler)?;
                        }
                        None => write(process, &Operand::X(0), Term::static_atom("badarg"))?,
                    }
                }
                Instruction::Noop | Instruction::Label(_) => {}
                Instruction::TestYield => {
                    // `emit_i_test_yield` (`jit/arm/instr_common.cpp:3097-3119`).
                    process.fcalls -= 1;
                    if process.fcalls <= 0 {
                        cursor.commit(process);
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
                    cursor.refresh(scheduler)?;
                }
                Instruction::End => {
                    let frame = current_frame(process);
                    return Err(error(format!(
                        "function {}:{} did not return",
                        cursor.module.name,
                        cursor.module.tables.name(frame.function)
                    )));
                }
                Instruction::Unsupported(opcode) => {
                    return Err(error(format!("unsupported BEAM opcode {opcode}")));
                }
                // ERTS `i_func_info` raises class `error` with the reason
                // `function_clause` (`emit_i_func_info_shared` in
                // `jit/arm/beam_asm_module.cpp:484-497`; emu
                // `beam_emu.c:575-580`). See [`function_clause`].
                Instruction::FunctionInfo {
                    module,
                    name,
                    arity,
                } => {
                    // ERTS finds the location of the function entry, which
                    // the `line` before `func_info` covers
                    // (`beam_common.c:1231-1233`).
                    let location = cursor
                        .module
                        .lines
                        .location(cursor.position - 1, Term::nil());
                    if let Some(outcome) =
                        function_clause(scheduler, module, name, *arity, location)?
                    {
                        return Ok(outcome);
                    }
                    cursor.refresh(scheduler)?;
                }
            }
        }
    }

    /// Call the import `import` of `code` with the arguments in the X
    /// registers, as `call_ext` does. The import holds the MFA and the call
    /// that the loader decided for it, so the call compares no name. A call
    /// of module code enters the export target in the slot of the import in
    /// `targets`, and makes no lookup by name, as `call_ext` branches
    /// through the export entry that the loader patched into the code
    /// (`emit_i_call_ext` and `emit_i_call_ext_only` in
    /// `jit/arm/instr_call.cpp:150-164`; `jit/asm_load.c:1198-1206`). An
    /// empty slot looks up the export by name and keeps a found target in
    /// the slot (see [`ImportTargets`]). A tail call first checks that the
    /// frame has `deallocate` Y registers, then replaces the frame. A tail
    /// call of `erlang:apply/2` or `erlang:apply/3` releases the Y registers
    /// before the apply, as the ERTS loader makes `i_apply_fun_last` and
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
    /// (`jit/arm/ops.tab:615-623`, `emu/ops.tab:854-862`). A call of
    /// `erlang:call_on_load_function/1` runs as the
    /// `i_call_on_load_function` instruction (`jit/arm/ops.tab:630-641`,
    /// `jit/arm/instr_bif.cpp:994-1031`): it calls the `-on_load` function
    /// of the module in `x0` as a body call, also in a tail call, which
    /// then continues at the `Deallocate` and the `Return` after it. A
    /// module with no code that waits for its function raises `badarg`.
    /// Return the outcome when the process stops.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Import 0 of `code` is `erlang:exit/1`. A tail call with no catch
    /// // tag exits the process.
    /// let outcome = program.call_external(&mut scheduler, &code, &targets, 0, Some(0))?;
    /// assert!(matches!(outcome, Some(Outcome::Exit { result: None, .. })));
    /// // Import 1 of `code` is `erlang:self/0`. A tail call that
    /// // deallocates no Y register from a frame with one Y register is an
    /// // error.
    /// assert!(program.call_external(&mut scheduler, &code, &targets, 1, Some(0)).is_err());
    /// ```
    fn call_external(
        &self,
        scheduler: &mut Scheduler,
        code: &BeamModule,
        targets: &[Cell<Option<ExportTarget>>],
        import: u32,
        deallocate: Option<u32>,
    ) -> Result<Option<Outcome>, BeamError> {
        // An external call sees each code operation that returned on
        // another scheduler, as the active code index of ERTS gives it.
        scheduler.sync_code();
        // The loader checks each import index against the import table, and
        // the code table makes one slot for each import.
        let index = import as usize;
        let Import {
            module,
            name,
            arity,
            kind,
        } = &code.imports[index];
        let arity = *arity;
        let tail = deallocate.is_some();
        let process = scheduler.current_process_mut();
        match deallocate {
            Some(deallocate) if matches!(kind, ImportKind::ApplyFun | ImportKind::Apply) => {
                release_frame(process, deallocate)?;
            }
            Some(deallocate) if deallocate as usize != current_frame(process).stack.len() => {
                return Err(error("invalid BEAM tail-call stack size"));
            }
            _ => {}
        }
        match *kind {
            ImportKind::LoadNif => {
                let module_id = current_frame(process).module_id;
                // `erlang:load_nif/2` runs on the newest code state of the
                // node and publishes its NIF library record, as the other
                // code operations do (see `scheduler::code`).
                let node = scheduler.is_multi().then(|| Arc::clone(scheduler.node()));
                let mut state = node.as_deref().map(|node| {
                    let state = node.code.lock();
                    scheduler.install_code(&state);
                    state
                });
                let result = native::load_nif(scheduler, module_id);
                if let Some(state) = state.as_mut() {
                    scheduler.publish_code(state);
                }
                drop(state);
                let result = result?;
                // The `load_nif/2` and `call_on_load_function/1`
                // instructions raise with the BIF frame, as each BIF does
                // (`jit/arm/instr_bif.cpp:994-1067`).
                let process = scheduler.current_process_mut();
                let arguments = call_arguments(&process.registers, arity)?;
                let result = bif_error(scheduler, module, name, arguments, result, 0)?;
                self.finish_bif(scheduler, result, false)
            }
            ImportKind::CallOnLoadFunction => {
                let module = read_x_register(&process.registers, 0)?.clone();
                let frame = match &module {
                    Term::Atom(module) => {
                        let name = scheduler.atoms.name_of(module).to_owned();
                        scheduler.code.on_load_frame(&name)?
                    }
                    _ => None,
                };
                match frame {
                    Some(frame) => {
                        enter_frame(scheduler.current_process_mut(), frame, false);
                        Ok(None)
                    }
                    None => {
                        let arguments = vec![module];
                        let badarg = BifResult::badarg();
                        let result = bif_error(scheduler, "erlang", name, arguments, badarg, 0)?;
                        self.finish_bif(scheduler, result, false)
                    }
                }
            }
            ImportKind::Bif { bif, heavy } => {
                if !heavy && light_bif_switch(process) {
                    return Ok(Some(Outcome::Yield));
                }
                let arguments = call_arguments(&process.registers, arity)?;
                let result = self.call_bif(scheduler, module, name, bif, &arguments)?;
                // The loader turns `call_ext_last` and `call_ext_only` of a
                // BIF into a body call and a return
                // (`jit/arm/ops.tab:709-714`), so the frame of the caller
                // is in the stack trace.
                let result = bif_error(scheduler, module, name, arguments, result, 0)?;
                self.finish_bif(scheduler, result, false)
            }
            ImportKind::ApplyFun => self.apply_fun(scheduler, tail),
            ImportKind::Apply => self.apply_mfa(scheduler, tail),
            ImportKind::Export => {
                // A registered native function runs at the `NifStart`
                // instruction of its stub, so this path enters the frame and
                // keeps the entry reduction charge and the yield check. The
                // registry check stays in `native::call`.
                let slot = &targets[index];
                if let Some(target) = slot.get() {
                    return enter_export(process, target.frame(), arity, tail);
                }
                match scheduler.synced_code().export_frame(module, name, arity)? {
                    Some(frame) => {
                        slot.set(Some(ExportTarget::of(&frame)));
                        enter_export(scheduler.current_process_mut(), frame, arity, tail)
                    }
                    None => self.undefined_function(scheduler, module, name, arity, tail),
                }
            }
        }
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
    /// let apply = scheduler.atoms.get("apply").unwrap();
    /// let erlang = scheduler.atoms.get("erlang").unwrap();
    /// let outcome = program.call_mfa(&mut scheduler, &erlang, &apply, 3, false)?;
    /// assert!(outcome.is_none());
    /// ```
    fn call_mfa(
        &self,
        scheduler: &mut Scheduler,
        module: &Atom,
        name: &Atom,
        arity: u32,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        // A BIF that reads the code table, such as `erlang:module_loaded/1`,
        // and the export of the MFA see each code operation that returned
        // on another scheduler.
        scheduler.sync_code();
        let resolved = {
            let atoms = &scheduler.atoms;
            resolve(atoms.name_of(module), atoms.name_of(name), arity)
        };
        let pbeam = {
            let atoms = &scheduler.atoms;
            is_pbeam_bif(atoms.name_of(module), atoms.name_of(name), arity)
        };
        if resolved.is_some() || pbeam {
            let arguments = {
                let process = scheduler.current_process_mut();
                call_arguments(&process.registers, arity)?
            };
            let result = match resolved {
                Some(id) => self.bif_slice(scheduler, id, &arguments)?,
                None => {
                    let (module_name, name) = {
                        let atoms = &scheduler.atoms;
                        (
                            atoms.name_of(module).to_owned(),
                            atoms.name_of(name).to_owned(),
                        )
                    };
                    self.named_bif(scheduler, &module_name, &name, &arguments)?
                }
            };
            // A tail apply or a tail fun call removed the frame of the
            // caller in ERTS, so its return address is in the function
            // before it. A trap keeps the call for an error after the trap.
            let result = if matches!(
                result,
                BifResult::Raise { .. }
                    | BifResult::ErrorWithCause { .. }
                    | BifResult::TrapBif { .. }
            ) {
                let (module_name, name) = {
                    let atoms = &scheduler.atoms;
                    (
                        atoms.name_of(module).to_owned(),
                        atoms.name_of(name).to_owned(),
                    )
                };
                let skip = usize::from(tail);
                bif_error(scheduler, &module_name, &name, arguments, result, skip)?
            } else {
                result
            };
            return self.finish_bif(scheduler, result, tail);
        }
        // A name that PBeam does not resolve as a BIF reaches the export
        // lookup. The name of the atom term comes from the run table.
        let (module_name, name) = {
            let atoms = &scheduler.atoms;
            (
                atoms.name_of(module).to_owned(),
                atoms.name_of(name).to_owned(),
            )
        };
        self.call_export(scheduler, &module_name, &name, arity, tail)
    }

    /// Call the export of `module:name/arity` with the arguments in the X
    /// registers, as `call_ext`, `apply`, and a fun call do. The calls of
    /// `erlang:apply/2` and `erlang:apply/3` run as the ERTS `i_apply_fun`
    /// and `i_apply` instructions. A function that a loaded module does not
    /// export goes to [`BeamProgram::undefined_function`]. The caller
    /// resolves the BIF of the MFA first when a load-time or a run-time fact
    /// exists, so this function searches no table. Return the outcome when
    /// the process stops.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `fixture:main/0` enters the frame of the export.
    /// let outcome = program.call_export(&mut scheduler, "fixture", "main", 0, false)?;
    /// assert!(outcome.is_none());
    /// ```
    fn call_export(
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
        match scheduler.synced_code().export_frame(module, name, arity)? {
            Some(frame) => enter_export(scheduler.current_process_mut(), frame, arity, tail),
            None => self.undefined_function(scheduler, module, name, arity, tail),
        }
    }

    /// Call a function that a loaded module does not export, with the
    /// arguments in `x0` to `x(arity - 1)`. ERTS goes to the export entry of
    /// the function, which calls `error_handler:undefined_function/3` with
    /// the module, the function, and the argument list `Args`
    /// (`call_error_handler` in `beam_common.c:1294-1342`; JIT
    /// `emit_export_trampoline` in `jit/arm/beam_asm_global.cpp:164-232`).
    /// The module name of the call is the module of the function, and not
    /// the name of a loaded module: ERTS looks up the export entry of the
    /// name and makes a stub entry when the module is not loaded
    /// (`erts_export_get_or_make_stub`, `export.c:228-242`; the call of
    /// `fixed_apply`, `beam_common.c:1559-1563`).
    ///
    /// `call_error_handler` finds the error handler of the process
    /// (`erts_proc_get_error_handler`, `erl_process.h:2420-2430`), which
    /// `erlang:process_flag(error_handler, Module)` sets, and finds
    /// `undefined_function/3` in it. It then puts the module, the function,
    /// and the argument list in `x0`, `x1`, and `x2` and runs that export.
    /// Thus `error_handler.beam` runs, and the error of the module load and
    /// the `undef` frame come from that code
    /// (`error_handler.erl:85-98,165-178,206-213`). An error handler without
    /// that export raises `error:undef` with the current frame
    /// (`call_error_handler`, `beam_common.c:1310-1316`).
    ///
    /// `tail` is the form of the failed call: ERTS enters the export of the
    /// handler with the stack of the caller, so its `return` continues at
    /// the continuation of that call, as `call_ext` or `call_ext_last` does
    /// (`emit_export_trampoline` in `jit/arm/beam_asm_global.cpp:164-232`).
    /// A handler that returns thus gives its value to the caller of a body
    /// call, and to the caller of the caller of a tail call.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `undef_handler:missing(m)` calls the handler with x0 = missing, x1 = [m].
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
        let process = scheduler.current_process_mut();
        let default_handler = process.error_handler.is_none();
        let handler = process
            .error_handler
            .clone()
            .unwrap_or_else(|| DEFAULT_ERROR_HANDLER.to_owned());
        // A call of a module that the run does not load makes the default
        // `error_handler` ask for a load of that module:
        // `error_handler:ensure_loaded/1` calls `code:ensure_loaded/1` or
        // `init:ensure_loaded/1` (`error_handler.erl:182-198`). A run with
        // no code server and no init process cannot answer that call:
        // `init:ensure_loaded/1` sends to the registered name `init` and
        // waits for the reply (`init.erl:463-464`, `485-490`), and the send
        // raises `badarg` when no process has that name. A custom handler
        // answers the call itself and needs no load, so the runtime error
        // applies only to the default handler.
        if scheduler.code.get(module).is_none() && default_handler && !loads_modules(scheduler) {
            return Err(error(format!(
                "unresolved external MFA {module}:{function}/{arity}"
            )));
        }
        let atom = |atoms: &AtomTable, name: &str| {
            atoms
                .get(name)
                .map(Term::Atom)
                .ok_or_else(|| error(format!("atom table does not hold {name}")))
        };
        let (module_atom, function_atom) = (
            atom(&scheduler.atoms, module)?,
            atom(&scheduler.atoms, function)?,
        );
        let process = scheduler.current_process_mut();
        let arguments = Term::list_from(call_arguments(&process.registers, arity)?);
        let lookup = scheduler
            .code
            .export_frame(&handler, "undefined_function", 3)?;
        let process = scheduler.current_process_mut();
        if let Some(frame) = lookup {
            enter_frame(process, frame, tail);
            write(process, &Operand::X(0), module_atom)?;
            write(process, &Operand::X(1), function_atom)?;
            write(process, &Operand::X(2), arguments)?;
            return Ok(None);
        }
        // ERTS sets the reason `undef` and the current MFA to the export
        // entry of the undefined function, so the stack trace frame is the
        // tuple of `build_stacktrace` (`beam_common.c:1310-1316`;
        // `utils.c:3448-3455`). The arguments of the failed call are the
        // list of the call.
        let frame = Term::tuple(vec![module_atom, function_atom, arguments, Term::nil()]);
        Ok(raise_with_stacktrace(
            process,
            ExceptionClass::Error,
            Term::static_atom("undef"),
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
        let mut call = {
            let process = scheduler.current_process_mut();
            call_arguments(&process.registers, 3)?
        };
        let start = self.is_start_apply(scheduler)?;
        let (module, name, arguments, proper) = loop {
            // Each wrong form raises the same `badarg`, so one pattern checks
            // the function, the module, and the list.
            let [Term::Atom(module), Term::Atom(name), list] = &mut call[..] else {
                return self.apply_error(scheduler, call, "badarg", start);
            };
            let Some((arguments, proper)) = list.list_elements() else {
                return self.apply_error(scheduler, call, "badarg", start);
            };
            let is_apply = {
                let atoms = &scheduler.atoms;
                (atoms.name_of(module), atoms.name_of(name)) == ("erlang", "apply")
            };
            if !proper || !is_apply || arguments.len() != 3 {
                break (module.clone(), name.clone(), arguments, proper);
            }
            call = arguments;
        };
        if arguments.len() > APPLY_MAX_ARGUMENTS {
            return self.apply_error(scheduler, call, "system_limit", start);
        }
        if !proper {
            return self.apply_error(scheduler, call, "badarg", start);
        }
        let arity = arguments.len() as u32;
        let process = scheduler.current_process_mut();
        if start {
            // ERTS applies this MFA from `beam_run_process`
            // (`erl_process.c:12616-12624`, `bif.c:1426-1427`) and does not
            // enter the `erlang:apply/3` body of `erlang.beam`. Thus the
            // `TestYield` of that body costs no reduction, and the apply
            // gives back the reduction that it used.
            process.fcalls += 1;
        }
        for (index, argument) in (0..).zip(arguments) {
            write(process, &Operand::X(index), argument)?;
        }
        process.start_apply = start;
        let outcome = self.call_mfa(scheduler, &module, &name, arity, tail);
        scheduler.current_process_mut().start_apply = false;
        outcome
    }

    /// Raise the error `reason` of `erlang:apply/3` with the arguments
    /// `call`, as ERTS `apply` raises it with the BIF frame of
    /// `erlang:apply/3` and the arguments in the registers
    /// (`beam_common.c:1434-1527`; `save_stacktrace`,
    /// `beam_common.c:903-1005`). In a start apply, the first frame of the
    /// process is not in the stack trace, as the apply has no frame in
    /// ERTS. Return the outcome when the process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `erlang:apply(1, f, [])` raises badarg with the first frame
    /// // {erlang, apply, [1, f, []], [{error_info, #{module => erl_erts_errors}}]}.
    /// let outcome = program.apply_error(&mut scheduler, call, "badarg", false)?;
    /// ```
    fn apply_error(
        &self,
        scheduler: &mut Scheduler,
        call: Vec<Term>,
        reason: &'static str,
        start: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        let raised = BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::static_atom(reason),
        };
        scheduler.current_process_mut().start_apply = start;
        let raised = bif_error(scheduler, "erlang", "apply", call, raised, 0);
        scheduler.current_process_mut().start_apply = false;
        self.finish_bif(scheduler, raised?, false)
    }

    fn is_start_apply(&self, scheduler: &mut Scheduler) -> Result<bool, BeamError> {
        let process = scheduler.current_process_mut();
        if let Some(frameless) = process.frameless_apply.replace(false) {
            return Ok(frameless);
        }
        let initial_call = process.initial_call.clone();
        let (module, function, arity) = scheduler
            .code
            .initial_call_names(&scheduler.atoms, &initial_call)?;
        Ok((module.as_str(), function.as_str(), arity) == ("erlang", "apply", 3))
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
        let list = read(process, &Operand::X(1))?;
        // ERTS `apply_fun` raises with the BIF frame of `erlang:apply/2`
        // and its two arguments (`beam_common.c:1734-1762`).
        let (arguments, failure) = match list.list_elements() {
            None => (Vec::new(), Some("badarg")),
            Some((arguments, _)) if arguments.len() > APPLY_FUN_MAX_ARGUMENTS => {
                (Vec::new(), Some("system_limit"))
            }
            Some((_, false)) => (Vec::new(), Some("badarg")),
            Some((arguments, true)) => (arguments, None),
        };
        if let Some(reason) = failure {
            let call = vec![read(process, &Operand::X(0))?, list];
            let raised = BifResult::Raise {
                class: ExceptionClass::Error,
                reason: Term::static_atom(reason),
            };
            let raised = bif_error(scheduler, "erlang", "apply", call, raised, 0)?;
            return self.finish_bif(scheduler, raised, false);
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
    /// entry that the loaded code of its module name holds, and a local fun
    /// whose module name holds no entry of the fun raises
    /// `error:{badfun, Fun}` (`beam_common.c:1697-1706`). An external fun
    /// calls its function as `call_ext` does, so a BIF runs as a BIF. A tail
    /// call replaces the active frame. Return the outcome when the process
    /// exits.
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
        let (target, free, fun) = match call {
            FunCall::Enter { target, free, fun } => (target, free, fun),
            FunCall::BadArity(fun) => {
                let process = scheduler.current_process_mut();
                let arguments = call_arguments(&process.registers, arity)?;
                let call = Term::tuple(vec![fun, Term::list_from(arguments)]);
                let reason = Term::tuple(vec![Term::static_atom("badarity"), call]);
                return Ok(raise(process, ExceptionClass::Error, reason));
            }
            FunCall::BadFun(fun) => {
                let process = scheduler.current_process_mut();
                let reason = Term::tuple(vec![Term::static_atom("badfun"), fun]);
                return Ok(raise(process, ExceptionClass::Error, reason));
            }
        };
        let lambda = match &target {
            Target::Local(lambda) => lambda,
            Target::External {
                module, function, ..
            } if tail
                && matches!(
                    (
                        scheduler.atoms.name_of(module),
                        scheduler.atoms.name_of(function),
                        arity,
                    ),
                    ("erlang", "error", 1..=3) | ("erlang", "exit" | "throw", 1)
                ) =>
            {
                let function = function.clone();
                return self.tail_call_error(scheduler, function, arity);
            }
            Target::External {
                module, function, ..
            } => return self.call_mfa(scheduler, module, function, arity, tail),
        };
        let total_arity = arity + free.len() as u32;
        {
            let process = scheduler.current_process_mut();
            for (register, value) in (arity..).zip(free) {
                write(process, &Operand::X(register), value)?;
            }
        }
        scheduler.sync_code();
        let frame = {
            let module = scheduler.atoms.name_of(lambda.module());
            match scheduler.code.fun_frame(module, lambda, total_arity)? {
                Some(frame) => frame,
                // The loaded code of the module name of the fun holds no
                // entry of the fun, so the code of the fun is not loaded and
                // the call raises `error:{badfun, Fun}`
                // (`beam_common.c:1697-1706`).
                None => {
                    let process = scheduler.current_process_mut();
                    let reason = Term::tuple(vec![Term::static_atom("badfun"), fun]);
                    return Ok(raise(process, ExceptionClass::Error, reason));
                }
            }
        };
        enter_frame(scheduler.current_process_mut(), frame, tail);
        Ok(None)
    }

    /// Run a tail call of the fun `fun erlang:error/1`,
    /// `fun erlang:error/2`, `fun erlang:error/3`, `fun erlang:exit/1`, or
    /// `fun erlang:throw/1`, with the name `function` and the arity
    /// `arity`. ERTS
    /// `i_call_fun_last` removes the frame before it calls the fun
    /// (`emit_i_call_fun2_last` in `jit/arm/instr_fun.cpp:425-455`). Only an
    /// apply puts the calling function back for an error BIF
    /// (`apply_bif_error_adjustment`, `beam_common.c:1345-1431`), so the
    /// first frame of the stack trace is the function of the caller frame.
    /// With no caller frame, the first frame is the initial call of the
    /// process with its arity, and the arguments are not in it
    /// (`build_stacktrace`, `beam_common.c:1245-1248`). For
    /// `fun erlang:error/3`, the location of that frame is the
    /// `error_info` location of the options, as `erlang:error/3` gives it
    /// (`get_error_info_from_exc`, `beam_common.c:1253`). The tail call has
    /// already released the Y registers of the frame, so the frame holds no
    /// catch tag. Return the outcome when the process exits.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // `tail(F) -> F(r, [a]).` as the start of a process.
    /// let outcome = program.tail_call_error(&mut scheduler, error, 2)?;
    /// // The process exits with {r, [{Module, tail, 1, []}]}.
    /// assert!(matches!(outcome, Some(Outcome::Exit { .. })));
    /// ```
    fn tail_call_error(
        &self,
        scheduler: &mut Scheduler,
        function: Atom,
        arity: u32,
    ) -> Result<Option<Outcome>, BeamError> {
        // The guard of the call admits only `error/1,2,3`, `exit/1`, and
        // `throw/1`, so the class follows from the name.
        let class = match scheduler.atoms.name_of(&function) {
            "exit" => ExceptionClass::Exit,
            "throw" => ExceptionClass::Throw,
            _ => ExceptionClass::Error,
        };
        let process = scheduler.current_process_mut();
        process.frames.pop();
        if !process.frames.is_empty() {
            let module = Atom::from_static("erlang").expect("erlang is a static atom");
            return self.call_mfa(scheduler, &module, &function, arity, false);
        }
        let reason = read(process, &Operand::X(0))?;
        let location = if arity == 3 {
            error_info_location(read_ref(process, &Operand::X(2))?)
        } else {
            Term::nil()
        };
        let initial_call = process.initial_call.clone();
        let (module, function, arity) = scheduler
            .code
            .initial_call_names(&scheduler.atoms, &initial_call)?;
        let frame = Term::tuple(vec![
            Term::Atom(scheduler.atoms.put(&module)?),
            Term::Atom(scheduler.atoms.put(&function)?),
            Term::Integer(arity as i64),
            location,
        ]);
        let process = scheduler.current_process_mut();
        let stacktrace = Term::list_from(vec![frame]);
        Ok(raise_with_stacktrace(process, class, reason, stacktrace))
    }

    /// Finish a BIF or NIF call with its result, as the ERTS `nif_bif.epilogue`
    /// does for BIFs and NIFs (`emu/bif_instrs.tab:604-636`; JIT
    /// `emit_bif_nif_epilogue` in `jit/arm/instr_bif.cpp:631-719`). A value
    /// goes to x0, and a tail call then returns from the frame with no
    /// reduction. A yield return writes its value to x0, does the return
    /// of a tail call, and switches out the process. An exception goes to
    /// the error handler. A trap writes its arguments to the X registers,
    /// enters the target function, and switches out the process. A target
    /// that is not loaded is kept for the next turn, which resolves it as the
    /// ERTS export stub does and enters `error_handler:undefined_function/3`.
    /// An exit ends the process, and no catch tag sees it. A trap to a BIF
    /// export keeps the export, its arguments, and `tail` in the process, and switches out the process:
    /// `run` then finishes the call through the export. Return the outcome
    /// when the process stops.
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
            BifResult::ErrorWithCause { reason, .. } => {
                Ok(raise(process, ExceptionClass::Error, reason))
            }
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
                let process = scheduler.current_process_mut();
                for (index, argument) in (0..).zip(arguments) {
                    write(process, &Operand::X(index), argument)?;
                }
                // ERTS resolves the target through its export entry. A
                // function that has no loaded export has a stub entry that
                // calls `error_handler:undefined_function/3`
                // (`erts_export_get_or_make_stub`, export.c:228-242;
                // `call_error_handler`, beam_common.c:1294-1342). Thus a
                // `re:run/2,3` call that runs before the pinned `re` module
                // is loaded traps to `re:grun/3` or `re:urun/3`, and the
                // error handler loads `re` and enters that function
                // (`error_handler.erl:85-98`).
                let frame = scheduler
                    .synced_code()
                    .export_frame(module, function, arity)?;
                match frame {
                    Some(frame) => {
                        let process = scheduler.current_process_mut();
                        enter_frame(process, frame, tail);
                        // A BIF trap always switches out the process
                        // (`context_switch_simplified` in
                        // `jit/arm/instr_bif.cpp:525-536`, `676-701`). The
                        // process enters the target when it runs again, so
                        // the entry costs one reduction of the next turn.
                        Ok(Some(Outcome::Yield))
                    }
                    None => {
                        // ERTS switches out at the trap, and the export
                        // stub of the target runs in the next turn
                        // (`context_switch_simplified` in
                        // `jit/arm/instr_bif.cpp:525-536`, `676-701`). Keep
                        // the call for `run`, so the switch out happens
                        // before the error handler.
                        scheduler.current_process_mut().pending_trap = Some(PendingTrap {
                            module,
                            function,
                            arity,
                            tail,
                        });
                        Ok(Some(Outcome::Yield))
                    }
                }
            }
            BifResult::TrapBif {
                module,
                function,
                arguments,
            } => {
                let origin = process.trap_origin.take();
                process.pending_bif = Some(PendingBif {
                    module,
                    function,
                    arguments,
                    tail,
                    origin,
                });
                Ok(Some(Outcome::Yield))
            }
            BifResult::Suspend {
                module,
                function,
                arguments,
            } => {
                process.suspended = true;
                process.pending_bif = Some(PendingBif {
                    module,
                    function,
                    arguments,
                    tail,
                    origin: None,
                });
                Ok(Some(Outcome::Wait))
            }
            BifResult::AwaitGrant => {
                process.pending_bif = Some(PendingBif {
                    module: "pbeam_cap",
                    function: "await_grant",
                    arguments: Vec::new(),
                    tail,
                    origin: None,
                });
                Ok(Some(Outcome::Wait))
            }
            BifResult::Exited(reason) => Ok(Some(Outcome::Exit {
                reason,
                result: None,
            })),
            // Only the entry of a NIF switches out to a dirty pool.
            BifResult::Dirty(_) => Err(error(
                "invariant error: a BIF result switches out to a dirty pool outside a NIF entry",
            )),
            BifResult::Hibernate {
                module,
                function,
                arguments,
            } => {
                // `hibernate_3` drops the stack and the catches, puts the
                // arguments of `apply/3` into the argument registers, and
                // traps to `beam_run_process` (`bif.c:1403-1427`), which
                // applies them after the wake-up. PBeam gives the process
                // the first frame of `erlang:apply/3` of `erlang.beam`,
                // whose `i_apply_only` is that apply, as a spawn of
                // `erlang:apply/3` does. That apply has no frame, as the
                // start apply of such a spawn, so the stack trace of a BIF
                // that it calls does not hold the `erlang.beam` frame.
                let frame = scheduler.code.mfa_frame("erlang", "apply", 3)?;
                let process = scheduler.current_process_mut();
                process.frames.clear();
                process.frames.push(frame);
                process.frameless_apply = Some(true);
                process.registers = vec![Some(module), Some(function), Some(arguments)];
                Ok(Some(Outcome::Hibernate))
            }
        }
    }

    /// Resolve the target of a BIF trap that waited for the next turn, as
    /// the ERTS export entry does when the process runs again
    /// (`call_error_handler`, beam_common.c:1294-1342). An export enters its
    /// frame, and the process continues in it. A target that the loaded
    /// modules do not export enters `error_handler:undefined_function/3`
    /// with the arguments in the X registers. Return the outcome when the
    /// process stops.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let outcome = program.resume_trap(&mut scheduler, "re", "grun", 3, false)?;
    /// assert!(outcome.is_none());
    /// ```
    fn resume_trap(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        function: &str,
        arity: u32,
        tail: bool,
    ) -> Result<Option<Outcome>, BeamError> {
        match scheduler
            .synced_code()
            .export_frame(module, function, arity)?
        {
            Some(frame) => {
                enter_frame(scheduler.current_process_mut(), frame, tail);
                Ok(None)
            }
            None => self.undefined_function(scheduler, module, function, arity, tail),
        }
    }
}

/// Return the outcome of a turn of the current process, `outcome`, after
/// the policy process ends its call of the policy code. When the current
/// process is the policy process and runs a call, each end of a turn after
/// more than `POLICY_REDUCTIONS` reductions of the call ends the call as an
/// overrun: a switch out for the budget, a wait in `receive`, a
/// hibernation, a dirty NIF call, a return, and an exception. Thus a call
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

/// Return true when the run can load a module at run time: it has the live
/// code server of `code:start_link/0` (`code.erl:1682-1712`), or the init
/// process that `pbeam_boot` registers and that answers `{ensure_loaded,
/// Module}` (`init.erl:730-750`). Else the load call of
/// `error_handler:ensure_loaded/1` reaches `init:ensure_loaded/1`, which
/// sends to the registered name `init` and waits for the reply
/// (`init.erl:463-464`, `485-490`); a send to a name that no process has
/// raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(!loads_modules(&scheduler));
/// ```
fn loads_modules(scheduler: &Scheduler) -> bool {
    ["code_server", "init"]
        .into_iter()
        .any(|name| scheduler.whereis(name).is_some())
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

/// Enter `frame`, the frame of an export of arity `arity`, with the
/// arguments in the X registers, as a remote call does. A tail call
/// replaces the active frame, and a body call pushes the frame.
///
/// # Examples
///
/// ```rust,ignore
/// let frame = scheduler.code.mfa_frame("fixture", "main", 0)?;
/// assert!(enter_export(&mut process, frame, 0, false)?.is_none());
/// ```
///
/// # Errors
///
/// An argument register that is not set, or that holds a match context, is
/// a runtime error, as [`check_remote_call_arguments`] gives it.
fn enter_export(
    process: &mut ProcessState,
    frame: Frame,
    arity: u32,
    tail: bool,
) -> Result<Option<Outcome>, BeamError> {
    check_remote_call_arguments(&process.registers, arity)?;
    enter_frame(process, frame, tail);
    Ok(None)
}

/// The spawn options that this runtime supports, as the flags of ERTS
/// `ErlSpawnOpts`, and the PBeam option `{capabilities, Map}`.
#[derive(Default)]
struct SpawnOptions {
    /// Link the new process to the caller (`SPO_LINK`).
    link: bool,
    /// The tag of the `DOWN` message of a monitor of the new process, or
    /// `None` with no monitor. With a monitor, the spawn returns
    /// `{Pid, Reference}` (`SPO_MONITOR`).
    monitor: Option<Term>,
    /// The start values of the `message_queue_data` flag and of the
    /// priority of the new process.
    flags: StartFlags,
    /// The restrictions of the capabilities of the new process, or `None` to
    /// give it a copy of each delegable capability of the caller.
    capabilities: Option<Vec<Restriction>>,
}

/// Start a process that runs `module:function` with the elements of
/// `arguments`, as ERTS `erl_create_process` does. Return the pid, or
/// `{Pid, Reference}` when `options.monitor` is set. A `module` or
/// `function` that is not an atom, and `arguments` that are not a proper
/// list, raise `badarg` (`erl_process.c:12490-12492`). A BIF, and a
/// function that a loaded module does not export, start the process with no
/// frame: the process calls the BIF or `undefined_function` when it first
/// runs. More than 1023 arguments also start the process, which then raises
/// `error:system_limit`, as the `apply` of the ERTS `i_apply_only` start
/// does (`beam_common.c:1501-1513`).
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
    let arguments: Vec<Term> = arguments.cloned().collect();
    let arity = u32::try_from(arguments.len()).map_err(|_| error("arity exceeds BEAM limit"))?;
    // A function that the loaded module does not export gives an undefined
    // start, which goes to `undefined_function` when the process first
    // runs, as the `i_apply_only` start of ERTS goes to its export entry.
    // More than `APPLY_MAX_ARGUMENTS` arguments also give an undefined start,
    // which raises `system_limit` when the process first runs. The `apply`
    // of ERTS checks this limit before it looks up the export, so the module
    // can be not loaded.
    //
    // A BIF also gives an undefined start, which runs the BIF when the
    // process first runs. The export of a BIF calls the BIF, and not the
    // stub body of the module, which is `erlang:nif_error(undefined)` in
    // `erlang.beam` (`emu/emu_load.c:704-713`, `jit/arm/ops.tab:917-927`).
    scheduler.sync_code();
    let export = {
        let atoms = &scheduler.atoms;
        let (module, function) = (atoms.name_of(module), atoms.name_of(function));
        if arguments.len() > APPLY_MAX_ARGUMENTS
            || resolve(module, function, arity).is_some()
            || is_pbeam_bif(module, function, arity)
        {
            None
        } else {
            scheduler.code.export_frame(module, function, arity)?
        }
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
        flags,
        capabilities,
    } = options;
    let spawned = scheduler.spawn(
        start,
        arguments,
        monitor,
        link,
        flags,
        capabilities.as_deref(),
    );
    let Ok((pid, reference)) = spawned? else {
        return BifResult::capability_denied(&scheduler.atoms);
    };
    Ok(BifResult::Value(match reference {
        Some(reference) => Term::tuple(vec![Term::Pid(pid), Term::Reference(reference)]),
        None => Term::Pid(pid),
    }))
}

/// The execution position of the run loop: the code of the active frame and
/// the index of the next instruction in it. The loop reads and writes this
/// local on the straight-line path, so an instruction that no frame change
/// follows does not touch the frame of the process. The loop refreshes the
/// cursor at each exit that changes the frame.
struct Cursor {
    /// The code of the active frame.
    module: Arc<BeamModule>,
    /// The export targets of the imports of `module` in this run.
    imports: ImportTargets,
    /// The index of the next instruction in `module.instructions`.
    position: usize,
}

impl Cursor {
    /// Return the position of the active frame of the current process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let cursor = Cursor::new(&mut scheduler)?;
    /// assert_eq!(cursor.position, 0);
    /// ```
    fn new(scheduler: &mut Scheduler) -> Result<Self, BeamError> {
        let LoadedModule { code, imports } = scheduler.current_module()?.clone();
        Ok(Cursor {
            module: code,
            imports,
            position: active_position(scheduler),
        })
    }

    /// Replace the position with the position of the active frame. Replace
    /// the code and its import targets too when the active frame is in
    /// another module, which the integer identity of the two modules
    /// decides.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// cursor.refresh(&mut scheduler)?;
    /// assert_eq!(cursor.position, current_frame(scheduler.current_process_mut()).next_instruction);
    /// ```
    fn refresh(&mut self, scheduler: &mut Scheduler) -> Result<(), BeamError> {
        let (module_id, position) = {
            let frame = current_frame(scheduler.current_process_mut());
            (frame.module_id, frame.next_instruction)
        };
        if module_id != self.module.id {
            let loaded = scheduler.current_module()?;
            self.module = Arc::clone(&loaded.code);
            self.imports = Rc::clone(&loaded.imports);
        }
        self.position = position;
        Ok(())
    }

    /// Write the position to the active frame, so that the process resumes
    /// there and a diagnostic reads it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// cursor.commit(process);
    /// assert_eq!(current_frame(process).next_instruction, cursor.position);
    /// ```
    fn commit(&self, process: &mut ProcessState) {
        current_frame(process).next_instruction = self.position;
    }
}

/// Return the index of the next instruction of the active frame.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(active_position(&mut scheduler), 0);
/// ```
fn active_position(scheduler: &mut Scheduler) -> usize {
    current_frame(scheduler.current_process_mut()).next_instruction
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

/// Change the map of the source operand, then write the result to the
/// destination operand. Return the outcome of the change.
///
/// ERTS writes the destination register only after the update succeeds
/// (`jit/arm/instr_map.cpp:687-716`), so a failure, a fail label, and a
/// raised exception leave every register as it was. The instruction takes
/// the map from the source register only when the source and the
/// destination name one slot and the term has one owner. This follows D11
/// of `docs/term-performance-design-codex-v2.md`: take a source register
/// only when it is also the destination, all other operand reads are
/// complete, and a failure preserves the required register state. A take
/// removes the term from the slot, so a failed change writes the unchanged
/// term back; the destination of a change that took no term keeps its
/// value. The change of a term with another owner copies the form of the
/// map first, as before.
///
/// # Errors
///
/// Return the error of `update` for a change that fails.
fn update_owned(
    process: &mut ProcessState,
    map: &Operand,
    destination: &Operand,
    update: impl FnOnce(&mut Map) -> Result<Option<Term>, BeamError>,
) -> Result<Update, BeamError> {
    let owned = match (register_of(map), register_of(destination)) {
        (Some(source), Some(target)) if register_index(&source) == register_index(&target) => {
            take_owned(process, target)
        }
        _ => None,
    };
    let taken = owned.is_some();
    let mut term = match owned {
        Some(term) => term,
        None => read(process, map)?,
    };
    if let Term::Map(entries) = &mut term {
        match update(entries) {
            Ok(None) => {
                write(process, destination, term)?;
                return Ok(Update::Written);
            }
            // The take removed the term from the slot of the destination, so
            // the unchanged term goes back. A change that took no term
            // leaves the destination register as it was, as ERTS does on a
            // failure.
            Ok(Some(reason)) => {
                if taken {
                    write(process, destination, term)?;
                }
                return Ok(Update::Reason(reason));
            }
            Err(error) => {
                if taken {
                    write(process, destination, term)?;
                }
                return Err(error);
            }
        }
    }
    // The operand is not a map. ERTS raises `{badmap, Map}` and writes no
    // register, so the destination register keeps its value. A take only
    // gives a map (see `take_owned`), so no term goes back here.
    Ok(Update::Missing(term.clone()))
}

/// The result of [`update_owned`].
enum Update {
    /// The change succeeded and its map is in the destination register.
    Written,
    /// The source operand does not hold a map. The value is the term of the
    /// operand.
    Missing(Term),
    /// The change of a map failed; the destination register keeps the map
    /// that it held.
    Reason(Term),
}

/// Replace the value of each existing key of `entries`, as ERTS
/// `erts_gc_update_map_exact` does (`beam_common.c:2203-2345`). The pairs go
/// in the flatmap key order, as the ERTS loader sorts them, so the first key
/// that the map does not hold is the first missing key in that order. Return
/// `None` when every key is in the map, or the exception reason
/// `{badkey, Key}` for a missing key. A key of a type that a PBeam map
/// cannot hold, for example a tuple, is a missing key.
///
/// The change reads every pair and checks every key before it writes one
/// value, because the map can be the map of a live register. A failed check
/// leaves `entries` unchanged.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let mut entries = Map::from_pairs(vec![(Term::static_atom("output"), Term::static_atom("raw"))], &atoms)?;
/// let cooked = vec![(Term::static_atom("output"), Term::static_atom("cooked"))];
/// assert_eq!(update_map_exact(&mut entries, cooked, &atoms)?, None);
/// let missing = vec![(Term::static_atom("input"), Term::static_atom("cooked"))];
/// assert_eq!(
///     update_map_exact(&mut entries, missing, &atoms)?,
///     Some(Term::tuple(vec![Term::static_atom("badkey"), Term::static_atom("input")]))
/// );
/// ```
///
/// # Errors
///
/// Return a runtime error when the key order depends on an alias reference
/// (`Map::sort_pairs`).
fn update_map_exact(
    entries: &mut Map,
    mut pairs: Vec<(Term, Term)>,
    atoms: &AtomTable,
) -> Result<Option<Term>, BeamError> {
    Map::sort_pairs(&mut pairs, atoms)?;
    Ok(entries
        .update(
            pairs
                .into_iter()
                .map(|(key, value)| (Insert::Set, key, value))
                .collect(),
            atoms,
        )
        .err()
        .map(|key| Term::tuple(vec![Term::static_atom("badkey"), key])))
}

/// Give up ownership of the term of the register slot of `destination`, or
/// return `None` when the slot does not exist, the slot is empty, or the
/// term has another owner. The last condition reads the count of the `Arc`
/// of the term, so a term that guest code can still observe is not taken.
/// The function is called only when the source and the destination name one
/// slot, so the take leaves the destination register unset.
///
/// An `Arc` count of one is not proof that a descendant is unique; the
/// caller must still copy a shared descendant (`Arc::make_mut`).
///
/// # Examples
///
/// ```rust,ignore
/// let mut process = ProcessState::new();
/// write(&mut process, &Operand::X(0), Term::Map(Map::new()))?;
/// assert!(matches!(take_owned(&mut process, &Operand::X(0)), Some(Term::Map(_))));
/// assert!(take_owned(&mut process, &Operand::X(0)).is_none());
/// ```
fn take_owned(process: &mut ProcessState, destination: Register) -> Option<Term> {
    match destination {
        Register::X(index) => process
            .registers
            .get_mut(index as usize)
            .and_then(Option::as_mut)
            .filter(|term| term.unique())
            .map(|slot| std::mem::replace(slot, Term::nil())),
        Register::Y(index) => process
            .frames
            .last_mut()
            .and_then(|frame| frame.stack.get_mut(index as usize))
            .and_then(|slot| match slot {
                StackSlot::Term(term) if term.unique() => {
                    Some(std::mem::replace(term, Term::nil()))
                }
                StackSlot::Term(_) | StackSlot::Empty | StackSlot::Catch(_) => None,
            }),
    }
}

/// Return a name of a register that two [`Register`] values share only when
/// they are the same slot.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(register_index(&Register::X(1)), register_index(&Register::X(1)));
/// assert_ne!(register_index(&Register::X(1)), register_index(&Register::Y(1)));
/// ```
fn register_index(register: &Register) -> (bool, u32) {
    match register {
        Register::X(index) => (true, *index),
        Register::Y(index) => (false, *index),
    }
}

/// The register slot of a `put_map_assoc` or `put_map_exact` operand. Both
/// the source and the destination of these instructions are a register, so
/// the value cannot be a literal.
enum Register {
    /// An X register at the index.
    X(u32),
    /// A Y register at the index.
    Y(u32),
}

/// Return the [`Register`] of an operand, or `None` for a literal. The
/// loader gives a `put_map_assoc` and a `put_map_exact` a register, so a
/// literal is not an operand of these instructions.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(register_of(&Operand::X(1)), Some(Register::X(1))));
/// assert!(register_of(&Operand::Value(Term::Integer(1))).is_none());
/// ```
fn register_of(operand: &Operand) -> Option<Register> {
    match operand {
        Operand::X(index) => Some(Register::X(*index)),
        Operand::Y(index) => Some(Register::Y(*index)),
        Operand::Value(_) | Operand::UnsupportedLiteral(_) => None,
    }
}

/// Borrow the value of each key in the map of a `has_map_fields`
/// instruction, or return `None` at the first key that the map does not
/// hold. The read copies neither the map nor a value.
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
/// `is_map` test before this instruction.
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

/// Write the value of each key of `pairs` to its destination, as ERTS
/// `beam_jit_get_map_elements` does (`beam_jit_common.cpp:706-767`), and
/// return false when the map does not hold a key. A flatmap is walked once,
/// because the loader sorts the pairs in flatmap key order. A hashmap finds
/// each key in turn. Each value is written when its key is found, so a
/// missing key can leave the earlier destinations written. These partial
/// writes match `beam_jit_get_map_elements`, but not the inline JIT path
/// (`jit/arm/instr_map.cpp:439-462`), which finds the keys from last to
/// first. Compiled code cannot see the difference: on the fail path the
/// validator marks an initialized destination as `any`
/// (`clobber_map_vals`, `beam_validator.erl:1311-1319`). An empty key list
/// matches every map, as in the inline JIT path
/// (`jit/arm/instr_map.cpp:404-415, 464`).
///
/// # Examples
///
/// ```rust,ignore
/// let pairs = vec![(Operand::Value(Term::Integer(1)), Operand::X(0))];
/// assert!(get_map_elements(&mut process, &map, &pairs)?);
/// ```
fn get_map_elements(
    process: &mut ProcessState,
    map: &Map,
    pairs: &[(Operand, Operand)],
) -> Result<bool, BeamError> {
    let Some(entries) = map.flat_pairs() else {
        for (key, destination) in pairs {
            let Some(value) = map.get(read_ref(process, key)?) else {
                return Ok(false);
            };
            write(process, destination, value.clone())?;
        }
        return Ok(true);
    };
    let mut wanted = pairs.iter();
    let mut next = wanted.next();
    for (key, value) in entries {
        let Some((wanted_key, destination)) = next else {
            break;
        };
        if key == read_ref(process, wanted_key)? {
            write(process, destination, value.clone())?;
            next = wanted.next();
        }
    }
    Ok(next.is_none())
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
/// let frame = Term::tuple(vec![module, function, Term::nil(), Term::nil()]);
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
            ExceptionClass::Error => Term::tuple(vec![reason, stacktrace]),
            ExceptionClass::Exit => reason,
            ExceptionClass::Throw => Term::tuple(vec![
                Term::tuple(vec![Term::static_atom("nocatch"), reason]),
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
    process.registers[2] = Some(Term::tuple(vec![class.clone(), stacktrace]));
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
/// let atoms = AtomTable::new();
/// let raw = Term::tuple(vec![Term::static_atom("exit"), Term::nil()]);
/// assert!(matches!(raw_stacktrace(&raw, &atoms)?, (ExceptionClass::Exit, stack) if stack.is_nil()));
/// assert!(matches!(raw_stacktrace(&Term::nil(), &atoms)?, (ExceptionClass::Error, _)));
/// ```
///
/// # Errors
///
/// Each other term gives a runtime invariant error. ERTS reads it as a
/// raw stack trace with no check, so its result is not defined.
fn raw_stacktrace(raw: &Term, atoms: &AtomTable) -> Result<(ExceptionClass, Term), BeamError> {
    if let Term::Tuple(parts) = raw
        && let [class, stack] = &parts[..]
        && let Some(class) = ExceptionClass::from_term(class)
    {
        return Ok((class, stack.clone()));
    }
    if raw.is_nil() {
        return Ok((ExceptionClass::Error, raw.clone()));
    }
    Err(error(format!(
        "invariant error: {} is not a raw stack trace",
        debug_term(raw, atoms)
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

/// Return the head and the tail of a nonempty proper or improper list, in
/// O(1) time, which `get_list` and `get_tl` read as the `CAR` and the `CDR`
/// of the first cons cell (emu `instrs.tab:389-407`; JIT
/// `jit/arm/instr_common.cpp:274-318`). The BEAM validator makes sure that
/// the source is a nonempty list.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let x = || Term::static_atom("x");
/// let list = Term::list_from(vec![Term::Integer(1), Term::Integer(2)]);
/// assert_eq!(nonempty_list(&list, &atoms)?, (&Term::Integer(1), &Term::list_from(vec![Term::Integer(2)])));
/// let improper = Term::list_with_tail(vec![Term::Integer(1)], x());
/// assert_eq!(nonempty_list(&improper, &atoms)?, (&Term::Integer(1), &x()));
/// assert!(nonempty_list(&Term::nil(), &atoms).is_err());
/// ```
fn nonempty_list<'a>(list: &'a Term, atoms: &AtomTable) -> Result<(&'a Term, &'a Term), BeamError> {
    list.cons_cell().ok_or_else(|| {
        error(format!(
            "BEAM list source {} is not a nonempty list",
            debug_term(list, atoms)
        ))
    })
}

/// Return the code position that a branch target names. The loader
/// replaces each branch label with the index of the instruction after the
/// label, so a loaded module reads the field and runs no lookup.
///
/// A target of 0 is the "no fail label" operand of a BIF. The arm of that
/// instruction handles 0 before this call, so any other instruction that
/// reaches this function with 0 keeps the error text of the removed
/// lookup.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(label_position(3)?, 3);
/// assert!(label_position(0).is_err());
/// ```
fn label_position(target: u32) -> Result<usize, BeamError> {
    if target == 0 {
        return Err(error("missing BEAM label 0"));
    }
    Ok(target as usize)
}

/// Return the label of the choice that equals `value` in the ordered choice
/// list of [`Instruction::SelectValue`], or `None` when no choice equals it.
///
/// The list is sorted by [`Term::compare_exact`], so a binary search over
/// the list visits about `log2(len)` choices instead of all of them. The
/// comparison is the table-aware exact term order, so two atoms compare by
/// name and not by atom index, and a value equals a choice only when the two
/// are exactly equal, as in the linear walk: `1` does not match `1.0`, and
/// `-0.0` does not match `0.0`.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let choices = vec![
///     (Term::static_atom("a"), 3),
///     (Term::static_atom("b"), 4),
/// ];
/// assert_eq!(
///     select_value_ordered(&Term::static_atom("b"), &choices, &atoms)?,
///     Some(4)
/// );
/// assert_eq!(
///     select_value_ordered(&Term::static_atom("c"), &choices, &atoms)?,
///     None
/// );
/// ```
///
/// # Errors
///
/// Return a runtime error when the value and a choice have no ERTS term
/// order, as two different references have none. The interpreter reports
/// that error and does not panic.
fn select_value_ordered(
    value: &Term,
    choices: &[(Term, u32)],
    atoms: &AtomTable,
) -> Result<Option<u32>, BeamError> {
    let mut low = 0;
    let mut high = choices.len();
    while low < high {
        let middle = low + (high - low) / 2;
        match value.compare_exact(&choices[middle].0, atoms)? {
            Ordering::Less => high = middle,
            Ordering::Greater => low = middle + 1,
            Ordering::Equal => return Ok(Some(choices[middle].1)),
        }
    }
    Ok(None)
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
/// write(&mut process, &Operand::X(0), Term::tuple(vec![Term::Integer(1)]))?;
/// assert!(matches!(read_ref(&process, &Operand::X(0))?, Term::Tuple(_)));
/// ```
fn read_ref<'a>(process: &'a ProcessState, operand: &'a Operand) -> Result<&'a Term, BeamError> {
    match read_any_ref(process, operand)? {
        Term::MatchContext(_) => Err(match_context_escape(operand)),
        value => Ok(value),
    }
}

/// Borrow a classified operand of [`Instruction::TestExactEqualOperands`].
/// The loader classified the operand, so this reads a checked register slot
/// or returns the stored term, and it matches no [`Operand`].
///
/// A register that holds a match context gives the same invariant error as
/// [`read_ref`]. A stored term comes from the literal table, which cannot
/// hold a match context.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(
///     read_test_operand(&process, &TestOperand::Value(Term::Integer(1)))?,
///     &Term::Integer(1)
/// );
/// ```
fn read_test_operand<'a>(
    process: &'a ProcessState,
    operand: &'a TestOperand,
) -> Result<&'a Term, BeamError> {
    match operand {
        TestOperand::Value(value) => Ok(value),
        TestOperand::X(index) => match read_x_register(&process.registers, *index)? {
            Term::MatchContext(_) => Err(match_context_escape(&Operand::X(*index))),
            value => Ok(value),
        },
        TestOperand::Y(index) => {
            let stack = process.frames.last().map_or(&[][..], |frame| &frame.stack);
            match read_y_register(stack, *index)? {
                Term::MatchContext(_) => Err(match_context_escape(&Operand::Y(*index))),
                value => Ok(value),
            }
        }
    }
}

/// Read one X register slot with the checked access of [`read_operand`].
///
/// # Examples
///
/// ```rust,ignore
/// let registers = vec![Some(Term::Integer(7))];
/// assert_eq!(read_x_register(&registers, 0)?, &Term::Integer(7));
/// assert!(read_x_register(&registers, 1).is_err());
/// ```
fn read_x_register(registers: &[Option<Term>], index: u32) -> Result<&Term, BeamError> {
    registers
        .get(index as usize)
        .and_then(Option::as_ref)
        .ok_or_else(|| error(format!("register x{index} is unset")))
}

/// Read one Y stack slot with the checked access of [`read_operand`]. A slot
/// that holds a catch tag, an empty slot, and an out-of-range index each
/// keep their own error text.
///
/// # Examples
///
/// ```rust,ignore
/// let stack = vec![StackSlot::Term(Term::Integer(7))];
/// assert_eq!(read_y_register(&stack, 0)?, &Term::Integer(7));
/// assert!(read_y_register(&stack, 1).is_err());
/// ```
fn read_y_register(stack: &[StackSlot], index: u32) -> Result<&Term, BeamError> {
    match stack.get(index as usize) {
        Some(StackSlot::Term(value)) => Ok(value),
        // The BEAM validator never lets an instruction read a catch tag.
        Some(StackSlot::Catch(_)) => {
            Err(error(format!("stack register y{index} holds a catch tag")))
        }
        Some(StackSlot::Empty) | None => Err(error(format!("stack register y{index} is unset"))),
    }
}

/// Read a copy of an operand from the process registers or the active
/// frame, also when it is a match context.
///
/// # Examples
///
/// ```rust,ignore
/// let context = Arc::new(MatchState::new(b"a"));
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
/// let context = Arc::new(MatchState::new(b"a"));
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
/// let atoms = AtomTable::new();
/// let context = Arc::new(MatchState::new(b"a"));
/// write(&mut process, &Operand::X(0), Term::MatchContext(context))?;
/// assert_eq!(match_context(&process, &Operand::X(0), &atoms)?.position(), 0);
/// ```
fn match_context(
    process: &ProcessState,
    operand: &Operand,
    atoms: &AtomTable,
) -> Result<Arc<MatchState>, BeamError> {
    let context = any_match_context(process, operand, atoms)?;
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
/// let context = Arc::new(MatchState::new(b"a"));
/// assert!(check_known_position(&context, &Operand::X(0)).is_ok());
/// context.forget_position();
/// assert!(check_known_position(&context, &Operand::X(0)).is_err());
/// ```
fn check_known_position(context: &MatchState, operand: &Operand) -> Result<(), BeamError> {
    if context.position_known() {
        Ok(())
    } else {
        Err(error(format!(
            "unsupported read of the match context in {operand:?} after a failed bs_match =:= command"
        )))
    }
}

/// Return the `bit_size/1` or `byte_size/1` result of a match context in
/// `values`, the argument values of the BIF `module:name`, or `None` for
/// each other BIF or argument. The BEAM validator lets these two BIFs get
/// a match context (`validate_bif_sources`, beam_validator.erl:1649-1654).
/// ERTS gives the bits from its position to its end
/// (`emit_bif_bit_size`, jit/arm/instr_guard_bifs.cpp:336-441; ops.tab:
/// 785-787), and for `byte_size/1` the same bits in bytes, rounded up
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
/// let context = Arc::new(MatchState::new(b"abc"));
/// context.skip(4);
/// let values = [Term::MatchContext(context)];
/// let size = match_context_size(BifId::ErlangByteSize1, &[Operand::X(0)], &values)?;
/// assert_eq!(size, Some(Term::Integer(3)));
/// assert!(match_context_size(BifId::ErlangHd1, &[Operand::X(0)], &values).is_err());
/// ```
fn match_context_size<T: Borrow<Term>>(
    bif: BifId,
    arguments: &[Operand],
    values: &[T],
) -> Result<Option<Term>, BeamError> {
    if matches!(bif, BifId::ErlangBitSize1 | BifId::ErlangByteSize1)
        && let [value] = values
        && let Term::MatchContext(context) = Borrow::<Term>::borrow(value)
    {
        check_known_position(context, &arguments[0])?;
        let bits = context.remaining_bits();
        let size = if bif == BifId::ErlangBitSize1 {
            bits
        } else {
            bits.div_ceil(8)
        };
        return Ok(Some(Term::Integer(size as i64)));
    }
    match values
        .iter()
        .position(|value| matches!(Borrow::<Term>::borrow(value), Term::MatchContext(_)))
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
/// let atoms = AtomTable::new();
/// let context = Arc::new(MatchState::new(b"a"));
/// write(&mut process, &Operand::X(0), Term::MatchContext(context))?;
/// assert_eq!(any_match_context(&process, &Operand::X(0), &atoms)?.position(), 0);
/// assert!(any_match_context(&process, &Operand::Value(Term::Integer(1)), &atoms).is_err());
/// ```
fn any_match_context(
    process: &ProcessState,
    operand: &Operand,
    atoms: &AtomTable,
) -> Result<Arc<MatchState>, BeamError> {
    match &read_any(process, operand)? {
        Term::MatchContext(context) => Ok(Arc::clone(context)),
        other => Err(error(format!(
            "invariant error: {} holds {}, not a match context",
            debug_operand(operand, atoms),
            debug_term(other, atoms)
        ))),
    }
}

/// Return the text of `operand` for a message, with the atom name of a
/// `Value` operand read from `atoms`.
///
/// [`Operand`] derives `Debug`, and a `Value` operand writes the term with
/// the `Debug` text of the term, which holds the atom index. This function
/// writes that term with [`debug_term`], so a message keeps the atom name.
/// The other forms write the same text as the derived `Debug`.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let operand = Operand::Value(Term::static_atom("a"));
/// assert_eq!(debug_operand(&operand, &atoms), "Value(Atom(\"a\"))");
/// assert_eq!(debug_operand(&Operand::X(0), &atoms), "X(0)");
/// ```
fn debug_operand(operand: &Operand, atoms: &AtomTable) -> String {
    match operand {
        Operand::Value(value) => format!("Value({})", debug_term(value, atoms)),
        other => format!("{other:?}"),
    }
}

/// Write a float register. The float registers grow as necessary.
///
/// # Examples
///
/// ```rust,ignore
/// write_float_register(&mut process, 2, 1.5);
/// assert_eq!(read_float_register(&process, 2)?, 1.5);
/// ```
fn write_float_register(process: &mut ProcessState, register: u32, value: f64) {
    let index = register as usize;
    if index >= process.float_registers.len() {
        process.float_registers.resize(index + 1, 0.0);
    }
    process.float_registers[index] = value;
}

/// Read a float register.
///
/// # Examples
///
/// ```rust,ignore
/// write_float_register(&mut process, 0, -2.5);
/// assert_eq!(read_float_register(&process, 0)?, -2.5);
/// ```
///
/// # Errors
///
/// A register that no instruction wrote is a runtime invariant error: the
/// compiler writes each float register before it reads it.
fn read_float_register(process: &ProcessState, register: u32) -> Result<f64, BeamError> {
    process
        .float_registers
        .get(register as usize)
        .copied()
        .ok_or_else(|| error(format!("float register fr{register} has no value")))
}

/// Write a value to an X register or a Y register of the active frame. A
/// process that starts at a BIF has no frame while the BIF runs, and the BIF
/// writes only X registers.
///
/// # Examples
///
/// ```rust,ignore
/// write(&mut process, &Operand::X(0), Term::Integer(1))?;
/// ```
fn write(process: &mut ProcessState, operand: &Operand, value: Term) -> Result<(), BeamError> {
    let stack = match process.frames.last_mut() {
        Some(frame) => frame.stack.as_mut_slice(),
        None => &mut [],
    };
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
    /// A fun of the called arity: the fun term, its entry, and its free
    /// variables.
    Enter {
        target: Target,
        free: Vec<Term>,
        fun: Term,
    },
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
                fun: Term::Fun(fun.clone()),
            },
            Term::Fun(_) => FunCall::BadArity(fun.clone()),
            _ => FunCall::BadFun(fun.clone()),
        }
    }
}

#[derive(Debug)]
struct Function {
    /// Identity of the function in the name table of its module.
    name: NameId,
    arity: u32,
    start: usize,
}

/// Find the function whose entry label is `target`, from the loader table.
/// The function starts after the label.
///
/// # Examples
///
/// ```rust,ignore
/// let function = function_at_label(&module, 2)?;
/// assert_eq!((module.tables.name(function.name), function.arity), ("main", 0));
/// ```
fn function_at_label(module: &BeamModule, target: u32) -> Result<Function, BeamError> {
    match module.tables.function_starts.get(&target) {
        Some(Some(entry)) => {
            let (name, arity) = &module.tables.functions[entry.function];
            Ok(Function {
                name: *name,
                arity: *arity,
                start: entry.start,
            })
        }
        Some(None) => Err(error(format!("entry label {target} has no function info"))),
        None => Err(error(format!(
            "entry label {target} not found in {}",
            module.name
        ))),
    }
}

/// Return the sum or the difference of a `bif` instruction of
/// `erlang:'+'/2` or `erlang:'-'/2` when both operands are ERTS small
/// integers, or `None` for each other case. A `None` call goes through the
/// BIF path, which gives the value, the exception, the reductions of a
/// bignum operand, or the runtime error of each other case, as
/// `emit_i_plus` and `emit_i_minus` add or subtract two small integers
/// inline and call the BIF for each other case
/// (`jit/arm/instr_arith.cpp:171-204`, `:360-391`). The inline sum of two
/// small integers costs no reduction, also when it is a bignum. An operand
/// that is unset or a match context gives `None`, so the BIF path reports
/// its error.
///
/// # Examples
///
/// ```rust,ignore
/// let operands = [Operand::Value(Term::Integer(1)), Operand::Value(Term::Integer(2))];
/// assert_eq!(small_integer_arith(&process, BifId::ErlangPlus2, &operands), Some(Term::Integer(3)));
/// assert_eq!(small_integer_arith(&process, BifId::ErlangMul2, &operands), None);
/// let big = [Operand::Value(Term::Integer(1 << 59)), Operand::Value(Term::Integer(1))];
/// assert_eq!(small_integer_arith(&process, BifId::ErlangPlus2, &big), None);
/// ```
fn small_integer_arith(process: &ProcessState, bif: BifId, operands: &[Operand]) -> Option<Term> {
    let (BifId::ErlangPlus2 | BifId::ErlangMinus2, [left, right]) = (bif, operands) else {
        return None;
    };
    let left = read_any_ref(process, left).ok()?.small_integer()?;
    let right = read_any_ref(process, right).ok()?.small_integer()?;
    // Two small integers fit 61 bits, so the i64 result is exact.
    Some(Term::Integer(if bif == BifId::ErlangPlus2 {
        left + right
    } else {
        left - right
    }))
}

/// Read the argument operands of a leaf `bif` instruction into a borrowed
/// [`BifArguments`], which holds at most [`MAX_BORROWED_ARGUMENTS`]
/// references. Each operand must be set, and a match context is a runtime
/// invariant error, as it is for a call argument. The loader gives a `bif`
/// instruction at most [`MAX_BORROWED_ARGUMENTS`] arguments, so this
/// function holds every argument of an instruction that the loader loads.
/// The read copies no term and writes no term: each reference points to
/// the literal of the instruction or to the register of the process.
///
/// # Examples
///
/// ```rust,ignore
/// let operands = [Operand::X(0)];
/// let arguments = read_bif_arguments(&process, &operands)?;
/// assert_eq!(arguments.with(|terms| terms.len()), 1);
/// assert!(read_bif_arguments(&process, &[Operand::X(7)]).is_err());
/// ```
///
/// # Errors
///
/// An unset operand, a match context, or a call with more than
/// [`MAX_BORROWED_ARGUMENTS`] arguments.
fn read_bif_arguments<'a>(
    process: &'a ProcessState,
    operands: &'a [Operand],
) -> Result<BifArguments<'a>, BeamError> {
    if operands.len() > MAX_BORROWED_ARGUMENTS {
        return Err(error(format!(
            "a BIF call holds more than {MAX_BORROWED_ARGUMENTS} borrowed arguments"
        )));
    }
    let mut arguments = BifArguments::new();
    for operand in operands {
        arguments.push(read_any_ref(process, operand)?);
    }
    Ok(arguments)
}

/// Read the argument operands of a `bif` instruction into an owned
/// [`OwnedBifArguments`], which holds at most [`MAX_BORROWED_ARGUMENTS`]
/// terms. Each operand must be set, and a match context is a runtime
/// invariant error, as it is for a call argument. The owned lane copies
/// each term, so a body of that lane may borrow the scheduler at the same
/// time.
///
/// # Examples
///
/// ```rust,ignore
/// let operands = [Operand::X(0)];
/// let arguments = read_owned_bif_arguments(&process, &operands)?;
/// assert_eq!(arguments.with(|terms| terms.len()), 1);
/// assert!(read_owned_bif_arguments(&process, &[Operand::X(7)]).is_err());
/// ```
///
/// # Errors
///
/// An unset operand, a match context, or a call with more than
/// [`MAX_BORROWED_ARGUMENTS`] arguments.
fn read_owned_bif_arguments(
    process: &ProcessState,
    operands: &[Operand],
) -> Result<OwnedBifArguments, BeamError> {
    if operands.len() > MAX_BORROWED_ARGUMENTS {
        return Err(error(format!(
            "a BIF call holds more than {MAX_BORROWED_ARGUMENTS} borrowed arguments"
        )));
    }
    let mut arguments = OwnedBifArguments::new();
    for operand in operands {
        arguments.push(read_any(process, operand)?);
    }
    Ok(arguments)
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
/// let context = Arc::new(MatchState::new(b"a"));
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
/// let context = Arc::new(MatchState::new(b"a"));
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
/// (erl_bits.c:400-424). Thus `MatchState::read_bits` gives the same
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
///   not known (`MatchState::forget_position`).
///
/// # Errors
///
/// A command that reads more bits than remain gives a runtime invariant
/// error. An integer outside the 64-bit signed range needs a bignum and
/// gives a runtime error (row "Integer range" of
/// docs/erts-differences.md).
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let context = Arc::new(MatchState::new(&[7, 8]));
/// write(&mut process, &Operand::X(0), Term::MatchContext(context))?;
/// let unit = NonZeroU64::new(8).unwrap();
/// let byte = BinaryMatchCommand::Integer { size: 8, little: false, signed: false, destination: Operand::X(1) };
/// assert!(binary_match(&mut process, &Operand::X(0), &[BinaryMatchCommand::EnsureAtLeast { size: 8, unit }, byte], &atoms)?);
/// assert_eq!(read(&process, &Operand::X(1))?, Term::Integer(7));
/// assert!(!binary_match(&mut process, &Operand::X(0), &[BinaryMatchCommand::EnsureExactly { size: 0 }], &atoms)?);
/// ```
fn binary_match(
    process: &mut ProcessState,
    context: &Operand,
    commands: &[BinaryMatchCommand],
    atoms: &AtomTable,
) -> Result<bool, BeamError> {
    let context = match_context(process, context, atoms)?;
    let state = &*context;
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
                // ERTS reads a segment of any size at any bit position: a
                // segment of more than 64 bits as a sub-bitstring
                // (`GET_BITSTRING`, jit/arm/instr_bs.cpp:3136-3176), and a
                // smaller one as a heap bitstring copy (`READ` and
                // `EXTRACT_BITSTRING`, instr_bs.cpp:3047-3054). The value is
                // the same, and `read_bytes` gives the segment a view of its
                // bits.
                let bytes = state.read_bytes(*size).ok_or_else(|| too_few_bits(size))?;
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
                // ERTS writes the bits after the position as a
                // sub-bitstring (`emit_bs_get_tail_shared`,
                // jit/arm/instr_bs.cpp:407-439), at any bit position.
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
    /// The segment operation: `binary`, `float`, `integer`, `utf8`,
    /// `utf16`, or `utf32`.
    operation: &'static str,
    /// The failed check: `type`, `unit`, or `size`, or for a `float`
    /// segment also `no_float` or `invalid`.
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
/// `string` segment, a `binary` segment with its handle and the reductions
/// of its copy, an `integer` segment with its term and its size in bits, or
/// the code point of a `utf8` segment.
enum BinaryPart<'a> {
    Bytes(Cow<'a, [u8]>, usize),
    /// A `binary` segment: its binary, the reductions of its copy, its
    /// segment number, and its unit. The handle keeps the arrays of the
    /// binary, so an append can share them and copy no byte of the segment.
    Shared(Binary, usize, u64, u64),
    /// A `binary` segment with a size: its segment number, its term, and
    /// the number of bits that it copies from the start of the term. ERTS
    /// checks the term at the construct pass (`erts_bs_put_binary`,
    /// erl_bits.c:1285-1308).
    Sized {
        segment: u64,
        value: Term,
        bits: u64,
    },
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
    /// A `utf16` segment with its term and its byte order. ERTS checks the
    /// term at the construct pass, so the part keeps it.
    Utf16 {
        segment: u64,
        value: Term,
        little: bool,
    },
    /// A `utf32` segment with its code point and its byte order. ERTS
    /// checks the value at the size pass, so the part keeps the code point.
    Utf32 {
        code_point: u32,
        little: bool,
    },
    /// A `float` segment with its term and its size in bits.
    Float {
        segment: u64,
        value: Term,
        bits: u64,
        little: bool,
    },
}

impl BinaryPart<'_> {
    /// Return the `type` failure of an `integer` part whose term is not an
    /// integer, or the failure of a `float` part (see `float_segment_bits`).
    fn type_failure(&self) -> Option<BinarySegmentFailure> {
        match self {
            BinaryPart::Integer { segment, value, .. } if !value.is_integer() => Some(
                BinarySegmentFailure::badarg(*segment, "integer", "type", value.clone()),
            ),
            BinaryPart::Float {
                segment,
                value,
                bits,
                ..
            } => float_segment_bits(*segment, value, *bits).err(),
            // A size in bits that is not a multiple of the unit fails in the
            // construct pass (`erts_bs_put_binary_all`, erl_bits.c:1321-1324;
            // `BSC_INFO_UNIT`, jit/arm/instr_bs.cpp:2033-2047).
            // A term that is not a bitstring, or that has fewer bits than
            // the size, fails with the info `type` or `short`
            // (`erts_bs_put_binary`, erl_bits.c:1290-1300; `BSC_INFO_DEPENDS`,
            // jit/arm/instr_bs.cpp:2068-2071).
            BinaryPart::Sized {
                segment,
                value,
                bits,
            } => match value {
                Term::Binary(binary) if binary.bit_size() as u64 >= *bits => None,
                Term::Binary(_) => Some(BinarySegmentFailure::badarg(
                    *segment,
                    "binary",
                    "short",
                    value.clone(),
                )),
                _ => Some(BinarySegmentFailure::badarg(
                    *segment,
                    "binary",
                    "type",
                    value.clone(),
                )),
            },
            BinaryPart::Shared(binary, _, segment, unit)
                if !(binary.bit_size() as u64).is_multiple_of(*unit) =>
            {
                Some(BinarySegmentFailure::badarg(
                    *segment,
                    "binary",
                    "unit",
                    Term::Binary(binary.clone()),
                ))
            }
            _ => None,
        }
    }

    /// Return true when the part writes at least one bit.
    fn has_bits(&self) -> bool {
        match self {
            BinaryPart::Bytes(bytes, _) => !bytes.is_empty(),
            BinaryPart::Shared(binary, ..) => binary.bit_size() > 0,
            BinaryPart::Integer { bits, .. }
            | BinaryPart::Float { bits, .. }
            | BinaryPart::Sized { bits, .. } => *bits > 0,
            BinaryPart::Utf8(_) | BinaryPart::Utf16 { .. } | BinaryPart::Utf32 { .. } => true,
        }
    }
}

/// Return the IEEE 754 bits of the term of a `float` segment of `bits`
/// bits in the low bits of the result, or its failure, as
/// `erts_bs_put_float` checks it (erl_bits.c:1340-1452). A size other than
/// 16, 32, or 64 fails with `invalid` and the size. A term that is not a
/// number fails with `type`, and a bignum that `big_to_double` cannot
/// convert fails with `no_float`. A small integer converts with one C cast
/// to the float of the size, and a bignum converts to a double first. A
/// value too large for 32 bits gives an infinity, as the C cast does, and
/// ERTS gives no error. PBeam does not write 16-bit floats yet (row
/// "Binary construction" of `docs/erts-differences.md`), so a number of
/// such a segment gives `None`, which the caller changes into a runtime
/// error.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(float_segment_bits(1, &Term::Float(1.5), 64), Ok(Some(0x3FF8_0000_0000_0000)));
/// assert_eq!(float_segment_bits(1, &Term::Integer(-3), 32), Ok(Some(0xC040_0000)));
/// assert!(float_segment_bits(1, &Term::static_atom("a"), 64).is_err());
/// ```
fn float_segment_bits(
    segment: u64,
    value: &Term,
    bits: u64,
) -> Result<Option<i64>, BinarySegmentFailure> {
    let fail = |info, term| BinarySegmentFailure::badarg(segment, "float", info, term);
    if !matches!(bits, 16 | 32 | 64) {
        return Err(fail("invalid", Term::Integer(bits as i64)));
    }
    let number = match (value.small_integer(), value) {
        (Some(small), _) => Ok(small),
        (None, Term::Float(float)) => Err(*float),
        (None, Term::Integer(_) | Term::BigInt(_)) => match value.number_float() {
            Some(float) => Err(float),
            None => return Err(fail("no_float", value.clone())),
        },
        _ => return Err(fail("type", value.clone())),
    };
    Ok(match (bits, number) {
        (64, Ok(small)) => Some((small as f64).to_bits() as i64),
        (64, Err(float)) => Some(float.to_bits() as i64),
        (32, Ok(small)) => Some(i64::from((small as f32).to_bits())),
        (32, Err(float)) => Some(i64::from((float as f32).to_bits())),
        _ => None,
    })
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
/// let size = Term::from_bigint(BigInt::from(1u128 << 64));
/// assert_eq!(integer_segment_bits(false, &size, 1), Err("system_limit"));
/// ```
fn integer_segment_bits(fixed: bool, size: &Term, unit: u64) -> Result<u64, &'static str> {
    match (size.small_integer(), size) {
        (Some(bits), _) if bits >= 0 && (fixed || unit == 1 || bits >> 52 == 0) => {
            Ok((bits as u64).saturating_mul(unit))
        }
        (Some(bits), _) if bits >= 0 => Err("system_limit"),
        (None, Term::Integer(bits)) if *bits >= 0 => Err("system_limit"),
        (None, Term::BigInt(bits)) if bits.sign() == num_bigint::Sign::Plus => Err("system_limit"),
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
/// Return one array for each segment of an append whose later segments each
/// write a whole byte range, with the reductions that ERTS charges for the
/// segment, or `None` when a segment writes bits. The caller charges the
/// reductions only for a `Some` result, so an append that writes bits charges
/// them once, in the writer path.
///
/// # Examples
///
/// ```rust,ignore
/// let string = BinaryPart::Bytes(Cow::Borrowed(b"ab"), 1);
/// assert_eq!(
///     shared_append_arrays(&[string]).map(|arrays| arrays.len()),
///     Some(1)
/// );
/// assert!(shared_append_arrays(&[BinaryPart::Utf8(65)]).is_none());
/// ```
fn shared_append_arrays(parts: &[BinaryPart<'_>]) -> Option<Vec<(Arc<Vec<u8>>, usize)>> {
    let mut arrays = Vec::new();
    for part in parts {
        match part {
            BinaryPart::Bytes(bytes, reductions) => {
                arrays.push((Arc::new(bytes.clone().into_owned()), *reductions));
            }
            // The chunks of a bitstring are not its bits, so the writer
            // copies its bits.
            BinaryPart::Shared(binary, reductions, ..) if binary.bit_size().is_multiple_of(8) => {
                for chunk in binary.chunks() {
                    arrays.push((chunk, *reductions));
                }
            }
            BinaryPart::Shared(..)
            | BinaryPart::Sized { .. }
            | BinaryPart::Integer { .. }
            | BinaryPart::Utf8(_)
            | BinaryPart::Float { .. } => {
                return None;
            }
            BinaryPart::Utf16 { .. } | BinaryPart::Utf32 { .. } => return None,
        }
    }
    Some(arrays)
}

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
/// the term of each `integer` segment of size 0. Else the result keeps the
/// arrays of that binary, which uses one reduction for each 8192 bits, as
/// the ERTS path for a binary that is not writable does
/// (erl_bits.c:1855-1924). PBeam has no writable binary (row "Binary
/// construction" of `docs/erts-differences.md`).
///
/// The construct pass writes the segments (jit/arm/instr_bs.cpp:
/// 1994-2486). A `string` segment keeps its bytes (`erts_bs_put_string`,
/// erl_bits.c:1641-1654). A `binary` segment keeps its binary and uses one
/// reduction for each 8192 bits (`beam_jit_bs_put_binary_all`,
/// jit/beam_jit_common.cpp:1043-1057). An `integer` segment with a term
/// that is not an integer fails with `type`, in the order of
/// `integer_type_failure`; else it writes the integer with no reduction
/// (`erts_bs_put_integer_be` and `erts_bs_put_integer_le`, erl_bits.c:
/// 926-1167). A `utf8` segment writes the UTF-8 bytes of its code point
/// (`emit_construct_utf8`). A `binary` part whose size in bits is not a
/// multiple of its unit fails with `unit` in the construct pass
/// (`erts_bs_put_binary_all`, erl_bits.c:1321-1324).
///
/// The result of an `append` segment that adds bits keeps the arrays of the
/// append binary and of each later `binary` or `string` segment, so the
/// physical copy of the old payload is absent (`Binary::concat_arrays`).
/// The append still copies at most one leaf of 4096 bytes when the later
/// segments fit in one leaf with the last array of the append binary. PBeam
/// still charges the copy reductions of each segment, because ERTS charges
/// them (`erl_bits.c:1855-1924`). Thus the logical reduction count is the
/// same as the ERTS count.
///
/// # Examples
///
/// ```rust,ignore
/// write(&mut process, &Operand::X(0), Term::binary(b"3".to_vec()))?;
/// let segments = [
///     BinarySegment::String(b"count: ".to_vec()),
///     BinarySegment::BinaryAll { segment: 2, unit: 8, source: Operand::X(0) },
/// ];
/// assert_eq!(
///     create_binary(atoms, &mut process, &segments)?,
///     Ok(Term::binary(b"count: 3".to_vec()))
/// );
/// write(&mut process, &Operand::X(0), Term::static_atom("a"))?;
/// assert_eq!(
///     create_binary(atoms, &mut process, &segments)?,
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
/// assert_eq!(
///     create_binary(atoms, &mut process, &segments)?,
///     Ok(Term::binary(vec![0x34, 0x12]))
/// );
/// ```
///
/// # Errors
///
/// A segment of a type, size, or unit that PBeam does not run yet is a
/// runtime error. A result whose size in bits is not a multiple of 8 is a
/// runtime error, because PBeam has no bitstring term. A match context in
/// a `binary` segment is a runtime invariant error.
fn create_binary(
    atoms: &AtomTable,
    process: &mut ProcessState,
    segments: &[BinarySegment],
) -> Result<Result<Term, BinarySegmentFailure>, BeamError> {
    let mut parts = Vec::with_capacity(segments.len());
    let mut append = None;
    let mut private = None;
    for segment in segments {
        match segment {
            BinarySegment::String(text) => {
                parts.push(BinaryPart::Bytes(Cow::Borrowed(text.as_slice()), 0));
            }
            BinarySegment::BinaryAll {
                segment,
                unit,
                source,
            } => {
                let mut value = read(process, source)?;
                let Term::Binary(bytes) = &mut value else {
                    return Ok(Err(BinarySegmentFailure::badarg(
                        *segment, "binary", "type", value,
                    )));
                };
                let reductions = bytes.bit_size() / BITS_PER_REDUCTION;
                parts.push(BinaryPart::Shared(
                    bytes.clone(),
                    reductions,
                    *segment,
                    *unit,
                ));
            }
            BinarySegment::Binary {
                segment,
                unit,
                source,
                size,
            } => match sized_binary_part(process, *segment, *unit, source, size)? {
                Ok(part) => parts.push(part),
                Err(failure) => return Ok(Err(failure)),
            },
            BinarySegment::Append {
                segment,
                unit,
                source,
            } => append = Some((*segment, *unit, read(process, source)?)),
            BinarySegment::PrivateAppend { source } => private = Some(source),
            BinarySegment::Float {
                segment,
                unit,
                little,
                source,
                size,
            } => {
                // The size pass checks the size of a `float` segment as the
                // size of an `integer` segment (jit/arm/instr_bs.cpp:
                // 1493-1505, 1639-1689).
                let fixed = matches!(size, Operand::Value(Term::Integer(literal)) if (0..1 << 56).contains(literal));
                let size = read(process, size)?;
                let bits = match integer_segment_bits(fixed, &size, *unit) {
                    Ok(bits) => bits,
                    Err(reason) => {
                        return Ok(Err(BinarySegmentFailure {
                            reason,
                            ..BinarySegmentFailure::badarg(*segment, "float", "size", size)
                        }));
                    }
                };
                parts.push(BinaryPart::Float {
                    segment: *segment,
                    value: read(process, source)?,
                    bits,
                    little: *little,
                });
            }
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
            BinarySegment::Utf16 {
                segment,
                little,
                source,
            } => {
                // ERTS computes the size of a `utf16` segment from the term
                // with no check and without reading the bit offset
                // (`am_utf16`, jit/arm/instr_bs.cpp:1751-1765), and
                // `erts_bs_put_utf16` then fails at the construct pass for
                // a term outside the range, with the cause
                // `{Segment, utf16, type, Value}`
                // (erl_bits.c:1231-1237; jit/arm/instr_bs.cpp:2418-2425).
                parts.push(BinaryPart::Utf16 {
                    segment: *segment,
                    value: read(process, source)?,
                    little: *little,
                });
            }
            BinarySegment::Utf32 {
                segment,
                little,
                source,
            } => {
                // ERTS checks the value at the size pass and fails with the
                // cause `{Segment, utf32, type, Value}`
                // (`am_utf32`, jit/arm/instr_bs.cpp:1766-1799).
                let value = read(process, source)?;
                let Some(code_point) = utf_segment_code_point(&value) else {
                    return Ok(Err(BinarySegmentFailure::badarg(
                        *segment, "utf32", "type", value,
                    )));
                };
                parts.push(BinaryPart::Utf32 {
                    code_point,
                    little: *little,
                });
            }
            BinarySegment::Unsupported(kind) => {
                return Err(error(format!(
                    "unsupported bs_create_bin segment {}",
                    atoms.name_of(kind)
                )));
            }
        }
    }
    if let Some(source) = private {
        return private_append(process, source, &parts);
    }
    let mut prefix = None;
    if let Some((segment, unit, mut value)) = append {
        let Term::Binary(start) = &mut value else {
            return Ok(Err(BinarySegmentFailure::badarg(
                segment, "binary", "type", value,
            )));
        };
        if unit > 1 && !(start.bit_size() as u64).is_multiple_of(unit) {
            return Ok(Err(BinarySegmentFailure::badarg(
                segment, "binary", "unit", value,
            )));
        }
        if !parts.iter().any(BinaryPart::has_bits) {
            // The construct pass still checks the term of each `integer`
            // segment of size 0 (erl_bits.c:934-936).
            if let Some((index, failure)) = parts
                .iter()
                .enumerate()
                .find_map(|(index, part)| Some((index, part.type_failure()?)))
            {
                return Ok(Err(match parts[index] {
                    BinaryPart::Integer { .. } => integer_type_failure(&parts, index),
                    _ => failure,
                }));
            }
            return Ok(Ok(Term::Binary(start.clone())));
        }
        let reductions = start.bit_size() / BITS_PER_REDUCTION;
        process.bump_reductions(i32::try_from(reductions).unwrap_or(i32::MAX));
        // The result shares the chunks of the append binary. The writer holds
        // only the later segments, so the old payload stays in its arrays.
        // The chunks of a bitstring are not its bits, so the writer copies
        // the bits of such a prefix first, with no more reductions.
        if start.bit_size().is_multiple_of(8) {
            prefix = Some(start.clone());
        } else {
            parts.insert(0, BinaryPart::Shared(start.clone(), 0, segment, 1));
        }
    }
    // An append whose later segments each write a whole byte range keeps the
    // arrays of the append binary and of each `binary` segment. The result
    // shares them and copies no byte of the old payload. When the later
    // segments fit in one leaf with the last array of the append binary,
    // `Binary::concat_arrays` merges them in one leaf and copies at most that
    // leaf. A segment that writes bits cannot share an array, and that append
    // writes its segments in the writer below.
    let shared = prefix.as_ref().and_then(|_| shared_append_arrays(&parts));
    if let (Some(start), Some(arrays)) = (prefix.as_ref(), shared) {
        let mut chunks = Vec::with_capacity(arrays.len());
        for (array, reductions) in arrays {
            chunks.push(array);
            process.bump_reductions(i32::try_from(reductions).unwrap_or(i32::MAX));
        }
        return Ok(Ok(Term::Binary(Binary::concat_arrays(start, chunks))));
    }
    let (bytes, bits) = match write_binary_parts(process, &parts)? {
        Ok(written) => written,
        Err(failure) => return Ok(Err(failure)),
    };
    Ok(Ok(Term::Binary(match &prefix {
        Some(start) if bits.is_multiple_of(8) => Binary::concat(start, &bytes),
        Some(start) => {
            let size = start.bit_size() + bits as usize;
            Binary::concat(start, &bytes).bits(0, size)
        }
        None => bitstring(bytes, bits),
    })))
}

/// Return the code point of a `utf16` or `utf32` segment of the term
/// `value`, or `None` for each term that ERTS fails: a term that is not an
/// integer, a bignum, a negative value, a value above 0x10FFFF, and a
/// surrogate (`erts_bs_put_utf16`, erl_bits.c:1221-1237; `am_utf32`,
/// jit/arm/instr_bs.cpp:1782-1796).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(utf_segment_code_point(&Term::Integer(0x1F600)), Some(0x1F600));
/// assert_eq!(utf_segment_code_point(&Term::Integer(-1)), None);
/// assert_eq!(utf_segment_code_point(&Term::Integer(0xD800)), None);
/// assert_eq!(utf_segment_code_point(&Term::static_atom("a")), None);
/// ```
fn utf_segment_code_point(value: &Term) -> Option<u32> {
    let Term::Integer(value) = value else {
        return None;
    };
    let code_point = u32::try_from(*value).ok()?;
    (!(0xD800..=0xDFFF).contains(&code_point) && code_point <= 0x0010_FFFF).then_some(code_point)
}

/// Run the construct pass of `bs_create_bin` over `parts` and return the
/// bytes and the number of bits that the parts write, or the failure of the first part in the
/// order of `integer_type_failure`. A `string` or `binary` part uses its
/// reductions. A `float` part writes the bits of `float_segment_bits`, as
/// `erts_bs_put_float` writes them (erl_bits.c:1454-1520).
///
/// # Examples
///
/// ```rust,ignore
/// let parts = [BinaryPart::Utf8('a')];
/// assert_eq!(write_binary_parts(&mut process, &parts)?, Ok((b"a".to_vec(), 8)));
/// ```
///
/// # Errors
///
/// A bignum `integer` part of more than 64 bits or a 16-bit `float` part is
/// a runtime error.
fn write_binary_parts(
    process: &mut ProcessState,
    parts: &[BinaryPart<'_>],
) -> Result<Result<(Vec<u8>, u64), BinarySegmentFailure>, BeamError> {
    let mut writer = BitWriter::default();
    for (index, part) in parts.iter().enumerate() {
        match part {
            BinaryPart::Bytes(bytes, reductions) => {
                writer.push_bytes(bytes);
                process.bump_reductions(i32::try_from(*reductions).unwrap_or(i32::MAX));
            }
            BinaryPart::Shared(..) if let Some(failure) = part.type_failure() => {
                return Ok(Err(failure));
            }
            BinaryPart::Shared(binary, reductions, ..) => {
                writer.push_binary(binary, binary.bit_size());
                process.bump_reductions(i32::try_from(*reductions).unwrap_or(i32::MAX));
            }
            BinaryPart::Sized {
                value: Term::Binary(binary),
                bits,
                ..
            } if binary.bit_size() as u64 >= *bits => {
                // `erts_bs_put_binary` uses one reduction for each 8192
                // copied bits (erl_bits.c:1306).
                writer.push_binary(binary, *bits as usize);
                let reductions = *bits / BITS_PER_REDUCTION as u64;
                process.bump_reductions(i32::try_from(reductions).unwrap_or(i32::MAX));
            }
            BinaryPart::Sized { .. } => {
                return Ok(Err(part
                    .type_failure()
                    .expect("a sized part that cannot be written has a failure")));
            }
            BinaryPart::Integer {
                value: Term::Integer(value),
                bits,
                little,
                ..
            } => writer.push_integer(*value, *bits, *little),
            // A segment keeps the low bits of the two's complement value
            // (`erts_new_bs_put_integer`, `erl_bits.c`), so a segment of at
            // most 64 bits takes the low 64 bits of a bignum.
            BinaryPart::Integer {
                value: Term::BigInt(value),
                bits,
                little,
                ..
            } if *bits <= 64 => writer.push_integer(low_64_bits(value), *bits, *little),
            BinaryPart::Integer {
                value: Term::BigInt(_),
                bits,
                ..
            } => {
                return Err(error(format!(
                    "unsupported bs_create_bin integer segment of {bits} bits of a bignum"
                )));
            }
            BinaryPart::Integer { .. } => return Ok(Err(integer_type_failure(parts, index))),
            BinaryPart::Utf8(code_point) => {
                writer.push_bytes(code_point.encode_utf8(&mut [0; 4]).as_bytes());
            }
            BinaryPart::Utf16 {
                segment,
                value,
                little,
            } => match utf_segment_code_point(value) {
                Some(code_point) if code_point < 0x1_0000 => {
                    writer.push_integer(i64::from(code_point), 16, *little);
                }
                Some(code_point) => {
                    let value = code_point - 0x1_0000;
                    writer.push_integer(i64::from(0xD800 | (value >> 10)), 16, *little);
                    writer.push_integer(i64::from(0xDC00 | (value & 0x3FF)), 16, *little);
                }
                None => {
                    return Ok(Err(BinarySegmentFailure::badarg(
                        *segment,
                        "utf16",
                        "type",
                        value.clone(),
                    )));
                }
            },
            BinaryPart::Utf32 { code_point, little } => {
                writer.push_integer(i64::from(*code_point), 32, *little);
            }
            BinaryPart::Float {
                segment,
                value,
                bits,
                little,
            } => match float_segment_bits(*segment, value, *bits) {
                Ok(Some(float)) => writer.push_integer(float, *bits, *little),
                Ok(None) => {
                    return Err(error("unsupported bs_create_bin float segment of 16 bits"));
                }
                Err(failure) => return Ok(Err(failure)),
            },
        }
    }
    Ok(Ok(writer.into_bytes()))
}

/// Return the part of a `binary` segment of a size, or the failure of its
/// size. The size pass checks the size as the size of an `integer` segment
/// (jit/arm/instr_bs.cpp:1639-1689). A literal size has no check
/// (jit/arm/instr_bs.cpp:1493-1505, 2005-2018). The function stays out of
/// line, so the code of the other segments keeps its size.
///
/// # Examples
///
/// ```rust,ignore
/// // <<A:2/binary>> with A in x0
/// let part = sized_binary_part(&mut process, 1, 8, &Operand::X(0), &Operand::Value(Term::Integer(2)))?;
/// assert!(matches!(part, Ok(BinaryPart::Sized { bits: 16, .. })));
/// ```
#[inline(never)]
fn sized_binary_part(
    process: &mut ProcessState,
    segment: u64,
    unit: u64,
    source: &Operand,
    size: &Operand,
) -> Result<Result<BinaryPart<'static>, BinarySegmentFailure>, BeamError> {
    let fixed =
        matches!(size, Operand::Value(Term::Integer(literal)) if (0..1 << 56).contains(literal));
    let size = read(process, size)?;
    Ok(match integer_segment_bits(fixed, &size, unit) {
        Ok(bits) => Ok(BinaryPart::Sized {
            segment,
            value: read(process, source)?,
            bits,
        }),
        Err(reason) => Err(BinarySegmentFailure {
            reason,
            ..BinarySegmentFailure::badarg(segment, "binary", "size", size)
        }),
    })
}

/// Return the bitstring of the `bits` bits at the start of `bytes`, the
/// output of a `BitWriter`. A size that is not a multiple of 8 bits gives a
/// view of the first `bits` bits, which copies no byte.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(bitstring(vec![0x10], 4).bit_size(), 4);
/// ```
fn bitstring(bytes: Vec<u8>, bits: u64) -> Binary {
    let binary = Binary::from(bytes);
    if bits.is_multiple_of(8) {
        binary
    } else {
        binary.bits(0, bits as usize)
    }
}

/// Run a `bs_create_bin` whose first segment is `private_append`, as ERTS
/// `erts_bs_private_append_checked` does (erl_bits.c:1928-2018; JIT
/// jit/arm/instr_bs.cpp:1865-1881). The segment has no check, and the
/// other segments write after the bytes of the writable binary in
/// `source`. ERTS changes the size of that binary term in place, so the
/// result is also the term of `source` (erl_bits.c:1978, 2017). PBeam
/// takes the binary out of `source`, so the binary has one owner when no
/// other term holds it, and writes the new bytes in its array
/// (`Binary::append_in_place`). Else it builds a new binary that shares
/// the old arrays (`Binary::concat`). PBeam charges no reduction. ERTS
/// calls `BUMP_REDS` when it grows the buffer (erl_bits.c:1974, 2010), but
/// the arm JIT drops that bump (jit/arm/instr_bs.cpp:1876-1880), so the
/// pinned ERTS on arm64 also charges none. The x86 JIT keeps the bump (row
/// "Binary construction" of `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// // <<Acc/binary, 3:8>> with Acc = <<1,2>> in x1
/// let parts = [BinaryPart::Integer { segment: 2, value: Term::Integer(3), bits: 8, little: false, fixed: true }];
/// assert_eq!(private_append(&mut process, &Operand::X(1), &parts)?, Ok(Term::binary(vec![1, 2, 3])));
/// ```
///
/// # Errors
///
/// A term in `source` that is not a binary is a runtime invariant error,
/// because the compiler gives this segment only a binary that
/// `bs_init_writable` made or that an earlier `private_append` gave. The
/// errors of `write_binary_parts` also apply.
fn private_append(
    process: &mut ProcessState,
    source: &Operand,
    parts: &[BinaryPart<'_>],
) -> Result<Result<Term, BinarySegmentFailure>, BeamError> {
    let (bytes, bits) = match write_binary_parts(process, parts)? {
        Ok(written) => written,
        Err(failure) => return Ok(Err(failure)),
    };
    let mut value = read(process, source)?;
    let Term::Binary(binary) = &mut value else {
        return Err(error(format!(
            "bs_create_bin private_append segment holds a term that is not a binary: {value:?}"
        )));
    };
    write(process, source, Term::Nil)?;
    if !binary.bit_size().is_multiple_of(8) {
        // The bits of the old bitstring do not end on a byte boundary, so
        // the writer copies them, and then the new bits.
        let mut writer = BitWriter::default();
        writer.push_binary(binary, binary.bit_size());
        writer.push_binary(&bitstring(bytes, bits), bits as usize);
        let (bytes, bits) = writer.into_bytes();
        *binary = bitstring(bytes, bits);
    } else {
        let size = binary.bit_size() + bits as usize;
        if !binary.append_in_place(&bytes) {
            *binary = Binary::concat(binary, &bytes);
        }
        if !bits.is_multiple_of(8) {
            *binary = binary.bits(0, size);
        }
    }
    write(process, source, value.clone())?;
    Ok(Ok(value))
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
/// This path gives no caller frames and no `file` or `line`, so the frame
/// is the only frame (row "Exception stack trace" of
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
    let cause = Term::tuple(vec![
        Term::Integer(failure.segment as i64),
        Term::static_atom(failure.operation),
        Term::static_atom(failure.info),
        failure.value,
    ]);
    // ponytail: each map form holds atom keys, so `from_pairs` cannot fail.
    let error_info = Map::from_pairs(
        vec![
            (Term::static_atom("cause"), cause),
            (
                Term::static_atom("function"),
                Term::static_atom("format_bs_fail"),
            ),
            (
                Term::static_atom("module"),
                Term::static_atom("erl_erts_errors"),
            ),
        ],
        &scheduler.atoms,
    )
    .expect("atom keys fit each map form");
    let location = Term::tuple(vec![Term::static_atom("error_info"), Term::Map(error_info)]);
    let frame = Term::tuple(vec![
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

/// Raise `error:function_clause` for the function `name/arity` of `module`
/// at its `func_info`, as ERTS `i_func_info` does: it sets the reason and
/// the current MFA (`emit_i_func_info_shared` in
/// `jit/arm/beam_asm_module.cpp:484-497`). The first frame of the stack
/// trace is `{Module, Function, Args, Location}`, where `Args` is the list
/// of the `arity` argument registers (`save_stacktrace`,
/// `beam_common.c:1011-1024`; `make_arglist`, `beam_common.c:1186-1195`).
/// The next frames are the frames of the callers, as in `erlang:error/2`:
/// the function has no frame of its own yet, so its caller is the next
/// frame. The stack trace keeps at most `backtrace_depth` frames, and one
/// frame for the depth 0 (`beam_common.c:884-885`). `Location` is the
/// location of the function entry, which `build_stacktrace` looks up for a
/// `function_clause` (`beam_common.c:1231-1233`). Return the outcome when
/// the process exits.
///
/// # Examples
///
/// ```rust,ignore
/// // x0 = x, x1 = 1 at the func_info of `pick/2`.
/// let outcome = function_clause(scheduler, "probe", "pick", 2, Term::nil())?;
/// // The stack trace starts with {Module, pick, [x, 1], []}.
/// ```
///
/// # Errors
///
/// An argument register that is not set or holds a match context, or a
/// caller frame with no `func_info` before its instruction, is a runtime
/// invariant error.
fn function_clause(
    scheduler: &mut Scheduler,
    module: &str,
    name: &str,
    arity: u32,
    location: Term,
) -> Result<Option<Outcome>, BeamError> {
    let arguments = call_arguments(&scheduler.current_process().registers, arity)?;
    let reason = Term::static_atom("function_clause");
    raise_at_entry(
        scheduler,
        (module, name, arguments, location),
        ExceptionClass::Error,
        reason,
    )
}

/// Raise an exception at the entry of the function `module:name` with
/// `arguments`: the stack trace starts with `{Module, Name, Arguments,
/// Location}` and holds at most `backtrace_depth` frames, with the caller frames
/// below it. A `function_clause` error and an exception of a NIF give this
/// stack trace, as ERTS `save_stacktrace` keeps the arguments of the
/// current function (`beam_common.c:880-935`).
///
/// # Examples
///
/// ```rust,ignore
/// // x0 = -1 at the entry of `crypto:strong_rand_bytes_nif/1`.
/// let entry = ("crypto", "strong_rand_bytes_nif", vec![Term::Integer(-1)], Term::nil());
/// let outcome = raise_at_entry(scheduler, entry, ExceptionClass::Error, badarg)?;
/// // The stack trace starts with {crypto, strong_rand_bytes_nif, [-1], []}.
/// ```
///
/// # Errors
///
/// A name that is not in the atom table, or a caller frame with no
/// `func_info` before its instruction, is a runtime invariant error.
fn raise_at_entry(
    scheduler: &mut Scheduler,
    (module, name, arguments, location): (&str, &str, Vec<Term>, Term),
    class: ExceptionClass,
    reason: Term,
) -> Result<Option<Outcome>, BeamError> {
    let atom = |name: &str| {
        scheduler
            .atoms
            .get(name)
            .map(Term::Atom)
            .ok_or_else(|| error(format!("invariant error: atom table does not hold {name}")))
    };
    let current = Term::tuple(vec![
        atom(module)?,
        atom(name)?,
        Term::list_from(arguments),
        location,
    ]);
    // The active frame is the frame of the function, at its entry or at a
    // jump to its `func_info`, so the caller frames start below it.
    let stacktrace = stacktrace_from(scheduler, current, 1)?;
    Ok(raise_with_stacktrace(
        scheduler.current_process_mut(),
        class,
        reason,
        stacktrace,
    ))
}

/// Raise the error `reason` of the instruction that the active frame runs,
/// as ERTS `save_stacktrace` saves the error of an instruction that is not
/// a BIF call: the first frame is `{Module, Function, Arity, Location}` of
/// the current function at that instruction, and the caller frames follow
/// (`beam_common.c:1011-1027`, `1229-1230`). The stack trace keeps at most
/// `backtrace_depth` frames, and one frame for the depth 0
/// (`beam_common.c:884-885`). The cursor must be committed, so that the
/// active frame names the instruction. Return the outcome when the process
/// exits.
///
/// # Examples
///
/// ```rust,ignore
/// cursor.commit(process);
/// let outcome = raise_in_function(scheduler, Term::static_atom("if_clause"))?;
/// // The stack trace starts with {Module, Function, Arity, [{file, F}, {line, L}]}.
/// ```
///
/// # Errors
///
/// A module that the code table does not hold, or an instruction with no
/// `func_info` before it, is a runtime invariant error.
fn raise_in_function(
    scheduler: &mut Scheduler,
    reason: Term,
) -> Result<Option<Outcome>, BeamError> {
    let mut frames = mfa_stacktrace(scheduler, 0)?;
    frames.truncate(scheduler.node().backtrace_depth().max(1));
    Ok(raise_with_stacktrace(
        scheduler.current_process_mut(),
        ExceptionClass::Error,
        reason,
        Term::list_from(frames),
    ))
}

/// Return the stack trace that starts with `first` and continues with the
/// caller frames of [`mfa_stacktrace`] without the `skip` newest frames.
/// It holds at most `backtrace_depth` frames, and one frame for the depth
/// 0, as `save_stacktrace` keeps the current function and at most
/// `MAX(erts_backtrace_depth - 1, 0)` code addresses
/// (`beam_common.c:884-885`).
///
/// # Examples
///
/// ```rust,ignore
/// let stacktrace = stacktrace_from(&mut scheduler, frame.clone(), 0)?;
/// // [Frame, {Module, Function, Arity, []} | Callers]
/// ```
///
/// # Errors
///
/// A module that the code table does not hold, or an instruction with no
/// `func_info` before it, is a runtime invariant error.
fn stacktrace_from(scheduler: &mut Scheduler, first: Term, skip: usize) -> Result<Term, BeamError> {
    let depth = scheduler.node().backtrace_depth().max(1);
    let mut frames = vec![first];
    frames.extend(mfa_stacktrace(scheduler, skip)?.into_iter().take(depth - 1));
    Ok(Term::list_from(frames))
}

/// Give the exception of the BIF `module:name` with `arguments` the stack
/// trace of ERTS, and return each other result unchanged. ERTS
/// `save_stacktrace` puts the frame `{Module, Name, Arguments, Location}`
/// of the BIF first, then the function that called the BIF, then the
/// continuation pointers of the stack (`beam_common.c:903-1005`,
/// `1051-1055`). The location is `[{error_info, #{module => Format}}]`
/// for a BIF of a module that has a format module, with the `cause` of an
/// [`BifResult::ErrorWithCause`] in the map (`beam_common.c:939-1002`),
/// and else `[]`. `skip` is 1 when a tail call of the BIF has removed the
/// frame of the calling function in ERTS, so that the return address is in
/// the caller of that function, and else 0. The error BIFs
/// `erlang:error/1,2,3`, `erlang:exit/1`, and `erlang:throw/1` give their
/// own stack trace (`beam_common.c:907-924`) as
/// [`BifResult::RaiseWithStacktrace`], so they keep it. A
/// [`BifResult::TrapBif`] keeps the call in `trap_origin` of the process,
/// so that an error after the trap gets the frame of the call.
///
/// # Examples
///
/// ```rust,ignore
/// let result = bif_error(&mut scheduler, "erlang", "atom_to_list", vec![Term::Integer(1)], BifResult::badarg(), 0)?;
/// // {erlang, atom_to_list, [1], [{error_info, #{module => erl_erts_errors}}]}
/// // is the first frame.
/// assert!(matches!(result, BifResult::RaiseWithStacktrace { .. }));
/// ```
///
/// # Errors
///
/// A full atom table, a module that the code table does not hold, or an
/// instruction with no `func_info` before it, is a runtime error.
fn bif_error(
    scheduler: &mut Scheduler,
    module: &str,
    name: &str,
    arguments: Vec<Term>,
    result: BifResult,
    skip: usize,
) -> Result<BifResult, BeamError> {
    let (class, reason, cause) = match result {
        BifResult::Raise { class, reason } => (class, reason, None),
        BifResult::ErrorWithCause { reason, cause } => (ExceptionClass::Error, reason, Some(cause)),
        result @ BifResult::TrapBif { .. } => {
            scheduler.current_process_mut().trap_origin = Some(BifCall {
                module: module.to_owned(),
                name: name.to_owned(),
                arguments,
                skip,
            });
            return Ok(result);
        }
        result => return Ok(result),
    };
    // The format modules of `save_stacktrace` (`beam_common.c:939-991`).
    let format = match module {
        "atomics" | "counters" | "erlang" | "erts_internal" | "persistent_term" => {
            Some("erl_erts_errors")
        }
        "code" | "os" => Some("erl_kernel_errors"),
        "binary" | "ets" | "lists" | "maps" | "math" | "re" | "unicode" => {
            Some("erl_stdlib_errors")
        }
        _ => None,
    };
    let location = match format {
        Some(format) => {
            let mut pairs = vec![(Term::static_atom("module"), Term::static_atom(format))];
            if let Some(cause) = cause {
                pairs.push((Term::static_atom("cause"), cause));
            }
            // ponytail: each map form holds atom keys, so `from_pairs` cannot fail.
            let map =
                Map::from_pairs(pairs, &scheduler.atoms).expect("atom keys fit each map form");
            let error_info = Term::tuple(vec![Term::static_atom("error_info"), Term::Map(map)]);
            Term::list_from(vec![error_info])
        }
        None => Term::nil(),
    };
    let frame = Term::tuple(vec![
        Term::Atom(scheduler.atoms.put(module)?),
        Term::Atom(scheduler.atoms.put(name)?),
        Term::list_from(arguments),
        location,
    ]);
    let stacktrace = stacktrace_from(scheduler, frame, skip)?;
    Ok(BifResult::RaiseWithStacktrace {
        class,
        reason,
        stacktrace,
    })
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
    let frame = current_frame(scheduler.current_process_mut());
    let (module_name, name, arity) = frame_function(module, frame)?;
    let atom = |name: &str| {
        scheduler
            .atoms
            .get(name)
            .map(Term::Atom)
            .ok_or_else(|| error(format!("invariant error: atom table does not hold {name}")))
    };
    Ok((atom(module_name)?, atom(name)?, arity))
}

/// Return the module, the function, and the arity of the function that holds
/// the next instruction of `frame`, from the `func_info` before that
/// instruction.
///
/// # Examples
///
/// ```rust,ignore
/// let (module, function, arity) = frame_function(module, &frame)?;
/// assert_eq!(function, "main");
/// ```
///
/// # Errors
///
/// An instruction with no `func_info` before it is a runtime invariant
/// error.
fn frame_function<'a>(
    module: &'a BeamModule,
    frame: &Frame,
) -> Result<(&'a str, &'a str, u32), BeamError> {
    let index = frame.next_instruction - 1;
    module.instructions[..index]
        .iter()
        .rev()
        .find_map(|instruction| match instruction {
            Instruction::FunctionInfo {
                module,
                name,
                arity,
            } => Some((module.as_str(), name.as_str(), *arity)),
            _ => None,
        })
        .ok_or_else(|| error("invariant error: the instruction has no func_info before it"))
}

/// Return the stack trace frames of the current process, from the active
/// frame to the first frame, without the `skip` newest frames, as ERTS
/// `build_stacktrace` walks the saved continuation pointers of an
/// exception (`beam_common.c:1205-1292`). Adjacent frames at the same
/// return position give one frame. Each frame is
/// `{Module, Function, Arity, Location}`, as `erts_build_mfa_item` gives a
/// frame without arguments (`utils.c:3430-3455`). `Location` is the
/// location of the instruction before the next instruction of the frame,
/// which is the call of a caller frame: ERTS looks up each continuation
/// pointer one byte before it (`beam_common.c:817-877`). While the first apply of a start at
/// `erlang:apply/3` runs, the first frame of the process is not in the
/// stack trace.
///
/// # Examples
///
/// ```rust,ignore
/// let frames = mfa_stacktrace(&mut scheduler, 0)?;
/// assert_eq!(frames.len(), scheduler.current_process().frames.len());
/// ```
///
/// # Errors
///
/// A module that the code table does not hold, or an instruction with no
/// `func_info` before it, is a runtime invariant error. A full atom table
/// gives the error of `AtomTable::put`.
fn mfa_stacktrace(scheduler: &mut Scheduler, skip: usize) -> Result<Vec<Term>, BeamError> {
    let mut frames: Vec<Frame> = scheduler
        .current_process()
        .frames
        .iter()
        .rev()
        .skip(skip)
        .cloned()
        .collect();
    // ERTS records one frame for each run of equal continuation pointers,
    // so a recursion gives one frame (`gather_stacktrace`,
    // `beam_common.c:817-834`).
    frames.dedup_by(|frame, previous| {
        frame.module_id == previous.module_id && frame.next_instruction == previous.next_instruction
    });
    let count = frames.len();
    let mut stacktrace = Vec::with_capacity(count);
    for (index, frame) in frames.into_iter().enumerate() {
        let module = Arc::clone(scheduler.code.module_by_id(frame.module_id)?);
        let (_, function, arity) = frame_function(&module, &frame)?;
        // The first apply of a start at `erlang:apply/3` is a body apply
        // with no frame in ERTS, so `apply_bif_error_adjustment` does not
        // keep the `erlang.beam` frame (`beam_common.c:1355-1366`).
        if index + 1 == count && scheduler.current_process().start_apply {
            continue;
        }
        stacktrace.push(Term::tuple(vec![
            Term::Atom(scheduler.atoms.put(&module.name)?),
            Term::Atom(scheduler.atoms.put(function)?),
            Term::Integer(arity.into()),
            module
                .lines
                .location(frame.next_instruction - 1, Term::nil()),
        ]));
    }
    Ok(stacktrace)
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
        Operand::X(index) => read_x_register(registers, *index),
        Operand::Y(index) => read_y_register(stack, *index),
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
        Operand::Value(_) => {
            return Err(error("invalid BEAM move destination"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/runtime/interpreter.rs"]
mod tests;

/// The micro-measurement of the instruction attribution of task
/// [#546](https://github.com/fahchen/pbeam/issues/546). It lives beside the
/// interpreter, because it reads the private operand, label, frame,
/// reduction, and BIF argument helpers.
#[cfg(test)]
#[path = "../../tests/runtime/interpreter/dispatch_attribution_costs.rs"]
mod dispatch_attribution_costs;

/// The measurement of the data structures and of the derivations that
/// `docs/data-structure-audit.md` reports. It lives beside the interpreter,
/// because it reads the private operand, label, and frame helpers.
#[cfg(test)]
#[path = "../../tests/runtime/interpreter/data_structure_costs.rs"]
mod data_structure_costs;

/// Call the live-byte sampler of a measured run with the bytes of the guest
/// containers of `scheduler`. Do nothing when the run has no sampler.
///
/// # Examples
///
/// ```rust,ignore
/// let mut peak = 0;
/// let mut sample = Some(&mut |live: usize| peak = live);
/// sample_live(&scheduler, &mut sample);
/// assert!(peak > 0);
/// ```
fn sample_live(scheduler: &Scheduler, sample: &mut Option<&mut dyn FnMut(usize)>) {
    if let Some(sample) = sample.as_deref_mut() {
        sample(scheduler.live_bytes());
    }
}
