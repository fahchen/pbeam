//! BIFs: the ERTS `bif.tab` table, the BIF dispatch, and BIF results.
//!
//! The `bifs!` table in `table.rs` lists each BIF with its implementation.
//! The `erlang`, `maps`, and `unicode` modules hold the BIF bodies that do
//! not fit on one table line.

use super::*;

mod erlang;
mod maps;
mod unicode;

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
}

impl BifResult {
    /// Return the value of a BIF called through a `bif` instruction with
    /// fail label 0. An exception or a trap from such a BIF is not supported.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = BifResult::Value(Term::Integer(1)).into_bif_value("erlang", "self", 0)?;
    /// assert_eq!(value, Term::Integer(1));
    /// let raise = BifResult::Raise { class: ExceptionClass::Exit, reason: Term::static_atom("badarg") };
    /// assert!(raise.into_bif_value("erlang", "node", 1).is_err());
    /// ```
    pub(super) fn into_bif_value(
        self,
        module: &str,
        name: &str,
        arity: usize,
    ) -> Result<Term, BeamError> {
        match self {
            BifResult::Value(value) => Ok(value),
            // ponytail: no BIF that a `bif` instruction reaches raises or traps; add it when one does.
            other => Err(error(format!(
                "unsupported {module}:{name}/{arity} result {other:?}"
            ))),
        }
    }
}

/// The state that a BIF body can use.
struct BifContext<'a> {
    /// The loaded program, in which a spawn BIF finds the start function.
    program: &'a BeamProgram,
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
        /// assert!(matches!(result, BifResult::Value(Term::List(_))));
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
        let mut context = BifContext {
            program: self,
            scheduler,
        };
        table::dispatch(&mut context, module, name, &arguments)
    }
}

/// Return whether an external call or a `bif` instruction goes to a BIF. An
/// entry of `BIFS` runs before the module code, as in ERTS. Every `erlang`
/// call goes to the BIF dispatch, which reports an unresolved MFA.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_bif("erlang", "self", 0));
/// assert!(is_bif("maps", "keys", 1));
/// assert!(is_bif("lists", "keyfind", 3));
/// assert!(!is_bif("maps", "keys", 2));
/// assert!(!is_bif("io", "put_chars", 2));
/// ```
pub(super) fn is_bif(module: &str, name: &str, arity: u32) -> bool {
    module == "erlang" || BIFS.binary_search(&(module, name, arity)).is_ok()
}
