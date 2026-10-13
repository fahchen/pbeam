//! Bodies of the `os` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;
use crate::runtime::capability::{CapabilityKind, Operation};
use crate::runtime::host::signal::{self, Disposition};
use crate::runtime::host::{clock, environment, filename_encoding};
use crate::runtime::unicode::{filename_from_native, native_filename};
use crate::term::Atom;

/// Run `os:getenv(Name)`: return the value of the environment variable
/// `Name` as a list of characters, or `false` when the environment has no
/// such variable, as `os_getenv_1` (`erl_bif_os.c:94-110`) does with
/// `erts_osenv_get_term` (`erl_osenv.c:239-262`).
///
/// `convert_key_to_native` (`erl_osenv.c:103-165`) makes the native bytes
/// of `Name` in the file name encoding of the host (see
/// [`native_filename`]). `[]` and a flat or nested proper list of
/// characters are valid. An atom, a binary (also in a list), an improper
/// list, a character 0, a character that the encoding cannot hold, and a
/// name with the byte `=` raise `badarg`. The empty name is valid, because
/// the `length == 0` test of ERTS compares a pointer. After these checks,
/// a caller without an `env` capability with `read` that permits the
/// native bytes of `Name` gets `error:capability_denied`, and the binding
/// does not read the environment. The lookup and the
/// value follow [`environment::variable`] and [`filename_from_native`],
/// which on macOS returns a runtime error for a value that ERTS composes
/// (see "OS environment" in `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// // PBEAM_GETENV_ASCII=yes
/// let name = Term::list_from(b"PBEAM_GETENV_ASCII".iter().map(|&byte| Term::Integer(byte.into())));
/// assert!(matches!(getenv(&mut scheduler, &name)?, BifResult::Value(value) if value.is_list()));
/// assert!(matches!(getenv(&mut scheduler, &Term::static_atom("HOME"))?, BifResult::Raise { .. }));
/// assert!(matches!(getenv(&mut scheduler, &Term::binary(b"HOME".to_vec()))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// A value that PBeam cannot give (see [`filename_from_native`]), and the
/// atom `capability_denied` in a full atom table.
pub(super) fn getenv(scheduler: &mut Scheduler, name: &Term) -> Result<BifResult, BeamError> {
    let encoding = filename_encoding();
    let Some(key) = native_filename(name, encoding).filter(|key| !key.contains(&b'=')) else {
        return Ok(BifResult::badarg());
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Env, Operation::Read, Some(&key))? {
        return Ok(denied);
    }
    Ok(Value(match environment::variable(&key) {
        Some(value) => filename_from_native(&value, encoding)?,
        None => Term::static_atom("false"),
    }))
}

/// Run `os:set_signal(Signal, Mode)`: set the host disposition of `Signal`
/// to `SIG_IGN`, `SIG_DFL`, or a handler, and return `ok`, as
/// `os_set_signal_2` (`erl_bif_os.c:145-160`) does. A `Mode` that is not
/// `ignore`, `default`, or `handle` raises `badarg` first. Then a `Signal`
/// that is not an atom of `signalterm_to_signum` (sys.c:697-722) raises
/// `badarg`, for example `sigint`, `sigkill`, or `sigsegv`.
///
/// After these checks, the caller needs a `signal` capability that permits
/// the name of `Signal`, with `ignore` for `ignore`, with `handle` for
/// `handle`, and with both operations for `default`. Else the binding
/// raises `error:capability_denied` and does not change the disposition.
///
/// ERTS gives the `badarg` of a `Mode` that is not valid the `error_info`
/// cause `badopt`, which PBeam does not model. After `handle`, the
/// scheduler sends `{notify, Signal}` to `erl_signal_server` for each
/// arrival of the signal (see `Scheduler::check_signals`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = set_signal(&mut scheduler, &Term::static_atom("sighup"), &Term::static_atom("ignore"))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "ok"));
/// let result = set_signal(&mut scheduler, &Term::static_atom("sigint"), &Term::static_atom("ignore"))?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn set_signal(
    scheduler: &mut Scheduler,
    signal: &Term,
    mode: &Term,
) -> Result<BifResult, BeamError> {
    let (disposition, operations): (_, &[Operation]) = match mode {
        Term::Atom(mode) if mode.index() == Atom::static_index("ignore") => {
            (Disposition::Ignore, &[Operation::Ignore])
        }
        Term::Atom(mode) if mode.index() == Atom::static_index("default") => (
            Disposition::Default,
            &[Operation::Handle, Operation::Ignore],
        ),
        Term::Atom(mode) if mode.index() == Atom::static_index("handle") => {
            (Disposition::Handle, &[Operation::Handle])
        }
        _ => return Ok(BifResult::badarg()),
    };
    let Term::Atom(signal) = signal else {
        return Ok(BifResult::badarg());
    };
    let Some(number) = signal::signal_number(signal.name()) else {
        return Ok(BifResult::badarg());
    };
    for &operation in operations {
        let name = Some(signal.name().as_bytes());
        if let Some(denied) = denial(scheduler, CapabilityKind::Signal, operation, name)? {
            return Ok(denied);
        }
    }
    signal::set_disposition(number, disposition);
    Ok(Value(Term::static_atom("ok")))
}

/// Run `os:system_time(Unit)`: convert the host `CLOCK_REALTIME` in
/// nanoseconds to `Unit` with no offset, as `os_system_time_1`
/// (`erl_time_sup.c:2595-2599`) does. A host time before 1970 traps for
/// each unit (see [`super::erlang::convert_time`]).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(system_time(&Term::static_atom("second")), BifResult::Value(Term::Integer(_))));
/// assert!(matches!(system_time(&Term::Integer(3)), BifResult::Trap { .. }));
/// assert!(matches!(system_time(&Term::Integer(0)), BifResult::Raise { .. }));
/// ```
pub(super) fn system_time(unit: &Term) -> BifResult {
    super::erlang::convert_time(clock::system_time(), 0, unit)
}
