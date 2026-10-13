use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const ETS_PROBE: &[u8] = include_bytes!("../../fixtures/ets/ets_probe.beam");

/// Run `ets_probe:Function()` under the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    let output = Command::new("erl")
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
fn rejects_tables_that_pbeam_cannot_model() {
    // ERTS 29.1.1 makes each of these tables: the types bag, bag,
    // duplicate_bag, and ordered_set, and two set tables whose heir is a
    // live process other than the caller. PBeam supports only set tables
    // without such a heir. ERTS
    // aborts the emulator for an insert with the key position 0 (status
    // 134, `Invalid tag in make_internal_hash`) and crashes for a found key
    // and an index that becomes 0 (status 139). This is the "ETS tables"
    // row in `docs/erts-differences.md`.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    for (case, detail) in [
        ("bag", "new/2 table type bag"),
        ("set_after_bag", "new/2 table type bag"),
        ("duplicate_bag", "new/2 table type duplicate_bag"),
        ("ordered_set", "new/2 table type ordered_set"),
        ("heir", "new/2 heir ProcessId("),
        ("heir_data", "new/2 heir ProcessId("),
        ("keypos", "insert/2 in a table with key position 0"),
        ("index", "lookup_element/4 index 4294967296 with position 0"),
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
}
