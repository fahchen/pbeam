//! The ERTS snapshot of the BIF table. A unit test of
//! `src/runtime/interpreter.rs` compares the BIF table with
//! `fixtures/bifs/is_builtin.txt`. This test checks the snapshot on the
//! pinned ERTS.

use std::{fs, process::Command};

/// `fixtures/bifs/is_builtin.txt` lists each exported function of the pinned
/// OTP installation for which `erlang:is_builtin/3` returns `true`. Each line
/// is `Module`, `Function`, and `Arity`, separated by a tab. The lines are in
/// the order of `(&str, &str, u32)`, with no duplicate. When the pinned OTP
/// changes, the `left` value of a failure is the new snapshot.
#[test]
fn is_builtin_snapshot_equals_the_pinned_erts() {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            concat!(
                "[io:format(\"~ts\\t~ts\\t~b~n\", [M, F, A]) ",
                "|| P <- filelib:wildcard(filename:join([code:root_dir(), \"lib\", \"*\", \"ebin\", \"*.beam\"])), ",
                "{ok, {M, [{exports, E}]}} <- [beam_lib:chunks(P, [exports])], ",
                "{F, A} <- E, erlang:is_builtin(M, F, A)], halt()."
            ),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "erl exited with {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut builtin: Vec<(&str, &str, u32)> = stdout
        .lines()
        .map(|line| {
            let mut field = line.split('\t');
            (
                field.next().unwrap(),
                field.next().unwrap(),
                field.next().unwrap().parse().unwrap(),
            )
        })
        .collect();
    builtin.sort_unstable();
    builtin.dedup();
    let snapshot: String = builtin
        .iter()
        .map(|(module, function, arity)| format!("{module}\t{function}\t{arity}\n"))
        .collect();
    assert_eq!(
        snapshot,
        fs::read_to_string("fixtures/bifs/is_builtin.txt").unwrap()
    );
}
