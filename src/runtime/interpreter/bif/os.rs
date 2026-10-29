//! Bodies of the `os` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::{BeamError, BifResult, Scheduler, Term, denial};
use crate::runtime::capability::{CapabilityKind, Operation};
use crate::runtime::host::signal::{self, Disposition};
use crate::runtime::host::{clock, environment, filename_encoding};
use crate::runtime::unicode::{filename_from_native, native_filename};

/// Run `os:env()`: return each environment variable of the run as the
/// tuple `{Name, Value}` of a flat string and a flat string, as `os_env_0`
/// (`erl_bif_os.c:82-91`) does with `erts_osenv_foreach_term`
/// (`erl_osenv.c:228-237`). The order follows the copy of the environment
/// (see [`environment::variables`]); ERTS walks a red-black tree in an
/// unordered order, so neither runtime gives a stated order.
///
/// A caller without an `env` capability with `read` that permits the native
/// bytes of a name gets `error:capability_denied`, as `os:getenv/1` does:
/// the call names each variable of the copy, so a capability restricted to
/// some names permits only those. Each key and value follows
/// [`filename_from_native`], which gives `badarg` where ERTS composes a
/// term (see "OS environment" in `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = env(&mut scheduler)?;
/// assert!(matches!(result, BifResult::Value(list) if list.is_list()));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table, and a value that
/// PBeam cannot give (see [`filename_from_native`]).
pub(super) fn env(scheduler: &mut Scheduler) -> Result<BifResult, BeamError> {
    let encoding = filename_encoding();
    let mut pairs = Vec::new();
    for (key, value) in environment::variables() {
        if let Some(denied) = denial(scheduler, CapabilityKind::Env, Operation::Read, Some(&key))? {
            return Ok(denied);
        }
        pairs.push(Term::tuple(vec![
            filename_from_native(&key, encoding)?,
            filename_from_native(&value, encoding)?,
        ]));
    }
    Ok(Value(Term::list_from(pairs)))
}

/// Run `os:putenv(Name, Value)`: set the environment variable `Name` to
/// `Value` and return `true`, as `os_putenv_2` (`erl_bif_os.c:113-127`)
/// does with `erts_osenv_put_term` (`erl_osenv.c:269-293`). `Name` follows
/// the checks of [`getenv`], and `Value` is a flat or nested proper list of
/// characters without a character 0. Each other term raises `badarg`.
///
/// PBeam changes the environment of the run and not the `environ` of the
/// process, as ERTS on Unix does (`erts_sys_explicit_8bit_putenv`,
/// `sys/unix/sys_env.c:66-80`). A child of a port gets the `environ` of the
/// process, so it does not see the change (row "OS environment" of
/// `docs/erts-differences.md`).
///
/// After these checks, a caller without an `env` capability with `write`
/// that permits the native bytes of `Name` gets `error:capability_denied`,
/// and the binding does not change the environment.
///
/// # Examples
///
/// ```rust,ignore
/// let name = Term::list_from(b"PBEAM_PUT".iter().map(|&byte| Term::Integer(byte.into())));
/// let value = Term::list_from(b"1".iter().map(|&byte| Term::Integer(byte.into())));
/// assert!(matches!(putenv(&mut scheduler, &name, &value)?, BifResult::Value(value) if value.is_atom()));
/// assert!(matches!(putenv(&mut scheduler, &name, &Term::Integer(1))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn putenv(
    scheduler: &mut Scheduler,
    name: &Term,
    value: &Term,
) -> Result<BifResult, BeamError> {
    let encoding = filename_encoding();
    let Some(key) = native_filename(name, encoding).filter(|key| !key.contains(&b'=')) else {
        return Ok(BifResult::badarg());
    };
    let Some(value) = native_filename(value, encoding) else {
        return Ok(BifResult::badarg());
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Env, Operation::Write, Some(&key))? {
        return Ok(denied);
    }
    environment::put(&key, &value);
    Ok(Value(Term::static_atom("true")))
}

/// Run `os:unsetenv(Name)`: remove the environment variable `Name` and
/// return `true`, as `os_unsetenv_1` (`erl_bif_os.c:129-142`) does with
/// `erts_osenv_unset_term` (`erl_osenv.c:295-311`). A name that the
/// environment does not hold also gives `true`. `Name` follows the checks
/// of [`getenv`], and each other term raises `badarg`.
///
/// After these checks, a caller without an `env` capability with `write`
/// that permits the native bytes of `Name` gets `error:capability_denied`,
/// and the binding does not change the environment.
///
/// # Examples
///
/// ```rust,ignore
/// let name = Term::list_from(b"PBEAM_PUT".iter().map(|&byte| Term::Integer(byte.into())));
/// assert!(matches!(unsetenv(&mut scheduler, &name)?, BifResult::Value(value) if value.is_atom()));
/// assert!(matches!(unsetenv(&mut scheduler, &Term::static_atom("HOME"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The atom `capability_denied` in a full atom table.
pub(super) fn unsetenv(scheduler: &mut Scheduler, name: &Term) -> Result<BifResult, BeamError> {
    let encoding = filename_encoding();
    let Some(key) = native_filename(name, encoding).filter(|key| !key.contains(&b'=')) else {
        return Ok(BifResult::badarg());
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Env, Operation::Write, Some(&key))? {
        return Ok(denied);
    }
    environment::unset(&key);
    Ok(Value(Term::static_atom("true")))
}

/// Run `os:getpid()`: return the process identifier of the host as a flat
/// string of its decimal digits, as `os_getpid_0` (`erl_bif_os.c:60-69`)
/// does with `sys_get_pid` (`sys/unix/sys.c:934-939`). The digits are those
/// of `getpid()`, which `std::process::id` reads.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(getpid(), BifResult::Value(pid) if pid.is_list()));
/// ```
pub(super) fn getpid() -> BifResult {
    let digits = std::process::id().to_string();
    Value(Term::list_from(
        digits.bytes().map(|byte| Term::Integer(byte.into())),
    ))
}

/// Run `os:perf_counter()`: return the performance counter of the host in
/// the `perf_counter` unit, as `os_perf_counter_0` (`erl_time_sup.c:2601-2605`)
/// does with `erts_sys_perf_counter`. The counter is exclusive, and it is
/// non-negative. ERTS counts it from the boot of the host and PBeam counts
/// it from the first call of the run (see [`clock::perf_counter`]).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(perf_counter(), BifResult::Value(Term::Integer(counter)) if counter >= 0));
/// ```
pub(super) fn perf_counter() -> BifResult {
    Value(Term::Integer(clock::perf_counter()))
}

/// Run `os:timestamp()`: return the OS system time of the host as the tuple
/// `{MegaSecs, Secs, MicroSecs}`, as `os_timestamp_0`
/// (`erl_bif_os.c:48-59`) does with `get_sys_now`
/// (`erl_time_sup.c:1935-1950`). The value comes from the host
/// `CLOCK_REALTIME` and not from the time correction of the run, so it can
/// differ from `erlang:timestamp/0`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(timestamp(), BifResult::Value(Term::Tuple(parts)) if parts.len() == 3));
/// ```
pub(super) fn timestamp() -> BifResult {
    super::erlang::timestamp(clock::system_time())
}

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
/// The `badarg` of a `Mode` that is not valid has the `error_info` cause
/// `badopt` (`erl_bif_os.c:149-150`). After `handle`, the
/// scheduler sends `{notify, Signal}` to `erl_signal_server` for each
/// arrival of the signal (see `Scheduler::check_signals`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = set_signal(&mut scheduler, &Term::static_atom("sighup"), &Term::static_atom("ignore"))?;
/// assert!(matches!(result, BifResult::Value(Term::Atom(atom)) if atom.is_static("ok"));
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
        Term::Atom(mode) if mode.is_static("ignore") => (Disposition::Ignore, &[Operation::Ignore]),
        Term::Atom(mode) if mode.is_static("default") => (
            Disposition::Default,
            &[Operation::Handle, Operation::Ignore],
        ),
        Term::Atom(mode) if mode.is_static("handle") => (Disposition::Handle, &[Operation::Handle]),
        _ => return Ok(BifResult::badopt()),
    };
    let Term::Atom(signal) = signal else {
        return Ok(BifResult::badarg());
    };
    let Some(number) = signal::signal_number(scheduler.atoms.name_of(signal)) else {
        return Ok(BifResult::badarg());
    };
    // The capability check borrows the scheduler mutably, so the name comes
    // out of the table first.
    let name = scheduler.atoms.name_of(signal).as_bytes().to_vec();
    for &operation in operations {
        if let Some(denied) = denial(scheduler, CapabilityKind::Signal, operation, Some(&name))? {
            return Ok(denied);
        }
    }
    signal::set_disposition(number, disposition);
    Ok(Value(Term::static_atom("ok")))
}

/// Run `os:system_time()`: return the host `CLOCK_REALTIME` in the native
/// unit, as `os_system_time_0` (`erl_time_sup.c:2589-2593`) does. The value
/// is above the small-integer range, so ERTS makes a bignum and PBeam makes
/// a [`Term::Integer`] above the small-integer maximum
/// (`crate::term::SMALL_INTEGER_MAX`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(system_time_0(), BifResult::Value(Term::Integer(time)) if time > 1_700_000_000_000_000_000));
/// ```
pub(super) fn system_time_0() -> BifResult {
    Value(Term::Integer(clock::system_time()))
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
