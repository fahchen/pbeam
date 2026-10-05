use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

mod common;

use common::pinned_beam;

const IO_START_PROBE: &[u8] = include_bytes!("../fixtures/io_start/io_start_probe.beam");

/// Run `io_start_probe:Function()` on the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/io_start",
            "-eval",
            &format!("io:format(\"~w~n\", [io_start_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn loads_the_pinned_io_lib_modules() {
    // `user_drv:init_shell/2` (user_drv.erl:374) calls `io_lib:bformat/2`,
    // which runs `io_lib_format`, `lists`, and `unicode`.
    // `prim_tty:init_term/1` (prim_tty.erl:284) calls `os:type/0`. The pinned
    // `io_lib` has `bs_get_integer2/7` (opcode 117), `gc_bif3/7` (opcode 152),
    // and `is_any_native_record/2` (opcode 186), which the start path does
    // not run.
    let modules = ["io_lib", "io_lib_format", "lists", "unicode", "os"].map(pinned_beam);
    let beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    BeamProgram::load(&beams).unwrap();
}

#[test]
fn takes_the_tail_and_the_length_of_lists_as_pinned_erts() {
    // `io_start_probe:lists/0` runs `bif` `erlang:tl/1` with fail label 0,
    // as `group:init/1` (group.erl:136) does, `get_tl` and `gc_bif`
    // `erlang:length/1` with fail label 0, as `proc_lib:trans_init/3`
    // (proc_lib.erl:939-944) does, and both BIFs in a guard. A term that is
    // not a list, and the tail of `[]`, raise `badarg` (`bif.c:2931-2938`,
    // `erl_bif_guard.c:210-224`). In a guard, `badarg` jumps to the fail
    // label, and the next clause runs.
    let program = BeamProgram::load(&[IO_START_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let list = |names: &[&str]| Term::List(names.iter().map(|name| atom(name)).collect());
    let expected = vec![
        list(&["b", "c"]),
        list(&[]),
        atom("badarg"),
        atom("badarg"),
        Term::Integer(0),
        Term::Integer(3),
        atom("badarg"),
        atom("badarg"),
        list(&["y", "z"]),
        atom("two"),
        atom("other"),
        atom("other"),
        atom("long"),
        atom("other"),
    ];
    assert_eq!(
        program.execute("io_start_probe", "lists", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("lists"),
        b"[[b,c],[],badarg,badarg,0,3,badarg,badarg,[y,z],two,other,other,long,other]\n"
    );
}

#[test]
fn negates_booleans_as_pinned_erts() {
    // `io_start_probe:nots/0` runs `bif` `erlang:not/1` with fail label 0, as
    // `group:init/1` (group.erl:149, 153) does, and through the `apply`
    // instruction. A term that is not a boolean raises `badarg`
    // (`erl_bif_op.c:84-91`; the JIT `bif_not`, `jit/arm/ops.tab:779-780`,
    // `instr_guard_bifs.cpp:1172-1203`). The pinned erlc compiles the guard
    // `(not X) =:= Y` to `bif` `erlang:not/1` with a fail label: `badarg`
    // jumps to the fail label, and the next clause runs.
    let program = BeamProgram::load(&[IO_START_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let expected = [
        "false", "true", "badarg", "badarg", "false", "badarg", "same", "other", "other",
    ]
    .map(atom);
    assert_eq!(
        program.execute("io_start_probe", "nots", &[]).unwrap(),
        Term::List(expected.to_vec())
    );
    assert_eq!(
        reference("nots"),
        b"[false,true,badarg,badarg,false,badarg,same,other,other]\n"
    );
}

#[test]
fn reads_and_updates_maps_as_pinned_erts() {
    // `io_start_probe:maps/0` runs `bif` `erlang:map_get/2` with fail label
    // 0, as `prim_tty:init/1` (prim_tty.erl:220) and `user_drv:init/1`
    // (user_drv.erl:202) do, and `put_map_exact` with fail label 0, as
    // `io_lib_format:count_small/2` (io_lib_format.erl:325) does, and both in
    // a guard. A missing key raises `{badkey, Key}`, and a term that is not a
    // map raises `{badmap, Map}` (`erl_map.c:250-269`, `beam_common.c:2203-2345`).
    // With two missing keys, `put_map_exact` reports `value` (atom index 745)
    // before `abs` (762), the flatmap key order. In a guard, the exception
    // jumps to the fail label, and the next clause runs.
    let program = BeamProgram::load(&[IO_START_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let tagged = |tag: &str, name: &str| Term::Tuple(vec![atom(tag), atom(name)]);
    let expected = vec![
        atom("cooked"),
        tagged("badkey", "ofd"),
        tagged("badmap", "foo"),
        atom("cooked"),
        atom("other"),
        atom("other"),
        Term::Map(
            Map::from_pairs(vec![
                (atom("input"), atom("cooked")),
                (atom("output"), atom("cooked")),
            ])
            .unwrap(),
        ),
        tagged("badkey", "output"),
        tagged("badkey", "value"),
        atom("updated"),
        atom("other"),
        atom("other"),
    ];
    assert_eq!(
        program.execute("io_start_probe", "maps", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("maps"),
        b"[cooked,{badkey,ofd},{badmap,foo},cooked,other,other,\
          #{input => cooked,output => cooked},{badkey,output},{badkey,value},\
          updated,other,other]\n"
    );
}

#[test]
fn reports_the_first_missing_map_key_as_pinned_erts() {
    // `io_start_probe:map_keys/0` runs `put_map_exact` with keys of each
    // type. The ERTS loader does not sort one pair, and it sorts more pairs
    // with `erts_cmp_flatmap_keys` (`beam_transform_helpers.c:54-115`,
    // `erl_utils.h:266-271`): atoms by atom index, each other two keys in
    // the term order. Thus `{value}` comes after `{abs}`, a tuple before a
    // map, and a map before `[]`. A tuple key is a missing key, and in a
    // guard it jumps to the fail label.
    let program = BeamProgram::load(&[IO_START_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let badkey = |key: Term| Term::Tuple(vec![atom("badkey"), key]);
    let tuple = |name: &str| Term::Tuple(vec![atom(name)]);
    let expected = vec![
        badkey(tuple("x")),
        badkey(tuple("x")),
        badkey(atom("b")),
        badkey(tuple("x")),
        badkey(Term::Integer(1)),
        badkey(tuple("x")),
        badkey(tuple("x")),
        badkey(tuple("abs")),
        badkey(Term::Tuple(vec![])),
        atom("other"),
        atom("updated"),
    ];
    assert_eq!(
        program.execute("io_start_probe", "map_keys", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("map_keys"),
        b"[{badkey,{x}},{badkey,{x}},{badkey,b},{badkey,{x}},{badkey,1},{badkey,{x}},\
          {badkey,{x}},{badkey,{abs}},{badkey,{}},other,updated]\n"
    );
}

#[test]
fn merges_maps_as_pinned_erts() {
    // `io_start_probe:merge/0` runs `maps:merge/2`, as `prim_tty:options/1`
    // (prim_tty.erl:329) does: the values of the second map win
    // (`erl_map.c:1257-1284`). A merge of a hashmap and a flatmap gives a
    // hashmap, which `Map` keeps in the ERTS layout. A first argument that is not a map raises
    // `{badmap, First}`, else a second argument that is not a map raises
    // `{badmap, Second}`.
    let program = BeamProgram::load(&[IO_START_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let map = |pairs: Vec<(Term, Term)>| Term::Map(Map::from_pairs(pairs).unwrap());
    let options = map(vec![
        (atom("input"), atom("cooked")),
        (atom("output"), atom("raw")),
    ]);
    let big = (1..=33)
        .map(|key| (Term::Integer(key), Term::Integer(key)))
        .chain([
            (Term::Integer(1), atom("one")),
            (Term::Integer(40), atom("forty")),
        ])
        .collect();
    let expected = vec![
        map(vec![
            (atom("input"), atom("cooked")),
            (atom("output"), atom("raw")),
            (atom("ofd"), atom("stdout")),
        ]),
        options.clone(),
        options,
        map(big),
        Term::Tuple(vec![atom("badmap"), atom("foo")]),
        Term::Tuple(vec![atom("badmap"), atom("bar")]),
        Term::Tuple(vec![atom("badmap"), atom("foo")]),
    ];
    assert_eq!(
        program.execute("io_start_probe", "merge", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("merge"),
        b"[#{input => cooked,output => raw,ofd => stdout},\
          #{input => cooked,output => raw},#{input => cooked,output => raw},\
          #{18 => 18,4 => 4,12 => 12,19 => 19,29 => 29,13 => 13,2 => 2,7 => 7,31 => 31,\
          8 => 8,10 => 10,23 => 23,9 => 9,15 => 15,32 => 32,1 => one,25 => 25,28 => 28,\
          20 => 20,6 => 6,11 => 11,17 => 17,24 => 24,40 => forty,14 => 14,33 => 33,3 => 3,\
          16 => 16,30 => 30,21 => 21,5 => 5,27 => 27,26 => 26,22 => 22},\
          {badmap,foo},{badmap,bar},{badmap,foo}]\n"
    );
}

#[test]
fn converts_atoms_and_lists_as_pinned_erts() {
    // `io_start_probe:atoms/0` runs `erlang:atom_to_list/1` and
    // `erlang:list_to_atom/1`, as `prim_tty:set_name/2`
    // (prim_tty.erl:569-571) does (`bif.c:3186-3236`). A term that is not an
    // atom, a term that is not a list, and an element that is not a Unicode
    // scalar value raise `badarg`. `list_to_atom/1` puts a new atom into the
    // atom table of the run, so the test compares it by its code points.
    let program = BeamProgram::load(&[IO_START_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let codes = |text: &str| {
        Term::List(
            text.chars()
                .map(|character| Term::Integer(i64::from(u32::from(character))))
                .collect(),
        )
    };
    let expected = vec![
        codes("user_drv"),
        codes(""),
        codes("héllo"),
        atom("badarg"),
        atom("true"),
        atom(""),
        codes("hé\u{400}\u{1F600}"),
        atom("badarg"),
        atom("badarg"),
        atom("badarg"),
        atom("badarg"),
        atom("badarg"),
        atom("badarg"),
    ];
    assert_eq!(
        program.execute("io_start_probe", "atoms", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("atoms"),
        b"[[117,115,101,114,95,100,114,118],[],[104,233,108,108,111],badarg,true,'',\
          [104,233,1024,128512],badarg,badarg,badarg,badarg,badarg,badarg]\n"
    );
}

#[test]
fn limits_atom_names_to_255_characters_as_pinned_erts() {
    // `io_start_probe:atom_limit/0`: `erlang:list_to_atom/1` makes an atom of
    // 255 characters. It counts each element before it checks it
    // (`utils.c:2862-2930`, `MAX_ATOM_CHARACTERS` in `atom.h`), so a 256th
    // element raises `system_limit`, also when it is not a character, and a
    // first element that is not a character raises `badarg`.
    let program = BeamProgram::load(&[IO_START_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let expected = vec![
        Term::Integer(255),
        atom("system_limit"),
        atom("system_limit"),
        atom("badarg"),
    ];
    assert_eq!(
        program
            .execute("io_start_probe", "atom_limit", &[])
            .unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("atom_limit"),
        b"[255,system_limit,system_limit,badarg]\n"
    );
}

#[test]
fn returns_the_host_os_type_as_pinned_erts() {
    // `io_start_probe:system/0` runs `os:type/0` (os.erl:383-384) from the
    // pinned `os.beam`, as `prim_tty:init_term/1` (prim_tty.erl:284) does,
    // and `erlang:system_info(os_type)` directly. ERTS returns `{unix,
    // Flavor}` (`erl_bif_info.c:3026-3027`), where `Flavor` is the lowercase
    // `uname` system name (`sys/unix/sys.c:835-865`). The test takes the
    // flavor from the PBeam result, so it holds on each Unix host, and the
    // ERTS reference checks it.
    let os = pinned_beam("os");
    let program = BeamProgram::load(&[IO_START_PROBE, &os]).unwrap();
    let result = program.execute("io_start_probe", "system", &[]).unwrap();
    let Term::List(results) = &result else {
        panic!("io_start_probe:system/0 must return a list");
    };
    let [os_type, system_info] = results.as_slice() else {
        panic!("io_start_probe:system/0 must return two terms");
    };
    assert_eq!(os_type, system_info);
    let Term::Tuple(elements) = os_type else {
        panic!("os:type/0 must return a tuple");
    };
    let [Term::Atom(family), Term::Atom(flavor)] = elements.as_slice() else {
        panic!("os:type/0 must return a tuple of two atoms");
    };
    assert_eq!(family.name(), "unix");
    let flavor = flavor.name();
    assert_eq!(
        reference("system"),
        format!("[{{unix,{flavor}}},{{unix,{flavor}}}]\n").into_bytes()
    );
}

#[test]
fn rejects_other_system_info_items() {
    // `io_start_probe:system_other/0` runs `erlang:system_info(otp_release)`.
    // ERTS returns the release as a string; PBeam supports only `os_type`
    // and returns a runtime error.
    let program = BeamProgram::load(&[IO_START_PROBE]).unwrap();
    let error = program
        .execute("io_start_probe", "system_other", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unsupported erlang:system_info/1 argument"),
        "{error}"
    );
    assert!(error.contains("otp_release"), "{error}");
    assert_eq!(reference("system_other"), b"[50,57]\n");
}

#[test]
fn formats_the_empty_noshell_slogan_as_pinned_erts() {
    // `user_drv:init_shell/2` (user_drv.erl:374) runs
    // `io_lib:bformat("~ts", [""])` with the empty slogan of `-noshell`
    // (user_drv.erl:271-273). It runs `io_lib_format`, `lists`, and
    // `unicode`, and `put_map_exact` in `io_lib_format:count_small/2`
    // (io_lib_format.erl:325).
    let modules = ["io_lib", "io_lib_format", "lists", "unicode"].map(pinned_beam);
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(IO_START_PROBE);
    let program = BeamProgram::load(&beams).unwrap();
    assert_eq!(
        program.execute("io_start_probe", "format", &[]).unwrap(),
        Term::Binary(Vec::new())
    );
    assert_eq!(reference("format"), b"<<>>\n");
}
