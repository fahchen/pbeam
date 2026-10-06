use std::process::{Command, Output};

use pbeam_runtime::BeamProgram;

const ROOT_EXIT_PROBE: &[u8] = include_bytes!("../fixtures/root_exit/root_exit_probe.beam");

/// The text of the exit reason of `root_exit_probe:terms/0`, as ERTS 29.1.1
/// writes it in `Runtime terminating during boot (...)`. `init` prints the
/// reason with `erts_internal:term_to_string/2` (`init.erl:665-672`), and
/// `erlang:halt/1` writes each byte of the text as one code point
/// (`bif.c:4372-4387`): the atom `'Ä'` gives `'\u{c3}\u{84}'`, and the
/// string `[160,255]` gives `"\u{a0}\u{ff}"`.
const TERMS_TEXT: &str = "{ok,'Elixir.Hello','hello world','','it\\'s','a\\nb','a\\033b','\\\\\\f\\t\\r\\b\\v','\u{7f}',end,'\\200','\u{c3}\u{84}',\u{c3}\u{a9},'\u{ce}\u{bb}','_x',aB_9,-7,576460752303423488,9223372036854775807,[],\"abc\",\"q\\\"uo\\n\t\",[1,2,3],[65|66],[a|b],[128],\"\u{a0}\u{ff}\",[300],{},{a,{b}},<<>>,<<\"abc\">>,<<\"a\\\"b\">>,<<1,2,200>>,<<97,10>>,#{},#{2=>b,a=>1,\"k\"=>[]},#{18=>18,4=>4,12=>12,19=>19,29=>29,13=>13,2=>2,7=>7,31=>31,8=>8,10=>10,23=>23,9=>9,15=>15,32=>32,1=>1,25=>25,28=>28,20=>20,6=>6,11=>11,17=>17,24=>24,14=>14,33=>33,3=>3,16=>16,30=>30,21=>21,5=>5,27=>27,26=>26,22=>22},fun lists:map/2,#Fun<root_exit_probe.0.120231133>,#Fun<root_exit_probe.1.120231133>}";

/// Run `root_exit_probe:Function()` under `erl -eval` and return its
/// output. Without `ERL_CRASH_DUMP_SECONDS=0`, ERTS writes
/// `erl_crash.dump` into the working directory.
///
/// # Examples
///
/// ```rust,ignore
/// let output = erts_run("terms");
/// assert_eq!(output.status.code(), Some(1));
/// ```
fn erts_run(function: &str) -> Output {
    Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/root_exit",
            "-eval",
            &format!("root_exit_probe:{function}()."),
        ])
        .env("ERL_CRASH_DUMP_SECONDS", "0")
        .output()
        .unwrap()
}

/// Return the reason text in the stderr of `erts_run(function)`: the text
/// in the parentheses of `Runtime terminating during boot (...)`, after
/// the `Error!` line of `start_it/1` (`init.erl:1552-1567`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(erts_reason("terms", &erts_run("terms")), TERMS_TEXT);
/// ```
fn erts_reason(function: &str, output: &Output) -> String {
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    let prefix = format!(
        "Error! Failed to eval: root_exit_probe:{function}().\r\n\r\nRuntime terminating during boot ("
    );
    stderr
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix(")\r\n"))
        .unwrap_or_else(|| panic!("unexpected ERTS stderr {stderr:?}"))
        .to_owned()
}

#[test]
fn writes_the_root_exit_reason_as_pinned_erts_prints_it() {
    // The reason of `terms/0` holds one term of each kind that PBeam
    // represents, except pids and references. PBeam writes the `%T` text
    // of `print_term` (`erl_printf_term.c:352-811`) in its error, as
    // `init` writes it in the halt text (row "Run end" in
    // `docs/erts-differences.md`).
    let program = BeamProgram::load(&[ROOT_EXIT_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("root_exit_probe", "terms", &[])
            .unwrap_err()
            .to_string(),
        format!("root process exited with {TERMS_TEXT}")
    );
    // PBeam numbers its first process and its first reference from 0
    // (row "Process and reference counters").
    assert_eq!(
        program
            .execute("root_exit_probe", "identifiers", &[])
            .unwrap_err()
            .to_string(),
        "root process exited with {<0.0.0>,#Ref<0.0.1.0>}"
    );
}

#[test]
fn ends_the_binary_with_the_erts_reason_text_and_status_1() {
    let erts = erts_run("terms");
    assert_eq!(erts_reason("terms", &erts), TERMS_TEXT);
    let pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"))
        .args([
            "root_exit_probe",
            "terms",
            "--",
            "fixtures/root_exit/root_exit_probe.beam",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&pbeam.stderr),
        format!("pbeam: root process exited with {TERMS_TEXT}\n")
    );
    for (name, output) in [("ERTS", &erts), ("PBeam", &pbeam)] {
        assert_eq!(
            (output.stdout.as_slice(), output.status.code()),
            (b"".as_slice(), Some(1)),
            "{name}"
        );
    }
}

#[test]
fn prints_the_root_pid_and_a_reference_in_the_erts_form() {
    // ERTS gives the `-eval` process another pid number, and each run a new
    // reference, so only the form compares: `{<0.N.0>,#Ref<0.N.N.N>}`.
    let output = erts_run("identifiers");
    assert_eq!(output.status.code(), Some(1));
    let reason = erts_reason("identifiers", &output);
    let is_number = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    let (pid_number, reference) = reason
        .strip_prefix("{<0.")
        .and_then(|rest| rest.strip_suffix(">}"))
        .and_then(|rest| rest.split_once(".0>,#Ref<0."))
        .unwrap_or_else(|| panic!("unexpected ERTS reason {reason:?}"));
    assert!(is_number(pid_number), "{reason}");
    let numbers: Vec<_> = reference.split('.').collect();
    assert_eq!(numbers.len(), 3, "{reason}");
    assert!(numbers.into_iter().all(is_number), "{reason}");
}
