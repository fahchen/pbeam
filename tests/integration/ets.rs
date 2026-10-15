use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const ETS_PROBE: &[u8] = include_bytes!("../../fixtures/ets/ets_probe.beam");

/// Run `ets_probe:Function()` under the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    reference_with(&[], function)
}

/// Run `ets_probe:Function()` under the pinned ERTS with the emulator
/// arguments `arguments` and return its `~w` output.
fn reference_with(arguments: &[&str], function: &str) -> Vec<u8> {
    let output = Command::new("erl")
        .args(arguments)
        .args([
            "-noshell",
            "-pa",
            "fixtures/ets",
            "-eval",
            &format!("io:format(\"~w~n\", [ets_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    output.stdout
}

#[test]
fn looks_up_a_key_in_a_loop_as_pinned_erts() {
    // `ets_probe:lookup_loop/0` inserts the 1000 objects {I, I * 2} and then
    // reads each key again in a loop of 2000 lookups. The result is the sum
    // of the values that the lookups read, and the pinned ERTS gives the same
    // result. The loop is the program-level evidence of the key index of
    // #543, and the test prints the least of 9 timed runs of the program.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(timed(&program, "lookup_loop"), Term::Integer(2_002_000));
    assert_eq!(reference("lookup_loop"), b"2002000\n");
}

#[test]
fn inserts_objects_in_a_loop_as_pinned_erts() {
    // `ets_probe:insert_loop/0` fills a set table with the 1000 objects
    // {I, I * 2} and returns the object of the last key, so the program-level
    // evidence shows the cost of an insert and not only the cost of a lookup.
    // The pinned ERTS gives the same result.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        timed(&program, "insert_loop"),
        Term::list_from([Term::tuple(vec![Term::Integer(1000), Term::Integer(2000)])])
    );
    assert_eq!(reference("insert_loop"), b"[{1000,2000}]\n");
}

/// Run `ets_probe:Function()` 9 times on PBeam, print the least of the timed
/// runs, and return the result of the last run.
fn timed(program: &BeamProgram, function: &str) -> Term {
    let mut best = std::time::Duration::MAX;
    let mut result = None;
    for _ in 0..9 {
        let start = std::time::Instant::now();
        result = Some(program.execute("ets_probe", function, &[]).unwrap());
        best = best.min(start.elapsed());
    }
    println!("COST ets_{function}_program {} ns", best.as_nanos());
    result.unwrap().term
}

#[test]
fn makes_tables_and_raises_badarg_as_pinned_erts() {
    // `ets_probe:new/0` makes an unnamed table, a named table, and a table
    // with each option that ERTS accepts (erl_db.c:2597-2710). A heir that
    // is not alive acts as `none` (erl_db.c:5319-5337). A heir that is the
    // caller makes a table, with and without data. Each case of the
    // last list raises `badarg`: a name that is not an atom, options that
    // are not a list, an option that is not valid, `ordered_set` with an
    // explicit lock count (erl_db.c:2717-2758), and a name in use
    // (erl_db.c:2821-2831), also with the type `bag`.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = || Term::tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program.execute("ets_probe", "new", &[]).unwrap().term,
        Term::list_from(vec![
            atom("reference"),
            atom("ets_probe_named"),
            badarg(),
            atom("reference"),
            atom("ets_probe"),
            atom("reference"),
            atom("reference"),
            atom("reference"),
            Term::list_from(vec![badarg(); 19]),
        ])
    );
    assert_eq!(
        reference("new"),
        b"[reference,ets_probe_named,{error,badarg},reference,ets_probe,reference,reference,reference,[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]]\n"
    );
}

#[test]
fn deletes_the_tables_of_an_owner_before_its_down_message_as_pinned_erts() {
    // `ets_probe:owner_exit/0` cannot make a table with the name of a table
    // that another process owns. After the `DOWN` message of the owner, the
    // name is free, because ERTS deletes the tables of an exiting process
    // before its monitors fire (erl_process.c:14360-14368). A table whose
    // heir is its owner goes to no process at the exit (erl_db.c:5076-5078),
    // so its name is free too.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("ets_probe", "owner_exit", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::tuple(vec![atom("error"), atom("badarg")]),
            atom("stopped"),
            atom("ets_probe_owned"),
            Term::tuple(vec![atom("error"), atom("badarg")]),
            atom("stopped"),
            atom("ets_probe_heir"),
        ])
    );
    assert_eq!(
        reference("owner_exit"),
        b"[{error,badarg},stopped,ets_probe_owned,{error,badarg},stopped,ets_probe_heir]\n"
    );
}

#[test]
fn inserts_objects_as_pinned_erts() {
    // `ets_probe:insert/0` adds one tuple and a list of tuples. An object
    // with a key in the table replaces the old object, also in one list
    // (erl_db_hash.c:1387-1402). `[]` returns `true`. Each case of the list
    // raises `badarg`: a term that is not a tuple, a tuple with fewer
    // elements than the key position, a list with one bad element, which
    // adds nothing (erl_db.c:1812-1827), and a table that does not exist.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = || Term::tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program.execute("ets_probe", "insert", &[]).unwrap().term,
        Term::list_from(vec![
            atom("true"),
            atom("true"),
            atom("true"),
            atom("true"),
            atom("true"),
            Term::list_from(vec![badarg(); 9]),
            Term::list_from(vec![atom("v1"), atom("v4"), atom("v3"), atom("d")]),
            atom("v"),
        ])
    );
    assert_eq!(
        reference("insert"),
        b"[true,true,true,true,true,[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[v1,v4,v3,d],v]\n"
    );
}

#[test]
fn looks_up_elements_as_pinned_erts() {
    // `ets_probe:lookup/0` reads elements and defaults. The key position
    // and the index keep only their low 32 bits (erl_db_util.h:346,
    // erl_db_util.h:151-155): the key position `(1 bsl 32) + 2` acts as 2,
    // and the index `(1 bsl 32) + 2` reads element 2. The key position
    // `1 bsl 31` is negative, so each insert of an object raises `badarg`.
    // Each case of the last list raises `badarg`: an index below 1, above
    // the size of the object, not an integer, negative after the
    // truncation, or not a small integer, and a table that does not exist.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = || Term::tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program.execute("ets_probe", "lookup", &[]).unwrap().term,
        Term::list_from(vec![
            atom("v"),
            atom("x"),
            atom("d"),
            atom("v"),
            atom("d"),
            atom("v"),
            badarg(),
            atom("true"),
            atom("d"),
            Term::list_from(vec![badarg(); 8]),
        ])
    );
    assert_eq!(
        reference("lookup"),
        b"[v,x,d,v,d,v,{error,badarg},true,d,[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]]\n"
    );
}

#[test]
fn checks_table_access_as_pinned_erts() {
    // `ets_probe:access/0` reads, writes, and writes `[]` to tables of other
    // processes (erl_db.c:769-798). A `public` table allows each operation,
    // a `protected` table only the read, and a `private` table none.
    // `protected` after `private` changes nothing (erl_db.c:2705-2706). A
    // table and its name are gone after the `DOWN` message of its owner.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = || Term::tuple(vec![atom("error"), atom("badarg")]);
    let all = || Term::tuple(vec![atom("owner"), atom("true"), atom("true")]);
    let none = || Term::tuple(vec![badarg(), badarg(), badarg()]);
    assert_eq!(
        program.execute("ets_probe", "access", &[]).unwrap().term,
        Term::list_from(vec![
            Term::list_from(vec![
                all(),
                Term::tuple(vec![atom("owner"), badarg(), badarg()]),
                none(),
                none(),
                all(),
            ]),
            badarg(),
            atom("owner"),
            atom("stopped"),
            badarg(),
            atom("ets_probe_owned"),
            atom("d"),
            Term::list_from(vec![atom("stopped"); 5]),
            badarg(),
        ])
    );
    assert_eq!(
        reference("access"),
        b"[[{owner,true,true},{owner,{error,badarg},{error,badarg}},{{error,badarg},{error,badarg},{error,badarg}},{{error,badarg},{error,badarg},{error,badarg}},{owner,true,true}],{error,badarg},owner,stopped,{error,badarg},ets_probe_owned,d,[stopped,stopped,stopped,stopped,stopped],{error,badarg}]\n"
    );
}

#[test]
fn reads_the_application_environment_from_ac_tab_as_pinned_erts() {
    // `application:get_env/3` calls `application_controller:get_env/3`,
    // which reads `ac_tab` with `ets:lookup_element/4`
    // (application_controller.erl:360-361). On ERTS,
    // `application_controller` makes `ac_tab` with `[set, public,
    // named_table, {read_concurrency, true}]`
    // (application_controller.erl:544-545). PBeam has no
    // `application_controller`, so `ets_probe:main/0` makes `ac_tab` with
    // these options in an owner process, then runs `env/0`. The reference
    // run calls `env/0` with the `ac_tab` of `application_controller` and
    // shows the same options with `ets:info/2`.
    let application = pinned_beam("application");
    let application_controller = pinned_beam("application_controller");
    let program = BeamProgram::load(&[&application, &application_controller, ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("ets_probe", "main", &[]).unwrap().term,
        Term::list_from(vec![atom("default"), atom("true"), atom("value")])
    );
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/ets",
            "-eval",
            "io:format(\"~w~n\", [[[ets:info(ac_tab, I) || I <- [type, protection, named_table, keypos, read_concurrency]], ets_probe:env()]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        b"[[set,public,true,1,true],[default,true,value]]\n"
    );
}

#[test]
fn keeps_the_objects_of_one_key_in_bag_tables_as_pinned_erts() {
    // `ets_probe:bag/0` fills a `bag`, a `duplicate_bag`, and a `[bag, set]`
    // table, which is a bag (erl_db.c:2599-2611). A new object goes before
    // the other objects of its key, and a lookup reverses the chain, so the
    // objects come in insertion order (erl_db_hash.c:1447-1490). A bag keeps
    // no second object that is exactly equal to one in the table, also in
    // one list insert (erl_db_hash.c:1422-1442). lookup_element/4 gives the
    // list of the elements, and an index above the size of one object
    // raises badarg (erl_db_hash.c:1563-1580). delete/2 removes each object
    // of the key (erl_db_hash.c:1613-1664).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let k = |value| Term::tuple(vec![atom("k"), int(value)]);
    let results = |table_type, objects: Vec<Term>, elements: Vec<i64>| {
        Term::list_from(vec![
            atom(table_type),
            Term::list_from(objects),
            Term::list_from(elements.into_iter().map(int)),
            atom("d"),
            Term::tuple(vec![atom("error"), atom("badarg")]),
            atom("true"),
            atom("true"),
            Term::nil(),
            atom("false"),
            Term::list_from(vec![Term::tuple(vec![atom("j"), int(1)])]),
        ])
    };
    let kx = Term::tuple(vec![atom("k"), int(4), atom("x")]);
    let bag = || results("bag", vec![k(1), k(2), k(3), kx.clone()], vec![1, 2, 3, 4]);
    let duplicate_bag = results(
        "duplicate_bag",
        vec![k(1), k(2), k(1), k(3), k(2), kx.clone(), k(3)],
        vec![1, 2, 1, 3, 2, 4, 3],
    );
    assert_eq!(
        program.execute("ets_probe", "bag", &[]).unwrap().term,
        Term::list_from(vec![bag(), duplicate_bag, bag()])
    );
    let bag =
        "[bag,[{k,1},{k,2},{k,3},{k,4,x}],[1,2,3,4],d,{error,badarg},true,true,[],false,[{j,1}]]";
    let duplicate_bag = "[duplicate_bag,[{k,1},{k,2},{k,1},{k,3},{k,2},{k,4,x},{k,3}],[1,2,1,3,2,4,3],d,{error,badarg},true,true,[],false,[{j,1}]]";
    assert_eq!(
        String::from_utf8(reference("bag")).unwrap(),
        format!("[{bag},{duplicate_bag},{bag}]\n")
    );
}

#[test]
fn matches_patterns_in_set_and_bag_tables_as_pinned_erts() {
    // `ets_probe:match/0` runs ets:match/2,3 on a set, a bag, and a
    // duplicate_bag table (erl_db.c:3771-3818). The matches come in the
    // reverse order of the traversal: the buckets from 0 up, and each
    // bucket chain in order (erl_db_hash.c:1835-1971, 2174-2185). '$$'
    // gives the variables in the order of their numbers
    // (erl_db_util.c:4664-4684). A bound key reads only its bucket
    // (erl_db_hash.c:3247-3290). ets:match/3 splits the matches over the
    // limit into the rest of the continuation (erl_db_hash.c:2187-2262). A
    // map key that is a variable or '_', a bad table, and a bad limit raise
    // badarg. The match of 4999 objects traps, and its result is the same.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let result = program.execute("ets_probe", "match", &[]).unwrap();
    let expected = "[[[{{1,2},v}],[{4,4}],[{3,three}],[{[115],z}],[{2,y}],[{<<98>>,f}],[{1,x}],[{c,3}],[{b,2}],[{a,1}]],[[{1,2},v],[4,4],[3,three],[[115],z],[2,y],[<<98>>,f],[1,x],[c,3],[b,2],[a,1]],[[v,{1,2}],[4,4],[three,3],[z,[115]],[y,2],[f,<<98>>],[x,1],[3,c],[2,b],[1,a]],[[{1,2}],[4],[3],[[115]],[2],[<<98>>],[1],[c],[b],[a]],[[{1,2},v],[4,4],[3,three],[[115],z],[2,y],[<<98>>,f],[1,x],[c,3],[b,2],[a,1]],[[4]],[[1]],[[]],[[f]],[],[[]],[],[],[],[],[],[[1]],[[n],[m]],[[m]],[],[[z]],[[],[],[],[],[],[],[],[],[],[]],[[1]],[],'$end_of_table',[[115]],[{[[c],[b],[a]],true,50,3,true,[],0},{[[{1,2}],[4],[3],[[115]],[2],[<<98>>],[1],[c],[b],[a]],'$end_of_table'},'$end_of_table','$end_of_table',{[[1]],'$end_of_table'}],[{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}],{error,badarg},[[[2.0]],[[1]],[[1.0]],[[z]],[[1]]],[[[[{j,1}],[{j,2}],[{k,1}],[{k,2}],[{k,1,x}]],[[1],[2]],[[j],[k]],[[x]],{[[2]],true,-1,1,true,[[1]],1},{[[{k,2}],[{k,1,x}]],true,54,2,true,[[{j,1}],[{j,2}],[{k,1}]],3}],[[[{j,1}],[{j,2}],[{k,1}],[{k,2}],[{k,1,x}],[{k,1}]],[[1],[2],[1]],[[j],[k],[k]],[[x]],{[[1]],true,-1,1,true,[[1],[2]],2},{[[{k,1,x}],[{k,1}]],true,54,2,true,[[{j,1}],[{j,2}],[{k,1}],[{k,2}]],4}]],[4999,[[4085,8170],[127,254],[4820,9640],[3793,7586],[82,164]],687245333,2000,506731475],[1001,3950707,1303,1001,[[367,2],[367,3],[367,4],[367,5],[367,6],[367,7],[367,8],[367,9],[367,10]],9],{error,badarg},{error,badarg}]";
    assert_eq!(text(&result.term, program.atoms()), expected);
    assert_eq!(
        String::from_utf8(reference("match")).unwrap(),
        format!("{expected}\n")
    );
}

#[test]
fn matches_a_deep_pattern_as_pinned_erts() {
    // `ets_probe:match_deep/0` matches a pattern 1000000 tuples deep. The
    // compile and the match use a work stack, as db_match_compile does
    // (erl_db_util.c:1700-2100), so the Rust stack does not overflow.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let result = program.execute("ets_probe", "match_deep", &[]).unwrap();
    assert_eq!(text(&result.term, program.atoms()), "[[x]]");
    assert_eq!(reference("match_deep"), b"[[x]]\n");
}

#[test]
fn ignores_write_concurrency_in_a_match_as_pinned_erts_with_one_scheduler() {
    // PBeam has one scheduler. ERTS with one scheduler makes no table with
    // fine locks (erl_db.c:2656-2657; erl_db_util.h:388-389), so a
    // traversal steps one bucket at a time (erl_db_hash.c:1023-1030,
    // 539-554) also with {write_concurrency, true}.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let result = program
        .execute("ets_probe", "match_write_concurrency", &[])
        .unwrap();
    let expected = "[300,[[141],[194],[257],[87],[278],[137]],870028092]";
    assert_eq!(text(&result.term, program.atoms()), expected);
    assert_eq!(
        String::from_utf8(reference_with(&["+S", "1"], "match_write_concurrency")).unwrap(),
        format!("{expected}\n")
    );
}

/// Return the `~w` text of `term`, for the terms of `ets_probe:match/0`.
fn text(term: &Term, atoms: &pbeam_runtime::AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom)
            if atoms
                .name_of(atom)
                .starts_with(|c: char| c.is_ascii_lowercase()) =>
        {
            atoms.name_of(atom).to_owned()
        }
        Term::Atom(atom) => format!("'{}'", atoms.name_of(atom)),
        Term::Integer(value) => value.to_string(),
        Term::Float(value) => format!("{value:?}"),
        Term::Binary(bytes) => format!(
            "<<{}>>",
            bytes
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn gives_the_table_type_as_pinned_erts() {
    // `ets_probe:info/0` reads the item type of a set and a bag table
    // (erl_db.c:5457-5469). A free name gives undefined before the item
    // check (erl_db.c:4544-4547). An unknown item, an item that is not an
    // atom, and a table that is not an atom or a reference raise badarg.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = || Term::tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program.execute("ets_probe", "info", &[]).unwrap().term,
        Term::list_from(vec![
            atom("set"),
            atom("bag"),
            atom("undefined"),
            atom("undefined"),
            Term::list_from(vec![badarg(); 4]),
        ])
    );
    assert_eq!(
        reference("info"),
        b"[set,bag,undefined,undefined,[{error,badarg},{error,badarg},{error,badarg},{error,badarg}]]\n"
    );
}

#[test]
fn rejects_tables_that_pbeam_cannot_model() {
    // ERTS 29.1.1 gives the size of a table, the binary item, and badarg
    // for a reference that is not a table, and it makes an ordered_set
    // table and two set tables whose heir is a live process other than the
    // caller. PBeam gives only the type item of ets:info/2, keeps no record
    // of a deleted table, and supports only hash tables without such a
    // heir. ERTS
    // aborts the emulator for an insert with the key position 0 (status
    // 134, `Invalid tag in make_internal_hash`) and crashes for a found key
    // and an index that becomes 0 (status 139). This is the "ETS tables"
    // row in `docs/erts-differences.md`.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    for (case, detail) in [
        ("info_size", "info/2 item size"),
        ("info_binary", "info/2 item binary"),
        (
            "info_reference",
            "info/2 of a reference that is not a live table",
        ),
        ("ordered_set", "new/2 table type ordered_set"),
        ("heir", "new/2 heir ProcessId("),
        ("heir_data", "new/2 heir ProcessId("),
        ("keypos", "insert/2 in a table with key position 0"),
        ("index", "lookup_element/4 index 4294967296 with position 0"),
        (
            "match_changed",
            "match/2,3 on a table that changed while the match trapped",
        ),
    ] {
        let message = format!("unsupported ets:{detail}");
        let case = Term::Atom(program.atom(case).unwrap());
        let error = program
            .execute("ets_probe", "unsupported", &[program.host_term(case)])
            .unwrap_err();
        assert!(
            error.to_string().contains(&message),
            "expected {message}, got {error}"
        );
    }
    // ets:match/1 reads the continuation of ets:match/3, and PBeam does not
    // run it yet.
    let case = Term::Atom(program.atom("match_1").unwrap());
    let error = program
        .execute("ets_probe", "unsupported", &[program.host_term(case)])
        .unwrap_err();
    assert_eq!(error.to_string(), "unresolved external MFA ets:match/1");
}
