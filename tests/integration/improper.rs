use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const IMPROPER_PROBE: &[u8] = include_bytes!("../../fixtures/improper/improper_probe.beam");

/// Run a function of `improper_probe` under the pinned ERTS and return the
/// `~w` text of its result.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(erl_reference("literals").starts_with("{[a|b],"));
/// ```
fn erl_reference(function: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/improper",
            "-eval",
            &format!("io:format(\"~w~n\", [improper_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    String::from_utf8(reference.stdout).unwrap()
}

/// Return the list of the integers.
fn integers(elements: &[i64]) -> Vec<Term> {
    elements
        .iter()
        .map(|element| Term::Integer(*element))
        .collect()
}

/// Return an improper list of elements and a tail.
fn improper(elements: Vec<Term>, tail: Term) -> Term {
    Term::list_with_tail(elements, tail)
}

#[test]
fn loads_compares_and_hashes_improper_list_literals_as_pinned_erts() {
    // `literals/0` loads improper list literals (a `LIST_EXT` whose tail is
    // not `NIL_EXT`), also in a tuple and as map keys. It compares pairs
    // with `is_ge`, `is_lt`, `erlang:'=:='/2`, and `erlang:'>='/2`
    // (`eq`, `utils.c:1137-1162`; `erts_cmp_compound`, `utils.c:1827-1851`),
    // and reads the flatmap key order and the hashmap order
    // (`make_internal_hash`, `erl_term_hashing.c:1783-1841`).
    let program = BeamProgram::load(&[IMPROPER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let atoms = |names: &str| Term::list_from(names.split(',').map(atom));
    let int = Term::Integer;
    let one_two = || improper(vec![int(1)], int(2));
    let nested = || improper(vec![one_two()], int(3));
    let hash_keys = [
        int(18),
        improper(vec![Term::list_from(vec![int(1)])], int(2)),
        nested(),
        int(4),
        improper(vec![int(256)], int(1)),
    ]
    .into_iter()
    .chain(integers(&[12, 19, 13, 2, 7, 8, 10, 23, 9]))
    .chain([
        improper(vec![atom("true")], atom("false")),
        one_two(),
        int(15),
    ])
    .chain([improper(integers(&[1, 2]), int(3))])
    .chain(integers(&[1, 25, 20, 6, 11, 17, 24, 14]))
    .chain([improper(vec![Term::nil()], int(-1))])
    .chain(integers(&[3, 16, 21, 5, 26, 22]))
    .collect::<Vec<_>>();
    assert_eq!(
        program.execute("improper_probe", "literals", &[]).unwrap(),
        Term::Tuple(vec![
            improper(vec![atom("a")], atom("b")),
            Term::Tuple(vec![improper(
                integers(&[1, 2]),
                Term::Tuple(vec![atom("t")])
            )]),
            atoms("lt,ge,lt,lt,lt,lt,lt,ge,lt"),
            atoms("lt,ge,lt,lt,lt,lt,lt,ge,lt"),
            atoms("false,false,false,false,false,false,false,true,false"),
            atoms("false,true,false,false,false,false,false,true,false"),
            Term::list_from(vec![
                atom("x"),
                improper(vec![int(1)], atom("x")),
                Term::list_from(vec![int(1)]),
                nested(),
            ]),
            atom("d"),
            Term::list_from(hash_keys),
            atom("v"),
        ])
    );
    assert_eq!(
        erl_reference("literals"),
        "{[a|b],{[1,2|{t}]},[lt,ge,lt,lt,lt,lt,lt,ge,lt],[lt,ge,lt,lt,lt,lt,lt,ge,lt],\
         [false,false,false,false,false,false,false,true,false],\
         [false,true,false,false,false,false,false,true,false],[x,[1|x],[1],[[1|2]|3]],d,\
         [18,[[1]|2],[[1|2]|3],4,[256|1],12,19,13,2,7,8,10,23,9,[true|false],[1|2],15,\
         [1,2|3],1,25,20,6,11,17,24,14,[[]|-1],3,16,21,5,26,22],v}\n"
    );
}

#[test]
fn builds_and_matches_improper_lists_as_pinned_erts() {
    // `instructions/0` builds `[alias | Ref]` with `put_list` and a register
    // tail, as `gen:do_call/4` does, and matches it in a receive with
    // `is_nonempty_list` and `get_list`. It reads improper lists with
    // `get_hd`, `get_tl`, and `get_list`, and tests them with `is_list`,
    // `is_nonempty_list`, and `is_nil`.
    let program = BeamProgram::load(&[IMPROPER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let imp = || improper(vec![int(1)], atom("x"));
    let two = || improper(vec![int(0), int(1)], atom("x"));
    assert_eq!(
        program
            .execute("improper_probe", "instructions", &[])
            .unwrap(),
        Term::list_from(vec![
            atom("reply"),
            imp(),
            two(),
            improper(vec![improper(vec![int(1)], int(2))], int(3)),
            int(0),
            imp(),
            atom("x"),
            Term::Tuple(vec![int(1), int(2), int(3)]),
            atom("nonempty"),
            atom("nil"),
            atom("other"),
            Term::list_from(vec![two()]),
            atom("true"),
        ])
    );
    assert_eq!(
        erl_reference("instructions"),
        "[reply,[1|x],[0,1|x],[[1|2]|3],0,[1|x],x,{1,2,3},nonempty,nil,other,[[0,1|x]],true]\n"
    );
}

#[test]
fn gives_the_list_bif_results_of_pinned_erts_for_improper_lists() {
    // `list_bifs/0` runs the list BIFs with improper lists. `'++'/2`,
    // `tl/1`, and `lists:reverse/2` return improper lists
    // (`erl_bif_lists.c:262-338`, 1195-1263; `bif.c:2931-2938`).
    // `lists:member/2`, `lists:keyfind/3`, and `lists:keymember/3` return a
    // match before the improper tail, else raise `badarg`
    // (`erl_bif_lists.c:1108-1141`, 1302-1381). `list_to_atom/1` raises
    // `system_limit` for 300 characters before it reads the tail
    // (`utils.c:2862-2905`). The spawn BIFs, `demonitor/2`, and the other
    // BIFs raise `badarg` for an improper list (`erl_process.c:12490-12492`,
    // `bif.c:613-654`), and `demonitor/2` with `[info]` gives `false` when
    // it removes no monitor. `demonitor/2` also raises `badarg` for a term
    // that is not a reference, after the option walk, and the spawn BIFs for
    // a module or function that is not an atom. `apply/3` and `apply/2`
    // count the elements of an improper argument list before they read the
    // tail, so 1024 (`apply/3`) or 1023 (`apply/2`) elements raise
    // `system_limit` and fewer raise `badarg` (`beam_common.c:1500-1516`,
    // `jit/arm/instr_fun.cpp:274-297`). An `apply/3` of `erlang:apply/3`
    // takes its inner list only from a proper list of three elements.
    let program = BeamProgram::load(&[IMPROPER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let error = |reason: &str| Term::Tuple(vec![atom("error"), atom(reason)]);
    let badarg = || error("badarg");
    let x = || atom("x");
    let imp = || improper(vec![int(1)], x());
    assert_eq!(
        program.execute("improper_probe", "list_bifs", &[]).unwrap(),
        Term::list_from(
            [
                imp(),
                improper(integers(&[1, 2]), x()),
                badarg(),
                x(),
                x(),
                improper(vec![int(2)], x()),
                badarg(),
                improper(vec![atom("a")], x()),
                improper(vec![atom("b"), atom("a"), atom("c")], x()),
                badarg(),
                imp(),
                atom("true"),
                badarg(),
                Term::Tuple(vec![atom("a")]),
                badarg(),
                atom("true"),
                badarg(),
                error("system_limit"),
            ]
            .into_iter()
            .chain(std::iter::repeat_with(badarg).take(10))
            .chain([atom("false"), atom("true")])
            .chain(std::iter::repeat_with(badarg).take(10))
            .chain([
                badarg(),
                error("system_limit"),
                badarg(),
                error("system_limit"),
                error("system_limit"),
                badarg(),
                badarg(),
            ])
        )
    );
    assert_eq!(
        erl_reference("list_bifs"),
        "[[1|x],[1,2|x],{error,badarg},x,x,[2|x],{error,badarg},[a|x],[b,a,c|x],\
         {error,badarg},[1|x],true,{error,badarg},{a},{error,badarg},true,{error,badarg},\
         {error,system_limit},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
         {error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
         {error,badarg},false,true,{error,badarg},{error,badarg},{error,badarg},\
         {error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
         {error,badarg},{error,badarg},{error,badarg},{error,system_limit},{error,badarg},\
         {error,system_limit},{error,system_limit},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn gives_the_iodata_and_chardata_results_of_pinned_erts_for_improper_lists() {
    // `iodata/0` runs the iodata and chardata BIFs with improper lists. A
    // binary tail of each list level is part of the data, and each other
    // tail raises `badarg`: `erlang:iolist_to_iovec/1` and the `re:compile/2`
    // pattern walk it as `generic_iolist_size` does
    // (`erl_iolist.c:153-236`), and `unicode:characters_to_binary/2` as
    // `utf8_need` does (`erl_unicode.c:342-505`, 963-965). `utf8_need` also
    // rejects `[foo]` and `[-1|x]` before the conversion. An improper option
    // list of `re:compile/2` raises `badarg`.
    let program = BeamProgram::load(&[IMPROPER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badarg = || Term::Tuple(vec![atom("error"), atom("badarg")]);
    let binary = |bytes: &[u8]| Term::Binary(bytes.to_vec());
    assert_eq!(
        program.execute("improper_probe", "iodata", &[]).unwrap(),
        Term::list_from(vec![
            Term::list_from(vec![binary(b"ab")]),
            Term::list_from(vec![binary(b"abc")]),
            badarg(),
            atom("ok"),
            badarg(),
            badarg(),
            binary(b"ab"),
            binary(&[195, 169]),
            badarg(),
            badarg(),
            badarg(),
        ])
    );
    assert_eq!(
        erl_reference("iodata"),
        "[[<<97,98>>],[<<97,98,99>>],{error,badarg},ok,{error,badarg},{error,badarg},\
         <<97,98>>,<<195,169>>,{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}
