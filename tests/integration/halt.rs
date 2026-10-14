use std::process::Command;

use pbeam_runtime::{BeamError, BeamProgram, HostTerm, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const HALT_PROBE: &[u8] = include_bytes!("../../fixtures/halt/halt_probe.beam");

/// Run `halt_probe:Function()` under `erl -eval` from a scratch working
/// directory, so that a crash dump does not go into the repository, and
/// return the status code and the output.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(erts("status"), (Some(3), String::new()));
/// ```
fn erts(function: &str) -> (Option<i32>, String) {
    let fixtures = std::env::current_dir().unwrap().join("fixtures/halt");
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            fixtures.to_str().unwrap(),
            "-eval",
            &format!("io:format(\"~w~n\", [halt_probe:{function}()]), halt()."),
        ])
        .current_dir(std::env::temp_dir())
        .env("ERL_CRASH_DUMP_SECONDS", "0")
        .output()
        .unwrap();
    assert!(output.stderr.is_empty(), "{output:?}");
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

/// Run `halt_probe:Function()` on PBeam with the pinned `erlang` module,
/// which holds `erlang:halt/0,1` (`erlang.erl:3357-3388`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pbeam("status"), Err(BeamError::Halt(259)));
/// ```
fn pbeam(function: &str) -> Result<HostTerm, BeamError> {
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, HALT_PROBE]).unwrap();
    program.execute("halt_probe", function, &[])
}

#[test]
fn raises_badarg_for_each_status_and_option_that_halt_rejects() {
    let pbeam = pbeam("badargs").unwrap();
    let Some(results) = pbeam
        .term
        .proper_list()
        .map(|results| results.collect::<Vec<_>>())
    else {
        panic!("{pbeam:?}");
    };
    assert_eq!(results.len(), 22);
    for result in results {
        assert!(
            matches!(result, Term::Tuple(pair) if matches!(pair.as_ref(),
                [Term::Atom(class), Term::Atom(reason)]
                    if pbeam.atoms().name_of(class) == "error"
                        && pbeam.atoms().name_of(reason) == "badarg")),
            "{pbeam:?}"
        );
    }
    let badargs = vec!["{error,badarg}"; 22].join(",");
    assert_eq!(erts("badargs"), (Some(0), format!("[{badargs}]\n")));
}

#[test]
fn stops_the_run_with_the_status_code_of_halt() {
    // `mask/0` gives 0x80000005 masked to INT_MAX, `status/0` gives 259,
    // and the host exit status is its low 8 bits. `others/0` halts while
    // two other processes can always run.
    for (function, code, status) in [
        ("mask", 5, 5),
        ("status", 259, 3),
        ("options", 7, 7),
        ("no_flush", 7, 7),
        ("timeout", 255, 255),
        ("others", 0, 0),
    ] {
        assert_eq!(pbeam(function), Err(BeamError::Halt(code)), "{function}");
        assert_eq!(erts(function), (Some(status), String::new()), "{function}");
    }
}

#[test]
fn gives_a_runtime_error_for_a_core_dump_or_a_crash_dump() {
    // ERTS aborts with a core dump for `abort` (status 134, `SIGABRT`), and
    // writes the slogan and `erl_crash.dump` for a string (status 1). A
    // slogan of 1024 characters with a tail that is not `[]` is also a
    // crash dump, as ERTS cuts it to 1023 characters (measured status 1).
    assert_eq!(
        pbeam("abort").unwrap_err().to_string(),
        "unsupported erlang:halt/2 with abort: PBeam writes no core dump"
    );
    for function in ["slogan", "long_slogan"] {
        assert_eq!(
            pbeam(function).unwrap_err().to_string(),
            "unsupported erlang:halt/2 with a crash dump slogan: PBeam writes no crash dump",
            "{function}"
        );
    }
}
