use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const ALIAS_PROBE: &[u8] = include_bytes!("../../fixtures/alias/alias_probe.beam");

/// Run a function of `alias_probe` under the pinned ERTS and return the
/// `~w` text of its result.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(erl_reference("targets()").starts_with("{[{error,badarg},"));
/// ```
fn erl_reference(call: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/alias",
            "-eval",
            &format!("io:format(\"~w~n\", [alias_probe:{call}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    String::from_utf8(reference.stdout).unwrap()
}

#[test]
fn monitors_a_process_by_pid_and_name_and_a_port_name_as_pinned_erts() {
    // `targets/0` runs `erlang:monitor/2` as `monitor` (bif.c:758-986):
    // `badarg` for each type and target that ERTS rejects, no monitor of
    // the caller (bif.c:781-784), the DOWN item `{Name, nonode@nohost}` of
    // a monitor by name, and the noproc DOWN message of a dead pid, a name
    // that no process has, and a port name.
    let program = BeamProgram::load(&[ALIAS_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let tuple = Term::tuple;
    let list = |elements: Vec<Term>| Term::list_from(elements);
    let badarg = || tuple(vec![atom("error"), atom("badarg")]);
    let named = |name: &str| tuple(vec![atom(name), atom("nonode@nohost")]);
    let falses = |count| list(vec![atom("false"); count]);
    let noproc = |kind: &str, item: Term| tuple(vec![atom(kind), item, atom("noproc")]);
    assert_eq!(
        program.execute("alias_probe", "targets", &[]).unwrap(),
        tuple(vec![
            list((0..12).map(|_| badarg()).collect()),
            falses(3),
            list(vec![
                tuple(vec![atom("t"), atom("normal")]),
                tuple(vec![named("alias_probe_named"), atom("boom")]),
                tuple(vec![named("alias_probe_named"), atom("boom")]),
            ]),
            list(vec![
                noproc("process", atom("t")),
                noproc("process", named("alias_probe_nobody")),
                noproc("port", named("alias_probe_nobody")),
                noproc("port", named("alias_probe_nobody")),
            ]),
            falses(4),
            list(vec![]),
        ])
    );
    assert_eq!(
        erl_reference("targets()"),
        "{[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[false,false,false],[{t,normal},{{alias_probe_named,nonode@nohost},boom},{{alias_probe_named,nonode@nohost},boom}],[{process,t,noproc},{process,{alias_probe_nobody,nonode@nohost},noproc},{port,{alias_probe_nobody,nonode@nohost},noproc},{port,{alias_probe_nobody,nonode@nohost},noproc}],[false,false,false,false],[]}\n"
    );
}

#[test]
fn returns_a_runtime_error_for_a_time_offset_monitor() {
    // ERTS 29.1.1 returns a reference for `monitor(time_offset,
    // clock_service)` (bif.c:952-969). PBeam has no time offset service
    // (row "Monitors" in `docs/erts-differences.md`).
    let program = BeamProgram::load(&[ALIAS_PROBE]).unwrap();
    let error = program
        .execute(
            "alias_probe",
            "unsupported",
            &[program.host_term(Term::Atom(program.atom("time_offset").unwrap()))],
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported time_offset monitor"),
        "{error}"
    );
    assert!(erl_reference("unsupported(time_offset)").starts_with("#Ref<"));
}

/// Return the result of `alias_probe:Function()` in PBeam, and a function
/// that makes an atom of the loaded program.
///
/// # Examples
///
/// ```rust,ignore
/// let (result, atom) = run("modes");
/// assert_eq!(result, Term::tuple(vec![/* ... */]));
/// ```
fn run(function: &str) -> (Term, impl Fn(&str) -> Term) {
    let program = BeamProgram::load(&[ALIAS_PROBE]).unwrap();
    let result = program.execute("alias_probe", function, &[]).unwrap();
    (result.term, move |name: &str| {
        Term::Atom(program.atom(name).unwrap())
    })
}

/// Return the error text of `alias_probe:unsupported(Case)` in PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(unsupported("priority").contains("unsupported monitor option priority"));
/// ```
fn unsupported(case: &str) -> String {
    let program = BeamProgram::load(&[ALIAS_PROBE]).unwrap();
    let case = Term::Atom(program.atom(case).unwrap());
    program
        .execute("alias_probe", "unsupported", &[program.host_term(case)])
        .unwrap_err()
        .to_string()
}

#[test]
fn reads_the_options_of_monitor_3_before_the_type_as_pinned_erts() {
    // `options/0`: `erts_monitor_opts` (bif.c:701-756) rejects each other
    // element and an improper tail with `badarg` before `monitor` checks the
    // type and the target (bif.c:988-997).
    let (result, atom) = run("options");
    let tuple = Term::tuple;
    let list = |elements: Vec<Term>| Term::list_from(elements);
    let badarg = || tuple(vec![atom("error"), atom("badarg")]);
    let ok = |value: &str| tuple(vec![atom("ok"), atom(value)]);
    let mut first: Vec<Term> = (0..9).map(|_| badarg()).collect();
    first.extend((0..5).map(|_| ok("true")));
    let mut second: Vec<Term> = (0..5).map(|_| badarg()).collect();
    second.push(ok("false"));
    assert_eq!(
        result,
        tuple(vec![tuple(vec![list(first), list(second)]), list(vec![])])
    );
    assert_eq!(
        erl_reference("options()"),
        "{{[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{ok,true},{ok,true},{ok,true},{ok,true},{ok,true}],[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{ok,false}]},[]}\n"
    );
}

#[test]
fn tags_down_messages_with_the_tag_option_as_pinned_erts() {
    // `tags/0`: the last `tag` option replaces `'DOWN'`
    // (erl_proc_sig_queue.c:6417-6429), also by name, for noproc, and for
    // a port name.
    let (result, atom) = run("tags");
    let tuple = Term::tuple;
    let list = |elements: Vec<Term>| Term::list_from(elements);
    let named = |name: &str| tuple(vec![atom(name), atom("nonode@nohost")]);
    let improper = Term::list_with_tail(vec![Term::Integer(1)], Term::Integer(2));
    assert_eq!(
        result,
        tuple(vec![
            list(vec![
                tuple(vec![atom("my_down"), atom("t"), atom("normal")]),
                tuple(vec![improper, atom("t"), atom("normal")]),
                tuple(vec![
                    atom("named"),
                    named("alias_probe_named"),
                    atom("boom")
                ]),
            ]),
            list(vec![
                tuple(vec![
                    atom("gone"),
                    atom("process"),
                    atom("t"),
                    atom("noproc")
                ]),
                tuple(vec![
                    atom("nobody"),
                    atom("process"),
                    named("alias_probe_nobody"),
                    atom("noproc"),
                ]),
                tuple(vec![
                    atom("p"),
                    atom("port"),
                    named("alias_probe_nobody"),
                    atom("noproc"),
                ]),
            ]),
            list(vec![]),
        ])
    );
    assert_eq!(
        erl_reference("tags()"),
        "{[{my_down,t,normal},{[1|2],t,normal},{named,{alias_probe_named,nonode@nohost},boom}],[{gone,process,t,noproc},{nobody,process,{alias_probe_nobody,nonode@nohost},noproc},{p,port,{alias_probe_nobody,nonode@nohost},noproc}],[]}\n"
    );
}

#[test]
fn ends_each_alias_mode_as_pinned_erts() {
    // `modes/0`: a `demonitor` alias ends at `demonitor`, a
    // `reply_demonitor` alias at its first message, which also removes the
    // monitor, and an `explicit_unalias` alias stays (bif.c:452-492,
    // erl_proc_sig_queue.c:6026-6070). A monitor of the caller makes no
    // alias (bif.c:781-784), and the last alias option wins.
    let (result, atom) = run("modes");
    let atoms = |names: &[&str]| Term::list_from(names.iter().map(|name| atom(name)));
    assert_eq!(
        result,
        Term::tuple(vec![
            atoms(&["true", "false", "true", "false", "false", "false"]),
            atoms(&["a1", "a2", "b1", "c1", "c2", "e1"]),
        ])
    );
    assert_eq!(
        erl_reference("modes()"),
        "{[true,false,true,false,false,false],[a1,a2,b1,c1,c2,e1]}\n"
    );
}

#[test]
fn ends_an_alias_at_the_down_message_unless_explicit_unalias_as_pinned_erts() {
    // `downs/0`: the DOWN message ends a `demonitor` and a
    // `reply_demonitor` alias, and keeps an `explicit_unalias` alias, also
    // for the noproc DOWN message (erl_proc_sig_queue.c:6463-6490).
    let (result, atom) = run("downs");
    let tuple = Term::tuple;
    let list = |elements: Vec<Term>| Term::list_from(elements);
    let pair = |tag: &str| tuple(vec![atom(tag), atom("normal")]);
    let noproc = |kind: &str| tuple(vec![atom("DOWN"), atom(kind), atom("noproc")]);
    assert_eq!(
        result,
        tuple(vec![
            list(vec![pair("my_down"), pair("DOWN"), pair("DOWN")]),
            list(vec![atom("false"); 3]),
            list(vec![noproc("process"), noproc("process"), noproc("port")]),
            list(vec![atom("g2"), atom("h2")]),
        ])
    );
    assert_eq!(
        erl_reference("downs()"),
        "{[{my_down,normal},{'DOWN',normal},{'DOWN',normal}],[false,false,false],[{'DOWN',process,noproc},{'DOWN',process,noproc},{'DOWN',port,noproc}],[g2,h2]}\n"
    );
}

#[test]
fn sends_an_exit_signal_through_an_active_alias_as_pinned_erts() {
    // `exits/0`: `erlang:exit/2` with an active alias sends the signal to
    // its owner (erl_proc_sig_queue.c:1650-1658); `normal` is ignored
    // without trap_exit, because a reference is never `exit2_suicide`
    // (bif.c:1750-1772). A reference that is not an active alias gives
    // `true` with no signal (erl_proc_sig_queue.c:4659-4672).
    let (result, atom) = run("exits");
    assert_eq!(
        result,
        Term::tuple(vec![
            Term::list_from(vec![atom("true"); 6]),
            atom("alive"),
            atom("boom"),
            atom("true"),
            Term::nil(),
        ])
    );
    assert_eq!(
        erl_reference("exits()"),
        "{[true,true,true,true,true,true],alive,boom,true,[]}\n"
    );
}

#[test]
fn returns_a_runtime_error_for_the_priority_monitor_option() {
    // ERTS 29.1.1 returns a reference and makes a priority DOWN message
    // (bif.c:711-713). PBeam has no priority messages (row "Monitors").
    let error = unsupported("priority");
    assert!(
        error.contains("unsupported monitor option priority"),
        "{error}"
    );
    assert!(erl_reference("unsupported(priority)").starts_with("#Ref<"));
}

#[test]
fn runs_erlang_send_2_and_send_3_as_pinned_erts() {
    // `sends/0`: `erlang:send/2` returns the message as `erl_send`
    // (bif.c:2845-2908). `erlang:send/3` reads its options first and raises
    // `badarg` before it sends, then returns `noconnect` only for another
    // node with `noconnect` (bif.c:2697-2799). The relay sends through a
    // `reply_demonitor` alias with `send/3`, which ends it, so `send/2` to
    // the alias drops `r2`.
    let (result, atom) = run("sends");
    let tuple = Term::tuple;
    let list = |elements: Vec<Term>| Term::list_from(elements);
    let badarg = || tuple(vec![atom("error"), atom("badarg")]);
    let ok = |value: &str| tuple(vec![atom("ok"), atom(value)]);
    let mut send3: Vec<Term> = (0..4).map(|_| ok("ok")).collect();
    send3.extend((0..3).map(|_| badarg()));
    send3.extend([
        ok("noconnect"),
        ok("ok"),
        ok("ok"),
        badarg(),
        badarg(),
        ok("ok"),
    ]);
    let atoms = |names: &[&str]| list(names.iter().map(|name| atom(name)).collect());
    assert_eq!(
        result,
        tuple(vec![
            list(vec![
                ok("m2"),
                badarg(),
                ok("m2"),
                ok("m2"),
                ok("m2"),
                badarg()
            ]),
            list(send3),
            atoms(&["m2", "m3", "m3", "m3", "m3"]),
            atoms(&["ok", "r2"]),
            atoms(&["r1"]),
        ])
    );
    assert_eq!(
        erl_reference("sends()"),
        "{[{ok,m2},{error,badarg},{ok,m2},{ok,m2},{ok,m2},{error,badarg}],[{ok,ok},{ok,ok},{ok,ok},{ok,ok},{error,badarg},{error,badarg},{error,badarg},{ok,noconnect},{ok,ok},{ok,ok},{error,badarg},{error,badarg},{ok,ok}],[m2,m3,m3,m3,m3],[ok,r2],[r1]}\n"
    );
}

#[test]
fn replies_to_a_gen_server_call_through_an_alias_as_pinned_erts() {
    // `call/0`: `gen:do_call/4` (gen.erl:251-282) monitors the server with
    // `[{alias, demonitor}]`, sends with `erlang:send/3`, and `gen:reply/2`
    // answers through the alias. After the timeout, `demonitor` with `flush`
    // ends the alias, so the late reply is dropped.
    let gen_server = pinned_beam("gen_server");
    let gen_module = pinned_beam("gen");
    let program = BeamProgram::load(&[ALIAS_PROBE, &gen_server, &gen_module]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let echo = |value| Term::tuple(vec![atom("echo"), Term::Integer(value)]);
    assert_eq!(
        program.execute("alias_probe", "call", &[]).unwrap(),
        Term::list_from(vec![echo(1), echo(2), atom("timeout"), Term::nil()])
    );
    assert_eq!(erl_reference("call()"), "[{echo,1},{echo,2},timeout,[]]\n");
}
