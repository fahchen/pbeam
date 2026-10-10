//! Capabilities, the security context of a process, and the policy that
//! issues the initial capabilities of a run (transition 2 of
//! `docs/capability-security-migration.md`).
//!
//! A capability is a runtime value, not a term: BEAM code cannot make one.
//! Each process holds its own copies in its [`SecurityContext`]. A check is
//! a mechanical test of these copies: it sends no message and runs no
//! policy code. A denied operation raises `error:capability_denied`, which
//! ERTS does not have (row "Capability checks" of `docs/erts-differences.md`).

use std::collections::BTreeSet;

use crate::{
    error::{BeamError, error},
    term::Term,
};

/// The resource kind that a capability covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityKind {
    /// Processes: spawn, links, monitors, and exit signals to other
    /// processes.
    Process,
    /// The terminal: writes, reads, raw mode, and the window size.
    Terminal,
    /// Ports: the start of an external program, its commands, and its close.
    Port,
    /// The OS environment variables.
    Env,
    /// The host signal dispositions.
    Signal,
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
    /// `terminal`: write output. `env`: set a variable.
    Write,
    /// `terminal`: read input. `env`: read a variable.
    Read,
    /// `terminal`: set raw mode.
    RawMode,
    /// `terminal`: read the window size.
    WindowSize,
    /// `port`: send a command to a port.
    Command,
    /// `port`: close a port.
    Close,
    /// `signal`: handle a host signal.
    Handle,
    /// `signal`: ignore a host signal.
    Ignore,
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
const KINDS: [KindSpec; 5] = [
    KindSpec {
        kind: CapabilityKind::Process,
        name: "process",
        operations: &[
            Operation::Spawn,
            Operation::Link,
            Operation::Monitor,
            Operation::ExitOther,
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
        ],
        constraint: None,
    },
    KindSpec {
        kind: CapabilityKind::Port,
        name: "port",
        operations: &[Operation::Spawn, Operation::Command, Operation::Close],
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
];

/// The atom of each operation in the `{capabilities, Map}` option.
const OPERATION_NAMES: [(Operation, &str); 12] = [
    (Operation::Spawn, "spawn"),
    (Operation::Link, "link"),
    (Operation::Monitor, "monitor"),
    (Operation::ExitOther, "exit_other"),
    (Operation::Write, "write"),
    (Operation::Read, "read"),
    (Operation::RawMode, "raw_mode"),
    (Operation::WindowSize, "window_size"),
    (Operation::Command, "command"),
    (Operation::Close, "close"),
    (Operation::Handle, "handle"),
    (Operation::Ignore, "ignore"),
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
    fn bit(self) -> u16 {
        1 << self as u16
    }

    /// Return the operation set of `operations`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Operation::set(&[Operation::Spawn, Operation::Link]), 3);
    /// ```
    fn set(operations: &[Operation]) -> u16 {
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
    /// Return whether the constraint permits `name`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Constraint::Any.permits("lists"));
    /// assert!(!Constraint::Only(BTreeSet::new()).permits("lists"));
    /// ```
    fn permits(&self, name: &str) -> bool {
        match self {
            Constraint::Any => true,
            Constraint::Only(names) => names.contains(name),
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
    operations: u16,
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
    /// `terminal` kind, which has no constraint field.
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
    /// when `name` is given, permits `name` in its constraint.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let spawn = Capability::new(CapabilityKind::Process, &[Operation::Spawn], Constraint::Any, false)?;
    /// assert!(spawn.permits(CapabilityKind::Process, Operation::Spawn, Some("lists")));
    /// assert!(!spawn.permits(CapabilityKind::Process, Operation::Link, None));
    /// ```
    fn permits(&self, kind: CapabilityKind, operation: Operation, name: Option<&str>) -> bool {
        self.kind == kind
            && self.operations & operation.bit() != 0
            && name.is_none_or(|name| self.constraint.permits(name))
    }
}

/// The source of the initial capabilities of a run. The launcher calls the
/// policy when it makes the root process. A Rust host can replace the
/// default [`AllowAll`] policy with [`crate::BeamProgram::execute_with_policy`].
pub trait Policy {
    /// Return the capabilities of the root process.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(AllowAll.initial().len(), 5);
    /// ```
    fn initial(&self) -> Vec<Capability>;
}

/// The default policy. The root process gets a delegable capability of
/// each kind with all operations and no constraint, so each process can do
/// each operation, and a run gives the same results as the pinned ERTS.
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
    /// assert_eq!(capabilities.len(), 5);
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

/// The security context of a process: its copies of its capabilities. The
/// process owns it, and the copies go when the process state goes. No check
/// finds a capability by a pid.
#[derive(Debug, Default)]
pub(super) struct SecurityContext {
    /// The capabilities that the process can use. The delegable ones also
    /// go to each child.
    capabilities: Vec<Capability>,
}

impl SecurityContext {
    /// Make the context that holds `capabilities`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let context = SecurityContext::new(AllowAll.initial());
    /// ```
    pub(super) fn new(capabilities: Vec<Capability>) -> Self {
        Self { capabilities }
    }

    /// Check `operation` of `kind`: return `Err(Denied)` unless a capability
    /// of the context permits it, and, when `name` is given, permits `name`
    /// in its constraint.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let context = SecurityContext::new(AllowAll.initial());
    /// assert_eq!(context.check(CapabilityKind::Process, Operation::Spawn, Some("lists")), Ok(()));
    /// assert_eq!(SecurityContext::default().check(CapabilityKind::Process, Operation::Link, None), Err(Denied));
    /// ```
    pub(super) fn check(
        &self,
        kind: CapabilityKind,
        operation: Operation,
        name: Option<&str>,
    ) -> Result<(), Denied> {
        if self
            .capabilities
            .iter()
            .any(|capability| capability.permits(kind, operation, name))
        {
            Ok(())
        } else {
            Err(Denied)
        }
    }

    /// Return the context of a new child. With no `restrictions`, the child
    /// gets a copy of each delegable capability. With `restrictions`, it gets
    /// a reduced copy of each delegable capability of a requested kind, and
    /// no other capability. A requested kind without a delegable capability,
    /// and a restriction that permits more than its capability, give
    /// `Err(Denied)`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let parent = SecurityContext::new(AllowAll.initial());
    /// assert_eq!(parent.delegate(None)?.capabilities.len(), 5);
    /// assert_eq!(SecurityContext::default().delegate(Some(&restrictions)), Err(Denied));
    /// ```
    pub(super) fn delegate(&self, restrictions: Option<&[Restriction]>) -> Result<Self, Denied> {
        let delegable = || {
            self.capabilities
                .iter()
                .filter(|capability| capability.delegable)
        };
        let Some(restrictions) = restrictions else {
            return Ok(Self::new(delegable().cloned().collect()));
        };
        if restrictions
            .iter()
            .any(|restriction| !delegable().any(|capability| capability.kind == restriction.kind))
        {
            return Err(Denied);
        }
        let mut capabilities = Vec::new();
        for capability in delegable() {
            if let Some(restriction) = restrictions
                .iter()
                .find(|restriction| restriction.kind == capability.kind)
            {
                capabilities.push(restriction.apply(capability)?);
            }
        }
        Ok(Self::new(capabilities))
    }
}

/// The request of the `{capabilities, Map}` spawn option for one kind. A
/// field that is `None` keeps the value of the copy of the parent.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Restriction {
    /// The kind of the capability that the restriction applies to.
    kind: CapabilityKind,
    /// The operations of the child copy, one [`Operation::bit`] each.
    operations: Option<u16>,
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
/// of `Map` is a kind atom: `process`, `terminal`, `port`, `env`, or
/// `signal`. Each value is a map with the optional keys `operations` (a
/// proper list of operation atoms of the kind), `delegable` (`true` or
/// `false`), and the constraint field of the kind: `modules` (atoms),
/// `executables` (strings), `names` (strings), or `signals` (atoms), with the
/// value `any` or a proper list of names. A string is a flat proper list of
/// characters. Return `None` for each other term, which raises `badarg`, as
/// a spawn option that is not valid does (`erl_process.c:12222-12434`).
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
            let spec = KINDS.iter().find(|spec| spec.name == kind.name())?;
            restriction(spec, fields)
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
    let mut restriction = Restriction {
        kind: spec.kind,
        operations: None,
        constraint: None,
        delegable: None,
    };
    for (field, value) in fields.iter() {
        let Term::Atom(field) = field else {
            return None;
        };
        match (field.name(), value) {
            ("operations", Term::List(operations)) => {
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

/// Read a constraint value: `any`, or a proper list of names in `form`.
/// Return `None` for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(constraint(NameForm::Atom, &Term::static_atom("any")), Some(Constraint::Any));
/// assert_eq!(constraint(NameForm::String, &Term::List(vec![])), Some(Constraint::Only(BTreeSet::new())));
/// ```
fn constraint(form: NameForm, value: &Term) -> Option<Constraint> {
    let names = match value {
        Term::Atom(any) if any.name() == "any" => return Some(Constraint::Any),
        Term::List(names) => names,
        _ => return None,
    };
    names
        .iter()
        .map(|name| match (form, name) {
            (NameForm::Atom, Term::Atom(name)) => Some(name.name().to_owned()),
            (NameForm::String, Term::List(characters)) => characters
                .iter()
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
