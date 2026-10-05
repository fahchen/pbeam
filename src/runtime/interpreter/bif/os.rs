//! Bodies of the `os` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;
use crate::runtime::host::clock;
use crate::runtime::host::signal::{self, Disposition};

/// Run `os:set_signal(Signal, Mode)`: set the host disposition of `Signal`
/// to `SIG_IGN`, `SIG_DFL`, or a handler, and return `ok`, as
/// `os_set_signal_2` (`erl_bif_os.c:145-160`) does. A `Mode` that is not
/// `ignore`, `default`, or `handle` raises `badarg` first. Then a `Signal`
/// that is not an atom of `signalterm_to_signum` (sys.c:697-722) raises
/// `badarg`, for example `sigint`, `sigkill`, or `sigsegv`.
///
/// ERTS gives the `badarg` of a `Mode` that is not valid the `error_info`
/// cause `badopt`, which PBeam does not model. The PBeam handler of
/// `handle` sends no `{notify, Signal}` message to `erl_signal_server`
/// (see "OS signals" in `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = set_signal(&Term::static_atom("sighup"), &Term::static_atom("ignore"));
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.name() == "ok"));
/// let result = set_signal(&Term::static_atom("sigint"), &Term::static_atom("ignore"));
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
pub(super) fn set_signal(signal: &Term, mode: &Term) -> BifResult {
    let disposition = match mode {
        Term::Atom(mode) if mode.name() == "ignore" => Disposition::Ignore,
        Term::Atom(mode) if mode.name() == "default" => Disposition::Default,
        Term::Atom(mode) if mode.name() == "handle" => Disposition::Handle,
        _ => return BifResult::badarg(),
    };
    match signal {
        Term::Atom(signal) if signal::set_signal(signal.name(), disposition) => {
            Value(Term::static_atom("ok"))
        }
        _ => BifResult::badarg(),
    }
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
