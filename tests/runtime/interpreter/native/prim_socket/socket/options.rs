use super::*;
use crate::term::AtomTable;

#[test]
fn gives_badarg_for_a_level_or_option_that_is_not_an_integer() {
    // `nif_setopt` and `nif_getopt` raise `error:badarg` for a level or an
    // option that is neither an integer nor a known atom
    // (prim_socket_nif.c:7835-7842, 7862-7865). `prim_socket` gives the NIFs
    // only integers and the atoms `socket` and `otp`
    // (prim_socket.erl:1237-1283), so no fixture reaches this case.
    let scheduler = &mut Scheduler::new(AtomTable::new());
    let Ok(BifResult::Raise { reason, .. }) = range_or_badarg(scheduler, &Term::Nil) else {
        panic!("no raise");
    };
    assert_eq!(reason, Term::static_atom("badarg"));
}
