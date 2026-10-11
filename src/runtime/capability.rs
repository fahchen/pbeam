//! Capabilities, the security context of a process, and the policy that
//! issues the initial capabilities of a run (transition 2 of
//! `docs/capability-security-migration.md`).
//!
//! A capability is a runtime value, not a term: BEAM code cannot make one.
//! Each process holds its own copies in its [`SecurityContext`]. A check is
//! a mechanical test of these copies: it sends no message and runs no
//! policy code. A denied operation raises `error:capability_denied`, which
//! ERTS does not have (row "Capability checks" of `docs/erts-differences.md`).
//!
//! Each copy has an identifier that the run never gives again. The run-wide
//! [`Delegations`] tree records the source of each copy and the copies that
//! went from it to children. A revocation follows these records to the
//! downstream copies, also through a holder that exited.

use std::collections::{BTreeSet, HashMap, VecDeque};

use super::{host::filename_encoding, unicode::native_filename};
use crate::{
    error::{BeamError, error},
    term::{ProcessId, Term},
};

/// The resource kind that a capability covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityKind {
    /// Processes: spawn, links, monitors, exit signals to other
    /// processes, messages, the inspection of other processes, and the
    /// group leader of other processes.
    Process,
    /// The terminal: writes, reads, raw mode, the window size, the setup,
    /// and the queries.
    Terminal,
    /// Ports: the start of an external program, its commands, its close,
    /// and its information.
    Port,
    /// The OS environment variables.
    Env,
    /// The host signal dispositions.
    Signal,
    /// The whole runtime: the stop of the run, the system flags, and the
    /// load of a NIF library.
    Runtime,
    /// The global persistent term store of the node.
    PersistentTerm,
}

/// An operation that a capability can permit. Each kind permits only its
/// own operations (see [`CapabilityKind::operations`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// `process`: make a process. `port`: start an external program.
    Spawn,
    /// `process`: link to a process.
    Link,
    /// `process`: monitor a process.
    Monitor,
    /// `process`: send an exit signal to another process.
    ExitOther,
    /// `process`: send a message.
    Send,
    /// `terminal`: write output. `env`: set a variable. `persistent_term`:
    /// store a term.
    Write,
    /// `terminal`: read input. `env`: read a variable. `persistent_term`:
    /// read the stored terms.
    Read,
    /// `terminal`: set raw mode.
    RawMode,
    /// `terminal`: read the window size.
    WindowSize,
    /// `terminal`: set up the terminal: read the terminfo entry, or set the
    /// locale of the OS process.
    Setup,
    /// `terminal`: ask if a standard fd is a terminal, make a TTY resource,
    /// or read a capability of the terminfo entry.
    Query,
    /// `port`: send a command to a port.
    Command,
    /// `port`: close a port.
    Close,
    /// `process`: read the state of another process. `port`: read the
    /// information of a port.
    Inspect,
    /// `process`: set the group leader of another process.
    GroupLeader,
    /// `signal`: handle a host signal.
    Handle,
    /// `signal`: ignore a host signal.
    Ignore,
    /// `runtime`: stop the run.
    Halt,
    /// `runtime`: set a system flag.
    SystemFlag,
    /// `runtime`: load a NIF library.
    LoadNif,
}

/// The value form of the names in a constraint field, in the term of the
/// `spawn_opt` option `{capabilities, Map}`.
#[derive(Debug, Clone, Copy)]
enum NameForm {
    /// Atoms, as module names and signal names are.
    Atom,
    /// Flat lists of characters, as file names and variable names are.
    String,
}

/// The fixed data of one capability kind.
struct KindSpec {
    /// The kind.
    kind: CapabilityKind,
    /// The atom of the kind in the `{capabilities, Map}` option.
    name: &'static str,
    /// The operations of the kind.
    operations: &'static [Operation],
    /// The constraint field of the kind and the form of its names, or
    /// `None` when the kind has no constraint field.
    constraint: Option<(&'static str, NameForm)>,
}

/// The kinds of the design table (section "Capability kinds" of
/// `docs/capability-security-migration.md`).
const KINDS: [KindSpec; 7] = [
    KindSpec {
        kind: CapabilityKind::Process,
        name: "process",
        operations: &[
            Operation::Spawn,
            Operation::Link,
            Operation::Monitor,
            Operation::ExitOther,
            Operation::Send,
            Operation::Inspect,
            Operation::GroupLeader,
        ],
        constraint: Some(("modules", NameForm::Atom)),
    },
    KindSpec {
        kind: CapabilityKind::Terminal,
        name: "terminal",
        operations: &[
            Operation::Write,
            Operation::Read,
            Operation::RawMode,
            Operation::WindowSize,
            Operation::Setup,
            Operation::Query,
        ],
        constraint: None,
    },
    KindSpec {
        kind: CapabilityKind::Port,
        name: "port",
        operations: &[
            Operation::Spawn,
            Operation::Command,
            Operation::Close,
            Operation::Inspect,
        ],
        constraint: Some(("executables", NameForm::String)),
    },
    KindSpec {
        kind: CapabilityKind::Env,
        name: "env",
        operations: &[Operation::Read, Operation::Write],
        constraint: Some(("names", NameForm::String)),
    },
    KindSpec {
        kind: CapabilityKind::Signal,
        name: "signal",
        operations: &[Operation::Handle, Operation::Ignore],
        constraint: Some(("signals", NameForm::Atom)),
    },
    KindSpec {
        kind: CapabilityKind::Runtime,
        name: "runtime",
        operations: &[Operation::Halt, Operation::SystemFlag, Operation::LoadNif],
        constraint: None,
    },
    KindSpec {
        kind: CapabilityKind::PersistentTerm,
        name: "persistent_term",
        operations: &[Operation::Read, Operation::Write],
        constraint: None,
    },
];

/// The atom of each operation in the `{capabilities, Map}` option.
const OPERATION_NAMES: [(Operation, &str); 20] = [
    (Operation::Spawn, "spawn"),
    (Operation::Link, "link"),
    (Operation::Monitor, "monitor"),
    (Operation::ExitOther, "exit_other"),
    (Operation::Send, "send"),
    (Operation::Write, "write"),
    (Operation::Read, "read"),
    (Operation::RawMode, "raw_mode"),
    (Operation::WindowSize, "window_size"),
    (Operation::Setup, "setup"),
    (Operation::Query, "query"),
    (Operation::Command, "command"),
    (Operation::Close, "close"),
    (Operation::Inspect, "inspect"),
    (Operation::GroupLeader, "group_leader"),
    (Operation::Handle, "handle"),
    (Operation::Ignore, "ignore"),
    (Operation::Halt, "halt"),
    (Operation::SystemFlag, "system_flag"),
    (Operation::LoadNif, "load_nif"),
];

impl CapabilityKind {
    /// Return the fixed data of the kind.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(CapabilityKind::Env.spec().name, "env");
    /// ```
    fn spec(self) -> &'static KindSpec {
        KINDS
            .iter()
            .find(|spec| spec.kind == self)
            .expect("KINDS has each kind")
    }

    /// Return the operations that a capability of this kind can permit.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(CapabilityKind::Env.operations(), [Operation::Read, Operation::Write]);
    /// ```
    pub fn operations(self) -> &'static [Operation] {
        self.spec().operations
    }

    /// Return the kind whose atom is `name`, as the `{capabilities, Map}`
    /// option, `pbeam_cap:revoke/2`, and `pbeam_cap:policy_revoke/2` name
    /// it, or `None` for another name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(CapabilityKind::named("env"), Some(CapabilityKind::Env));
    /// assert_eq!(CapabilityKind::named("file"), None);
    /// ```
    pub(super) fn named(name: &str) -> Option<Self> {
        KINDS
            .iter()
            .find(|spec| spec.name == name)
            .map(|spec| spec.kind)
    }
}

impl Operation {
    /// Return the bit of the operation in an operation set.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Operation::Spawn.bit(), 1);
    /// assert_eq!(Operation::Link.bit(), 2);
    /// ```
    fn bit(self) -> u32 {
        1 << self as u32
    }

    /// Return the operation set of `operations`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Operation::set(&[Operation::Spawn, Operation::Link]), 3);
    /// ```
    fn set(operations: &[Operation]) -> u32 {
        operations
            .iter()
            .fold(0, |set, operation| set | operation.bit())
    }
}

/// The constraint field of a capability: the names that the capability
/// permits, for example the modules that a child can start in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constraint {
    /// Each name.
    Any,
    /// Only these names. A module or a signal name is the atom text. An
    /// executable or a variable name is the text of its characters.
    Only(BTreeSet<String>),
}

impl Constraint {
    /// Return whether the constraint permits `name`, a name in `form`: the
    /// bytes of an atom name for [`NameForm::Atom`], or the native bytes of a
    /// file name or a variable name for [`NameForm::String`]. A permitted
    /// string is compared as its native bytes in the file name encoding of
    /// the host, as ERTS gives the name to the host (see
    /// [`native_filename`]). A permitted string that the encoding cannot
    /// hold permits no name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Constraint::Any.permits(b"lists", NameForm::Atom));
    /// let home = Constraint::Only(BTreeSet::from([String::from("HOME")]));
    /// assert!(home.permits(b"HOME", NameForm::String));
    /// assert!(!home.permits(b"PATH", NameForm::String));
    /// ```
    fn permits(&self, name: &[u8], form: NameForm) -> bool {
        let Constraint::Only(names) = self else {
            return true;
        };
        match form {
            NameForm::Atom => names.iter().any(|permitted| permitted.as_bytes() == name),
            NameForm::String => names.iter().any(|permitted| {
                let characters = permitted
                    .chars()
                    .map(|character| Term::Integer(u32::from(character).into()));
                native_filename(&Term::list_from(characters), filename_encoding())
                    .is_some_and(|permitted| permitted == name)
            }),
        }
    }

    /// Return whether this constraint permits no name that `other` does
    /// not permit.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let none = Constraint::Only(BTreeSet::new());
    /// assert!(none.within(&Constraint::Any));
    /// assert!(!Constraint::Any.within(&none));
    /// ```
    fn within(&self, other: &Constraint) -> bool {
        match (self, other) {
            (_, Constraint::Any) => true,
            (Constraint::Any, Constraint::Only(_)) => false,
            (Constraint::Only(names), Constraint::Only(permitted)) => names.is_subset(permitted),
        }
    }
}

/// A capability: a kind, the operations that it permits, the constraint of
/// its kind, and whether its holder can give it to a child. It is not a
/// term, and no BIF makes one from a term. Only the launcher issues
/// capabilities, through [`Policy::initial`], and a parent gives copies to
/// its child at spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    /// The resource kind.
    kind: CapabilityKind,
    /// The permitted operations, one [`Operation::bit`] each.
    operations: u32,
    /// The constraint field of the kind. A kind without a constraint field
    /// has `Constraint::Any`.
    constraint: Constraint,
    /// True when the holder gives a copy to each child that it spawns.
    delegable: bool,
}

impl Capability {
    /// Make a capability of `kind` that permits `operations` within
    /// `constraint`. With `delegable`, the holder gives a copy to each child
    /// that it spawns; else the holder can only use it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let modules = Constraint::Only(BTreeSet::from([String::from("worker")]));
    /// assert!(Capability::new(CapabilityKind::Process, &[Operation::Spawn], modules, true).is_ok());
    /// assert!(Capability::new(CapabilityKind::Env, &[Operation::Spawn], Constraint::Any, false).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// An operation of another kind, and a `Constraint::Only` for the
    /// `terminal`, `runtime`, or `persistent_term` kind, which has no
    /// constraint field.
    pub fn new(
        kind: CapabilityKind,
        operations: &[Operation],
        constraint: Constraint,
        delegable: bool,
    ) -> Result<Self, BeamError> {
        let spec = kind.spec();
        if let Some(operation) = operations
            .iter()
            .find(|operation| !spec.operations.contains(operation))
        {
            return Err(error(format!(
                "{operation:?} is not an operation of the {} capability",
                spec.name
            )));
        }
        if spec.constraint.is_none() && constraint != Constraint::Any {
            return Err(error(format!(
                "the {} capability has no constraint field",
                spec.name
            )));
        }
        Ok(Self {
            kind,
            operations: Operation::set(operations),
            constraint,
            delegable,
        })
    }

    /// Return whether the capability permits `operation` of `kind`, and,
    /// when `name` is given, permits `name` in its constraint (see
    /// [`Constraint::permits`]). A kind without a constraint field
    /// permits each name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let spawn = Capability::new(CapabilityKind::Process, &[Operation::Spawn], Constraint::Any, false)?;
    /// assert!(spawn.permits(CapabilityKind::Process, Operation::Spawn, Some(b"lists")));
    /// assert!(!spawn.permits(CapabilityKind::Process, Operation::Link, None));
    /// ```
    fn permits(&self, kind: CapabilityKind, operation: Operation, name: Option<&[u8]>) -> bool {
        self.kind == kind
            && self.operations & operation.bit() != 0
            && name.is_none_or(|name| {
                // A kind without a constraint field has `Constraint::Any`,
                // which permits each name in each form.
                let form = kind
                    .spec()
                    .constraint
                    .map_or(NameForm::Atom, |(_, form)| form);
                self.constraint.permits(name, form)
            })
    }
}

/// The source of the initial capabilities of a run. The launcher calls the
/// policy when it makes the root process. A Rust host gives a policy to
/// [`crate::BeamProgram::execute_with_policy`]. A run with no policy,
/// [`crate::BeamProgram::execute`], skips each check.
pub trait Policy {
    /// Return the capabilities of the root process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(AllowAll.initial().len(), 7);
    /// ```
    fn initial(&self) -> Vec<Capability>;

    /// Return the kinds of the initial capabilities that the launcher
    /// revokes now. The launcher calls this function before each scheduling
    /// step. For each kind, the root process gets a revocation signal for
    /// each of its copies of that kind that the launcher issued, and the
    /// revocation then goes to each downstream copy. While each process
    /// waits, the run calls this function again only after a timer, a port,
    /// a terminal, a dirty IO call, or a host signal wakes the scheduler.
    /// With none of them, the run ends. Thus a revocation that a host thread
    /// gives while each process waits takes effect only at the next of
    /// these events. The default revokes nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(AllowAll.revocations().is_empty());
    /// ```
    fn revocations(&self) -> Vec<CapabilityKind> {
        Vec::new()
    }
}

/// The policy that permits each operation, for the tests of the checks. The
/// root process gets a delegable capability of each kind with all
/// operations and no constraint, so each check passes, and a run gives the
/// same results as the pinned ERTS. Unlike a run with no policy, each check
/// runs, and a child gets copies.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAll;

impl Policy for AllowAll {
    /// Return a delegable capability of each kind that permits each
    /// operation of the kind and each name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let capabilities = AllowAll.initial();
    /// assert_eq!(capabilities.len(), 7);
    /// let process = Capability::new(CapabilityKind::Process, CapabilityKind::Process.operations(), Constraint::Any, true)?;
    /// assert_eq!(capabilities[0], process);
    /// ```
    fn initial(&self) -> Vec<Capability> {
        KINDS
            .iter()
            .map(|spec| Capability {
                kind: spec.kind,
                operations: Operation::set(spec.operations),
                constraint: Constraint::Any,
                delegable: true,
            })
            .collect()
    }
}

/// A denied check or a denied delegation. The caller raises
/// `error:capability_denied` before the operation has an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Denied;

/// The identifier of one capability copy. The run never gives an
/// identifier again, so a delegation record cannot name a later copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct CapabilityId(u64);

/// The source of a capability copy: the field `source` of the design record
/// (section "Capability kinds" of `docs/capability-security-migration.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    /// The launcher issued the copy to the root process or to the policy
    /// process.
    Launcher,
    /// The policy process granted the copy at run time, with `grant/3` of
    /// the policy module.
    Policy,
    /// A parent gave the copy to its child at spawn, from its own copy with
    /// this identifier.
    Copy(CapabilityId),
}

/// One copy of a capability in a security context.
#[derive(Debug)]
struct Held {
    /// The identifier of the copy.
    id: CapabilityId,
    /// The capability.
    capability: Capability,
}

/// The security context of a process: its copies of its capabilities, or
/// the boot authority. The process owns it. When the process leaves the
/// process table, the [`Delegations`] tree releases the copies. No check
/// finds a capability by a pid.
#[derive(Debug, Default)]
pub(super) struct SecurityContext {
    /// The capabilities that the process can use. The delegable ones also
    /// go to each child. A boot context has none.
    capabilities: Vec<Held>,
    /// True for a boot process: a process of the trusted boot, as the
    /// launcher, not under the policy (row "Boot authority" of
    /// `docs/erts-differences.md`). Each check of a boot
    /// process passes without copies, and a child that it spawns without
    /// `{capabilities, Map}` is a boot process. Only the launcher makes a
    /// boot root process, and only `pbeam_cap:enter_runtime/2` of the root
    /// process clears the flag, one way.
    boot: bool,
    /// True for a boot process that `pbeam_cap:serve(Pid, io)` registered
    /// as an IO service: the runtime to boot gate permits a message from a
    /// runtime process to it when the message is an IO request that the
    /// sender may make (see `io_service::send_denial` of the BIFs). Only a boot
    /// context has the flag, and a child does not get it.
    io_service: bool,
}

impl SecurityContext {
    /// Return the context of a boot process: no copies, and each check
    /// passes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let boot = SecurityContext::boot();
    /// assert_eq!(boot.check(CapabilityKind::Runtime, Operation::Halt, None), Ok(()));
    /// ```
    pub(super) fn boot() -> Self {
        Self {
            capabilities: Vec::new(),
            boot: true,
            io_service: false,
        }
    }

    /// Return whether the context is the context of a boot process (see
    /// [`Self::boot`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(SecurityContext::boot().is_boot());
    /// assert!(!SecurityContext::default().is_boot());
    /// ```
    pub(super) fn is_boot(&self) -> bool {
        self.boot
    }

    /// Register the boot process of this context as an IO service (see
    /// `pbeam_cap:serve/2`). Return `false` and change nothing when the
    /// context is not a boot context.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut boot = SecurityContext::boot();
    /// assert!(boot.serve_io());
    /// assert!(boot.serves_io());
    /// assert!(!SecurityContext::default().serve_io());
    /// ```
    pub(super) fn serve_io(&mut self) -> bool {
        self.io_service = self.boot;
        self.boot
    }

    /// Return whether the context is the context of an IO service (see
    /// [`Self::serve_io`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!SecurityContext::boot().serves_io());
    /// ```
    pub(super) fn serves_io(&self) -> bool {
        self.io_service
    }

    /// Check `operation` of `kind`: return `Err(Denied)` unless the context
    /// is a boot context or a capability of the context permits it, and,
    /// when `name` is given, permits `name` in its constraint. `name` is the
    /// bytes of a module or signal atom, or the native bytes of an
    /// executable or variable name.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let context = delegations.hold(ProcessId(0), AllowAll.initial().into_iter().map(|c| (Source::Launcher, c)));
    /// assert_eq!(context.check(CapabilityKind::Process, Operation::Spawn, Some(b"lists")), Ok(()));
    /// assert_eq!(SecurityContext::default().check(CapabilityKind::Process, Operation::Link, None), Err(Denied));
    /// ```
    pub(super) fn check(
        &self,
        kind: CapabilityKind,
        operation: Operation,
        name: Option<&[u8]>,
    ) -> Result<(), Denied> {
        if self.boot
            || self
                .capabilities
                .iter()
                .any(|held| held.capability.permits(kind, operation, name))
        {
            Ok(())
        } else {
            Err(Denied)
        }
    }

    /// Return the copies for a new child, each with its source. With no
    /// `restrictions`, the child gets a copy of each delegable capability.
    /// With `restrictions`, it gets a reduced copy of each delegable
    /// capability of a requested kind, and no other capability. A requested
    /// kind without a delegable capability, and a restriction that permits
    /// more than its capability, give `Err(Denied)`. [`Delegations::hold`]
    /// makes the context of the child from the copies. A boot context has
    /// no copies, so it gives no copies, and its child is a boot process
    /// (see `Scheduler::spawn`). It gives `Err(Denied)` for each
    /// `restrictions`: a boot spawn with `{capabilities, Map}` has no
    /// consumer yet (entry "Spawn a runtime process from a boot process" of
    /// `docs/backlog.md`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(parent.delegate(None)?.len(), 7);
    /// assert_eq!(SecurityContext::default().delegate(Some(&restrictions)), Err(Denied));
    /// assert_eq!(SecurityContext::boot().delegate(Some(&restrictions)), Err(Denied));
    /// ```
    pub(super) fn delegate(
        &self,
        restrictions: Option<&[Restriction]>,
    ) -> Result<Vec<(Source, Capability)>, Denied> {
        if self.boot && restrictions.is_some() {
            return Err(Denied);
        }
        let delegable = || {
            self.capabilities
                .iter()
                .filter(|held| held.capability.delegable)
        };
        let Some(restrictions) = restrictions else {
            return Ok(delegable()
                .map(|held| (Source::Copy(held.id), held.capability.clone()))
                .collect());
        };
        if restrictions
            .iter()
            .any(|restriction| !delegable().any(|held| held.capability.kind == restriction.kind))
        {
            return Err(Denied);
        }
        let mut copies = Vec::new();
        for held in delegable() {
            if let Some(restriction) = restrictions
                .iter()
                .find(|restriction| restriction.kind == held.capability.kind)
            {
                copies.push((Source::Copy(held.id), restriction.apply(&held.capability)?));
            }
        }
        Ok(copies)
    }

    /// Return the identifiers of the copies of `kind` in the context.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(root.ids(CapabilityKind::Env).len(), 1);
    /// ```
    pub(super) fn ids(&self, kind: CapabilityKind) -> Vec<CapabilityId> {
        self.capabilities
            .iter()
            .filter(|held| held.capability.kind == kind)
            .map(|held| held.id)
            .collect()
    }

    /// Add the copies of `granted`, a context that [`Delegations::hold`]
    /// made for the same process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let granted = delegations.hold(id, [(Source::Policy, capability)]);
    /// context.add(granted);
    /// ```
    pub(super) fn add(&mut self, granted: SecurityContext) {
        self.capabilities.extend(granted.capabilities);
    }

    /// Delete the copy `id`, when the context has it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// context.remove(id);
    /// assert!(context.ids(CapabilityKind::Process).is_empty());
    /// ```
    pub(super) fn remove(&mut self, id: CapabilityId) {
        self.capabilities.retain(|held| held.id != id);
    }
}

/// The delegation record of one capability copy.
#[derive(Debug)]
struct Record {
    /// The process that got the copy.
    holder: ProcessId,
    /// The kind of the capability of the copy.
    kind: CapabilityKind,
    /// The source of the copy.
    source: Source,
    /// The copies that went from this copy to children, in the order of
    /// their identifiers.
    receivers: BTreeSet<CapabilityId>,
    /// True after the holder left the process table and released the copy.
    /// A released record stays while it has receivers, so that a
    /// revocation from above still reaches them.
    released: bool,
}

/// The delegation records of a run: for each capability copy, its holder,
/// its source, and the copies that went from it to children. The run owns
/// one tree, as it owns the registered names. A check does not read it: a
/// check reads only the copies of the caller. A record lives while its copy
/// lives, and after its holder exits while it has receivers. Thus a
/// revocation goes through a holder that exited to the copies below it.
#[derive(Debug, Default)]
pub(super) struct Delegations {
    /// The number of the next copy identifier.
    next_id: u64,
    /// The records by copy identifier.
    records: HashMap<CapabilityId, Record>,
}

impl Delegations {
    /// Give the process `holder` new copies with their sources, record each
    /// copy, add each copy to the receivers of its source copy, and return
    /// the context of the process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let root = delegations.hold(ProcessId(0), AllowAll.initial().into_iter().map(|c| (Source::Launcher, c)));
    /// let child = delegations.hold(ProcessId(1), root.delegate(None)?);
    /// ```
    pub(super) fn hold(
        &mut self,
        holder: ProcessId,
        copies: impl IntoIterator<Item = (Source, Capability)>,
    ) -> SecurityContext {
        let capabilities = copies
            .into_iter()
            .map(|(source, capability)| {
                let id = CapabilityId(self.next_id);
                self.next_id += 1;
                if let Source::Copy(parent) = source {
                    self.records
                        .get_mut(&parent)
                        .expect("the parent holds its source copy")
                        .receivers
                        .insert(id);
                }
                let record = Record {
                    holder,
                    kind: capability.kind,
                    source,
                    receivers: BTreeSet::new(),
                    released: false,
                };
                self.records.insert(id, record);
                Held { id, capability }
            })
            .collect();
        SecurityContext {
            capabilities,
            boot: false,
            io_service: false,
        }
    }

    /// Return the copies that went from the copies `from` to the process
    /// `holder`, also when `holder` exited and its record stays.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(delegations.given(&parent.ids(CapabilityKind::Process), child).len(), 1);
    /// ```
    pub(super) fn given(&self, from: &[CapabilityId], holder: ProcessId) -> Vec<CapabilityId> {
        from.iter()
            .filter_map(|id| self.records.get(id))
            .flat_map(|record| &record.receivers)
            .filter(|receiver| self.records[receiver].holder == holder)
            .copied()
            .collect()
    }

    /// Return the copies of `kind` that the process `holder` holds, or held
    /// before its exit and that keep a released record, in the order of
    /// their identifiers. The walk reads each record of the run.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(delegations.held(worker, CapabilityKind::Env), vec![id]);
    /// ```
    pub(super) fn held(&self, holder: ProcessId, kind: CapabilityKind) -> Vec<CapabilityId> {
        let mut held: Vec<_> = self
            .records
            .iter()
            .filter(|(_, record)| record.holder == holder && record.kind == kind)
            .map(|(&id, _)| id)
            .collect();
        held.sort_unstable();
        held
    }

    /// Start the revocation of the copy `id`, which a process holds or
    /// gave, so it has a record. Return the revocation signal to its holder
    /// when the holder lives. A released record has no holder to handle a
    /// signal, so the revocation goes on at once (see [`Self::revoke`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(delegations.revocation(id), vec![(holder, id)]);
    /// ```
    pub(super) fn revocation(&mut self, id: CapabilityId) -> Vec<(ProcessId, CapabilityId)> {
        let record = &self.records[&id];
        if record.released {
            self.revoke(id)
        } else {
            vec![(record.holder, id)]
        }
    }

    /// Remove the record of the revoked copy `id` and return a revocation
    /// signal for each live downstream copy. The holder of the copy calls
    /// this when it handles its revocation signal. The walk goes through
    /// each released record below the copy and removes it, so the
    /// revocation also reaches the receivers of a holder that exited. A
    /// copy with no record, for example a copy that a revocation before
    /// removed, gives no signal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // The root gave a copy to a worker that exited and gave it to its child.
    /// assert_eq!(delegations.revoke(root_copy), vec![(grandchild, grandchild_copy)]);
    /// ```
    pub(super) fn revoke(&mut self, id: CapabilityId) -> Vec<(ProcessId, CapabilityId)> {
        let Some(receivers) = self.remove(id) else {
            return Vec::new();
        };
        let mut signals = Vec::new();
        let mut walk = VecDeque::from_iter(receivers);
        while let Some(receiver) = walk.pop_front() {
            let record = &self.records[&receiver];
            if record.released {
                let record = self
                    .records
                    .remove(&receiver)
                    .expect("a receiver has a record");
                walk.extend(record.receivers);
            } else {
                signals.push((record.holder, receiver));
            }
        }
        signals
    }

    /// Release the copies of `context`, whose holder left the process
    /// table. A copy with no receivers loses its record. A copy with
    /// receivers keeps a released record for a later revocation.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// delegations.release(std::mem::take(&mut process.security));
    /// ```
    pub(super) fn release(&mut self, context: SecurityContext) {
        for held in context.capabilities {
            let record = self
                .records
                .get_mut(&held.id)
                .expect("a held copy has a record");
            if record.receivers.is_empty() {
                self.remove(held.id);
            } else {
                record.released = true;
            }
        }
    }

    /// Remove the record of the copy `id` from the tree and from the
    /// receivers of its source, and return its receivers. A released source
    /// that has no receivers left loses its record too, and so on up the
    /// tree. Return `None` when the copy has no record.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(delegations.remove(id).is_some());
    /// assert!(delegations.remove(id).is_none());
    /// ```
    fn remove(&mut self, id: CapabilityId) -> Option<BTreeSet<CapabilityId>> {
        let record = self.records.remove(&id)?;
        let mut child = id;
        let mut source = record.source;
        while let Source::Copy(parent) = source
            && let Some(parent_record) = self.records.get_mut(&parent)
        {
            parent_record.receivers.remove(&child);
            if !parent_record.released || !parent_record.receivers.is_empty() {
                break;
            }
            source = parent_record.source;
            child = parent;
            self.records.remove(&parent);
        }
        Some(record.receivers)
    }
}

/// The request of the `{capabilities, Map}` spawn option for one kind. A
/// field that is `None` keeps the value of the copy of the parent.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Restriction {
    /// The kind of the capability that the restriction applies to.
    kind: CapabilityKind,
    /// The operations of the child copy, one [`Operation::bit`] each.
    operations: Option<u32>,
    /// The constraint of the child copy.
    constraint: Option<Constraint>,
    /// Whether the child can give its copy to its own children.
    delegable: Option<bool>,
}

impl Restriction {
    /// Return the child copy of `capability` with this restriction. A
    /// restriction that permits an operation or a name that `capability`
    /// does not permit gives `Err(Denied)`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let restriction = Restriction { kind: CapabilityKind::Process, operations: Some(Operation::Spawn.bit()), constraint: None, delegable: None };
    /// assert!(restriction.apply(&AllowAll.initial()[0]).is_ok());
    /// ```
    fn apply(&self, capability: &Capability) -> Result<Capability, Denied> {
        let operations = self.operations.unwrap_or(capability.operations);
        let constraint = self
            .constraint
            .clone()
            .unwrap_or_else(|| capability.constraint.clone());
        if operations & !capability.operations != 0 || !constraint.within(&capability.constraint) {
            return Err(Denied);
        }
        Ok(Capability {
            kind: capability.kind,
            operations,
            constraint,
            delegable: self.delegable.unwrap_or(capability.delegable),
        })
    }
}

/// Read the value of the `spawn_opt` option `{capabilities, Map}`. Each key
/// of `Map` is a kind atom: `process`, `terminal`, `port`, `env`, `signal`,
/// `runtime`, or `persistent_term`. Each value is a map with the optional
/// keys `operations` (a proper list of operation atoms of the kind),
/// `delegable` (`true` or `false`), and the constraint field of the kind:
/// `modules` (atoms), `executables` (strings), `names` (strings), or
/// `signals` (atoms), with the value `any` or a proper list of names. A
/// string is a flat proper list of characters. Return `None` for each other
/// term, which raises `badarg`, as a spawn option that is not valid does
/// (`erl_process.c:12222-12434`).
///
/// # Examples
///
/// ```rust,ignore
/// // #{process => #{operations => [spawn], modules => [worker]}}
/// assert!(restrictions(&option).is_some());
/// assert!(restrictions(&Term::static_atom("all")).is_none());
/// ```
pub(super) fn restrictions(term: &Term) -> Option<Vec<Restriction>> {
    let Term::Map(kinds) = term else {
        return None;
    };
    kinds
        .iter()
        .map(|(kind, fields)| {
            let Term::Atom(kind) = kind else {
                return None;
            };
            restriction(CapabilityKind::named(kind.name())?.spec(), fields)
        })
        .collect()
}

/// Read the restriction map `fields` of the kind of `spec`, as [`restrictions`]
/// describes. Return `None` for a map that is not valid.
///
/// # Examples
///
/// ```rust,ignore
/// // #{operations => [write], delegable => false}
/// assert!(restriction(CapabilityKind::Terminal.spec(), &fields).is_some());
/// ```
fn restriction(spec: &KindSpec, fields: &Term) -> Option<Restriction> {
    let Term::Map(fields) = fields else {
        return None;
    };
    restriction_fields(spec, fields.iter())
}

/// Read the `(field, value)` pairs of a restriction map of the kind of
/// `spec`, as [`restrictions`] describes. Return `None` for a pair that is
/// not valid.
///
/// # Examples
///
/// ```rust,ignore
/// // #{operations => [write]}
/// let restriction = restriction_fields(CapabilityKind::Terminal.spec(), fields.iter());
/// assert_eq!(restriction.and_then(|restriction| restriction.delegable), None);
/// ```
fn restriction_fields<'a>(
    spec: &KindSpec,
    fields: impl Iterator<Item = (&'a Term, &'a Term)>,
) -> Option<Restriction> {
    let mut restriction = Restriction {
        kind: spec.kind,
        operations: None,
        constraint: None,
        delegable: None,
    };
    for (field, value) in fields {
        let Term::Atom(field) = field else {
            return None;
        };
        match (field.name(), value) {
            ("operations", _) if let Some(operations) = value.proper_list() => {
                let mut set = 0;
                for operation in operations {
                    let Term::Atom(operation) = operation else {
                        return None;
                    };
                    let (operation, _) =
                        OPERATION_NAMES.iter().find(|(operation_kind, name)| {
                            *name == operation.name() && spec.operations.contains(operation_kind)
                        })?;
                    set |= operation.bit();
                }
                restriction.operations = Some(set);
            }
            ("delegable", Term::Atom(delegable)) => {
                restriction.delegable = Some(match delegable.name() {
                    "true" => true,
                    "false" => false,
                    _ => return None,
                });
            }
            (field, value) => {
                let (constraint_field, form) = spec.constraint?;
                if field != constraint_field {
                    return None;
                }
                restriction.constraint = Some(constraint(form, value)?);
            }
        }
    }
    Some(restriction)
}

/// Read a capability that the code of the policy module gives: a map with
/// the key `kind`, a kind atom, and the keys `operations`, `delegable`, and
/// the constraint field of the kind, each with a value that [`restrictions`]
/// reads. A kind with no constraint field has no such key. Return `None` for
/// each other term, also for a map that has no such key or another key.
///
/// # Examples
///
/// ```rust,ignore
/// // #{kind => env, operations => [read], delegable => false, names => ["HOME"]}
/// assert!(capability(&home).is_some());
/// // #{kind => env, operations => [read], delegable => false}
/// assert!(capability(&no_names).is_none());
/// ```
pub(super) fn capability(term: &Term) -> Option<Capability> {
    let Term::Map(fields) = term else {
        return None;
    };
    let is_kind = |field: &Term| matches!(field, Term::Atom(field) if field.name() == "kind");
    let (_, Term::Atom(kind)) = fields.iter().find(|(field, _)| is_kind(field))? else {
        return None;
    };
    let spec = CapabilityKind::named(kind.name())?.spec();
    let fields = restriction_fields(spec, fields.iter().filter(|(field, _)| !is_kind(field)))?;
    let constraint = match spec.constraint {
        Some(_) => fields.constraint?,
        None => Constraint::Any,
    };
    Some(Capability {
        kind: spec.kind,
        operations: fields.operations?,
        constraint,
        delegable: fields.delegable?,
    })
}

/// Read a constraint value: `any`, or a proper list of names in `form`.
/// Return `None` for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(constraint(NameForm::Atom, &Term::static_atom("any")), Some(Constraint::Any));
/// assert_eq!(constraint(NameForm::String, &Term::nil()), Some(Constraint::Only(BTreeSet::new())));
/// ```
fn constraint(form: NameForm, value: &Term) -> Option<Constraint> {
    let names = match value {
        Term::Atom(any) if any.name() == "any" => return Some(Constraint::Any),
        _ => value.proper_list()?,
    };
    names
        .map(|name| match (form, name) {
            (NameForm::Atom, Term::Atom(name)) => Some(name.name().to_owned()),
            (NameForm::String, _) => name
                .proper_list()?
                .map(|character| match character {
                    Term::Integer(code) => u32::try_from(*code).ok().and_then(char::from_u32),
                    _ => None,
                })
                .collect(),
            _ => None,
        })
        .collect::<Option<_>>()
        .map(Constraint::Only)
}
