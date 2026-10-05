use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

mod common;

use common::pinned_beam;

const FUN_PROBE: &[u8] = include_bytes!("../fixtures/funs/fun_probe.beam");

/// Run `fun_probe:Function()` on the pinned ERTS and return its `~w` output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/funs",
            "-eval",
            &format!("io:format(\"~w~n\", [fun_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn applies_module_functions_as_pinned_erts() {
    // `fun_probe:mfas/0` calls `erlang:apply/3` as `call_ext`,
    // `call_ext_only`, and the `apply` instruction, as
    // `proc_lib:init_p_do_apply/3` (`proc_lib.erl:331-333`) does. An apply
    // of `erlang:apply/3` runs its inner call. A module or function that is
    // not an atom, and an argument term that is not a list, raise
    // `error:badarg`. A list of 1024 arguments raises `error:system_limit`,
    // as ERTS `apply` (`beam_common.c:1434-1526`) does.
    let program = BeamProgram::load(&[FUN_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let mut expected: Vec<Term> = ["v", "w", "z", "y", "s"].map(atom).into();
    expected.extend(std::iter::repeat_n(atom("badarg"), 8));
    expected.push(atom("system_limit"));
    assert_eq!(
        program.execute("fun_probe", "mfas", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("mfas"),
        b"[v,w,z,y,s,badarg,badarg,badarg,badarg,badarg,badarg,badarg,badarg,system_limit]\n"
    );
}

#[test]
fn makes_and_calls_local_funs_as_pinned_erts() {
    // `fun_probe:locals/0` makes funs with `make_fun3`, with and without
    // free variables, as `standard_error:start/0` and `gen:call/4` do, and
    // calls them with `call_fun` and `call_fun2`, as `gen:do_for_proc/2`
    // does. Two funs of one lambda are equal when their free variables are
    // equal, and they compare by their free variables (`utils.c:1241-1276`,
    // `utils.c:2151-2208`). A call with the wrong arity raises
    // `{badarity, {Fun, Args}}`, and a call of a term that is not a fun
    // raises `{badfun, Term}` (`beam_common.c:1605-1731`).
    let program = BeamProgram::load(&[FUN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |first: &str, second| Term::Tuple(vec![atom(first), second]);
    let expected = vec![
        pair("tag", atom("one")),
        atom("const"),
        pair("tag", atom("tail")),
        atom("same"),
        atom("different"),
        atom("same"),
        atom("true"),
        atom("false"),
        Term::Tuple(vec![
            atom("badarity"),
            atom("same"),
            Term::List(vec![Term::Integer(1), Term::Integer(2)]),
        ]),
        pair("badfun", atom("notfun")),
        pair("badfun", pair("fun_probe", atom("id"))),
    ];
    assert_eq!(
        program.execute("fun_probe", "locals", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("locals"),
        b"[{tag,one},const,{tag,tail},same,different,same,true,false,\
          {badarity,same,[1,2]},{badfun,notfun},{badfun,{fun_probe,id}}]\n"
    );
}

#[test]
fn tests_funs_with_is_function_as_pinned_erts() {
    // `fun_probe:tests/0` runs the pinned `lists:foreach/2`, which tests its
    // fun with `is_function2` (`lists.erl:2637-2641`), as
    // `standard_error:setopts/1` does. `kind/1` has the `is_function` and
    // `is_function2` tests, and the other items call the BIFs
    // `is_function/1` and `is_function/2` (`erl_bif_op.c:223-267`). A
    // negative arity or an arity that is not an integer raises `badarg`, and
    // the guard of `arity_guard/2` makes that `badarg` fail the clause.
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[&lists, FUN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let mut expected = vec![Term::List(vec![atom("a"), atom("b")])];
    expected.extend(
        [
            "one", "any", "none", "true", "false", "true", "false", "false", "true", "false",
            "false", "badarg", "badarg",
        ]
        .map(atom),
    );
    assert_eq!(
        program.execute("fun_probe", "tests", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("tests"),
        b"[[a,b],one,any,none,true,false,true,false,false,true,false,false,badarg,badarg]\n"
    );
}
