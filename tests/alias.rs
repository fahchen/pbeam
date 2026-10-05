use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const ALIAS_PROBE: &[u8] = include_bytes!("../fixtures/alias/alias_probe.beam");

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
    let tuple = Term::Tuple;
    let list = Term::List;
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
            &[Term::Atom(program.atom("time_offset").unwrap())],
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
