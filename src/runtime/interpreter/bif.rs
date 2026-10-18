//! BIFs: the ERTS `bif.tab` table, the BIF dispatch, and BIF results.
//!
//! The `bifs!` table in `table.rs` lists each BIF with its implementation.
//! The `binary`, `erlang`, `erts_internal`, `ets`, `file`, `hash`, `iolist`,
//! `lists`, `maps`, `math`, `os`, `re`, and `unicode` modules hold the BIF
//! bodies that do not fit on one table line.
//! The `pbeam_cap` module holds the bodies of the PBeam BIFs, which are not
//! in `bif.tab`. The `io_service` module holds the IO service entry of the
//! runtime to boot gate.

use super::*;
use crate::runtime::{
    capability::{CapabilityKind, Operation},
    host::output::IovecWrite,
    scheduler::Flushed,
};
use crate::term::ProcessId;

mod atomics;
mod binary;
mod erlang;
mod erts_internal;
mod ets;
mod file;
mod float;
mod hash;
mod integer;
mod io_service;
mod iolist;
mod lists;
mod maps;
mod math;
mod os;
mod packet;
mod pbeam_cap;
mod persistent_term;
mod re;
mod subtract;
mod term;
mod timer;
mod unicode;
mod ycf;

pub(super) use erlang::{error_info_location, is_function, send};
pub(super) use iolist::iodata_bytes;

/// What a BIF call does to the calling process.
#[derive(Debug)]
pub(super) enum BifResult {
    /// The BIF returns this value.
    Value(Term),
    /// The BIF raises an exception in the calling process.
    Raise {
        /// Class of the exception.
        class: ExceptionClass,
        /// Reason of the exception, without a stack trace.
        reason: Term,
    },
    /// The BIF raises `error:Reason` with the extended error information
    /// `cause`, as `BIF_ERROR(p, Reason | EXF_HAS_EXT_INFO)` with
    /// `p->fvalue = Cause` does. The location of the BIF frame gets
    /// `#{cause => Cause, module => Module}` (`save_stacktrace`,
    /// `beam_common.c:993-1002`).
    ErrorWithCause {
        /// Reason of the exception, without a stack trace.
        reason: Term,
        /// The `cause` of the `error_info` map.
        cause: Term,
    },
    /// The BIF raises an exception with this stack trace in the calling
    /// process, as `erlang:error/2` does with the frame of its caller
    /// (`EXC_ERROR_2`, `beam_common.c:478-495`) and `erlang:raise/3` does
    /// with the frames that it gets (`bif.c:1626-1692`).
    RaiseWithStacktrace {
        /// Class of the exception.
        class: ExceptionClass,
        /// Reason of the exception.
        reason: Term,
        /// Stack trace of the exception.
        stacktrace: Term,
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
    /// The BIF continues in the export of a BIF after the calling process
    /// switches out, as `BIF_TRAP` to a BIF export does: the process
    /// switches out at once (`context_switch_simplified` in
    /// `jit/arm/instr_bif.cpp:525-536`, `676-701`), and the export runs with
    /// these arguments when the process runs again. The export costs no
    /// reduction (`jit/arm/instr_bif.cpp:721-789`). Its value is the value of
    /// the BIF call. See [`BeamProgram::bif_trap`] for the exports.
    TrapBif {
        /// Module of the BIF export, for example `lists`.
        module: &'static str,
        /// Function of the BIF export, for example `member`.
        function: &'static str,
        /// Arguments of the BIF export, in `x0` onwards.
        arguments: Vec<Term>,
    },
    /// The BIF returns this value after the calling process switches out,
    /// as `ERTS_BIF_PREP_YIELD_RETURN` does (`bif.h:445-452`): the BIF
    /// traps to `bif_return_trap`, which returns the value when the process
    /// runs again.
    YieldReturn(Term),
    /// The NIF continues on a dirty IO thread with this host work, as a NIF
    /// with the `ERL_NIF_DIRTY_JOB_IO_BOUND` flag does
    /// (`static_schedule_dirty_nif`, `erl_nif.c:3437-3477`). The calling
    /// process switches out, and it gets the result of the call when the
    /// work completes.
    DirtyWrite(IovecWrite),
    /// PBeam only: the calling process waits for the answer of the policy
    /// process to its `pbeam_cap:request/2` call. It continues through the
    /// export `pbeam_cap:await_grant/0` each time it runs again, until the
    /// grant signal gives the answer. ERTS has no such wait, because it has
    /// no policy process.
    AwaitGrant,
    /// The calling process exits with this reason, and no catch tag sees
    /// it, as `ERTS_BIF_EXITED` does when a BIF that handles the signals of
    /// its caller finds that a signal ended the caller (`bif.c:1708-1710`,
    /// `5482-5483`).
    Exited(Term),
    /// The calling process hibernates, as `hibernate_3` does
    /// (`bif.c:1366-1427`): it drops its stack and its catches, waits for a
    /// message when its queue has none (`erts_hibernate`,
    /// `beam_common.c:1568-1602`), and then calls `apply(module, function,
    /// arguments)` with an empty stack (`beam_run_process`).
    Hibernate {
        /// Module of the call after the wake-up, an atom.
        module: Term,
        /// Function of the call after the wake-up, an atom.
        function: Term,
        /// Arguments of the call after the wake-up, a proper list.
        arguments: Term,
    },
}

impl BifResult {
    /// Return the `error:badarg` exception, which ERTS raises with
    /// `BIF_ERROR(p, BADARG)` for a BIF argument that is not valid.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(BifResult::badarg(), BifResult::Raise { class: ExceptionClass::Error, .. }));
    /// ```
    pub(super) fn badarg() -> BifResult {
        BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::static_atom("badarg"),
        }
    }

    /// Return the `error:badarg` exception with the `error_info` cause
    /// `badopt`, which ERTS `spawn_opt_4` raises for an option that
    /// `erts_parse_spawn_opts` rejects (`bif.c:1040-1051`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(BifResult::badopt(), BifResult::ErrorWithCause { .. }));
    /// ```
    pub(super) fn badopt() -> BifResult {
        BifResult::ErrorWithCause {
            reason: Term::static_atom("badarg"),
            cause: Term::static_atom("badopt"),
        }
    }

    /// Return the `error:badarith` exception, which ERTS raises with
    /// `BADARITH` for an arithmetic operand that is not a number
    /// (`erl_arith.c`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(BifResult::badarith(), BifResult::Raise { class: ExceptionClass::Error, .. }));
    /// ```
    pub(super) fn badarith() -> BifResult {
        BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::static_atom("badarith"),
        }
    }

    /// Return the `error:capability_denied` exception of an operation that
    /// the capabilities of the caller do not permit. ERTS has no such
    /// exception, so the atom goes into the atom table of the run at the
    /// first denial.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let denied = BifResult::capability_denied(&mut scheduler.atoms)?;
    /// assert!(matches!(denied, BifResult::Raise { class: ExceptionClass::Error, .. }));
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of `AtomTable::put` for a full atom table.
    pub(super) fn capability_denied(atoms: &mut AtomTable) -> Result<BifResult, BeamError> {
        Ok(BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::Atom(atoms.put("capability_denied")?),
        })
    }
}

/// Return the `error:capability_denied` exception when no capability of the
/// current process permits `operation` of `kind`, and, when `name` is given,
/// permits `name` in its constraint (see `SecurityContext::check`). Else
/// return `None`. An effect entry calls this after its argument checks and
/// before its first effect, so a denied call has no effect (row "Capability
/// checks" of `docs/erts-differences.md`). A run that skips the checks
/// returns `None` before it reads the capabilities (see
/// `Scheduler::without_checks`).
///
/// # Examples
///
/// ```rust,ignore
/// // `AllowAll` permits each operation.
/// assert!(denial(&mut scheduler, CapabilityKind::Env, Operation::Read, Some(b"HOME"))?.is_none());
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for a full atom table.
pub(super) fn denial(
    scheduler: &mut Scheduler,
    kind: CapabilityKind,
    operation: Operation,
    name: Option<&[u8]>,
) -> Result<Option<BifResult>, BeamError> {
    if scheduler.skips_checks()
        || scheduler
            .current_process()
            .security
            .check(kind, operation, name)
            .is_ok()
    {
        return Ok(None);
    }
    BifResult::capability_denied(&mut scheduler.atoms).map(Some)
}

/// Return the `error:capability_denied` exception when the current process
/// is not a boot process and `target` is a boot process (see
/// `Scheduler::enters_boot`): the runtime to boot gate. Else return
/// `None`. An entry calls this after its capability check and before its
/// first effect, so a denied call has no effect. Only a run in boot mode
/// has boot processes (row "Boot authority" of `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// // The current process entered the runtime; `user` is a boot process.
/// assert!(boot_denial(&mut scheduler, user)?.is_some());
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for a full atom table.
pub(super) fn boot_denial(
    scheduler: &mut Scheduler,
    target: ProcessId,
) -> Result<Option<BifResult>, BeamError> {
    if scheduler.enters_boot(target) {
        return BifResult::capability_denied(&mut scheduler.atoms).map(Some);
    }
    Ok(None)
}

/// Return the `error:capability_denied` exception when `target` is not the
/// current process and no capability of the current process permits
/// `operation` of the `process` kind (see [`denial`]), or when the runtime
/// to boot gate denies the call (see [`boot_denial`]). Else return `None`.
/// The process inspection BIFs check `inspect` and the group leader BIFs
/// check `group_leader` with this function after their argument checks and
/// before the process lookup, so a denied call does not tell whether a
/// process has the pid, except whether a live process is a boot process. A
/// call for the caller has no check.
///
/// # Examples
///
/// ```rust,ignore
/// let caller = scheduler.current_process().id;
/// assert!(other_process_denial(&mut scheduler, caller, Operation::Inspect)?.is_none());
/// ```
///
/// # Errors
///
/// The errors of `AtomTable::put` for a full atom table.
pub(super) fn other_process_denial(
    scheduler: &mut Scheduler,
    target: ProcessId,
    operation: Operation,
) -> Result<Option<BifResult>, BeamError> {
    if target == scheduler.current_process().id {
        return Ok(None);
    }
    match denial(scheduler, CapabilityKind::Process, operation, None)? {
        Some(denied) => Ok(Some(denied)),
        None => boot_denial(scheduler, target),
    }
}

/// Return the runtime error of a BIF that ERTS runs with a port argument
/// and that PBeam does not run with a port yet: `erlang:exit/2`,
/// `erlang:monitor/2,3` with the type `port`, `erlang:register/2`,
/// `erlang:send/2,3`, the `send` instruction, and `erlang:unlink/1`.
///
/// # Examples
///
/// ```rust,ignore
/// let error = unsupported_port("erlang:unlink/1");
/// assert!(error.to_string().starts_with("unsupported erlang:unlink/1 with a port"));
/// ```
fn unsupported_port(bif: &str) -> BeamError {
    error(format!(
        "unsupported {bif} with a port: see \"Ports\" in docs/erts-differences.md"
    ))
}

/// The state that a BIF body can use.
struct BifContext<'a> {
    /// The scheduler that runs the calling process.
    scheduler: &'a mut Scheduler,
}

/// The largest number of arguments of the argument lanes of a `bif`
/// instruction.
///
/// The instruction set gives a `bif` instruction at most the three
/// arguments of `gc_bif3`, so a lane that holds this many arguments covers
/// every `bif` instruction. The borrowed lane holds this many references
/// and the owned lane holds this many terms.
pub(super) const MAX_BORROWED_ARGUMENTS: usize = 3;

/// The placeholder of a slot that no argument filled.
///
/// The array of [`BifArguments`] is fully initialized, because `&Term` is
/// `Copy` and `[&UNUSED_ARGUMENT; 3]` writes three references. A slot at or
/// above the length holds this term and stays outside the slice of
/// [`BifArguments::with`]. `Term::Nil` is a unit variant, so the static is
/// a constant expression, and `Term` is `Sync` (`src/term.rs`).
static UNUSED_ARGUMENT: Term = Term::Nil;

/// The argument terms of one BIF call of at most
/// [`MAX_BORROWED_ARGUMENTS`] terms, as references.
///
/// The leaf lane reads the arguments into this struct one time, and
/// [`Self::with`] passes them to a body as a borrowed slice. The body takes
/// no scheduler, so the borrow of the process stays valid in safe Rust (see
/// `docs/bif-argument-borrow-design.md`). The struct holds no `Term` value,
/// so it needs no `Term::nil()` and no `Arc` operation.
///
/// # Examples
///
/// ```rust,ignore
/// let one = Term::Integer(1);
/// let two = Term::Integer(2);
/// let mut arguments = BifArguments::new();
/// arguments.push(&one);
/// arguments.push(&two);
/// assert_eq!(arguments.with(|terms| terms.to_vec()), vec![&one, &two]);
/// ```
pub(super) struct BifArguments<'a> {
    /// The references to the arguments. A slot at or above the length holds
    /// [`UNUSED_ARGUMENT`] and stays outside the slice of [`Self::with`].
    terms: [&'a Term; MAX_BORROWED_ARGUMENTS],
    /// The number of references that [`Self::push`] wrote.
    len: usize,
}

impl<'a> BifArguments<'a> {
    /// Return an empty argument list.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(BifArguments::new().with(|terms| terms.len()), 0);
    /// ```
    pub(super) fn new() -> Self {
        Self {
            terms: [&UNUSED_ARGUMENT; MAX_BORROWED_ARGUMENTS],
            len: 0,
        }
    }

    /// Add one reference to the end of the list.
    ///
    /// # Panics
    ///
    /// The reader checks the count against [`MAX_BORROWED_ARGUMENTS`] before
    /// it pushes, so a full list is a caller error.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let one = Term::Integer(1);
    /// let mut arguments = BifArguments::new();
    /// arguments.push(&one);
    /// assert_eq!(arguments.with(|terms| terms.to_vec()), vec![&one]);
    /// ```
    pub(super) fn push(&mut self, term: &'a Term) {
        self.terms[self.len] = term;
        self.len += 1;
    }

    /// Call `body` with the arguments as a borrowed slice of references.
    /// The slice is valid only for the call of `body`, which takes no
    /// scheduler.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let one = Term::Integer(1);
    /// let mut arguments = BifArguments::new();
    /// arguments.push(&one);
    /// assert_eq!(arguments.with(|terms| terms.len()), 1);
    /// ```
    pub(super) fn with<R>(&self, body: impl FnOnce(&[&'a Term]) -> R) -> R {
        body(&self.terms[..self.len])
    }
}

/// The argument terms of one BIF call of at most
/// [`MAX_BORROWED_ARGUMENTS`] terms, as owned copies.
///
/// The owned lane copies each argument into this struct one time, and
/// [`Self::with`] passes them to a body as a borrowed slice. The body may
/// borrow the scheduler at the same time, because the slice borrows this
/// struct and not the scheduler. A body that stores the arguments for a
/// later turn copies them again (see [`BifResult::TrapBif`]).
///
/// # Examples
///
/// ```rust,ignore
/// let mut arguments = OwnedBifArguments::new();
/// arguments.push(Term::Integer(1));
/// arguments.push(Term::Integer(2));
/// assert_eq!(arguments.with(|terms| terms.to_vec()), vec![Term::Integer(1), Term::Integer(2)]);
/// ```
pub(super) struct OwnedBifArguments {
    /// The owned terms of the arguments. A slot at or above the length holds
    /// `nil` and stays outside the slice of [`Self::with`].
    terms: [Term; MAX_BORROWED_ARGUMENTS],
    /// The number of terms that [`Self::push`] wrote.
    len: usize,
}

impl OwnedBifArguments {
    /// Return an empty argument list.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(OwnedBifArguments::new().with(|terms| terms.len()), 0);
    /// ```
    pub(super) fn new() -> Self {
        Self {
            terms: std::array::from_fn(|_| Term::nil()),
            len: 0,
        }
    }

    /// Add one owned argument term to the end of the list.
    ///
    /// # Panics
    ///
    /// The reader checks the count against [`MAX_BORROWED_ARGUMENTS`] before
    /// it pushes, so a full list is a caller error.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut arguments = OwnedBifArguments::new();
    /// arguments.push(Term::Integer(1));
    /// assert_eq!(arguments.with(|terms| terms.to_vec()), vec![Term::Integer(1)]);
    /// ```
    pub(super) fn push(&mut self, term: Term) {
        self.terms[self.len] = term;
        self.len += 1;
    }

    /// Call `body` with the arguments as a borrowed slice. The slice is valid
    /// only for the call of `body`, which may borrow the scheduler at the
    /// same time.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut arguments = OwnedBifArguments::new();
    /// arguments.push(Term::Integer(1));
    /// assert_eq!(arguments.with(|terms| terms.len()), 1);
    /// ```
    pub(super) fn with<R>(&self, body: impl FnOnce(&[Term]) -> R) -> R {
        body(&self.terms[..self.len])
    }
}

/// Build the BIF identities and the BIF dispatch from one table.
///
/// Each entry is a `bif.tab` entry: `module:name/arity`, then `unsupported`
/// or one `|context, [arguments]| body` pattern, never both and never neither:
/// either mistake does not compile. A function that ERTS runs as BEAM code is
/// not a BIF and is not in this table. A pattern can have a guard:
/// `|context, [arguments]| where guard => body`. A body returns
/// `Result<BifResult, BeamError>`. A name that is not a Rust identifier, such
/// as the operator `+`, is a string literal: `erlang:"+"/2`.
///
/// An entry that the table implements names its [`BifId`] variant:
/// `erlang:hd/1 as ErlangHd1 => |_, [list]| ...`. The name is the module,
/// the function, and the arity in CamelCase, and it is unique in the table.
/// An operator uses the name of its operation: `"+"` is `Plus`, `"++"` is
/// `Append`, `"--"` is `Subtract`, `"=:="` is `ExactEqual`, and `"=/="` is
/// `NotExactEqual`. An `unsupported` entry has no variant and resolves to
/// [`BifId::Unimplemented`].
///
/// The macro expands into `BIFS`, the entries as `(module, name, arity)`,
/// `BifId`, one variant for each implemented entry in table order,
/// `resolve`, the MFA to identity lookup of the loader, and `dispatch`, one
/// `match` arm for each implemented entry. A run reads no name and searches
/// no table.
///
/// Each arm runs its body in a closure. The closure body keeps the span of
/// its table line, so line coverage measures each table pattern. A body in
/// the arm itself counts only against the lines of this macro. One pattern
/// for each entry keeps one pattern on each table line.
///
/// # Examples
///
/// ```rust,ignore
/// bifs! {
///     erlang:"+"/2 as ErlangPlus2 => |_, [left, right]| erlang::arith("+", left, right, i64::checked_add, |l, r| l + r),
///     erlang:self/0 as ErlangSelf0 => |cx, []| Ok(Value(Term::Pid(cx.scheduler.current_process_mut().id))),
///     lists:keyfind/3 => unsupported,
/// }
/// assert_eq!(BIFS, &[("erlang", "+", 2), ("erlang", "self", 0), ("lists", "keyfind", 3)]);
/// assert_eq!(resolve("erlang", "+", 2), Some(BifId::ErlangPlus2));
/// assert_eq!(resolve("erlang", "self", 1), None);
/// assert_eq!(resolve("lists", "keyfind", 3), Some(BifId::Unimplemented(2)));
/// ```
macro_rules! bifs {
    (@form unsupported) => {};
    (@form | $context:pat_param |) => {};
    (@form $($tokens:tt)*) => {
        compile_error!("a bifs! entry needs `unsupported` or one `|context, [arguments]| body`, not both");
    };
    (@name $name:literal) => { $name };
    (@name $name:ident) => { stringify!($name) };
    (@identity $variant:ident) => { Some(BifId::$variant) };
    (@identity) => { None };
    (
        $(
            $module:ident : $name:tt / $arity:literal $(as $variant:ident)? => $($unsupported:ident)?
            $( |$context:pat_param, $arguments:pat_param| $(where $guard:expr =>)? $body:expr )?
        ),* $(,)?
    ) => {
        const _: () = {
            $( bifs!(@form $($unsupported)? $(| $context |)?); )*
        };

        /// The `bif.tab` entries of the table, as `(module, name, arity)`,
        /// sorted for a binary search.
        ///
        /// # Examples
        ///
        /// ```rust,ignore
        /// assert!(BIFS.binary_search(&("lists", "keyfind", 3)).is_ok());
        /// ```
        pub(in crate::runtime::interpreter) const BIFS: &[(&str, &str, u32)] = &[
            $( (stringify!($module), bifs!(@name $name), $arity), )*
        ];

        /// The identity of one BIF operation.
        ///
        /// The table gives one variant to each entry that it implements, in
        /// table order. The dispatch of a run matches on the variant, so a
        /// BIF call costs one integer compare and no name compare.
        /// [`BifId::Unimplemented`] names an entry that the table does not
        /// implement; its value is the position of the entry in [`BIFS`],
        /// for the error message of the run.
        ///
        /// # Examples
        ///
        /// ```rust,ignore
        /// assert_eq!(resolve("erlang", "hd", 1), Some(BifId::ErlangHd1));
        /// ```
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum BifId {
            $( $($variant,)? )*
            /// An entry of [`BIFS`] that the table does not implement, as
            /// the position of the entry in `BIFS`.
            Unimplemented(u16),
        }

        /// The identity of each entry of [`BIFS`], in the same order.
        const BIF_IDS: &[Option<BifId>] = &[ $( bifs!(@identity $($variant)?), )* ];

        /// Return the identity of the BIF `module:name/arity`, or `None`
        /// when the MFA is not a `bif.tab` entry of [`BIFS`].
        ///
        /// The loader calls this once for each `bif` instruction, which
        /// stores the identity in the instruction, and once for each import
        /// through `import_kind`, which stores the identity in the
        /// `ImportKind::Bif` of the import. The ERTS loader binds the
        /// import of a BIF to the BIF in the same way (`emu_load.c`). A
        /// `call_mfa` of an `apply` calls it once for each call. A run then
        /// searches no table for a resolved BIF.
        ///
        /// # Examples
        ///
        /// ```rust,ignore
        /// assert_eq!(resolve("erlang", "hd", 1), Some(BifId::ErlangHd1));
        /// assert_eq!(resolve("lists", "keyfind", 3).is_some(), true);
        /// assert_eq!(resolve("io", "put_chars", 2), None);
        /// ```
        pub(crate) fn resolve(module: &str, name: &str, arity: u32) -> Option<BifId> {
            #[cfg(test)]
            super::RESOLVE_CALLS.with(|calls| calls.set(calls.get() + 1));
            let index = BIFS.binary_search(&(module, name, arity)).ok()?;
            Some(BIF_IDS[index].unwrap_or(BifId::Unimplemented(index as u16)))
        }

        /// Return the module, the name, and the arity of the entry of `id`.
        ///
        /// An identity of the table does not carry the position of its
        /// entry, so a search finds it. Only an error message calls this
        /// function.
        ///
        /// # Examples
        ///
        /// ```rust,ignore
        /// assert_eq!(mfa(BifId::ErlangHd1), ("erlang", "hd", 1));
        /// ```
        pub(crate) fn mfa(id: BifId) -> (&'static str, &'static str, u32) {
            if let BifId::Unimplemented(index) = id {
                return BIFS[index as usize];
            }
            BIFS[BIF_IDS
                .iter()
                .position(|entry| *entry == Some(id))
                .expect("an implemented BIF identity is an entry of BIFS")]
        }

        /// Run the table entry of `id`, or report an unresolved MFA.
        ///
        /// The loader gives an identity the arity of its table entry, so
        /// the `match` of a call always reaches the arm of the entry. An
        /// argument pattern that the call does not match, for example
        /// `link/1` of a term that is not a pid, gives the unresolved MFA
        /// error of the entry, as the name match of the dispatch did before
        /// the identity existed.
        ///
        /// # Examples
        ///
        /// ```rust,ignore
        /// let result = dispatch(&mut context, BifId::MapsKeys1, &[Term::Map(Map::new())])?;
        /// assert!(matches!(result, BifResult::Value(keys) if keys.is_nil()));
        /// ```
        pub(super) fn dispatch(
            context: &mut BifContext<'_>,
            id: BifId,
            arguments: &[Term],
        ) -> Result<BifResult, BeamError> {
            // The cleanup of a fixation on a table that another process
            // deleted runs as process system work at each scheduling
            // (`erts_schedule_ets_free_fixation`, `erl_db.c:5263-5301`).
            match (id, arguments) {
                $($(
                    (BifId::$variant, $arguments) $(if $guard)? => {
                        (|$context: &mut BifContext<'_>| $body)(context)
                    }
                )?)*
                (id, _) => {
                    let (module, name, _) = mfa(id);
                    Err(unresolved_mfa(module, name, arguments.len()))
                }
            }
        }
    };
}

/// Return the runtime error of an MFA that the BIF table does not implement.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(unresolved_mfa("erlang", "date", 0).to_string(), "unresolved external MFA erlang:date/0");
/// ```
fn unresolved_mfa(module: &str, name: &str, arity: usize) -> BeamError {
    error(format!("unresolved external MFA {module}:{name}/{arity}"))
}

mod table;

/// The `bif.tab` entries of [`table`], which the unit test of the BIF table
/// reads. The run reads an entry through [`BifId`] only.
#[cfg(test)]
pub(in crate::runtime::interpreter) use table::BIFS;
pub(crate) use table::{BifId, mfa, resolve};
pub(super) use table::{dispatch_leaf, is_leaf};

#[cfg(test)]
thread_local! {
    /// The number of calls of [`table::resolve`] on this test thread.
    ///
    /// The result of a search does not show in the value of a run, so a test
    /// of a path that must not search `BIFS` reads this counter. The counter
    /// is per thread, because the test harness runs the tests of one binary
    /// on parallel threads.
    pub(in crate::runtime::interpreter) static RESOLVE_CALLS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Return the number of calls of [`table::resolve`] on this test thread.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(resolve_calls(), 0);
/// ```
#[cfg(test)]
pub(in crate::runtime::interpreter) fn resolve_calls() -> usize {
    RESOLVE_CALLS.with(std::cell::Cell::get)
}

impl BeamProgram {
    /// Run the BIF of `module:name/arity` with the arguments, resolving the
    /// MFA of the name. The result is a value, an exception, or a trap into
    /// BEAM code.
    ///
    /// The name path runs a `bif.tab` entry or a PBeam BIF of
    /// [`PBEAM_BIFS`]. The trap export path and the test of the runtime call
    /// it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = program.bif(&mut scheduler, "erlang", "make_ref", vec![])?;
    /// assert!(matches!(value, BifResult::Value(Term::Reference(_))));
    /// ```
    pub(super) fn bif(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        name: &str,
        arguments: Vec<Term>,
    ) -> Result<BifResult, BeamError> {
        let resolved = resolve(module, name, arguments.len() as u32);
        match resolved {
            Some(id) => self.bif_slice(scheduler, id, &arguments),
            None => self.named_bif(scheduler, module, name, &arguments),
        }
    }

    /// Run the BIF of an external call or an apply.
    ///
    /// `resolved` is the identity of the MFA when the caller holds one: the
    /// loader stores it in a `call_ext` instruction, and the identity lookup
    /// of the call gives it for a `bif` instruction. `None` selects the name
    /// path of a PBeam BIF. The arguments are borrowed, so a caller that
    /// holds them in an [`OwnedBifArguments`] passes them with no copy.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let resolved = resolve("erlang", "make_ref", 0);
    /// let value = program.call_bif(&mut scheduler, "erlang", "make_ref", resolved, &[])?;
    /// assert!(matches!(value, BifResult::Value(Term::Reference(_))));
    /// ```
    pub(super) fn call_bif(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        name: &str,
        resolved: Option<BifId>,
        arguments: &[Term],
    ) -> Result<BifResult, BeamError> {
        match resolved {
            Some(id) => self.bif_slice(scheduler, id, arguments),
            None => self.named_bif(scheduler, module, name, arguments),
        }
    }

    /// Run a BIF of a name that no [`BIFS`] entry holds: a PBeam BIF of
    /// [`PBEAM_BIFS`], or the unresolved MFA error of the run.
    ///
    /// The trap export path reaches this function with the name of a
    /// `TrapBif` target, and a `call_ext` of a PBeam BIF reaches it from
    /// [`BeamProgram::call_bif`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let result = program.named_bif(&mut scheduler, "pbeam_cap", "revoke", &[holder, kind])?;
    /// let error = program.named_bif(&mut scheduler, "erlang", "date", &[]).unwrap_err();
    /// assert_eq!(error.to_string(), "unresolved external MFA erlang:date/0");
    /// ```
    pub(super) fn named_bif(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        name: &str,
        arguments: &[Term],
    ) -> Result<BifResult, BeamError> {
        match (module, name, arguments) {
            ("pbeam_cap", "revoke", [holder, kind]) => pbeam_cap::revoke(scheduler, holder, kind),
            ("pbeam_cap", "request", [kind, request]) => {
                pbeam_cap::request(scheduler, kind, request)
            }
            ("pbeam_cap", "policy_revoke", [holder, kind]) => {
                pbeam_cap::policy_revoke(scheduler, holder, kind)
            }
            ("pbeam_cap", "enter_runtime", [module, function]) => {
                pbeam_cap::enter_runtime(scheduler, module, function)
            }
            ("pbeam_cap", "serve", [pid, kind]) => pbeam_cap::serve(scheduler, pid, kind),
            _ => Err(unresolved_mfa(module, name, arguments.len())),
        }
    }

    /// Run the BIF of an identity that the loader resolved. A run of this
    /// call compares no name and searches no table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = program.bif_slice(&mut scheduler, BifId::ErlangMakeRef0, &[])?;
    /// assert!(matches!(value, BifResult::Value(Term::Reference(_))));
    /// ```
    pub(super) fn bif_slice(
        &self,
        scheduler: &mut Scheduler,
        id: BifId,
        arguments: &[Term],
    ) -> Result<BifResult, BeamError> {
        let mut context = BifContext { scheduler };
        table::dispatch(&mut context, id, arguments)
    }

    /// Run the export of a BIF trap with the arguments of the trap, as the
    /// process does when it runs again after [`BifResult::TrapBif`]. Most
    /// BIFs trap to their own export with new arguments, for example
    /// `lists:member/2` with the rest of the list. These exports are not
    /// `bif.tab` entries, and no Erlang code can call them:
    ///
    /// - `erlang:length/2`, with the rest of the list and the count so far.
    ///   ERTS traps to `erlang:length/3`, which also keeps the original list
    ///   for the stack trace of an error (`erl_bif_guard.c:48-55`, `226-247`).
    ///   PBeam gives a BIF error the stack trace `[]`.
    /// - `erlang:'++'/3`, with the original lists and the rest of the left
    ///   list. ERTS traps to `'++'/2` with a magic reference to the same state
    ///   (`erl_bif_lists.c:91-109`, `298-333`).
    /// - `erlang:'--'/10`, with the stage and the state of a subtraction
    ///   ([`subtract::continue_trap`]). ERTS traps to `'--'/2` with a magic
    ///   reference to the state (`erl_bif_lists.c:1015-1024`).
    /// - `maps:merge_trap/2`, with the merged map and the steps that remain
    ///   ([`maps::merge_trap`]). ERTS traps to `maps:merge_trap/1` with a
    ///   magic reference to the walk state (`erl_map.c:134-136`).
    /// - `maps:from_list_trap/2`, `maps:keys_trap/2`, and
    ///   `maps:values_trap/2`, with the argument and the YCF units done
    ///   ([`maps::from_list`], [`maps::keys`], [`maps::values`]). ERTS
    ///   traps to `maps:from_list/1`, `maps:keys/1`, and `maps:values/1`
    ///   with a magic reference to the YCF state (`utils.c:3718-3731`).
    /// - `maps:from_keys_trap/3`, with the keys, the value, and the YCF
    ///   units done ([`maps::from_keys`]). ERTS traps to `maps:from_keys/2`
    ///   with a magic reference to the YCF state.
    /// - `persistent_term:get_all_trap/2`, with the result and the number of
    ///   pairs that remain ([`persistent_term::get_all_trap`]). ERTS keeps
    ///   the table and the result so far (`erl_bif_persistent.c:263-265`).
    /// - `erlang:characters_to_utf8_trap/3`, `erlang:characters_to_list_trap_1/4`,
    ///   `_2/2`, `_3/3`, and `_4/2`, with the state of a chardata
    ///   conversion ([`unicode`]). ERTS traps to the exports of the same
    ///   names with other arguments: a magic reference to the state, or the
    ///   result binary so far (`erl_unicode.c:83-101`).
    /// - `persistent_term:put_common_trap/1`, with the slots of the table
    ///   copy that remain ([`persistent_term::put_trap`]). ERTS traps to
    ///   `persistent_term:put_common_trap/3` with a magic reference to the
    ///   copy state (`erl_bif_persistent.c:176-177`, `650-651`).
    /// - `persistent_term:erase_trap/1`, with the slots of the table copies
    ///   that remain ([`persistent_term::erase_trap`]). ERTS traps to
    ///   `persistent_term:erase/1` with a magic reference to the copy state
    ///   (`erl_bif_persistent.c:432-433`).
    /// - `ets:insert/6`, with the table reference, the objects, the units
    ///   done, the key position, the steps of a `bag` insert or `false`, and
    ///   the state of the check ([`ets::insert_list`]). ERTS traps to
    ///   `ets:insert/2` with a magic reference to the YCF state
    ///   (`erl_db.c:2297-2332`).
    /// - `ets:insert_new/7`, with the arguments of `ets:insert/6` and the
    ///   [`ets::New`] state. ERTS traps to `ets:insert_new/2` in the same
    ///   way.
    /// - `ets:select_trap/7`, with the table reference, the pattern, the
    ///   next bucket, the limit, the matches so far, their number, and the
    ///   change count of the table ([`ets::match_trap`]). ERTS traps to
    ///   `ets:select_trap/1` with a tuple that holds a magic reference to the
    ///   compiled pattern (`erl_db_hash.c:2264-2307`).
    /// - `ets:delete_trap/0`, which frees the segments of the table that
    ///   `ets:delete/1` deleted ([`ets::delete_trap`]). The segments that
    ///   remain stay in the ETS store under the caller. ERTS traps to
    ///   `ets:delete_trap/1` with a bignum that holds the table pointer
    ///   (`erl_db.c:3064-3077`, `5378-5397`).
    /// - `erts_internal:list_to_bitstring_continue/5`, with the phase and the
    ///   walk state of `erlang:iolist_to_binary/1` and `erlang:list_to_binary/1`
    ///   ([`iolist::l2b_size`], [`iolist::l2b_copy`]). ERTS traps to
    ///   `erts_internal:list_to_bitstring_continue/1` with a magic reference
    ///   to the walk state (`erl_iolist.c:53-55`, `764-835`).
    /// - `erts_internal:binary_to_list_continue/4`, with the binary, the
    ///   offset, the bytes that remain, and the list so far of
    ///   `erlang:binary_to_list/1,3` ([`erlang::b2l_chunk`]). ERTS traps to
    ///   `erts_internal:binary_to_list_continue/1` with a magic reference to
    ///   the state (`binary.c:41-53`, `292-304`).
    /// - `erlang:binary_find_trap/3`, with the subject, the search state, and
    ///   the pattern of `binary:match/2` or `binary:split/2,3`
    ///   ([`binary::find_trap`]). ERTS traps to the same export with magic references to the search state
    ///   and to the compiled pattern (`erl_bif_binary.c:75-77`, `1935-1952`).
    /// - `erts_internal:ptab_list_continue/5`, with the state of an
    ///   `erlang:processes/0` call ([`erlang::ptab_list`]). ERTS traps to
    ///   `erts_internal:ptab_list_continue/2` with the result so far and a
    ///   magic reference to the state (`erl_ptab.c:770-780`).
    /// - `erlang:bif_handle_signals_return/2`, with the process that the BIF
    ///   read and its result: it handles the signals of the caller up to a
    ///   flush signal and returns the result (`bif.c:5452-5495`).
    /// - `erts_internal:await_exit/0`: it handles the signals of the caller
    ///   until the exit signal of `exit(self(), Reason)` ends the caller
    ///   (`bif.c:1698-1716`).
    /// - `pbeam_cap:await_grant/0`, after [`BifResult::AwaitGrant`]: it
    ///   returns the answer of the policy process, or waits again
    ///   ([`pbeam_cap::await_grant`]). ERTS has no such export.
    ///
    /// The last two run again after a switch when their reductions are used
    /// up (`ERTS_BIF_YIELD2`, `ERTS_BIF_YIELD0`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let rest = Term::list_from(vec![Term::Integer(1)]);
    /// let result = program.bif_trap(&mut scheduler, "erlang", "length", vec![rest, Term::Integer(64000)])?;
    /// assert!(matches!(result, BifResult::Value(Term::Integer(64001))));
    /// ```
    pub(super) fn bif_trap(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        function: &str,
        mut arguments: Vec<Term>,
    ) -> Result<BifResult, BeamError> {
        if let (
            "ets",
            "insert" | "insert_new",
            [
                Term::Reference(id),
                objects,
                Term::Integer(done),
                Term::Integer(keypos),
                bag,
                check,
                new @ ..,
            ],
        ) = (module, function, arguments.as_slice())
        {
            let mut context = BifContext { scheduler };
            let state = ets::ListState {
                done: *done as usize,
                bag: bag.is_list().then(|| bag.clone()),
                check: ets::Check::from_term(check),
                new: new.first().map_or(ets::New::No, ets::New::from_term),
            };
            return ets::insert_list(&mut context, *id, objects, *keypos as i32, state);
        }
        if let (
            "ets",
            "select_trap",
            [
                Term::Reference(id),
                pattern,
                Term::Integer(bucket),
                Term::Integer(chunk),
                matches,
                Term::Integer(got),
                Term::Integer(writes),
            ],
        ) = (module, function, arguments.as_slice())
        {
            let state = ets::MatchState {
                chunk: *chunk as usize,
                matches: matches.clone(),
                got: *got as usize,
                writes: *writes as u64,
                resumed: true,
            };
            let mut context = BifContext { scheduler };
            return ets::match_trap(&mut context, *id, pattern, *bucket as usize, state);
        }
        if let ("ets", "lookup" | "member" | "delete" | "insert" | "insert_new", [table, key]) =
            (module, function, arguments.as_slice())
        {
            let mut context = BifContext { scheduler };
            let result = match function {
                "lookup" => ets::lookup(&mut context, table, key)?,
                "member" => ets::member(&mut context, table, key)?,
                "delete" => ets::delete(&mut context, table, key)?,
                "insert_new" => ets::insert_new(&mut context, table, key)?,
                _ => ets::insert(&mut context, table, key)?,
            };
            return Ok(result);
        }
        if let ("ets", "match", [table, pattern]) = (module, function, arguments.as_slice()) {
            let mut context = BifContext { scheduler };
            return ets::match_objects(&mut context, table, pattern, None);
        }
        if let ("ets", "match", [table, pattern, limit]) = (module, function, arguments.as_slice())
        {
            let mut context = BifContext { scheduler };
            return ets::match_objects(&mut context, table, pattern, Some(limit));
        }
        if let ("ets", "info", [table, item]) = (module, function, arguments.as_slice()) {
            let mut context = BifContext { scheduler };
            return ets::info(&mut context, table, item);
        }
        if let ("ets", "lookup_element", [table, key, index, default]) =
            (module, function, arguments.as_slice())
        {
            let mut context = BifContext { scheduler };
            return ets::lookup_element(&mut context, table, key, index, Some(default));
        }
        if let ("ets", "lookup_element", [table, key, index]) =
            (module, function, arguments.as_slice())
        {
            let mut context = BifContext { scheduler };
            return ets::lookup_element(&mut context, table, key, index, None);
        }
        if let ("ets", "delete_trap", []) = (module, function, arguments.as_slice()) {
            return ets::delete_trap(&mut BifContext { scheduler });
        }
        // The copy phase takes the result bytes out of the owned trap
        // arguments, so that no turn copies the bytes of an earlier turn
        // again.
        if let (
            "erts_internal",
            "list_to_bitstring_continue",
            [_, term, stack, Term::Integer(size), Term::Binary(bytes)],
        ) = (module, function, arguments.as_mut_slice())
            && let Some(stack) = stack.proper_list()
        {
            let bytes = std::mem::take(bytes).take_bytes();
            let process = scheduler.current_process_mut();
            let reds_left = process.bif_reductions_left()? as i64;
            return Ok(iolist::l2b_copy(
                process,
                reds_left,
                term,
                &stack.collect::<Vec<_>>(),
                *size as usize,
                bytes,
            ));
        }
        // A resume takes the lists of the state out of the owned trap
        // arguments, so that no turn copies them.
        if let (
            "erts_internal",
            "ptab_list_continue",
            [
                Term::Integer(tix),
                found,
                gone,
                Term::Integer(pid_ix),
                result,
            ],
        ) = (module, function, arguments.as_mut_slice())
        {
            let state = erlang::PtabList {
                tix: *tix,
                found: std::mem::replace(found, Term::Nil),
                gone: std::mem::replace(gone, Term::Nil),
                pid_ix: *pid_ix,
                result: std::mem::replace(result, Term::Nil),
            };
            return erlang::ptab_list(scheduler, state);
        }
        // The hidden export of `erlang:md5/1`, which is not in `bif.tab`
        // (`erl_bif_chksum.c:42-52`, `481`).
        if let ("erlang", "md5_trap", [context, rest]) = (module, function, arguments.as_slice()) {
            return hash::md5_trap(scheduler, context, rest);
        }
        if let (
            "erlang",
            "binary_find_trap",
            [subject @ Term::Binary(bytes), Term::Tuple(state), pattern],
        ) = (module, function, arguments.as_slice())
        {
            let (process, patterns) = scheduler.current_process_and_patterns_mut();
            return binary::find_trap(process, patterns, subject, bytes, state, pattern);
        }
        let (process, atoms) = scheduler.current_process_and_atoms();
        match (module, function, arguments.as_slice()) {
            ("erlang", "length", [list, Term::Integer(count)]) => {
                erlang::length(process, list, *count)
            }
            ("erlang", "++", [left, right, rest]) => {
                erlang::append_chunk(process, left, right, rest)
            }
            ("erlang", "--", state) if let Ok(state) = <&[Term; 10]>::try_from(state) => {
                subtract::continue_trap(process, atoms, state)
            }
            ("maps", "merge_trap", [merged, Term::Integer(steps)]) => {
                maps::merge_trap(process, merged.clone(), *steps as usize)
            }
            ("maps", "from_list_trap", [list, Term::Integer(done)]) => {
                maps::from_list(process, list, *done as usize, atoms)
            }
            ("maps", "from_keys_trap", [keys, value, Term::Integer(done)]) => {
                maps::from_keys(process, keys, value, *done as usize, atoms)
            }
            ("maps", "keys_trap", [map, Term::Integer(done)]) => {
                maps::keys(process, map, *done as usize)
            }
            ("maps", "values_trap", [map, Term::Integer(done)]) => {
                maps::values(process, map, *done as usize)
            }
            ("persistent_term", "get_all_trap", [all, Term::Integer(entries)]) => Ok(
                persistent_term::get_all_trap(process, all.clone(), *entries as usize),
            ),
            ("erlang", "characters_to_utf8_trap", [Term::Binary(bytes), rest, encoding]) => {
                unicode::utf8_trap(process, bytes.clone().to_bytes(), rest, encoding)
            }
            (
                "erlang",
                "characters_to_list_trap_1",
                [
                    Term::Binary(bytes),
                    Term::Integer(characters),
                    rest,
                    Term::Atom(encoding),
                ],
            ) => unicode::list_trap(
                process,
                bytes.clone().to_bytes(),
                *characters as usize,
                rest,
                encoding.is_static("latin1"),
            ),
            ("erlang", "characters_to_list_trap_2", [result, Term::Integer(remaining)]) => {
                unicode::finish_trap(
                    process,
                    result.clone(),
                    *remaining as usize,
                    "characters_to_list_trap_2",
                )
            }
            (
                "erlang",
                "characters_to_list_trap_3",
                [
                    Term::Binary(bytes),
                    Term::Integer(end),
                    Term::Integer(characters),
                ],
            ) => unicode::analyze_turn(process, bytes, *end as usize, *characters as usize),
            ("erlang", "characters_to_list_trap_4", [result, Term::Integer(remaining)]) => {
                unicode::finish_trap(
                    process,
                    result.clone(),
                    *remaining as usize,
                    "characters_to_list_trap_4",
                )
            }
            ("persistent_term", "put_common_trap", [Term::Integer(copied)]) => {
                persistent_term::put_trap(process, *copied as usize)
            }
            ("persistent_term", "erase_trap", [Term::Integer(copied)]) => {
                persistent_term::erase_trap(process, *copied as usize)
            }
            ("erts_internal", "iolist_size_continue", [term, stack, Term::Integer(size)])
                if let Some(stack) = stack.proper_list() =>
            {
                iolist::iolist_size_continue(
                    process,
                    term,
                    &stack.collect::<Vec<_>>(),
                    *size as usize,
                )
            }
            (
                "erts_internal",
                "iolist_to_iovec_continue",
                [Term::Atom(phase), arg, term, stack],
            ) if phase.is_static("size")
                && let Some(stack) = stack.proper_list() =>
            {
                iolist::iovec_size(process, arg, term, &stack.collect::<Vec<_>>())
            }
            (
                "erts_internal",
                "iolist_to_iovec_continue",
                [_, term, stack, iovec, Term::Binary(merged)],
            ) if let Some(stack) = stack.proper_list()
                && let Some(iovec) = iovec.proper_list() =>
            {
                let reds_left = process.bif_reductions_left()? as i64;
                Ok(iolist::iovec_copy(
                    process,
                    reds_left,
                    term,
                    &stack.collect::<Vec<_>>(),
                    iovec.cloned().collect(),
                    merged.clone().to_bytes(),
                ))
            }
            (
                "erts_internal",
                "binary_to_list_continue",
                [
                    Term::Binary(binary),
                    Term::Integer(offset),
                    Term::Integer(remaining),
                    list,
                ],
            ) => {
                let reds_left = process.bif_reductions_left()?;
                Ok(erlang::b2l_chunk(
                    process,
                    binary,
                    *offset as usize,
                    *remaining as usize,
                    list.clone(),
                    reds_left,
                ))
            }
            (
                "erts_internal",
                "list_to_bitstring_continue",
                [Term::Atom(phase), arg, term, stack, Term::Integer(size)],
            ) if phase.is_static("size")
                && let Some(stack) = stack.proper_list() =>
            {
                let reds_left = process.bif_reductions_left()? as i64;
                Ok(iolist::l2b_size(
                    process,
                    reds_left,
                    arg,
                    term,
                    &stack.collect::<Vec<_>>(),
                    *size as usize,
                ))
            }
            ("erlang", "bif_handle_signals_return", [_, value]) => {
                Ok(match scheduler.flush_signals()? {
                    Flushed::Done => BifResult::Value(value.clone()),
                    Flushed::Exited(reason) => BifResult::Exited(reason),
                    Flushed::Yield => BifResult::TrapBif {
                        module: "erlang",
                        function: "bif_handle_signals_return",
                        arguments: arguments.clone(),
                    },
                })
            }
            ("pbeam_cap", "await_grant", []) => pbeam_cap::await_grant(scheduler),
            ("erts_internal", "await_exit", []) => Ok(match scheduler.await_exit()? {
                Some(reason) => BifResult::Exited(reason),
                None => BifResult::TrapBif {
                    module: "erts_internal",
                    function: "await_exit",
                    arguments: Vec::new(),
                },
            }),
            _ => self.bif(scheduler, module, function, arguments),
        }
    }
}

/// The PBeam BIFs: functions of PBeam modules that ERTS does not have. They
/// are not `bif.tab` entries, and no BEAM module defines them.
const PBEAM_BIFS: [(&str, &str, u32); 5] = [
    ("pbeam_cap", "enter_runtime", 2),
    ("pbeam_cap", "policy_revoke", 2),
    ("pbeam_cap", "request", 2),
    ("pbeam_cap", "revoke", 2),
    ("pbeam_cap", "serve", 2),
];

/// Return whether an external call or a `bif` instruction goes to a BIF: the
/// MFA is an entry of `BIFS`, the `bif.tab` entries, so it runs before the
/// module code, as in ERTS, or an entry of `PBEAM_BIFS`. Any other MFA,
/// also in `erlang`, runs the BEAM code of its module.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_bif("erlang", "self", 0));
/// assert!(is_bif("maps", "keys", 1));
/// assert!(is_bif("lists", "keyfind", 3));
/// assert!(is_bif("pbeam_cap", "revoke", 2));
/// assert!(!is_bif("erlang", "spawn_monitor", 3));
/// assert!(!is_bif("maps", "keys", 2));
/// assert!(!is_bif("io", "put_chars", 2));
/// ```
pub(super) fn is_bif(module: &str, name: &str, arity: u32) -> bool {
    resolve(module, name, arity).is_some() || is_pbeam_bif(module, name, arity)
}

/// Return whether an MFA is a PBeam BIF of [`PBEAM_BIFS`]: a function of a
/// PBeam module that ERTS does not have.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_pbeam_bif("pbeam_cap", "revoke", 2));
/// assert!(!is_pbeam_bif("erlang", "self", 0));
/// ```
pub(super) fn is_pbeam_bif(module: &str, name: &str, arity: u32) -> bool {
    PBEAM_BIFS.contains(&(module, name, arity))
}

/// The heavy BIFs: the `hbif` entries of the ERTS `bif.tab`
/// (`bif.tab:64`, `74`, `131`, `132`, `134`, `180`, `470`). The ERTS loader
/// turns a `call_ext` of a heavy BIF into `i_call_ext`, which calls the BIF
/// through its export, not as a light BIF (`jit/arm/ops.tab:687-692`). Thus
/// the call costs no reduction.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(HEAVY_BIFS.contains(&("erlang", "put", 2)));
/// ```
const HEAVY_BIFS: [(&str, &str, u32); 7] = [
    ("erlang", "erase", 1),
    ("erlang", "hibernate", 3),
    ("erlang", "process_info", 1),
    ("erlang", "process_info", 2),
    ("erlang", "put", 2),
    ("erts_internal", "check_process_code", 1),
    ("erts_internal", "garbage_collect", 1),
];

/// Return whether a BIF is a heavy BIF of [`HEAVY_BIFS`]. A `call_ext` of
/// any other BIF calls it as a light BIF.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_heavy_bif("erlang", "put", 2));
/// assert!(!is_heavy_bif("erlang", "whereis", 1));
/// ```
pub(super) fn is_heavy_bif(module: &str, name: &str, arity: u32) -> bool {
    HEAVY_BIFS.contains(&(module, name, arity))
}

/// Return the call that a `call_ext` of the import `module:name/arity`
/// makes. The loader calls this one time for each import, as the ERTS
/// loader transforms the `call_ext` of each import one time
/// (`jit/arm/ops.tab:615-714`), so a call does not classify its MFA. A
/// PBeam BIF has no [`BifId`], so its call still matches the name in
/// `BeamProgram::named_bif`. The checks
/// keep the order of the removed checks of the call: `erlang:load_nif/2`
/// and `erlang:call_on_load_function/1` before the BIFs, and `erlang:apply/2,3`, which are not BIFs, at the
/// export.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(import_kind("erlang", "apply", 3), ImportKind::Apply);
/// assert_eq!(import_kind("erlang", "put", 2), ImportKind::Bif { bif: resolve("erlang", "put", 2), heavy: true });
/// assert_eq!(import_kind("lists", "reverse", 1), ImportKind::Export);
/// ```
pub(crate) fn import_kind(module: &str, name: &str, arity: u32) -> ImportKind {
    match (module, name, arity) {
        ("erlang", "apply", 2) => ImportKind::ApplyFun,
        ("erlang", "apply", 3) => ImportKind::Apply,
        ("erlang", "load_nif", 2) => ImportKind::LoadNif,
        ("erlang", "call_on_load_function", 1) => ImportKind::CallOnLoadFunction,
        _ => {
            let bif = resolve(module, name, arity);
            if bif.is_some() || is_pbeam_bif(module, name, arity) {
                ImportKind::Bif {
                    bif,
                    heavy: is_heavy_bif(module, name, arity),
                }
            } else {
                ImportKind::Export
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/interpreter/bif.rs"]
mod tests;

/// Return the extra reductions of a `links` item, `floor(Links / 4)`
/// (`erl_bif_info.c:1777`), and 0 for another item.
pub(crate) fn links_reductions(item: &Term, value: &Term) -> i32 {
    if item != &Term::static_atom("links") {
        return 0;
    }
    let links = value
        .tuple_elements()
        .and_then(|elements| elements.get(1))
        .and_then(|links| links.proper_list())
        .map(|links| links.count())
        .unwrap_or(0);
    i32::try_from(links / 4).unwrap_or(i32::MAX)
}
