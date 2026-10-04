use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

mod common;

use common::pinned_beam;

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
