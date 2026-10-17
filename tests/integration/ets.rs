use std::process::Command;
use std::sync::Arc;

use num_bigint::BigInt;
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
fn inserts_new_objects_as_pinned_erts() {
    // `ets_probe:insert_new/0` runs `ets:insert_new/2` in a set, a bag, and
    // a duplicate_bag table. A key that the table holds gives `false`, and
    // a list with such a key inserts no object (erl_db.c:2091-2098). A key
    // that only the list holds two times goes in (erl_db.c:1829-1842). Each
    // bad object, improper list, missing table, and table of another
    // process raises `badarg`, and inserts no object (erl_db.c:2387-2440).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let result = program.execute("ets_probe", "insert_new", &[]).unwrap();
    let expected = String::from_utf8(reference("insert_new")).unwrap();
    assert_eq!(
        format!("{}\n", text(&result.term, result.atoms())),
        expected
    );
    assert!(expected.starts_with("[{[true,false,false,true,true,false,false,true],"));
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
fn completes_a_busy_list_insert_as_pinned_erts() {
    // `ets_probe:busy/0` runs a list insert of 24000 objects alone, and
    // with a process that uses the table while the insert traps after its
    // copy (`ETS_INSERT_2_LIST_GLOBAL`, `erl_db.c:2077-2107`,
    // `2310-2319`). The second process completes the insert first, so its
    // lookup finds the last object, and it charges the units of the work
    // with its own reductions (`handle_lacking_permission`,
    // `erl_db.c:769-798`, `2152-2197`). The insert of the table then
    // returns `true` with no unit left (`erl_db.c:2223-2244`). Each number
    // is the reductions of the process, and the pinned ERTS with one
    // scheduler gives the same numbers.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let result = program.execute("ets_probe", "busy", &[]).unwrap();
    let expected = String::from_utf8(reference_with(&["+S", "1"], "busy")).unwrap();
    assert_eq!(text(&result.term, result.atoms()), expected.trim_end());
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
    // ERTS 29.1.1 gives the binary item, and it makes two set tables whose
    // heir is a live process other than the caller. PBeam gives no binary
    // item, keeps no record of a deleted table, and keeps no such heir. ERTS
    // aborts the emulator for an insert with the key position 0 (status
    // 134, `Invalid tag in make_internal_hash`) and crashes for a found key
    // and an index that becomes 0 (status 139). This is the "ETS tables"
    // row in `docs/erts-differences.md`.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    for (case, detail) in [
        (
            "delete_inserting",
            "delete/1 of a table that a trapped insert uses",
        ),
        ("info_binary", "info/2 item binary"),
        ("info_stats", "info/2 item stats"),
        ("info_data", "info/2 item data"),
        (
            "info_all_reference",
            "info/1 of a reference that is not a live table",
        ),
        ("select_unsupported", "match specification function round/1"),
        (
            "select_delete_unsupported",
            "match specification function round/1",
        ),
        (
            "info_reference",
            "info/2 of a reference that is not a live table",
        ),
        ("ordered_next", "next/2 on an ordered_set table"),
        ("ordered_select", "select/2,3 on an ordered_set table"),
        (
            "ordered_select_delete",
            "select_delete/2 on an ordered_set table",
        ),
        ("heir", "new/2 heir ProcessId("),
        ("heir_data", "new/2 heir ProcessId("),
        ("keypos", "insert/2 in a table with key position 0"),
        ("keypos_new", "insert_new/2 in a table with key position 0"),
        ("index", "lookup_element/4 index 4294967296 with position 0"),
        (
            "match_changed",
            "match/2,3 or ets:select/2,3 on a table that changed while the match trapped",
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

#[test]
fn deletes_tables_as_pinned_erts() {
    // `ets:delete/1` asks for `DB_WRITE` (`erl_db.c:3021`): each process can
    // delete a `public` table, and only the owner can delete another table.
    // The table loses its name and its reference at once
    // (`erl_db.c:3053-3061`), so a later call raises `badarg`, and the name
    // is free again. Each other term raises `badarg`.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    let owner = Term::tuple(vec![atom("k"), atom("owner")]);
    assert_eq!(
        program
            .execute("ets_probe", "delete_table", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("true"),
            badarg.clone(),
            badarg.clone(),
            atom("true"),
            atom("undefined"),
            atom("ets_probe_named"),
            atom("true"),
            badarg.clone(),
            atom("stopped"),
            badarg.clone(),
            Term::list_from(vec![owner]),
            atom("stopped"),
            Term::list_from(vec![badarg; 4]),
        ])
    );
    assert_eq!(
        reference("delete_table"),
        b"[true,{error,badarg},{error,badarg},true,undefined,ets_probe_named,true,{error,badarg},stopped,{error,badarg},[{k,owner}],stopped,[{error,badarg},{error,badarg},{error,badarg},{error,badarg}]]\n"
    );
}

#[test]
fn traps_the_delete_of_a_large_table_as_pinned_erts() {
    // Each segment uses 32 reductions and 1 for each object
    // (`erl_db_hash.c:3140-3149`). The small table frees in the turn of
    // the caller, so the spawned process has not run. The table of 50000
    // objects needs more reductions than the turn has, so the delete traps
    // to `ets:delete_trap` (`erl_db.c:3064-3077`, `5378-5397`), and the
    // spawned process sends `ping` before the delete returns. One ERTS
    // scheduler gives the same order.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("ets_probe", "delete_trap", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("true"),
            atom("none"),
            atom("true"),
            atom("ping"),
            atom("true"),
            atom("ping"),
        ])
    );
    assert_eq!(
        reference_with(&["+S", "1"], "delete_trap"),
        b"[true,none,true,ping,true,ping]\n"
    );
}

#[test]
fn frees_the_owned_tables_of_a_killed_deleter_in_owned_list_order_as_pinned_erts() {
    // `ets_probe:delete_order/0` kills a process while its `ets:delete/1` of
    // a large table traps. The exit frees the owned list from its head
    // (`erl_db.c:5055-5096`). The large table of the process keeps its
    // position after the small table, and a public table of another owner
    // goes to the end of the list (`erl_db.c:628-650`, `3031-3051`). So in
    // both cases a lookup of the small table raises `badarg` in the first
    // loop, before the `DOWN` message. One ERTS scheduler gives the same
    // loops.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let pair = |first, down| Term::tuple(vec![Term::Integer(first), Term::Integer(down)]);
    assert_eq!(
        program
            .execute("ets_probe", "delete_order", &[])
            .unwrap()
            .term,
        Term::list_from(vec![pair(1, 10), pair(1, 9)])
    );
    assert_eq!(
        reference_with(&["+S", "1"], "delete_order"),
        b"[{1,10},{1,9}]\n"
    );
}

/// Run `ets_probe:Function()` on PBeam with the pinned `ets` module and
/// compare its `~w` text with the output of the pinned ERTS with one
/// scheduler.
fn assert_probe_matches_erts(program: &BeamProgram, function: &str) {
    let result = program.execute("ets_probe", function, &[]).unwrap().term;
    assert_eq!(
        format!("{}\n", text(&result, program.atoms())),
        String::from_utf8(reference_with(&["+S", "1"], function)).unwrap(),
        "{function}"
    );
}

#[test]
fn runs_the_select_and_iteration_bifs_as_pinned_erts() {
    // `ets_probe:select/0`, `match_object/0`, `iterate/0`, `update_element/0`,
    // `info_items/0`, `delete_all/0`, and `select_reductions/0` run the new
    // `ets` BIFs. ERTS runs with one scheduler, as PBeam does, so
    // `ets:info/2` gives `false` for `write_concurrency` and
    // `decentralized_counters` (erl_db.c:2656-2657, 2776-2779).
    let ets = pinned_beam("ets");
    let program = BeamProgram::load(&[&ets, ETS_PROBE]).unwrap();
    for function in [
        "select",
        "match_object",
        "iterate",
        "update_element",
        "info_items",
        "delete_all",
        "select_reductions",
        "ordered_set",
        "ordered_set_reductions",
        "take",
    ] {
        assert_probe_matches_erts(&program, function);
    }
}

#[test]
fn returns_the_safe_fixtable_results_of_pinned_erts() {
    // `ets_safe_fixtable_2` (`erl_db.c:1136-1164`) gives true for `true`
    // and for `false`, and raises badarg for another second argument and
    // for a term that is not a table of the run.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program
            .execute("ets_probe", "safe_fixtable", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("true"),
            atom("true"),
            atom("true"),
            atom("true"),
            badarg.clone(),
            badarg.clone(),
            badarg.clone(),
            badarg,
        ])
    );
    assert_eq!(
        reference("safe_fixtable"),
        b"[true,true,true,true,{error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn keeps_a_deleted_key_of_a_fixed_table_for_next_as_pinned_erts() {
    // A fixed table keeps the key of each deleted object for ets:next/2
    // (`erl_db_hash.c:49-60`), and an unfixed table gives badarg for a key
    // that the table does not hold.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("ets_probe", "fixtable", &[]).unwrap().term,
        Term::list_from(vec![
            atom("$end_of_table"),
            Term::tuple(vec![atom("error"), atom("badarg")]),
        ])
    );
    assert_eq!(reference("fixtable"), b"['$end_of_table',{error,badarg}]\n");
}

#[test]
fn continues_a_fixed_traversal_over_deleted_keys_as_pinned_erts() {
    // Each deleted key follows the deleted key before it, so the walk ends
    // at the end of the table (`erl_db_hash.c:49-60`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_chain", &[])
            .unwrap()
            .term,
        Term::list_from(vec![atom("$end_of_table")])
    );
    assert_eq!(reference("fixtable_chain"), b"['$end_of_table']\n");
}

#[test]
fn keeps_the_size_of_a_fixed_table_as_pinned_erts() {
    // An insert or a delete while the table is fixed does not split or join
    // a bucket (`erl_db_hash.c:49-60`); the size still counts the objects.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_resize", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::Integer(10_000), Term::Integer(0)])
    );
    assert_eq!(reference("fixtable_resize"), b"[10000,0]\n");
}

#[test]
fn gives_the_next_live_key_of_a_fixed_table_as_pinned_erts() {
    // The ghost of the deleted key holds its successor, so `ets:next/2`
    // gives the live key after it (`erl_db_hash.c:49-60`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_successor", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::Integer(2)])
    );
    assert_eq!(reference("fixtable_successor"), b"[2]\n");
}

/// Return the single-key list that the probe `function` gives, with
/// `eot` for `'$end_of_table'` and `int` for an integer key.
fn fixture_fixed(function: &str, expected: Term, text: &[u8]) {
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program.execute("ets_probe", function, &[]).unwrap().term,
        Term::list_from(vec![expected])
    );
    assert_eq!(reference(function), text);
}

#[test]
fn counts_each_fix_of_a_table_as_pinned_erts() {
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let eot = Term::Atom(program.atom("$end_of_table").unwrap());
    fixture_fixed("fixtable_fix_count", eot, b"['$end_of_table']\n");
}

#[test]
fn gives_a_key_inserted_after_a_fixed_delete_as_pinned_erts() {
    fixture_fixed("fixtable_insert_after_delete", Term::Integer(2), b"[2]\n");
}

#[test]
fn keeps_the_position_of_a_cleared_fixed_table_as_pinned_erts() {
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let eot = Term::Atom(program.atom("$end_of_table").unwrap());
    fixture_fixed("fixtable_clear", eot, b"['$end_of_table']\n");
}

#[test]
fn keeps_the_position_of_a_deleted_object_of_a_fixed_table_as_pinned_erts() {
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let eot = Term::Atom(program.atom("$end_of_table").unwrap());
    fixture_fixed("fixtable_delete_object", eot, b"['$end_of_table']\n");
}

#[test]
fn follows_a_same_bucket_predecessor_of_a_fixed_table_as_pinned_erts() {
    // The ghost of the deleted key follows its live predecessor, and then
    // the ghost of a predecessor that the run deleted too
    // (`erl_db_hash.c:49-60`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let eot = Term::Atom(program.atom("$end_of_table").unwrap());
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_same_bucket", &[])
            .unwrap()
            .term,
        Term::list_from(vec![eot.clone(), eot])
    );
    assert_eq!(
        reference("fixtable_same_bucket"),
        b"['$end_of_table','$end_of_table']\n"
    );
}

#[test]
fn keeps_the_position_of_a_wide_clear_of_a_fixed_table_as_pinned_erts() {
    // Each key of a 1000-object table ends its traversal after the clear,
    // also when the bucket of its ghost is outside the 256 buckets of the
    // new table.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_clear_wide", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::Integer(1000)])
    );
    assert_eq!(reference("fixtable_clear_wide"), b"[1000]\n");
}

#[test]
fn gives_a_key_after_a_live_predecessor_of_a_fixed_table_as_pinned_erts() {
    // The deleted key follows its live predecessor, so a key that the run
    // inserts after the predecessor is after the deleted key
    // (`erl_db_hash.c:49-60`).
    fixture_fixed(
        "fixtable_live_after_predecessor",
        Term::Integer(136),
        b"[136]\n",
    );
}

#[test]
fn ends_a_fixed_traversal_at_two_ghosts_that_follow_each_other_as_pinned_erts() {
    // The deleted keys of one bucket follow each other, so the ghost walk
    // ends at the end of the table (`erl_db_hash.c:49-60`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let eot = Term::Atom(program.atom("$end_of_table").unwrap());
    fixture_fixed("fixtable_cycle", eot, b"['$end_of_table']\n");
}

#[test]
fn gives_a_key_inserted_before_the_successor_as_pinned_erts() {
    // The deleted key keeps its chain node, so a key that the run inserts
    // into a bucket before the saved successor is the next key
    // (`erl_db_hash.c:49-60`).
    fixture_fixed(
        "fixtable_insert_before_successor",
        Term::Integer(17),
        b"[17]\n",
    );
}

#[test]
fn reuses_the_position_of_a_reinserted_key_as_pinned_erts() {
    // The insert of a deleted key reuses its chain node
    // (`erl_db_hash.c:49-60`), so the traversal of the key continues after
    // its old position.
    fixture_fixed("fixtable_reinsert", Term::Integer(146), b"[146]\n");
}

#[test]
fn keeps_the_clear_positions_of_reinserted_keys_as_pinned_erts() {
    // The clear keeps each chain node, and the reinserted key reuses its
    // old position, which is before the other deleted key
    // (`erl_db_hash.c:49-60`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let eot = Term::Atom(program.atom("$end_of_table").unwrap());
    fixture_fixed("fixtable_clear_reinsert", eot, b"['$end_of_table']\n");
}

#[test]
fn skips_the_deleted_nodes_of_a_clear_as_pinned_erts() {
    // The clear keeps the chain nodes of the live keys and skips the nodes
    // that an earlier delete kept (`erl_db_hash.c:49-60`, `4027-4028`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let eot = Term::Atom(program.atom("$end_of_table").unwrap());
    fixture_fixed("fixtable_clear_after_delete", eot, b"['$end_of_table']\n");
}

#[test]
fn reuses_the_bag_node_of_a_reinserted_key_as_pinned_erts() {
    // The insert of the key reuses its chain node and frees the other
    // pseudo-deleted objects of the key (`erl_db_hash.c:49-60`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_bag_reinsert", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::list_from(vec![Term::tuple(vec![atom("k"), Term::Integer(3)])]),
            atom("true"),
            Term::list_from(vec![Term::tuple(vec![atom("j"), Term::Integer(4)])]),
        ])
    );
    assert_eq!(
        reference("fixtable_bag_reinsert"),
        b"[[{k,3}],true,[{j,4}]]\n"
    );
}

#[test]
fn keeps_the_buckets_of_a_fixed_table_that_loses_its_objects_as_pinned_erts() {
    // A select_delete of every object of a fixed table does not join
    // buckets (`erl_db_hash.c:49-60`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_shrink", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::Integer(5000), Term::Integer(0)])
    );
    assert_eq!(reference("fixtable_shrink"), b"[5000,0]\n");
}

#[test]
fn reuses_the_duplicate_bag_node_of_a_reinserted_key_as_pinned_erts() {
    // The insert of the key reuses its chain node and frees the other
    // pseudo-deleted objects of the key (`erl_db_hash.c:49-60`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_duplicate_bag_reinsert", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::list_from(vec![Term::tuple(vec![atom("k"), Term::Integer(3)])]),
            atom("true"),
            Term::list_from(vec![Term::tuple(vec![atom("j"), Term::Integer(4)])]),
        ])
    );
    assert_eq!(
        reference("fixtable_duplicate_bag_reinsert"),
        b"[[{k,3}],true,[{j,4}]]\n"
    );
}

#[test]
fn gives_the_fixation_state_of_an_info_item_as_pinned_erts() {
    // `fixed` gives whether a process fixed the table, and `safe_fixed` of
    // an unfixed table gives false (`erl_db.c:5447-5625`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_info", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("false"),
            atom("true"),
            atom("false"),
            atom("false"),
        ])
    );
    assert_eq!(reference("fixtable_info"), b"[false,true,false,false]\n");
}

#[test]
fn helps_a_fixed_clear_from_another_process_as_pinned_erts() {
    // A process that uses the table while a fixed clear does not finish
    // marks the next buckets, 8 for each reduction
    // (`delete_all_objects_continue`, `erl_db.c:3300-3320`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_clear_help", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::Integer(100000), Term::Integer(0)])
    );
    assert_eq!(reference("fixtable_clear_help"), b"[100000,0]\n");
}

#[test]
fn clears_a_large_fixed_table_in_turns_as_pinned_erts() {
    // The clear marks 8 buckets for each reduction, so a table with more
    // buckets than the turn allows traps and continues in a later turn
    // (`erl_db_hash.c:3007-3051`, `erl_db.c:3255-3279`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_clear_large", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::Integer(33000), Term::Integer(0)])
    );
    assert_eq!(reference("fixtable_clear_large"), b"[33000,0]\n");
}

#[test]
fn inserts_a_list_new_into_a_fixed_bag_key_as_pinned_erts() {
    // A list `ets:insert_new/2` does its own membership walk and then
    // inserts as a put, which restores the pseudo-deleted object
    // (`erl_db.c:1858`, `1943`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let object = |value| Term::tuple(vec![atom("k"), Term::Integer(value)]);
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_insert_new_list", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("true"),
            Term::list_from(vec![object(2), object(1)]),
        ])
    );
    assert_eq!(
        reference("fixtable_insert_new_list"),
        b"[true,[{k,2},{k,1}]]\n"
    );
}

#[test]
fn inserts_new_into_a_fixed_bag_key_as_pinned_erts() {
    // The key-clash walk of `ets:insert_new/2` adds a new object for a key
    // whose objects are all pseudo-deleted (`erl_db_hash.c:1412-1422`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let object = |value| Term::tuple(vec![atom("k"), Term::Integer(value)]);
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_insert_new", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("true"),
            Term::list_from(vec![object(1)]),
            Term::list_from(vec![object(1)]),
            Term::BigInt(Arc::new(BigInt::from(u64::MAX))),
        ])
    );
    assert_eq!(
        reference("fixtable_insert_new"),
        b"[true,[{k,1}],[{k,1}],18446744073709551615]\n"
    );
}

#[test]
fn clears_a_table_after_a_fixed_take_as_pinned_erts() {
    // A completed clear resets the item counter, so a table that a take
    // counted below 0 reports the size 0 (`RESET_NITEMS`,
    // `erl_db_hash.c:3048-3050`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_take_clear", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::BigInt(Arc::new(BigInt::from(u64::MAX))),
            Term::Integer(0),
        ])
    );
    assert_eq!(
        reference("fixtable_take_clear"),
        b"[18446744073709551615,0]\n"
    );
}

#[test]
fn takes_a_key_of_a_fixed_bag_as_pinned_erts() {
    // The take keeps the first live object as a pseudo-deleted object and
    // frees the records after it; the item counter falls by each record, so
    // the size wraps below 0 (`db_take_hash`, `erl_db_hash.c:2949-2995`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let object = |value| Term::tuple(vec![atom("k"), Term::Integer(value)]);
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_take", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::list_from(vec![object(1), object(3)]),
            Term::BigInt(Arc::new(BigInt::from(u64::MAX))),
            Term::nil(),
            Term::list_from(vec![
                Term::list_from(vec![Term::tuple(vec![atom("j"), Term::Integer(9)])]),
                Term::list_from(vec![Term::tuple(vec![atom("l"), Term::Integer(10)])]),
            ]),
        ])
    );
    assert_eq!(
        reference("fixtable_take"),
        b"[[{k,1},{k,3}],18446744073709551615,[],[[{j,9}],[{l,10}]]]\n"
    );
}

#[test]
fn deletes_a_table_that_the_process_fixed_as_pinned_erts() {
    // The explicit delete of a fixed table removes the fixations and frees
    // the table (`erl_db.c:3053-3061`, `5304-5318`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_delete_table", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::Atom(program.atom("true").unwrap())])
    );
    assert_eq!(reference("fixtable_delete_table"), b"[true]\n");
}

#[test]
fn deletes_the_key_of_a_fixed_ordered_set_as_pinned_erts() {
    // An ordered_set deletes as usual while fixed, because its traversal is
    // not supported and its compaction moves objects
    // (`erl_db_hash.c:49-60`; row "ETS tables" of `docs/erts-differences.md`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_ordered_set", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::nil(), Term::Integer(0)])
    );
    assert_eq!(reference("fixtable_ordered_set"), b"[[],0]\n");
}

#[test]
fn stops_at_the_safe_fixed_details_of_a_fixed_table() {
    // PBeam does not keep the fixations of each process with their times
    // and counts, so the item returns a runtime error (row "ETS tables" of
    // `docs/erts-differences.md`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let error = program
        .execute("ets_probe", "fixtable_safe_fixed", &[])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported ets:info/2 item safe_fixed"),
        "{error}"
    );
}

#[test]
fn keeps_the_deleted_objects_of_a_bag_key_as_pinned_erts() {
    // The insert of the key keeps the objects of the deleted key, so a later
    // insert compares them too, and the lookup gives the live objects
    // (`erl_db_hash.c:1422-1452`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let object = |value| Term::tuple(vec![atom("k"), Term::Integer(value)]);
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_bag_repeat", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::list_from(vec![object(3), object(4)])])
    );
    assert_eq!(reference("fixtable_bag_repeat"), b"[[{k,3},{k,4}]]\n");
}

#[test]
fn keeps_a_partially_deleted_bag_object_as_pinned_erts() {
    // A select_delete of one object of a bag keeps it as a pseudo-deleted
    // object of the live key (`erl_db_hash.c:2634-2648`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let object = |value| Term::tuple(vec![atom("k"), Term::Integer(value)]);
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_bag_partial", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::Integer(1),
            Term::list_from(vec![object(1), object(3)]),
        ])
    );
    assert_eq!(reference("fixtable_bag_partial"), b"[1,[{k,1},{k,3}]]\n");
}

#[test]
fn counts_a_double_deleted_bag_key_once_as_pinned_erts() {
    // The delete of the key marks the live object only, so the size is 0
    // and no slot is counted twice (`erl_db_hash.c:49-60`).
    fixture_fixed("fixtable_bag_double_delete", Term::Integer(0), b"[0]\n");
}

#[test]
fn restores_the_object_of_a_kept_bag_key_as_pinned_erts() {
    // The insert of the object that a select delete kept restores it and
    // keeps the insertion order (`erl_db_hash.c:1426-1437`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let object = |value| Term::tuple(vec![atom("k"), Term::Integer(value)]);
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_bag_restore", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::list_from(vec![object(1), object(2)])])
    );
    assert_eq!(reference("fixtable_bag_restore"), b"[[{k,1},{k,2}]]\n");
}

#[test]
fn keeps_a_bag_key_gone_after_its_last_live_object_as_pinned_erts() {
    // The second select delete leaves the key without a live object, so the
    // unfix frees each slot and the lookup gives no object
    // (`erl_db_hash.c:2634-2648`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_bag_last_live", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::nil(), Term::nil()])
    );
    assert_eq!(reference("fixtable_bag_last_live"), b"[[],[]]\n");
}

#[test]
fn skips_a_pseudo_deleted_object_for_the_newest_live_object_as_pinned_erts() {
    // The newest live object of the key after the delete skips the
    // pseudo-deleted object behind it (`erl_db_hash.c:2634-2648`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_bag_skip_deleted", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::list_from(vec![Term::tuple(vec![
            atom("k"),
            Term::Integer(1),
        ])])])
    );
    assert_eq!(reference("fixtable_bag_skip_deleted"), b"[[{k,1}]]\n");
}

#[test]
fn keeps_a_bag_key_with_a_deleted_node_and_a_live_object_as_pinned_erts() {
    // The chain node of the key is pseudo-deleted, but the key still has a
    // live object, so the next key passes the key
    // (`erl_db_hash.c:1147-1200`).
    fixture_fixed("fixtable_next_live_behind_node", Term::Integer(2), b"[2]\n");
}

#[test]
fn clears_the_live_objects_behind_a_deleted_node_as_pinned_erts() {
    // The clear of a fixed table frees every live object of the key, also
    // behind a pseudo-deleted chain node (`erl_db_hash.c:4027-4028`).
    fixture_fixed("fixtable_clear_behind_node", Term::nil(), b"[[]]\n");
}

#[test]
fn restores_a_bag_object_behind_a_deleted_node_as_pinned_erts() {
    // The insert restores the pseudo-deleted object and moves it to the
    // head of the object list (`erl_db_hash.c:1426-1437`).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let object = |value| Term::tuple(vec![atom("k"), Term::Integer(value)]);
    assert_eq!(
        program
            .execute("ets_probe", "fixtable_bag_restore_behind_node", &[])
            .unwrap()
            .term,
        Term::list_from(vec![Term::list_from(vec![
            object(1),
            object(3),
            object(4),
            object(2),
        ])])
    );
    assert_eq!(
        reference("fixtable_bag_restore_behind_node"),
        b"[[{k,1},{k,3},{k,4},{k,2}]]\n"
    );
}
