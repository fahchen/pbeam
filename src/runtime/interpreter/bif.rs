//! BIFs: the ERTS `bif.tab` table, the BIF dispatch, and BIF results.
//!
//! The `bifs!` table in `table.rs` lists each BIF with its implementation.
//! The `binary`, `erlang`, `erts_internal`, `ets`, `file`, `iolist`,
//! `lists`, `maps`, `os`, `re`, and `unicode` modules hold the BIF bodies
//! that do not fit on one table line.
//! The `pbeam_cap` module holds the bodies of the PBeam BIFs, which are not
//! in `bif.tab`.

use super::*;
use crate::runtime::{
    capability::{CapabilityKind, Operation},
    host::output::IovecWrite,
    scheduler::Flushed,
};
use crate::term::ProcessId;

mod binary;
mod erlang;
mod erts_internal;
mod ets;
mod file;
mod iolist;
mod lists;
mod maps;
mod os;
mod pbeam_cap;
mod persistent_term;
mod re;
mod unicode;
mod ycf;

pub(super) use erlang::{is_function, send};
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

/// Build the BIF list and the BIF dispatch from one table.
///
/// Each entry is a `bif.tab` entry: `module:name/arity`, then `unsupported`
/// or one `|context, [arguments]| body` pattern, never both and never neither:
/// either mistake does not compile. A function that ERTS runs as BEAM code is
/// not a BIF and is not in this table. A pattern can have a guard:
/// `|context, [arguments]| where guard => body`. A body returns
/// `Result<BifResult, BeamError>`. A name that is not a Rust identifier, such
/// as the operator `+`, is a string literal: `erlang:"+"/2`.
///
/// The macro expands into `BIFS`, the entries as
/// `(module, name, arity)`, and `dispatch`, one `match` with an arm for each
/// pattern in table order. A call that no arm matches gives an unresolved MFA
/// error.
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
///     erlang:"+"/2 => unsupported,
///     erlang:self/0 => |cx, []| Ok(Value(Term::Pid(cx.scheduler.current_process_mut().id))),
///     lists:keyfind/3 => unsupported,
///     maps:keys/1 => |_, [map]| maps::keys(map),
/// }
/// assert_eq!(
///     BIFS,
///     &[("erlang", "+", 2), ("erlang", "self", 0), ("lists", "keyfind", 3), ("maps", "keys", 1)]
/// );
/// ```
macro_rules! bifs {
    (@form unsupported) => {};
    (@form | $context:pat_param |) => {};
    (@form $($tokens:tt)*) => {
        compile_error!("a bifs! entry needs `unsupported` or one `|context, [arguments]| body`, not both");
    };
    (@name $name:literal) => { $name };
    (@name $name:ident) => { stringify!($name) };
    (
        $(
            $module:ident : $name:tt / $arity:literal => $($unsupported:ident)?
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

        /// Run the table pattern that matches a call, or report an
        /// unresolved MFA.
        ///
        /// # Examples
        ///
        /// ```rust,ignore
        /// let result = dispatch(&mut context, "maps", "keys", &[Term::Map(Map::new())])?;
        /// assert!(matches!(result, BifResult::Value(keys) if keys.is_nil()));
        /// ```
        pub(super) fn dispatch(
            context: &mut BifContext<'_>,
            module: &str,
            name: &str,
            arguments: &[Term],
        ) -> Result<BifResult, BeamError> {
            match (module, name, arguments.len(), arguments) {
                $($(
                    (stringify!($module), bifs!(@name $name), $arity, $arguments) $(if $guard)? => {
                        (|$context: &mut BifContext<'_>| $body)(context)
                    }
                )?)*
                _ => Err(error(format!(
                    "unresolved external MFA {module}:{name}/{}",
                    arguments.len()
                ))),
            }
        }
    };
}

mod table;

pub(super) use table::BIFS;

impl BeamProgram {
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
    pub(super) fn bif(
        &self,
        scheduler: &mut Scheduler,
        module: &str,
        name: &str,
        arguments: Vec<Term>,
    ) -> Result<BifResult, BeamError> {
        match (module, name, arguments.as_slice()) {
            ("pbeam_cap", "revoke", [holder, kind]) => {
                return pbeam_cap::revoke(scheduler, holder, kind);
            }
            ("pbeam_cap", "request", [kind, request]) => {
                return pbeam_cap::request(scheduler, kind, request);
            }
            ("pbeam_cap", "policy_revoke", [holder, kind]) => {
                return pbeam_cap::policy_revoke(scheduler, holder, kind);
            }
            ("pbeam_cap", "enter_runtime", [module, function]) => {
                return pbeam_cap::enter_runtime(scheduler, module, function);
            }
            _ => {}
        }
        let mut context = BifContext { scheduler };
        table::dispatch(&mut context, module, name, &arguments)
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
    /// - `maps:merge_trap/2`, with the merged map and the steps that remain
    ///   ([`maps::merge_trap`]). ERTS traps to `maps:merge_trap/1` with a
    ///   magic reference to the walk state (`erl_map.c:134-136`).
    /// - `maps:from_list_trap/2` and `maps:keys_trap/2`, with the argument
    ///   and the YCF units done ([`maps::from_list`], [`maps::keys`]). ERTS
    ///   traps to `maps:from_list/1` and `maps:keys/1` with a magic
    ///   reference to the YCF state (`utils.c:3718-3731`).
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
    /// - `ets:insert/4`, with the table reference, the objects, the units
    ///   done, and the key position ([`ets::insert_list`]). ERTS traps to
    ///   `ets:insert/2` with a magic reference to the YCF state
    ///   (`erl_db.c:2297-2332`).
    /// - `erts_internal:list_to_bitstring_continue/5`, with the phase and the
    ///   walk state of `erlang:iolist_to_binary/1` and `erlang:list_to_binary/1`
    ///   ([`iolist::l2b_size`], [`iolist::l2b_copy`]). ERTS traps to
    ///   `erts_internal:list_to_bitstring_continue/1` with a magic reference
    ///   to the walk state (`erl_iolist.c:53-55`, `764-835`).
    /// - `erlang:binary_find_trap/3`, with the subject, the search state, and
    ///   the pattern of `binary:match/2` or `binary:split/2,3`
    ///   ([`binary::find_trap`]). ERTS traps to the same export with magic references to the search state
    ///   and to the compiled pattern (`erl_bif_binary.c:75-77`, `1935-1952`).
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
            "insert",
            [
                Term::Reference(id),
                objects,
                Term::Integer(done),
                Term::Integer(keypos),
            ],
        ) = (module, function, arguments.as_slice())
        {
            let mut context = BifContext { scheduler };
            return ets::insert_list(&mut context, *id, objects, *done as usize, *keypos as i32);
        }
        // The copy phase takes the result bytes out of the owned trap
        // arguments, so that no turn copies them again.
        if let (
            "erts_internal",
            "list_to_bitstring_continue",
            [_, term, stack, Term::Integer(size), Term::Binary(bytes)],
        ) = (module, function, arguments.as_mut_slice())
            && let Some(stack) = stack.proper_list()
        {
            let process = scheduler.current_process_mut();
            let reds_left = process.bif_reductions_left()? as i64;
            return Ok(iolist::l2b_copy(
                process,
                reds_left,
                term,
                &stack.collect::<Vec<_>>(),
                *size as usize,
                std::mem::take(bytes),
            ));
        }
        let process = scheduler.current_process_mut();
        match (module, function, arguments.as_slice()) {
            ("erlang", "length", [list, Term::Integer(count)]) => {
                erlang::length(process, list, *count)
            }
            ("erlang", "++", [left, right, rest]) => {
                erlang::append_chunk(process, left, right, rest)
            }
            ("maps", "merge_trap", [merged, Term::Integer(steps)]) => {
                maps::merge_trap(process, merged.clone(), *steps as usize)
            }
            ("maps", "from_list_trap", [list, Term::Integer(done)]) => {
                maps::from_list(process, list, *done as usize)
            }
            ("maps", "keys_trap", [map, Term::Integer(done)]) => {
                maps::keys(process, map, *done as usize)
            }
            ("persistent_term", "get_all_trap", [all, Term::Integer(entries)]) => Ok(
                persistent_term::get_all_trap(process, all.clone(), *entries as usize),
            ),
            ("erlang", "characters_to_utf8_trap", [Term::Binary(bytes), rest, encoding]) => {
                unicode::utf8_trap(process, bytes.clone(), rest, encoding)
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
                bytes.clone(),
                *characters as usize,
                rest,
                encoding.name() == "latin1",
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
            ) if phase.name() == "size"
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
                    merged.clone(),
                ))
            }
            (
                "erts_internal",
                "list_to_bitstring_continue",
                [Term::Atom(phase), arg, term, stack, Term::Integer(size)],
            ) if phase.name() == "size"
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
            (
                "erlang",
                "binary_find_trap",
                [subject @ Term::Binary(bytes), Term::Tuple(state), pattern],
            ) => binary::find_trap(process, subject, bytes, state, pattern),
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
const PBEAM_BIFS: [(&str, &str, u32); 4] = [
    ("pbeam_cap", "enter_runtime", 2),
    ("pbeam_cap", "policy_revoke", 2),
    ("pbeam_cap", "request", 2),
    ("pbeam_cap", "revoke", 2),
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
    BIFS.binary_search(&(module, name, arity)).is_ok()
        || PBEAM_BIFS.contains(&(module, name, arity))
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
