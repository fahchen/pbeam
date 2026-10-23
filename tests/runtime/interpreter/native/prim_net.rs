use super::*;

#[test]
fn checks_the_command_map_as_nif_command() {
    // `prim_net:debug/1` always gives a map with `command => debug`, and
    // `nif_command/1` is not exported, so only a direct call reaches these
    // checks (`prim_net_nif.c:897-954`).
    let atoms = AtomTable::new();
    let atom = |name| Term::Atom(atoms.put(name).unwrap());
    let map = |pairs: Vec<(Term, Term)>| Term::Map(Map::from_pairs(pairs, &atoms).unwrap());
    let badarg = format!("{:?}", BifResult::badarg());
    let run = |argument: &Term| format!("{:?}", command(&atoms, argument).unwrap());
    assert_eq!(run(&atom("x")), badarg);
    assert_eq!(run(&map(vec![(atom("data"), atom("false"))])), badarg);
    assert_eq!(run(&map(vec![(atom("command"), atom("debug"))])), badarg);
    let other = map(vec![
        (atom("command"), atom("x")),
        (atom("data"), atom("false")),
    ]);
    let BifResult::Raise { reason, .. } = command(&atoms, &other).unwrap() else {
        panic!("the command raises");
    };
    let detail = Term::tuple(vec![atom("command"), atom("x")]);
    assert_eq!(reason, Term::tuple(vec![atom("invalid"), detail]));
}

#[test]
fn gives_minus_one_for_a_failed_gethostname() {
    assert_eq!(
        gethostname(Err(nix::Error::EINVAL)),
        Term::tuple(vec![Term::static_atom("error"), Term::Integer(-1)])
    );
}
