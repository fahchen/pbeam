//! The policy process of a run: a trusted process that runs the code of a
//! policy module, a BEAM module in any BEAM language (section "Policy
//! module, policy process, and policy table" of
//! `docs/capability-security-migration.md`). ERTS has no policy process.
//!
//! The launcher makes the policy process before the root process. The
//! policy process has no parent and no link, so it is outside each
//! supervisor tree. Its first call is `initial/1` of the policy module,
//! which gives the capabilities of the root process. In boot mode, the
//! root process starts with the boot authority, and the call of
//! `initial/1` waits for `pbeam_cap:enter_runtime/2` of the root process
//! (see [`Scheduler::request_runtime`]). Then it runs one
//! `grant/3` call for each `pbeam_cap:request/2` call, in the order of the
//! requests, and sends the answer to the requester as a grant signal. A
//! call that does not give a grant form, that raises an exception, or that
//! uses more than [`POLICY_REDUCTIONS`] reductions is a denial. In a call,
//! the policy code can revoke the copies of a kind of a process with
//! `pbeam_cap:policy_revoke/2` (see [`Scheduler::policy_revoke`]). The process
//! keeps its state between the calls, for example its dictionary and its
//! messages. The launcher gives it the capabilities of `AllowAll`, so each
//! effect of the policy code is permitted, as for a process of a run with
//! the `AllowAll` policy.

use std::collections::VecDeque;

use super::Scheduler;
use crate::{
    error::{BeamError, error},
    runtime::{
        capability::{self, AllowAll, Capability, CapabilityKind, Denied, Policy, Source},
        process::{Frame, ProcessState, Start},
        signal::Signal,
    },
    term::{Atom, AtomTable, Map, ProcessId, Term, print_term},
};

/// The largest number of reductions of one call of the policy code. A
/// call that uses more is a denial at the end of the turn of the policy
/// process, also when the call waits or returns in that turn.
/// ERTS has no policy code and no such limit; PBeam takes the value of 25
/// turns of `CONTEXT_REDS`.
pub(in crate::runtime) const POLICY_REDUCTIONS: u64 = 100_000;

/// The policy process of a run and its calls.
#[derive(Debug)]
pub(in crate::runtime) struct PolicyProcess {
    /// The pid of the policy process.
    id: ProcessId,
    /// The call of the policy code that runs now, or `None` while the
    /// process waits for a request.
    call: Option<PolicyCall>,
    /// The requests that wait for their call, oldest first.
    requests: VecDeque<Request>,
    /// The reduction total of the process at the start of the call.
    start: u64,
    /// The result of `initial/1` until the launcher takes it: the
    /// capabilities of the root process, or the cause of a denial. The
    /// launcher makes the root process with them, or, in boot mode, sends
    /// them to the root process (see [`Scheduler::enter_runtime`]).
    initial: Option<Result<Vec<Capability>, String>>,
}

/// A call of the policy code.
#[derive(Debug, Clone, Copy)]
enum PolicyCall {
    /// `initial/1` for the root process.
    Initial,
    /// `grant/3` for the request of this process.
    Grant(ProcessId),
}

/// A request that waits for its call of the policy code: a
/// `pbeam_cap:request/2` call that waits for `grant/3`, or, in boot mode,
/// the `pbeam_cap:enter_runtime/2` call of the root process that waits for
/// `initial/1`.
#[derive(Debug)]
struct Request {
    /// The process that waits for the answer of `grant/3`, or `None` for
    /// `initial/1`, whose answer the launcher takes.
    requester: Option<ProcessId>,
    /// The arguments of the call: for `grant/3`, the kind, the request, and
    /// the requester map; for `initial/1`, the requester map.
    arguments: Vec<Term>,
}

/// How a call of the policy code ended.
#[derive(Debug)]
pub(in crate::runtime) enum PolicyEnd {
    /// The call returned this value.
    Returned(Term),
    /// The call raised an exception that it did not catch, with this exit
    /// reason.
    Raised(Term),
    /// The call used more than [`POLICY_REDUCTIONS`] reductions.
    Overrun,
}

impl Scheduler {
    /// Make the policy process, which calls `initial/1` of the policy
    /// module through `frame` with the requester map of the root process
    /// `Module:Function/Arity`. The launcher calls this before it makes the
    /// root process, and the root process keeps the pid `<0.0.0>`. In boot
    /// mode, `root` is `None`: the process waits for a request, and its
    /// first call is the `initial/1` call of `pbeam_cap:enter_runtime/2`
    /// (see [`Self::request_runtime`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.start_policy(frame, Some(("policy_probe", "main", 0)))?;
    /// ```
    ///
    /// # Errors
    ///
    /// The pid overflows, or the atom table is full.
    pub(in crate::runtime) fn start_policy(
        &mut self,
        frame: Frame,
        root: Option<(&str, &str, usize)>,
    ) -> Result<(), BeamError> {
        let arguments = match root {
            Some((module, function, arity)) => {
                let initial_call = (module.to_owned(), function.to_owned(), arity);
                vec![self.requester(super::ROOT, None, &initial_call)?]
            }
            None => Vec::new(),
        };
        let id = self.next_pid()?;
        let mut process = ProcessState::new(id, Start::Entry(frame), arguments);
        let issued = AllowAll.initial().into_iter();
        process.security = self
            .delegations
            .hold(id, issued.map(|capability| (Source::Launcher, capability)));
        self.insert(process);
        self.policy = Some(PolicyProcess {
            id,
            call: root.map(|_| PolicyCall::Initial),
            requests: VecDeque::new(),
            start: 0,
            initial: None,
        });
        Ok(())
    }

    /// Queue the `initial/1` call of the policy process for
    /// `pbeam_cap:enter_runtime(Module, Function)` of the current process,
    /// a boot process, with the requester map `#{pid => Root, parent =>
    /// none, initial_call => {Module, Function, 0}}`. The current process
    /// sets its answer to `None` and waits. When the call ends, the launcher
    /// takes its result (see [`Self::take_policy_initial`]): a denial stops
    /// the run, and the capabilities go to the root process with the enter
    /// runtime signal (see [`Self::enter_runtime`]). After the exit of the
    /// policy process, the request stays in the queue, and the root process
    /// waits until the run ends.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.request_runtime(&module, &function)?;
    /// ```
    ///
    /// # Errors
    ///
    /// A boot process that is not the root process gives a runtime error:
    /// the boot authority of another process is not implemented yet (row
    /// "Boot authority" of `docs/erts-differences.md`). The atom table is
    /// full.
    pub(in crate::runtime) fn request_runtime(
        &mut self,
        module: &Atom,
        function: &Atom,
    ) -> Result<(), BeamError> {
        if self.current_process().id != super::ROOT {
            return Err(error(
                "unsupported pbeam_cap:enter_runtime/2 in a boot process that is not the root process: see \"Boot authority\" in docs/erts-differences.md",
            ));
        }
        let initial_call = (
            self.atoms.name_of(module).to_owned(),
            self.atoms.name_of(function).to_owned(),
            0,
        );
        let requester = self.requester(super::ROOT, None, &initial_call)?;
        self.current_process_mut().policy_reply = None;
        let policy = self
            .policy
            .as_mut()
            .expect("a boot run has a policy process");
        policy.requests.push_back(Request {
            requester: None,
            arguments: vec![requester],
        });
        Ok(())
    }

    /// Send the capabilities that `initial/1` gave in boot mode to the root
    /// process, from the policy process, as the enter runtime signal. The
    /// root process replaces its boot authority with the copies when it
    /// handles the signal, and its `pbeam_cap:enter_runtime/2` call returns.
    /// A root process that exited drops the signal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.enter_runtime(capabilities);
    /// ```
    pub(in crate::runtime) fn enter_runtime(&mut self, capabilities: Vec<Capability>) {
        let id = self
            .policy
            .as_ref()
            .expect("a boot run has a policy process")
            .id;
        // A root process that exited drops the signal.
        let _ = self.send_signal(Some(id), super::ROOT, Signal::EnterRuntime { capabilities });
    }

    /// Return the result of `initial/1` once, after the call ended: the
    /// capabilities of the root process, or the cause of the denial.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.take_policy_initial().is_none());
    /// ```
    pub(in crate::runtime) fn take_policy_initial(
        &mut self,
    ) -> Option<Result<Vec<Capability>, String>> {
        self.policy.as_mut()?.initial.take()
    }

    /// Queue a `grant/3` call of the policy process for the current
    /// process, with the capability kind atom `kind` and the request term
    /// `request`. The current process sets its answer to `None` and waits
    /// for the grant signal. Return `Err(Denied)` when the run has no live
    /// policy process, or when the current process is the policy process,
    /// which cannot wait for its own answer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(scheduler.request_grant(kind, request)?, Ok(()));
    /// ```
    ///
    /// # Errors
    ///
    /// The atom table is full.
    pub(in crate::runtime) fn request_grant(
        &mut self,
        kind: Term,
        request: Term,
    ) -> Result<Result<(), Denied>, BeamError> {
        let caller = self.current_process();
        let (id, parent, initial_call) = (caller.id, caller.parent, caller.initial_call.clone());
        let live = self
            .policy
            .as_ref()
            .is_some_and(|policy| policy.id != id && self.live_process(policy.id).is_some());
        if !live {
            return Ok(Err(Denied));
        }
        let initial_call = self.code.initial_call_names(&self.atoms, &initial_call)?;
        let requester = self.requester(id, parent, &initial_call)?;
        self.current_process_mut().policy_reply = None;
        let policy = self.policy.as_mut().expect("the policy process lives");
        policy.requests.push_back(Request {
            requester: Some(id),
            arguments: vec![kind, request, requester],
        });
        Ok(Ok(()))
    }

    /// Revoke the copies of `kind` of the process `holder`, for
    /// `pbeam_cap:policy_revoke/2`, whatever their source: the launcher, the
    /// policy, or a parent. Only the policy process can revoke them. A copy
    /// that `holder` holds gets a revocation signal from the policy process,
    /// and `holder` sends the revocation on to the copies that it gave when
    /// it handles the signal. After the exit of `holder`, its released
    /// records send the revocation at once to the live copies below them
    /// (see `Delegations::revocation`). Return `Err(Denied)` when the
    /// current process is not the policy process. A holder with no copy of
    /// `kind` and no released record of it gives `Ok(())` with no effect, as
    /// does each call in a run that skips the checks.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The current process is the policy process.
    /// assert_eq!(scheduler.policy_revoke(worker, CapabilityKind::Env), Ok(()));
    /// ```
    pub(in crate::runtime) fn policy_revoke(
        &mut self,
        holder: ProcessId,
        kind: CapabilityKind,
    ) -> Result<(), Denied> {
        if self.skip_checks {
            return Ok(());
        }
        let Some(policy) = self.policy_current() else {
            return Err(Denied);
        };
        let id = policy.id;
        for copy in self.delegations.held(holder, kind) {
            let signals = self.delegations.revocation(copy);
            self.send_revocations(Some(id), signals);
        }
        Ok(())
    }

    /// Start the call of the oldest request when the policy process waits
    /// for a request, and return the name of the function, `grant` or
    /// `initial`, and the arguments of the call. The launcher then gives the
    /// frame of the function to [`Self::begin_policy_call`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// if let Some((function, arguments)) = scheduler.next_policy_request() {
    ///     scheduler.begin_policy_call(frame, arguments);
    /// }
    /// ```
    pub(in crate::runtime) fn next_policy_request(&mut self) -> Option<(&'static str, Vec<Term>)> {
        let policy = self
            .policy
            .as_mut()
            .filter(|policy| policy.call.is_none())?;
        let request = policy.requests.pop_front()?;
        policy.call = Some(
            request
                .requester
                .map_or(PolicyCall::Initial, PolicyCall::Grant),
        );
        let function = if request.requester.is_some() {
            "grant"
        } else {
            "initial"
        };
        Some((function, request.arguments))
    }

    /// Make the policy process run `frame` with `arguments`, and give it
    /// Erlang work.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.begin_policy_call(frame, arguments);
    /// ```
    pub(in crate::runtime) fn begin_policy_call(&mut self, frame: Frame, arguments: Vec<Term>) {
        let policy = self.policy.as_mut().expect("the run has a policy process");
        let id = policy.id;
        let process = self.processes.get_mut(&id).expect("the process is alive");
        process.frames = vec![frame];
        process.registers = arguments.into_iter().map(Some).collect();
        policy.start = process.reductions;
        self.activate(id);
    }

    /// Return whether the current process is the policy process and waits
    /// for a request: a message woke it, but it has no call to run.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!scheduler.policy_idle());
    /// ```
    pub(in crate::runtime) fn policy_idle(&self) -> bool {
        self.policy_current()
            .is_some_and(|policy| policy.call.is_none())
    }

    /// Return whether the current process is the policy process and runs a
    /// call of the policy code.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.policy_running());
    /// ```
    pub(in crate::runtime) fn policy_running(&self) -> bool {
        self.policy_current()
            .is_some_and(|policy| policy.call.is_some())
    }

    /// Return whether the call of the policy code that runs now used more
    /// than [`POLICY_REDUCTIONS`] reductions, with the current turn.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!scheduler.policy_overrun());
    /// ```
    pub(in crate::runtime) fn policy_overrun(&mut self) -> bool {
        let start = self.policy.as_ref().expect("the policy process runs").start;
        let used = self.current_process().reductions + self.current_reductions() - start;
        used > POLICY_REDUCTIONS
    }

    /// End the call of the policy code that runs now in the current process,
    /// the policy process, as `end` gives. The process drops the frames and
    /// the registers of the call, its receive timer, and the receive
    /// position, and keeps the rest of its state. For `initial/1`, a list of
    /// capability maps gives the capabilities of the root process, and each
    /// other end gives the cause of a denial. For `grant/3`, the requester
    /// gets a grant signal: with the capability of a `{grant, Capability}`
    /// value, and with no capability for each other end. A requester that
    /// exited gets no signal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.end_policy_call(PolicyEnd::Returned(Term::static_atom("deny")));
    /// ```
    pub(in crate::runtime) fn end_policy_call(&mut self, end: PolicyEnd) {
        self.cancel_receive_timer();
        let process = self.current_process_mut();
        let id = process.id;
        process.frames.clear();
        process.registers.clear();
        process.pending_bif = None;
        process.length_trap = None;
        process.mailbox.set_save_first();
        let policy = self.policy.as_mut().expect("the policy process runs");
        match policy.call.take().expect("a call runs") {
            PolicyCall::Initial => {
                policy.initial = Some(match end {
                    PolicyEnd::Returned(value) => initial_capabilities(&self.atoms, &value)
                        .ok_or_else(|| format!("initial/1 returned {}", text(&value))),
                    PolicyEnd::Raised(reason) => {
                        Err(format!("initial/1 exited with {}", text(&reason)))
                    }
                    PolicyEnd::Overrun => Err(format!(
                        "initial/1 used more than {POLICY_REDUCTIONS} reductions"
                    )),
                });
            }
            PolicyCall::Grant(requester) => {
                let capability = match end {
                    PolicyEnd::Returned(value) => granted(&self.atoms, &value),
                    PolicyEnd::Raised(_) | PolicyEnd::Overrun => None,
                };
                // A requester that exited drops the signal.
                let _ = self.send_signal(Some(id), requester, Signal::Grant { capability });
            }
        }
    }

    /// Handle the exit of the process `id` with `reason` when it is the
    /// policy process: a call of `initial/1` gives the cause of a denial,
    /// and the requester of the call that runs and each request that waits
    /// get a denial. After the exit, each new request is denied. The
    /// launcher does not start the policy process again.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// scheduler.policy_exited(id, &Term::static_atom("killed"));
    /// ```
    pub(super) fn policy_exited(&mut self, id: ProcessId, reason: &Term) {
        let Some(policy) = self.policy.as_mut().filter(|policy| policy.id == id) else {
            return;
        };
        // A request of `initial/1` gets no answer, so its root process stays
        // a boot process and waits until the run ends.
        let mut denied: Vec<_> = policy
            .requests
            .drain(..)
            .filter_map(|r| r.requester)
            .collect();
        match policy.call.take() {
            Some(PolicyCall::Initial) => {
                let cause = format!("the policy process exited with {}", text(reason));
                policy.initial = Some(Err(cause));
            }
            Some(PolicyCall::Grant(requester)) => denied.insert(0, requester),
            None => {}
        }
        for requester in denied {
            // A requester that exited drops the signal.
            let _ = self.send_signal(Some(id), requester, Signal::Grant { capability: None });
        }
    }

    /// Return the policy process state when the current process is the
    /// policy process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(scheduler.policy_current().is_some());
    /// ```
    fn policy_current(&self) -> Option<&PolicyProcess> {
        let current = self.current_process?;
        self.policy.as_ref().filter(|policy| policy.id == current)
    }

    /// Return the requester map of the process `pid` for the policy code:
    /// `#{pid => Pid, parent => Parent | none, initial_call => {M, F, A}}`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let map = scheduler.requester(ProcessId(0), None, &(String::from("m"), String::from("main"), 0))?;
    /// ```
    ///
    /// # Errors
    ///
    /// The atom table is full.
    fn requester(
        &mut self,
        pid: ProcessId,
        parent: Option<ProcessId>,
        initial_call: &(String, String, usize),
    ) -> Result<Term, BeamError> {
        let (module, function, arity) = initial_call;
        let initial_call = Term::tuple(vec![
            Term::Atom(self.atoms.put(module)?),
            Term::Atom(self.atoms.put(function)?),
            Term::Integer(*arity as i64),
        ]);
        let parent = parent.map_or_else(|| Term::static_atom("none"), Term::Pid);
        let pairs = vec![
            (Term::static_atom("pid"), Term::Pid(pid)),
            (Term::static_atom("parent"), parent),
            (Term::static_atom("initial_call"), initial_call),
        ];
        Ok(Term::Map(
            Map::from_pairs(pairs).expect("a map takes atom keys"),
        ))
    }
}

/// Read the value of `initial/1`: a proper list of capability maps (see
/// `capability::capability`). Return `None` for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(initial_capabilities(&Term::nil()), Some(vec![]));
/// assert_eq!(initial_capabilities(&Term::static_atom("all")), None);
/// ```
fn initial_capabilities(atoms: &AtomTable, value: &Term) -> Option<Vec<Capability>> {
    let capabilities = value.proper_list()?;
    capabilities
        .map(|capability| capability::capability(atoms, capability))
        .collect()
}

/// Read the value of `grant/3`: the capability of `{grant, Capability}`,
/// or `None` for `deny` and for each term that is not a grant form.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(granted(&Term::static_atom("deny")), None);
/// ```
fn granted(atoms: &AtomTable, value: &Term) -> Option<Capability> {
    let Term::Tuple(elements) = value else {
        return None;
    };
    match elements.as_ref() {
        [Term::Atom(grant), capability] if grant.name() == "grant" => {
            capability::capability(atoms, capability)
        }
        _ => None,
    }
}

/// Return the ERTS text of `term`, as `erlang:halt/1` prints it, for the
/// cause of a denial.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::static_atom("deny")), "deny");
/// ```
fn text(term: &Term) -> String {
    print_term(term).into_iter().map(char::from).collect()
}
