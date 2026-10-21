//! Bodies of the timer BIFs that do not fit on one table line: the absolute
//! forms of `erlang:start_timer/4` and `erlang:send_after/4`, and
//! `erlang:read_timer/1`.

use super::BifResult::Value;
use super::*;
use crate::runtime::capability::{CapabilityKind, Operation};
use crate::runtime::process::TimerReference;
use crate::runtime::timer::{Receiver, Timer, Timers};

/// Run `erlang:start_timer(Time, Dest, Msg, Options)`, as ERTS `start_timer_4`
/// (`erl_hl_timer.c:2622-2642`) does: parse the option list
/// (`erl_hl_timer.c:2352-2398`), then run the checks and make the timer of
/// [`setup`] with the message `{timeout, Reference, Msg}`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = start_timer_4(&mut context, &Term::Integer(100), &Term::Pid(pid), &msg, &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// The errors of [`setup`].
pub(super) fn start_timer_4(
    context: &mut BifContext<'_>,
    time: &Term,
    receiver: &Term,
    message: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    setup_4(context, time, receiver, message, options, true)
}

/// Run `erlang:send_after(Time, Dest, Msg, Options)`, as ERTS `send_after_4`
/// (`erl_hl_timer.c:2583-2603`) does: the option list and the checks of
/// [`start_timer_4`], and [`setup`] with `wrap` 0, so `Dest` gets `Msg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = send_after_4(&mut context, &Term::Integer(100), &Term::Pid(pid), &msg, &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// The errors of [`setup`].
pub(super) fn send_after_4(
    context: &mut BifContext<'_>,
    time: &Term,
    receiver: &Term,
    message: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    setup_4(context, time, receiver, message, options, false)
}

/// Start a timer of [`start_timer_4`] and [`send_after_4`]. A bad option
/// list gives `badarg` with the `error_info` cause `badopt`, and a bad time
/// gives `badarg` with the cause `time`, as `BIF_P->fvalue` does
/// (`erl_hl_timer.c:2588-2598`, `2627-2637`). Then run the checks and make
/// the timer of [`setup`].
///
/// # Examples
///
/// ```rust,ignore
/// let result = setup_4(&mut context, &Term::Integer(100), &Term::Pid(pid), &msg, &Term::nil(), false)?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// The errors of [`setup`].
fn setup_4(
    context: &mut BifContext<'_>,
    time: &Term,
    receiver: &Term,
    message: &Term,
    options: &Term,
    wrap: bool,
) -> Result<BifResult, BeamError> {
    let Some(absolute) = absolute_option(options) else {
        return Ok(BifResult::badopt());
    };
    let scheduler = &mut *context.scheduler;
    let end = match *time {
        Term::Integer(msec) if absolute => Timers::absolute_position(msec),
        Term::Integer(msec) => Timers::timeout_position(scheduler.timers.now(), msec),
        _ => None,
    };
    let Some(end) = end else {
        return Ok(BifResult::ErrorWithCause {
            reason: Term::static_atom("badarg"),
            cause: Term::static_atom("time"),
        });
    };
    setup(scheduler, end, receiver, message, wrap)
}

/// Read the options of `start_timer/4` and `send_after/4` as
/// `parse_bif_timer_options` (`erl_hl_timer.c:2352-2398`) does with only the
/// `abs` option: return `Some(true)` when the last `{abs, true}` sets an
/// absolute time, `Some(false)` for a list without it, and `None` for a list
/// that is not valid. Each `{async, _}` and `{info, _}` element is also a
/// `badarg` here, because those BIFs pass no pointer for them.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(absolute_option(&Term::nil()), Some(false));
/// assert_eq!(absolute_option(&Term::static_atom("abs")), None);
/// ```
fn absolute_option(options: &Term) -> Option<bool> {
    let options = options.proper_list()?;
    let mut absolute = false;
    for option in options {
        let Term::Tuple(fields) = option else {
            return None;
        };
        let [Term::Atom(name), Term::Atom(value)] = fields.as_ref() else {
            return None;
        };
        let value = if value.is_static("true") {
            true
        } else if value.is_static("false") {
            false
        } else {
            return None;
        };
        if !name.is_static("abs") {
            return None;
        }
        absolute = value;
    }
    Some(absolute)
}

/// Run `erlang:read_timer(TimerRef)`, as ERTS `read_timer_1`
/// (`erl_hl_timer.c:2661-2664`) does through `access_bif_timer`
/// (`erl_hl_timer.c:2237-2340`): a term that is not a reference raises
/// `badarg`; a reference of an active BIF timer gives the milliseconds left
/// (`get_time_left`, `erl_hl_timer.c:681-689`); each other reference gives
/// `false`. A read does not cancel the timer. Each reference of PBeam is a
/// reference of its scheduler, so the `no_timer_result` path
/// (`erl_hl_timer.c:2209-2234`) covers a reference that has no timer.
///
/// # Examples
///
/// ```rust,ignore
/// let result = read_timer(&mut context, &Term::Reference(reference))?;
/// assert!(matches!(result, BifResult::Value(Term::Integer(_))));
/// ```
///
/// # Errors
///
/// The errors of [`Timers::now`] give no error, and this function adds
/// none.
pub(super) fn read_timer(
    context: &mut BifContext<'_>,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    let Term::Reference(reference) = reference else {
        return Ok(BifResult::badarg());
    };
    let scheduler = &mut *context.scheduler;
    // The timer queue stays with each scheduler, so a timer of another
    // scheduler cannot be read here.
    if scheduler.remote_timer_owner(*reference).is_some() {
        return Err(error(
            "unsupported: a BIF timer of another scheduler in a run of execute_on_schedulers",
        ));
    }
    let result = match scheduler.timers.bif_timer_end(*reference) {
        Some(end) => Term::Integer(Timers::time_left(scheduler.timers.now(), end)),
        None => Term::static_atom("false"),
    };
    Ok(Value(result))
}

/// Start the BIF timer of [`start_timer_4`] and [`send_after_4`] at the end
/// tick `end`, as `setup_bif_timer` (`erl_hl_timer.c:1804-1871`) does:
/// `Dest` is a pid or an atom, else `badarg`; the capability check and the
/// runtime to boot gate run before the first effect; a new reference goes
/// into the timer tree of a live pid; and `wrap` makes the message
/// `{timeout, Reference, Msg}`, and else the message is `Msg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = setup(&mut scheduler, 1100, &Term::Pid(pid), &msg, false)?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// The errors of [`Scheduler::make_ref`], [`denial`], and [`boot_denial`].
fn setup(
    scheduler: &mut Scheduler,
    end: i64,
    receiver: &Term,
    message: &Term,
    wrap: bool,
) -> Result<BifResult, BeamError> {
    let receiver = match receiver {
        Term::Pid(id) => Receiver::Process(*id),
        Term::Atom(name) => Receiver::Name(name.clone()),
        _ => return Ok(BifResult::badarg()),
    };
    if matches!(receiver, Receiver::Name(_)) && scheduler.is_multi() {
        return Err(error(
            "unsupported: a named BIF timer in a run of execute_on_schedulers",
        ));
    }
    if let Some(denied) = denial(scheduler, CapabilityKind::Process, Operation::Send, None)? {
        return Ok(denied);
    }
    let target = match &receiver {
        Receiver::Process(id) => Some(*id),
        Receiver::Name(name) => scheduler.whereis(scheduler.atoms.name_of(name)),
    };
    // The timer queue stays with each scheduler, so a timer that fires on
    // another scheduler cannot reach its receiver.
    if target.is_some_and(|target| scheduler.remote_owner(target).is_some()) {
        return Err(error(
            "unsupported: a BIF timer to a process of another scheduler in a run of execute_on_schedulers",
        ));
    }
    if let Some(target) = target
        && let Some(denied) = boot_denial(scheduler, target)?
    {
        return Ok(denied);
    }
    let reference = scheduler.make_ref()?;
    if !matches!(receiver, Receiver::Process(id) if scheduler.live_process(id).is_none()) {
        if let Receiver::Process(id) = receiver
            && let Some(process) = scheduler.process_mut(id)
        {
            process.bif_timers.insert(TimerReference(reference), ());
        }
        let message = if wrap {
            Term::tuple(vec![
                Term::static_atom("timeout"),
                Term::Reference(reference),
                message.clone(),
            ])
        } else {
            message.clone()
        };
        scheduler.timers.start(
            end,
            Timer::Bif {
                reference,
                receiver,
                message,
            },
        );
        scheduler
            .node()
            .register_timer(reference, scheduler.index());
    }
    Ok(Value(Term::Reference(reference)))
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/timer.rs"]
mod tests;
