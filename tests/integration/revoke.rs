//! Capability revocation: the delegation records, the revocation signal,
//! and its cascade to the downstream copies. ERTS has no capabilities, so
//! `pbeam_cap:revoke/2` raises `error:undef` there. The pinned ERTS run
//! gives the results of the permitted part, and the revocations of the
//! default policy are the oracle of each denial.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{AllowAll, BeamProgram, Policy, Term};

const REVOKE_PROBE: &[u8] = include_bytes!("../../fixtures/capability/revoke_probe.beam");

/// Return the `~w` text of a term of atoms, lists, and tuples.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::List(vec![])), "[]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::List(elements) => format!("[{}]", join(elements)),
        Term::Tuple(elements) => format!("{{{}}}", join(elements)),
        other => panic!("the probe returns no {other:?}"),
    }
}

/// Run `revoke_probe:Function()` on PBeam with `policy` and return the
/// `~w` text.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pbeam("released", &AllowAll), "denied");
/// ```
fn pbeam(function: &str, policy: &dyn Policy) -> String {
    let erlang = pinned_beam("erlang");
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erlang, &erts_internal, REVOKE_PROBE]).unwrap();
    let result = program
        .execute_with_policy("revoke_probe", function, &[], policy)
        .unwrap();
    text(&result)
}

/// Run `revoke_probe:Function()` on the pinned ERTS with one scheduler and
/// return the printed result.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(erts("released"), "{error,undef}");
/// ```
fn erts(function: &str) -> String {
    let output = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/capability",
            "-eval",
            &format!("io:format(\"~w\", [revoke_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{output:?}"
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn revokes_a_given_copy_and_its_downstream_copies_when_the_holders_handle_the_signal() {
    // Each spawn before the revocation gives the ERTS result. The second
    // revocation signal finds no copy. After the revocation, the next spawn
    // of the worker and of its child is denied, both holders live, and the
    // copy of the caller stays.
    assert_eq!(
        pbeam("cascade", &AllowAll),
        "{[ok,ok],[ok,ok],[denied,denied,ok],[true,true]}"
    );
    assert_eq!(
        erts("cascade"),
        "{[ok,ok],[{error,undef},{error,undef}],[ok,ok,ok],[true,true]}"
    );
}

#[test]
fn denies_a_revocation_from_a_process_that_did_not_give_the_copy() {
    // The root process did not give the copy of the child of the worker,
    // and the launcher gave its own copy. The worker did not give the copy
    // of its parent. A name and a kind that is not valid raise badarg. The
    // worker can revoke the copy of its child.
    assert_eq!(
        pbeam("denied", &AllowAll),
        "[denied,denied,denied,{error,badarg},{error,badarg},ok,denied,ok]"
    );
    let undef = ["{error,undef}"; 6].join(",");
    assert_eq!(erts("denied"), format!("[{undef},ok,ok]"));
}

#[test]
fn revokes_through_a_holder_that_exited() {
    assert_eq!(pbeam("exited", &AllowAll), "{ok,ok,denied}");
    assert_eq!(erts("exited"), "{ok,{error,undef},ok}");
    // The worker sends the revocation through the record of its child,
    // which exited, to the grandchild.
    assert_eq!(pbeam("below", &AllowAll), "{ok,[denied,denied]}");
    assert_eq!(erts("below"), "{{error,undef},[ok,ok]}");
    // The worker handles its revocation signal in its exit.
    assert_eq!(pbeam("exiting", &AllowAll), "{ok,denied}");
    assert_eq!(erts("exiting"), "{{error,undef},ok}");
}

#[test]
fn releases_the_copies_and_the_records_when_the_holders_exit() {
    assert_eq!(pbeam("released", &AllowAll), "denied");
    assert_eq!(erts("released"), "{error,undef}");
}
