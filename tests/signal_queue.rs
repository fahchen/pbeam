//! The signal queue on one scheduler: the time when a process handles a
//! control signal, the order of messages and signals, and the reductions
//! of the handling, compared with pinned ERTS (`erl +S 1`).

mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const SIGNAL_QUEUE_PROBE: &[u8] =
    include_bytes!("../fixtures/signal_queue/signal_queue_probe.beam");

/// The result of each case of `signal_queue_probe:all/0` on pinned ERTS
/// 29.1.1 with one scheduler, as `io:format("~w")` prints it.
const CASES: &[(&str, &str)] = &[
    // The monitor down signal of a dead target is not handled yet, so
    // `demonitor/2` finds the monitor; the signal is then dropped.
    ("demonitor_dead", "{true,[]}"),
    ("demonitor_dead_received", "{noproc,false,[]}"),
    ("demonitor_name", "{true,[]}"),
    // The kill signal is handled when the target runs, after the message
    // to the caller is sent.
    (
        "exit_then_self_msg",
        "{killed,[after_exit,{'EXIT',killed}]}",
    ),
    ("exit_then_monitor", "{noproc,[]}"),
    ("exit_then_link_trap", "{true,killed,{'EXIT',noproc},[]}"),
    ("unlink_after_exit", "{true,boom,[]}"),
    ("unlink_after_exit_trap", "{true,{'EXIT',boom},[]}"),
    ("alive_after_exit", "false"),
    ("info_after_exit", "undefined"),
    ("gl_after_exit", "badarg"),
    ("gl_after_msg", "false"),
    ("gl_after_link", "false"),
    ("gl_order", "[c,after_gl]"),
    ("gl_set_order", "[after_gl,c]"),
    // A control signal puts a waiting process in the run queue at once;
    // a monitor signal waits in the pending slot.
    ("link_order", "[b,c]"),
    ("monitor_order", "[c,b]"),
    ("monitor_message_order", "[c,b]"),
    ("demonitor_order", "[b,c]"),
    ("exit_trap_order", "[b,c]"),
    ("exit_normal_order", "[b,c]"),
    ("unlink_order", "[c,b]"),
    ("link_again_order", "[c,b]"),
    ("spawn_link_order", "[c,b]"),
    ("monitor_dead_order", "[{'DOWN',noproc},c]"),
    ("sender_order", "[m1,{'EXIT',e1},m2,{'EXIT',boom}]"),
    ("down_after_exit", "{normal,[{'EXIT',normal}]}"),
    ("alias_after_down", "[{'DOWN',normal}]"),
    ("alias_kill", "[c,{'DOWN',killed}]"),
    ("alias_exit_then_kill", "boom"),
    ("alive_after_monitor", "true"),
    // A BIF that read another process traps to handle the signals of its
    // caller, and the trap switches out the caller.
    ("alive_self_message", "[x,c,y]"),
    ("alive_no_message", "[y,c]"),
    ("alive_trap", "[c,y]"),
    ("info_self_message", "[x,c,y]"),
    ("info_request", "{{message_queue_len,1},[c,y]}"),
    ("monitor_then_kill", "killed"),
    ("demonitor_then_kill", "[]"),
    ("alias_self_flush", "{true,[]}"),
    ("kill_runnable", "[{'EXIT',killed},c]"),
    ("unlink_dead", "true"),
    // `exit(self(), Reason)` ends the caller in its next turn.
    ("self_kill", "[c,{'DOWN',killed}]"),
    ("self_boom", "[c,{'DOWN',boom}]"),
    ("self_normal", "[c,{'DOWN',normal}]"),
    ("self_trap", "[{ret,true},{'EXIT',boom},{'DOWN',normal},c]"),
    ("alias_alive_request", "badarg"),
    // A request reply of a process that exits, and of a pid that no
    // process has.
    ("alive_request_exit", "false"),
    ("alive_request_dead", "{false,false}"),
    ("gl_request_dead", "badarg"),
    (
        "request_badargs",
        "[badarg,badarg,badarg,badarg,badarg,badarg]",
    ),
    ("exiting_target", "{badarg,undefined}"),
    ("reply_to_exiting", "killed"),
    // The cases below send many signals to the case process before it
    // handles them, so the handling stops at its limit and continues in a
    // later turn.
    ("unlink_unhandled_exit", "{true,[]}"),
    ("hibernate_signals", "m"),
    ("pending_replace", "{noproc,normal}"),
    ("alias_to_dead_owner", "{false,badarg,[]}"),
    ("info_request_backlog", "{message_queue_len,1}"),
    ("marker_exit", "{died,killed}"),
    ("flush_exit", "{died,killed}"),
    ("flush_yield", "{message_queue_len,1}"),
    ("return_exit", "{died,killed}"),
    ("return_yield", "true"),
    ("await_yield", "{died,killed}"),
    ("receive_yield", "ok"),
];

/// Return the `~w` text of a term of atoms, integers, lists, and tuples. A
/// pid is `<pid>`, and a reference is `#Ref`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::List(vec![])), "[]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => {
            let name = atom.name();
            let bare = name.starts_with(|c: char| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@');
            if bare {
                name.to_owned()
            } else {
                format!("'{name}'")
            }
        }
        Term::Integer(value) => value.to_string(),
        Term::List(elements) => format!("[{}]", join(elements)),
        Term::Tuple(elements) => format!("{{{}}}", join(elements)),
        // The expected results hold no pid and no reference. A wrong result
        // can hold one, so the text shows it for the failure message.
        Term::Pid(_) => "<pid>".to_owned(),
        Term::Reference(_) => "#Ref".to_owned(),
        other => panic!("the probe returns no {other:?}"),
    }
}

/// Return the expected `~w` text of `all/0` of a probe from its cases.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(expected(&[("a", "1")]), "[{a,1}]");
/// ```
fn expected(cases: &[(&str, &str)]) -> String {
    let cases: Vec<String> = cases
        .iter()
        .map(|(name, value)| format!("{{{name},{value}}}"))
        .collect();
    format!("[{}]", cases.join(","))
}

/// Run `Module:all()` on pinned ERTS with one scheduler and return the
/// `~w` text.
///
/// # Examples
///
/// ```rust,ignore
/// let text = erts_all("signal_queue_probe");
/// ```
fn erts_all(module: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/signal_queue",
            "-eval",
            &format!("io:format(\"~w~n\", [{module}:all()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    String::from_utf8(reference.stdout).unwrap()
}

/// Run `Module:all()` on PBeam with the pinned `erlang` and
/// `erts_internal` modules, and return the `~w` text.
///
/// # Examples
///
/// ```rust,ignore
/// let text = pbeam_all("signal_queue_probe", SIGNAL_QUEUE_PROBE);
/// ```
fn pbeam_all(module: &str, probe: &[u8]) -> String {
    let erlang = pinned_beam("erlang");
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erlang, &erts_internal, probe]).unwrap();
    text(&program.execute(module, "all", &[]).unwrap())
}

#[test]
fn handles_control_signals_when_the_receiver_handles_its_signals_as_pinned_erts() {
    let expected = expected(CASES);
    assert_eq!(
        pbeam_all("signal_queue_probe", SIGNAL_QUEUE_PROBE),
        expected
    );
    assert_eq!(erts_all("signal_queue_probe"), format!("{expected}\n"));
}

const SIGNAL_REDS_PROBE: &[u8] = include_bytes!("../fixtures/signal_queue/signal_reds_probe.beam");

/// The reductions of each case of `signal_reds_probe:all/0` on pinned ERTS
/// 29.1.1 with one scheduler, as `io:format("~w")` prints them.
const REDUCTIONS: &[(&str, &str)] = &[
    ("none", "0"),
    // Each signal counts 1, and the handling uses the count divided by 4,
    // plus 1 (`erl_proc_sig_queue.c:6968`, `7160`). A link adds 0, an
    // unlink that removes the link 4, a monitor 2, a demonitor 1 and 1 more
    // for the target monitor, and an ignored exit signal 2.
    ("link", "1"),
    ("link_unlink_10", "16"),
    ("link_10", "1"),
    ("monitor", "1"),
    ("monitor_10", "8"),
    ("monitor_demonitor_10", "16"),
    ("exit_normal", "1"),
    ("exit_normal_100", "76"),
    ("exit_normal_500", "376"),
    ("exit_trap", "6"),
    ("exit_trap_10", "20"),
    ("group_leader", "6"),
    ("is_process_alive", "2"),
    ("process_info", "1"),
    ("unlink_ack", "{8,3}"),
    // A receive marker in the middle queue part counts 1 when the handling
    // passes it.
    ("marker", "236"),
    // A DOWN message with an immediate reason counts 8, and with another
    // reason 1 (`erl_proc_sig_queue.c:4817-5060`).
    ("down", "[{normal,7},{7,7},{[],7},{[a],5},{{shutdown,x},5}]"),
    // A process-info request that B handles in `loop_rec` gives the
    // reductions of the current turn too (`erl_bif_info.c:2114-2121`).
    ("info_running", "106"),
    // An active process handles at most 100 reductions of signals at
    // schedule-in (`erl_process.c:10318-10352`), then the rest in
    // `loop_rec`, which also stretches the limit (`erl_proc_sig_queue.c:7180-7256`).
    (
        "active",
        "[{1,5},{10,12},{133,104},{134,105},{135,106},{300,230},{1000,756},{6000,4511}]",
    ),
    ("waiting", "[{1,1},{1000,751},{6000,4506}]"),
];

#[test]
fn uses_the_reductions_of_the_signal_handling_as_pinned_erts() {
    let expected = expected(REDUCTIONS);
    assert_eq!(pbeam_all("signal_reds_probe", SIGNAL_REDS_PROBE), expected);
    assert_eq!(erts_all("signal_reds_probe"), format!("{expected}\n"));
}

const LINK_EXIT_PROBE: &[u8] = include_bytes!("../fixtures/signal_queue/link_exit_probe.beam");

/// The result of each case of `link_exit_probe:all/0` on pinned ERTS 29.1.1
/// with one scheduler, as `io:format("~w")` prints it. The case process
/// gets the messages of its members in arrival order.
const LINK_EXITS: &[(&str, &str)] = &[
    // An abnormal link exit signal ends a member that does not trap exits
    // with the same reason, also `kill`, and its exit sends the reason
    // through its links (`erl_proc_sig_queue.c:4758-4785`).
    (
        "chain",
        "[{down,c,boom},{down,b,boom},{exit,a,boom},{down,a,boom}]",
    ),
    (
        "chain_kill",
        "[{down,c,kill},{down,b,kill},{exit,a,kill},{down,a,kill}]",
    ),
    ("chain_normal", "{normal,true,true,[]}"),
    // `exit(B, kill)` ends B with `killed`, and B sends `killed`.
    (
        "exit_kill_links",
        "[{exit,b,killed},{down,b,killed},{down,c,killed}]",
    ),
    // A member that runs a loop handles the signal at schedule-in.
    ("busy_linked", "[{down,c,boom},{exit,b,boom},{down,b,boom}]"),
    ("untrapped", "{died,boom}"),
    // `link/1` to a process that does not exist raises `noproc` without
    // `trap_exit`. With `trap_exit` the caller sends itself the link exit
    // signal `noproc`, and it acts on it when it handles it
    // (`bif.c:162-183`, `411-423`).
    ("link_dead", "{error,noproc}"),
    ("link_dead_trap", "noproc"),
    ("link_dead_order", "[a,{'EXIT',noproc},b]"),
    ("link_dead_untrap", "{died,noproc}"),
    // A target that did not handle its kill signal is not exiting, so the
    // link is made, and the target answers the link signal with `noproc`.
    ("link_exiting", "{died,noproc}"),
    ("link_exiting_trap", "noproc"),
    ("link_self", "{true,[]}"),
];

#[test]
fn propagates_exit_signals_through_links_as_pinned_erts() {
    let expected = expected(LINK_EXITS);
    assert_eq!(pbeam_all("link_exit_probe", LINK_EXIT_PROBE), expected);
    assert_eq!(erts_all("link_exit_probe"), format!("{expected}\n"));
}
