//! Runs `apply`, local and external funs and `is_function` on PBeam and compares them with the
//! pinned ERTS.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const FUN_PROBE: &[u8] = include_bytes!("../../fixtures/funs/fun_probe.beam");
const TAIL_CATCH: &[u8] = include_bytes!("../../fixtures/funs/tail_catch.beam");
const FUN_INDEX: &[u8] = include_bytes!("../../fixtures/funs/fun_index.beam");

/// Run `fun_probe:Function()` on the pinned ERTS and return its `~w` output.
fn reference(function: &str) -> Vec<u8> {
    reference_of("fun_probe", function)
}

/// Run `Module:Function()` on the pinned ERTS and return its `~w` output.
fn reference_of(module: &str, function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/funs",
            "-eval",
            &format!("io:format(\"~w~n\", [{module}:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
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
        program.execute("fun_probe", "mfas", &[]).unwrap().term,
        Term::list_from(expected)
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
    let pair = |first: &str, second| Term::tuple(vec![atom(first), second]);
    let expected = vec![
        pair("tag", atom("one")),
        atom("const"),
        pair("tag", atom("tail")),
        atom("same"),
        atom("different"),
        atom("same"),
        atom("true"),
        atom("false"),
        Term::tuple(vec![
            atom("badarity"),
            atom("same"),
            Term::list_from(vec![Term::Integer(1), Term::Integer(2)]),
        ]),
        pair("badfun", atom("notfun")),
        pair("badfun", pair("fun_probe", atom("id"))),
    ];
    assert_eq!(
        program.execute("fun_probe", "locals", &[]).unwrap().term,
        Term::list_from(expected)
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
    let mut expected = vec![Term::list_from(vec![atom("a"), atom("b")])];
    expected.extend(
        [
            "one", "any", "none", "true", "false", "true", "false", "false", "true", "false",
            "false", "badarg", "badarg",
        ]
        .map(atom),
    );
    assert_eq!(
        program.execute("fun_probe", "tests", &[]).unwrap().term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("tests"),
        b"[[a,b],one,any,none,true,false,true,false,false,true,false,false,badarg,badarg]\n"
    );
}

#[test]
fn applies_funs_as_pinned_erts() {
    // `fun_probe:applies/0` calls `erlang:apply/2` as `call_ext`,
    // `call_ext_only`, and `call_ext_last`, and as an apply of
    // `erlang:apply/3` with two elements. The JIT `i_apply_fun` raises
    // `badarg` for an argument term that is not a list before it tests the
    // fun, and `system_limit` for a list of 1023 elements
    // (`jit/arm/instr_fun.cpp:253-328`). A list of 1022 elements reaches the
    // fun and raises `badarity`. The pinned `erlang.beam` is loaded, so a
    // call that enters its `apply/2` body, which calls itself, never returns.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, FUN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |first: &str, second: &str| Term::tuple(vec![atom(first), atom(second)]);
    let badarity = |arguments| Term::tuple(vec![atom("badarity"), atom("same"), arguments]);
    let expected = vec![
        pair("a", "x"),
        pair("a", "u"),
        pair("a", "t"),
        pair("a", "r"),
        atom("badarg"),
        pair("badfun", "notfun"),
        atom("badarg"),
        badarity(Term::list_from(vec![Term::Integer(1), Term::Integer(2)])),
        badarity(atom("same")),
        atom("system_limit"),
    ];
    assert_eq!(
        program.execute("fun_probe", "applies", &[]).unwrap().term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("applies"),
        b"[{a,x},{a,u},{a,t},{a,r},badarg,{badfun,notfun},badarg,\
          {badarity,same,[1,2]},{badarity,same,same},system_limit]\n"
    );
}

#[test]
fn spawns_a_fun_at_erlang_apply_2_as_pinned_erts() {
    // `erlang:spawn/1` tests the fun with `is_function` and starts the new
    // process at `erlang:apply(F, [])` (`erlang.erl:10601-10605`), as
    // `standard_error:start/0` does. The process runs the body of
    // `erlang:apply/2` in the pinned `erlang.beam`, which is
    // `call_ext_only erlang:apply/2` (`erlang.erl:10557-10558`). A term that
    // is not a fun raises `badarg` through `badarg_with_info/1`.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, FUN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("fun_probe", "spawned", &[]).unwrap().term,
        Term::list_from(vec![atom("spawned"), atom("badarg")])
    );
    assert_eq!(reference("spawned"), b"[spawned,badarg]\n");
}

#[test]
fn makes_and_calls_external_funs_as_pinned_erts() {
    // `fun_probe:externals/0` makes external funs with `erlang:make_fun/3`,
    // as `gen_server:init_it/6` does (`gen_server.erl:2234, 2355-2364`), and
    // calls them with `call_fun` and `call_fun2`, as the `try_handle_*`
    // functions of `gen_server` do. A call runs the function of the fun, so
    // `fun erlang:self/0` runs the BIF. Two external funs are equal when
    // their modules, functions, and arities are equal, and an external fun
    // is greater than a local fun (`utils.c:1270-1273`,
    // `utils.c:2189-2207`). A module or function that is not an atom, and an
    // arity that is not an integer in `0..=255`, raise `badarg`
    // (`bif.c:4467-4485`). The literal `fun fun_probe:id/1` loads from
    // `EXPORT_EXT` in the literal table (`external.c:5006-5041`), as
    // `fun edlin_expand:expand/2` in `group:init/3` does, and is equal to the
    // fun of `erlang:make_fun/3`.
    let program = BeamProgram::load(&[FUN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let mut expected: Vec<Term> = [
        "v",
        "w",
        "same",
        "same",
        "different",
        "true",
        "false",
        "true",
        "one",
    ]
    .map(atom)
    .into();
    expected.push(Term::tuple(vec![
        atom("badarity"),
        atom("same"),
        Term::list_from(vec![Term::Integer(1), Term::Integer(2)]),
    ]));
    expected.extend(std::iter::repeat_n(atom("badarg"), 5));
    expected.extend(["same", "lit", "same"].map(atom));
    assert_eq!(
        program.execute("fun_probe", "externals", &[]).unwrap().term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("externals"),
        b"[v,w,same,same,different,true,false,true,one,{badarity,same,[1,2]},\
          badarg,badarg,badarg,badarg,badarg,same,lit,same]\n"
    );
}

#[test]
fn releases_the_frame_before_a_tail_apply_as_pinned_erts() {
    // `tail_catch:run/0` calls each inner function from a `try` in
    // `outer/2`. Each inner function makes a tail call while its own `try`
    // tag is live, which only hand-made BEAM code can do
    // (`fixtures/funs/tail_catch.asm`). ERTS releases the frame before
    // `i_apply_fun_last`, `i_apply_last`, and `apply_last`
    // (`jit/arm/ops.tab:649-657`, `jit/arm/instr_call.cpp:307-316`), so the
    // `badfun` and `badarg` of the apply reach the `try` of `outer/2`. A BIF
    // tail call is `call_light_bif`, `deallocate`, and `return`
    // (`jit/arm/ops.tab:711-712`), so the `badarg` of `erlang:whereis/1`
    // reaches the `try` of the inner function.
    let program = BeamProgram::load(&[TAIL_CATCH]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |first: &str, second| Term::tuple(vec![atom(first), second]);
    let expected = vec![
        pair("outer", pair("badfun", atom("notfun"))),
        pair("outer", atom("badarg")),
        pair("outer", atom("badarg")),
        pair("inner", atom("badarg")),
    ];
    assert_eq!(
        program.execute("tail_catch", "run", &[]).unwrap().term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference_of("tail_catch", "run"),
        b"[{outer,{badfun,notfun}},{outer,badarg},{outer,badarg},{inner,badarg}]\n"
    );
}

#[test]
fn gives_fun_information_as_pinned_erts() {
    // `fun_probe:infos/0` asks `erlang:fun_info/2` for the items that
    // `Elixir.BadArityError` and `Elixir.Agent.Server` read. A local fun
    // gives `local`, its module and function, its arity, its free variables
    // in `FunT` order, `1` for `refc`, the `Index` field of its `FunT` entry
    // for `new_index` (`erl_bif_info.c:3954-3957`, `jit/asm_load.c:1218-1223`),
    // and the old unique value of the entry for `uniq`
    // (`erl_bif_info.c:3968-3971`). An external fun gives `external`, its
    // module and function, its arity, `[]` for `env`, and `undefined` for
    // `refc`, `index`, `new_index`, `uniq`, `new_uniq`, and `pid`
    // (`erl_bif_info.c:3943-3969`, `erl_bif_info.c:3985-3990`). A term that
    // is not a fun, an item that the BIF does not know, and `native` and
    // `native_address` raise `badarg` (`erl_bif_info.c:3924-3926`,
    // `erl_bif_info.c:4003-4004`).
    let program = BeamProgram::load(&[FUN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let pair = |first: &str, second: Term| Term::tuple(vec![atom(first), second]);
    let mut expected = vec![
        pair("type", atom("local")),
        pair("module", atom("fun_probe")),
        pair("name", atom("-infos/0-fun-0-")),
        pair("arity", Term::Integer(1)),
        pair(
            "env",
            Term::list_from(vec![
                Term::Integer(7),
                Term::tuple(vec![atom("a"), atom("b")]),
            ]),
        ),
        pair("refc", Term::Integer(1)),
        pair("uniq", Term::Integer(45_737_141)),
        pair("new_index", Term::Integer(6)),
        pair("type", atom("local")),
        pair("module", atom("fun_probe")),
        pair("name", atom("-infos/0-fun-1-")),
        pair("arity", Term::Integer(0)),
        pair("env", Term::nil()),
        pair("refc", Term::Integer(1)),
        pair("uniq", Term::Integer(45_737_141)),
        pair("new_index", Term::Integer(7)),
        pair("type", atom("external")),
        pair("module", atom("lists")),
        pair("name", atom("reverse")),
        pair("arity", Term::Integer(1)),
        pair("env", Term::nil()),
        pair("refc", atom("undefined")),
        pair("index", atom("undefined")),
        pair("new_index", atom("undefined")),
        pair("uniq", atom("undefined")),
        pair("new_uniq", atom("undefined")),
        pair("pid", atom("undefined")),
    ];
    expected.extend(["badarg", "badarg", "badarg", "badarg"].map(atom));
    assert_eq!(
        program.execute("fun_probe", "infos", &[]).unwrap().term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("infos"),
        b"[{type,local},{module,fun_probe},{name,'-infos/0-fun-0-'},\
          {arity,1},{env,[7,{a,b}]},{refc,1},{uniq,45737141},{new_index,6},\
          {type,local},{module,fun_probe},{name,'-infos/0-fun-1-'},\
          {arity,0},{env,[]},{refc,1},{uniq,45737141},{new_index,7},\
          {type,external},{module,lists},{name,reverse},\
          {arity,1},{env,[]},{refc,undefined},{index,undefined},\
          {new_index,undefined},{uniq,undefined},{new_uniq,undefined},\
          {pid,undefined},badarg,badarg,badarg,badarg]\n"
    );
}

#[test]
fn gives_the_fun_entry_items_of_a_local_fun_as_pinned_erts() {
    // `fun_info_2` gives the old index of the fun entry for `index`, the
    // module MD5 for `new_uniq`, and `erts_init_process_id`, `<0.0.0>`, for
    // `pid` (`erl_bif_info.c:3943-3967`). The loader keeps the MD5 that
    // `beamfile_read` computes (`beam_file.c:1751-1830`).
    let program = BeamProgram::load(&[FUN_PROBE]).unwrap();
    let mut texts = Vec::new();
    for item in ["index", "new_uniq", "pid"] {
        let item = Term::Atom(program.atom(item).unwrap());
        let result = program
            .execute(
                "fun_probe",
                "fun_info_unsupported",
                &[program.host_term(item)],
            )
            .unwrap();
        let value = result.term.tuple_elements().unwrap()[1].clone();
        texts.push(match value {
            Term::Integer(index) => format!("{{index,{index}}}"),
            Term::Pid(_) => {
                assert_eq!(format!("{value:?}"), "Pid(ProcessId(0))");
                "{pid,<0.0.0>}".to_owned()
            }
            binary => {
                let bytes = binary.binary_bytes().unwrap();
                let bytes = bytes.iter().map(u8::to_string).collect::<Vec<_>>();
                format!("{{new_uniq,<<{}>>}}", bytes.join(","))
            }
        });
    }
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/funs",
            "-eval",
            "io:format(\"~w~n\", [[fun_probe:fun_info_unsupported(I) || I <- [index, new_uniq, pid]]]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(
        String::from_utf8_lossy(&reference.stdout),
        format!("[{}]\n", texts.join(","))
    );
}

#[test]
fn prints_funs_as_pinned_erts() {
    // `fun_probe:texts/0` calls `erlang:fun_to_list/1` and `io_lib:format/2`
    // with `~p` and `~w`. A local fun prints `#Fun<Module.OldIndex.OldUniq>`
    // (`erl_printf_term.c:678-690`), and an external fun prints `fun M:F/A`
    // with quoted atoms, also for a module that is not loaded
    // (`erl_printf_term.c:691-723`). A term that is not a fun raises
    // `error:badarg` (`bif.c:4492-4494`).
    let modules: Vec<Vec<u8>> = [
        "erlang",
        "erl_scan",
        "epp",
        "lists",
        "maps",
        "ets",
        "io_lib",
        "io_lib_format",
        "io_lib_pretty",
        "string",
    ]
    .iter()
    .map(|module| pinned_beam(module))
    .collect();
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(FUN_PROBE);
    let program = BeamProgram::load(&beams).unwrap();
    let local = "#Fun<fun_probe.10.45737141>";
    let both = format!("{local} {local}");
    let texts = [
        local,
        "#Fun<fun_probe.11.45737141>",
        "fun lists:reverse/1",
        "fun 'A b':'c d'/0",
        "fun no_such_module:f/3",
        &both,
        "fun lists:reverse/1 fun lists:reverse/1",
        "fun 'A b':'c d'/0 fun 'A b':'c d'/0",
    ];
    let mut expected: Vec<Term> = texts
        .iter()
        .map(|text| Term::list_from(text.bytes().map(|byte| Term::Integer(byte.into()))))
        .collect();
    expected.push(Term::Atom(program.atom("badarg").unwrap()));
    assert_eq!(
        program.execute("fun_probe", "texts", &[]).unwrap().term,
        Term::list_from(expected)
    );
    let codes: Vec<String> = texts
        .iter()
        .map(|text| {
            let bytes: Vec<String> = text.bytes().map(|byte| byte.to_string()).collect();
            format!("[{}]", bytes.join(","))
        })
        .collect();
    assert_eq!(
        String::from_utf8(reference("texts")).unwrap(),
        format!("[{},badarg]\n", codes.join(","))
    );
}

#[test]
fn prints_the_fun_entry_position_as_pinned_erts() {
    // Each `Index` field of the `FunT` chunk of `fun_index` is its position
    // plus 100 (`fixtures/funs/fun_index.src`). ERTS prints the position,
    // which the loader keeps as `ErlFunEntry.old_index`
    // (`jit/asm_load.c:1218-1223`, `erl_printf_term.c:685-686`).
    let program = BeamProgram::load(&[FUN_INDEX]).unwrap();
    let texts = ["#Fun<fun_index.0.50994165>", "#Fun<fun_index.1.50994165>"];
    assert_eq!(
        program.execute("fun_index", "texts", &[]).unwrap().term,
        Term::list_from(
            texts
                .iter()
                .map(|text| Term::list_from(text.bytes().map(|byte| Term::Integer(byte.into()))))
        )
    );
    let codes: Vec<String> = texts
        .iter()
        .map(|text| {
            let bytes: Vec<String> = text.bytes().map(|byte| byte.to_string()).collect();
            format!("[{}]", bytes.join(","))
        })
        .collect();
    assert_eq!(
        String::from_utf8(reference_of("fun_index", "texts")).unwrap(),
        format!("[{}]\n", codes.join(","))
    );
}

#[test]
fn gives_the_mfa_of_funs_as_pinned_erts() {
    // `fun_info_mfa_1` gives `{Module, Name, Arity}` of a local fun from
    // the function of its fun entry and of an external fun from its export
    // entry, also of a module that is not loaded, and raises `badarg` for a
    // term that is not a fun (`erl_bif_info.c:4011-4047`). The arity of a
    // local fun leaves out its free variables.
    let program = BeamProgram::load(&[FUN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let mfa = |module: &str, name: &str, arity: i64| {
        Term::tuple(vec![atom(module), atom(name), Term::Integer(arity)])
    };
    assert_eq!(
        program.execute("fun_probe", "info_mfas", &[]).unwrap().term,
        Term::list_from(vec![
            mfa("fun_probe", "-info_mfas/0-fun-0-", 2),
            mfa("fun_probe", "-info_mfas/0-fun-1-", 0),
            mfa("lists", "reverse", 1),
            mfa("no_such_module", "f", 3),
            atom("badarg"),
        ])
    );
    assert_eq!(
        reference("info_mfas"),
        b"[{fun_probe,'-info_mfas/0-fun-0-',2},{fun_probe,'-info_mfas/0-fun-1-',0},\
          {lists,reverse,1},{no_such_module,f,3},badarg]\n"
    );
}
