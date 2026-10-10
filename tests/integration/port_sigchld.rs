use std::process::Command;

use pbeam_runtime::BeamProgram;

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PORT_PROBE: &[u8] = include_bytes!("../../fixtures/port/port_probe.beam");

#[test]
fn stops_the_run_for_the_exit_status_of_a_child_that_the_host_reaped() {
    // This test is the only test of its crate, because it sets the
    // disposition of `SIGCHLD` for the test process to `SIG_IGN`. The host
    // then reaps each child, and the waiter thread of a port gets no wait
    // status. ERTS reaps its children in `erl_child_setup`, a separate
    // program (erl_child_setup.c:396-420), so the pinned ERTS still gives
    // the status (row "Ports" in docs/erts-differences.md).
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/port",
            "-eval",
            "io:format(\"~w~n\", [port_probe:sigchld_ignored()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(String::from_utf8_lossy(&reference.stdout), "7\n");
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[PORT_PROBE, &erlang]).unwrap();
    let error = program
        .execute("port_probe", "sigchld_ignored", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with(
            "unsupported exit status of a port program after os:set_signal(sigchld, ignore)"
        ),
        "{error}"
    );
}
