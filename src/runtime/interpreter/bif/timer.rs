//! Bodies of the timer BIFs that do not fit on one table line: the absolute
//! forms of `erlang:start_timer/4` and `erlang:send_after/4`, and
//! `erlang:read_timer/1`.

use super::BifResult::Value;
use super::{BeamError, BifContext, BifResult, Scheduler, Term, boot_denial, denial};
use crate::runtime::capability::{CapabilityKind, Operation};
use crate::runtime::scheduler::node::ReceiverSite;
use crate::runtime::timer::{Receiver, Timer, Timers};

/// The BIF that starts a BIF timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TimerKind {
    /// `erlang:send_after/3,4`: the receiver gets the message as it is.
    SendAfter,
    /// `erlang:start_timer/3,4`: the receiver gets `{timeout, Reference, Msg}`.
    StartTimer,
}

/// How `erlang:read_timer/2` gives the time left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadReply {
    /// The call returns the value (`{async, false}`).
    Value,
    /// The caller gets the message `{read_timer, TimerRef, Result}`, and the
    /// call returns `ok` (`{async, true}`).
    Message,
}

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
    setup_4(
        context,
        time,
        receiver,
        message,
        options,
        TimerKind::StartTimer,
    )
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
    setup_4(
        context,
        time,
        receiver,
        message,
        options,
        TimerKind::SendAfter,
    )
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
/// let result = setup_4(&mut context, &Term::Integer(100), &Term::Pid(pid), &msg, &Term::nil(), TimerKind::SendAfter)?;
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
    kind: TimerKind,
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
        return Ok(BifResult::badarg_with_cause("time"));
    };
    setup(scheduler, end, receiver, message, kind)
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
/// (`erl_hl_timer.c:2661-2664`) does: [`read`] without `async`.
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
/// This function gives no error.
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn read_timer(
    context: &mut BifContext<'_>,
    reference: &Term,
) -> Result<BifResult, BeamError> {
    Ok(read(context, reference, ReadReply::Value))
}

/// Run `erlang:read_timer(TimerRef, Options)`, as ERTS `read_timer_2`
/// (`erl_hl_timer.c:2666-2676`) does: `parse_bif_timer_options`
/// (`erl_hl_timer.c:2352-2398`) reads a list of `{async, Bool}`, and the
/// last value wins. Each other element, also `{info, Bool}` and
/// `{abs, Bool}`, raises `badarg`, because `read_timer_2` passes no pointer
/// for them. The options are checked before `TimerRef`. Then [`read`] runs
/// with `async`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = read_timer_2(&mut context, &Term::Reference(reference), &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Integer(_))));
/// ```
///
/// # Errors
///
/// This function gives no error.
#[expect(
    clippy::unnecessary_wraps,
    reason = "BIF entry of the dispatch table in `table.rs`; every BIF returns `Result<BifResult, BeamError>`"
)]
pub(super) fn read_timer_2(
    context: &mut BifContext<'_>,
    reference: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let Some(reply) = async_option(options) else {
        return Ok(BifResult::badarg());
    };
    Ok(read(context, reference, reply))
}

/// Read the options of `read_timer/2` as `parse_bif_timer_options`
/// (`erl_hl_timer.c:2352-2398`) does with only the `async` option. Return
/// `None` for a list that is not valid.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(async_option(&Term::nil()), Some(ReadReply::Value));
/// assert_eq!(async_option(&Term::static_atom("async")), None);
/// ```
fn async_option(options: &Term) -> Option<ReadReply> {
    let mut reply = ReadReply::Value;
    for option in options.proper_list()? {
        let Term::Tuple(fields) = option else {
            return None;
        };
        let [Term::Atom(name), Term::Atom(value)] = fields.as_ref() else {
            return None;
        };
        if !name.is_static("async") {
            return None;
        }
        reply = if value.is_static("true") {
            ReadReply::Message
        } else if value.is_static("false") {
            ReadReply::Value
        } else {
            return None;
        };
    }
    Some(reply)
}

/// Read a BIF timer as `access_bif_timer` (`erl_hl_timer.c:2237-2340`)
/// does without a cancel and with `info`: a term that is not a reference
/// raises `badarg`; a reference of an active BIF timer gives the
/// milliseconds left (`get_time_left`, `erl_hl_timer.c:681-689`); each
/// other reference gives `false`. A read does not cancel the timer. With
/// `async`, the caller gets the message `{read_timer, TimerRef, Result}`
/// and the call gives `ok` (`send_async_info`, `erl_hl_timer.c:1992-2034`;
/// `no_timer_result`, `erl_hl_timer.c:2209-2234`).
///
/// The node holds the end of each active timer, so a read of a timer of
/// another scheduler gives its result at once. For such a timer, ERTS
/// sends the request to the owner scheduler and gives `ok` at once
/// (`erl_hl_timer.c:2272-2295`); the reply comes later. A read changes no
/// state, so the message and its value are the same.
fn read(context: &mut BifContext<'_>, reference: &Term, reply: ReadReply) -> BifResult {
    let Term::Reference(reference) = reference else {
        return BifResult::badarg();
    };
    let scheduler = &mut *context.scheduler;
    let result = match scheduler.node().timer_end(*reference) {
        Some(end) => Term::Integer(Timers::time_left(scheduler.timers.now(), end)),
        None => Term::static_atom("false"),
    };
    if reply == ReadReply::Value {
        return Value(result);
    }
    let caller = scheduler.current_process_mut().id;
    let message = Term::tuple(vec![
        Term::static_atom("read_timer"),
        Term::Reference(*reference),
        result,
    ]);
    scheduler.deliver(caller, message);
    Value(Term::static_atom("ok"))
}

/// Start the BIF timer of [`start_timer_4`] and [`send_after_4`] at the end
/// tick `end`, as `setup_bif_timer` (`erl_hl_timer.c:1804-1871`) does:
/// `Dest` is a pid or an atom, else `badarg`; the capability check and the
/// runtime to boot gate run before the first effect; a new reference goes
/// into the timer tree of a live pid; and [`TimerKind::StartTimer`] makes
/// the message `{timeout, Reference, Msg}`, and else the message is `Msg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = setup(&mut scheduler, 1100, &Term::Pid(pid), &msg, TimerKind::SendAfter)?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// ```
///
/// # Errors
///
/// The errors of [`Scheduler::make_ref`], [`denial`], and [`boot_denial`].
pub(super) fn setup(
    scheduler: &mut Scheduler,
    end: i64,
    receiver: &Term,
    message: &Term,
    kind: TimerKind,
) -> Result<BifResult, BeamError> {
    let receiver = match receiver {
        Term::Pid(id) => Receiver::Process(*id),
        Term::Atom(name) => Receiver::Name(name.clone()),
        _ => return Ok(BifResult::badarg()),
    };
    if let Some(denied) = denial(scheduler, CapabilityKind::Process, Operation::Send, None)? {
        return Ok(denied);
    }
    let target = match &receiver {
        Receiver::Process(id) => Some(*id),
        Receiver::Name(name) => scheduler.whereis(scheduler.atoms.name_of(name)),
    };
    if let Some(target) = target
        && let Some(denied) = boot_denial(scheduler, target)?
    {
        return Ok(denied);
    }
    let reference = scheduler.make_ref()?;
    let index = scheduler.index();
    // The timer stays with this scheduler. The node holds the timer tree
    // of the receiver, so each scheduler can cancel the timer.
    let node = scheduler.node();
    let active = match receiver {
        Receiver::Process(id) => match scheduler.remote_owner(id) {
            Some(_) => node.register_timer(reference, index, end, Some(id), ReceiverSite::Remote),
            None => {
                scheduler.live_process(id).is_some()
                    && node.register_timer(reference, index, end, Some(id), ReceiverSite::Local)
            }
        },
        Receiver::Name(_) => node.register_timer(reference, index, end, None, ReceiverSite::Local),
    };
    if active {
        let message = if kind == TimerKind::StartTimer {
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
    }
    Ok(Value(Term::Reference(reference)))
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/timer.rs"]
mod tests;
