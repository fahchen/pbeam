//! Unit tests of the fun newtype of `src/term/fun.rs`.

use super::*;

/// Return a local fun with the free variables `free`.
fn fun(free: Vec<Term>) -> Term {
    let module = Atom::from_static("lists").unwrap();
    Term::Fun(Fun::new(Lambda::new(module, 0, 7, 53, 1), free))
}

/// Return the shared thing of a fun term.
fn thing(term: &Term) -> &Arc<FunThing> {
    let Term::Fun(fun) = term else {
        panic!("not a fun: {term:?}");
    };
    fun.arc()
}

/// Return the free variables of a fun term.
fn free(term: &Term) -> &[Term] {
    let Term::Fun(fun) = term else {
        panic!("not a fun: {term:?}");
    };
    fun.free()
}

#[test]
fn shares_the_thing_between_the_copies_of_a_fun() {
    // A copy of a fun shares the thing of the source, as an ERTS copy of a
    // fun term is the same boxed pointer to the `ErlFunThing`
    // (`copy_struct`, copy.c:812-842).
    let source = fun(vec![Term::Integer(1), Term::Integer(2)]);
    let copy = source.clone();
    assert!(Arc::ptr_eq(thing(&source), thing(&copy)));
    // A new fun with equal free variables is a different thing.
    assert!(!Arc::ptr_eq(
        thing(&source),
        thing(&fun(vec![Term::Integer(1), Term::Integer(2)]))
    ));
    assert_eq!(source, copy);
    // A copy of a deep fun shares its thing.
    let deep = (0..100).fold(fun(vec![Term::Integer(1)]), |inner, _| fun(vec![inner]));
    assert!(Arc::ptr_eq(thing(&deep), thing(&deep.clone())));
}

#[test]
fn takes_the_free_variables_of_a_fun_only_from_the_last_copy() {
    // A fun that another copy holds gives no free variables, and the other
    // copy keeps them.
    let mut first = fun(vec![Term::Integer(1), Term::Integer(2)]);
    let mut second = first.clone();
    let Term::Fun(first_fun) = &mut first else {
        unreachable!()
    };
    assert_eq!(first_fun.take_last(), None);
    assert_eq!(free(&second), &[Term::Integer(1), Term::Integer(2)]);
    // The last holder takes the free variables.
    let Term::Fun(second_fun) = &mut second else {
        unreachable!()
    };
    drop(first);
    assert_eq!(
        second_fun.take_last(),
        Some(vec![Term::Integer(1), Term::Integer(2)])
    );
    // The thing of the last holder is gone after the take.
    assert_eq!(second_fun.take_last(), None);
}

#[test]
fn compares_funs_by_their_thing_and_their_free_variables() {
    // Two funs that hold one thing are equal with no walk of the free
    // variables.
    let source = fun(vec![Term::Integer(1)]);
    assert_eq!(source, source.clone());
    // A fun and a second fun with equal elements are equal, and a different
    // lambda or different free variables differ.
    assert_eq!(source, fun(vec![Term::Integer(1)]));
    assert_ne!(source, fun(vec![Term::Integer(2)]));
    assert_ne!(source, fun(vec![]));
    assert_ne!(source, Term::tuple(vec![]));
    // An external fun has no free variables and is equal to a second
    // external fun of the same module, function, and arity.
    let module = Atom::from_static("lists").unwrap();
    let external = |function: &str, arity| {
        Term::Fun(Fun::external(
            module.clone(),
            Atom::from_static(function).unwrap(),
            arity,
        ))
    };
    assert_eq!(external("get", 1), external("get", 1));
    assert_ne!(external("get", 1), external("get", 2));
    assert_ne!(external("get", 1), source);
}

#[test]
fn drops_a_deep_fun_without_recursion() {
    // `Drop` moves the free variables of a fun onto its work stack, so a
    // fun that nests 1,000,000 deep does not overflow the Rust stack.
    let mut deep = fun(vec![Term::Integer(1)]);
    for _ in 0..1_000_000 {
        deep = fun(vec![deep]);
    }
    // The shared thing of a copy gives no free variables to the drop of the
    // copy.
    let copy = deep.clone();
    drop(deep);
    assert_eq!(free(&copy).len(), 1);
    drop(copy);
}

#[test]
fn drains_the_free_variables_of_a_fun_with_the_work_stack() {
    // `Term::take_nested` moves the free variables of a fun onto the work
    // stack and leaves the thing empty, so the drop of a deep fun walks a
    // stack and does not recurse.
    let mut pending = Vec::new();
    let mut term = fun(vec![Term::Integer(1), Term::Integer(2)]);
    Term::take_nested(&mut term, &mut pending);
    assert_eq!(pending, vec![Term::Integer(1), Term::Integer(2)]);
    // The thing of the fun is gone after the move: `take_nested` took it.
    let Term::Fun(fun) = &mut term else {
        unreachable!()
    };
    assert_eq!(fun.take_last(), None);
}
