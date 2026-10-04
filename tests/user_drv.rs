use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

mod common;

use common::pinned_beam;

const USER_DRV_PROBE: &[u8] = include_bytes!("../fixtures/user_drv/user_drv_probe.beam");

#[test]
fn runs_the_pinned_unicode_characters_to_binary_as_pinned_erts() {
    // The pinned `unicode` module has `bs_get_utf16` and `bs_get_utf32`.
    // `characters_to_binary/1` runs the module code, which calls
    // `unicode:characters_to_binary/2`. That call runs the BIF, not the
    // `erlang:nif_error(undef)` stub of the module.
    let unicode = pinned_beam("unicode");
    let program = BeamProgram::load(&[&unicode]).unwrap();
    assert_eq!(
        program
            .execute(
                "unicode",
                "characters_to_binary",
                &[Term::List(vec![
                    Term::Binary(b"Hello".to_vec()),
                    Term::Integer(10),
                ])],
            )
            .unwrap(),
        Term::Binary(b"Hello\n".to_vec())
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            "io:format(\"~p~n\", [unicode:characters_to_binary([<<\"Hello\">>, 10])]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"<<\"Hello\\n\">>\n");
}

#[test]
fn runs_iolist_to_iovec_of_terms_that_are_not_lists_as_pinned_erts() {
    // `iolist_to_iovec/1` returns `[]` for `<<>>` and `[]`, and raises
    // `badarg` for `foo`, as `iolist_to_iovec_1` (`erl_iolist.c`) does.
    let program = BeamProgram::load(&[USER_DRV_PROBE]).unwrap();
    assert_eq!(
        program.execute("user_drv_probe", "iovec", &[]).unwrap(),
        Term::List(vec![
            Term::List(vec![]),
            Term::List(vec![]),
            Term::Atom(program.atom("badarg").unwrap()),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/user_drv",
            "-eval",
            "io:format(\"~p~n\", [user_drv_probe:iovec()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"[[],[],badarg]\n");
}
