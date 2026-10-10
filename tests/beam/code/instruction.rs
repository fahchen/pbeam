use super::super::{read_integer, read_integer_after_first, read_operand, read_value};
use super::{
    OPCODE_BADMATCH, OPCODE_BIF0, OPCODE_BS_GET_INTEGER2, OPCODE_BS_GET_UTF16, OPCODE_BS_GET_UTF32,
    OPCODE_BS_INIT_WRITABLE, OPCODE_BS_START_MATCH4, OPCODE_FADD, OPCODE_FCONV, OPCODE_FDIV,
    OPCODE_FMOVE, OPCODE_FMUL, OPCODE_FSUB, OPCODE_GC_BIF3, OPCODE_GET_RECORD_ELEMENTS,
    OPCODE_GET_TL, OPCODE_HAS_MAP_FIELDS, OPCODE_INIT_YREGS, OPCODE_IS_ANY_NATIVE_RECORD,
    OPCODE_PUT_MAP_ASSOC, OPCODE_PUT_MAP_EXACT, OPCODE_PUT_RECORD, OPCODE_PUT_TUPLE2,
    OPCODE_RAW_RAISE, OPCODE_TEST_HEAP, OPCODE_TEST_IS_ATOM, OPCODE_TEST_IS_BINARY,
    OPCODE_TEST_IS_LIST, OPCODE_TEST_IS_MAP, OPCODE_TEST_IS_PID, OPCODE_TEST_IS_PORT,
    OPCODE_TEST_NE, OPCODE_TEST_TAGGED_TUPLE, OPCODE_TRY_CASE_END, parse_code,
    read_binary_match_commands, read_binary_segments, read_move_operand, read_operand_pairs,
    read_pair_count, read_record_updates, skip_allocation, skip_counted_operands,
    skip_integer_payload, skip_operand, skip_unsupported_instruction, test_kind,
    unsupported_opcode_operand_count,
};
use crate::{
    beam::code::{BinaryMatchCommand, BinarySegment, Instruction, Operand, TestKind},
    term::{Atom, AtomTable, Lambda, Term},
};

#[test]
fn reads_operand_pairs_and_rejects_other_lists() {
    let atoms = vec![Atom::from_static("output").unwrap()];
    // {list,[{atom,output},{x,1}]}
    let pairs = read_operand_pairs(&[0x17, 0x20, 0x12, 0x13], &mut 0, &atoms, &[]).unwrap();
    assert!(matches!(
        pairs.as_slice(),
        [(Operand::Value(Term::Atom(key)), Operand::X(1))] if key.name() == "output"
    ));
    assert!(read_operand_pairs(&[0x27, 0x20], &mut 0, &atoms, &[]).is_err());
    assert!(read_operand_pairs(&[0x17, 0x10, 0x12], &mut 0, &atoms, &[]).is_err());
}

#[test]
fn rejects_invalid_operands_and_code_headers() {
    let mut cursor = 0;
    assert!(read_integer(&[0x12], &mut cursor, 1).is_err());
    assert!(read_integer_after_first(&[], &mut 0, 0x19).is_err());
    assert!(read_integer_after_first(&[], &mut 0, 0x09).is_err());
    assert!(read_operand(&[], &mut 0, 0).is_err());
    assert!(read_value(&[0x03], &mut 0, &[]).is_err());
    assert!(read_value(&[0x12], &mut 0, &[]).is_err());
    assert!(parse_code(&[], &[], &[], &[], &[], &[]).is_err());
    assert!(parse_code(&[0; 20], &[], &[], &[], &[], &[]).is_err());
    assert!(
        parse_code(
            &[0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .is_err()
    );
}

#[test]
fn sign_extends_large_integer_operands_as_erts() {
    let integer = |bytes: &[u8]| read_value(bytes, &mut 0, &[]).unwrap();
    // erlc 29.1.1 writes {integer,-1} as [0x19, 0xFF, 0xFF].
    assert_eq!(integer(&[0x19, 0xFF, 0xFF]), Term::Integer(-1));
    // {integer,-101714049538469044}: size code 6 gives eight bytes.
    assert_eq!(
        integer(&[0xD9, 0xFE, 0x96, 0xA3, 0x9C, 0x60, 0x97, 0xD3, 0x4C]),
        Term::Integer(-101714049538469044)
    );
    // A leading zero byte keeps a large payload positive.
    assert_eq!(integer(&[0x39, 0x00, 0xFF, 0xFF]), Term::Integer(65535));
    // The short and extended forms are never negative.
    assert_eq!(integer(&[0xF1]), Term::Integer(15));
    assert_eq!(integer(&[0xE9, 0xFF]), Term::Integer(2047));
}

#[test]
fn reads_only_supported_move_literals() {
    let literals = vec![Some(Term::Binary(b"ok".to_vec())), None];
    assert!(matches!(
        &read_move_operand(&[0x47, 0x00], &mut 0, &[], &literals).unwrap(),
        Operand::Value(Term::Binary(value)) if value == b"ok"
    ));
    let mut cursor = 0;
    assert!(matches!(
        read_move_operand(&[0x57, 0x03, 0x50], &mut cursor, &[], &literals).unwrap(),
        Operand::X(0)
    ));
    assert_eq!(cursor, 3);
    assert!(matches!(
        read_move_operand(&[0x47, 0x10], &mut 0, &[], &literals).unwrap(),
        Operand::UnsupportedLiteral(1)
    ));
    assert!(matches!(
        &read_move_operand(&[0x17, 0x10], &mut 0, &[], &literals).unwrap(),
        Operand::Value(Term::List(values)) if values.is_empty()
    ));
    // erlc 29.1.1 writes {integer,18446744073709551616} as 0xF9, the
    // extra byte count 0, and nine payload bytes. It loads as an
    // operand that fails when it runs; a short payload is malformed.
    let mut cursor = 0;
    let huge = [0xF9, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(matches!(
        read_move_operand(&huge, &mut cursor, &[], &literals).unwrap(),
        Operand::UnsupportedInteger
    ));
    assert_eq!(cursor, huge.len());
    assert!(read_move_operand(&huge[..10], &mut 0, &[], &literals).is_err());
    assert!(read_move_operand(&[0x47, 0x20], &mut 0, &[], &literals).is_err());
    assert!(read_move_operand(&[0x27, 0x00], &mut 0, &[], &literals).is_err());
    assert!(read_move_operand(&[0x06], &mut 0, &[], &literals).is_err());
    assert_eq!(
        read_value(&[0x02], &mut 0, &[]).unwrap(),
        Term::List(Vec::new())
    );
}

#[test]
fn rejects_unsupported_select_val_forms() {
    let code = |instructions: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend_from_slice(instructions);
        code
    };

    assert!(parse_code(&code(&[59, 3, 21, 39]), &[], &[], &[], &[], &[]).is_err());
    assert!(parse_code(&code(&[59, 3, 21, 23, 16]), &[], &[], &[], &[], &[]).is_err());
    // A register as a selection value, with two items present.
    assert!(
        parse_code(&code(&[59, 3, 21, 23, 32, 3, 21]), &[], &[], &[], &[], &[])
            .unwrap_err()
            .to_string()
            .contains("unsupported BEAM selection value")
    );
    // A count larger than the remaining bytes fails before any allocation.
    for opcode in [59, 60] {
        assert!(
            parse_code(&code(&[opcode, 3, 21, 23, 32, 3]), &[], &[], &[], &[], &[])
                .unwrap_err()
                .to_string()
                .contains("exceeds code chunk")
        );
    }
}

#[test]
fn decodes_build_stacktrace_without_operands() {
    // `try f() catch C:R:S -> S end` compiles to `{move,{x,2},{x,0}}`
    // and `build_stacktrace`.
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([0x40, 0x23, 0x03, 0xA0, 0x03]);
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Move(Operand::X(2), Operand::X(0)),
            Instruction::BuildStacktrace,
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_raise_with_a_raw_stack_trace_and_a_reason() {
    // `raise_probe:rethrow/1` ends with `{bif,raise,{f,0},[{x,2},{x,1}],{x,0}}`,
    // which `beam_asm` writes as `raise x2 x1`. ERTS also loads other
    // sources, such as `raise y0 nil` (`emu/ops.tab:286-288`).
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([0x6C, 0x23, 0x13, 0x6C, 0x04, 0x02, 0x03]);
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Raise {
                trace: Operand::X(2),
                value: Operand::X(1),
            },
            Instruction::Raise {
                trace: Operand::Y(0),
                value: Operand::Value(Term::List(values)),
            },
            Instruction::End,
        ] if values.is_empty()
    ));
}

#[test]
fn decodes_try_and_catch_instructions_with_a_y_register() {
    let code = |instructions: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend_from_slice(instructions);
        code.push(0x03);
        code
    };
    let (instructions, _) = parse_code(
        &code(&[
            0x68, 0x04, 0x75, // try y0 f7
            0x68, 0x14, 0x0D, 0x11, // try y1 f17
            0x3E, 0x04, 0xA5, // catch y0 f10
            0x69, 0x04, // try_end y0
            0x6A, 0x04, // try_case y0
            0x3F, 0x04, // catch_end y0
        ]),
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Catch {
                register: 0,
                label: 7
            },
            Instruction::Catch {
                register: 1,
                label: 17
            },
            Instruction::Catch {
                register: 0,
                label: 10
            },
            Instruction::TryEnd { register: 0 },
            Instruction::TryCase { register: 0 },
            Instruction::CatchEnd { register: 0 },
            Instruction::End,
        ]
    ));
    // An X register cannot hold a catch tag.
    assert!(
        parse_code(&code(&[0x68, 0x03, 0x75]), &[], &[], &[], &[], &[])
            .unwrap_err()
            .to_string()
            .contains("unexpected BEAM operand tag 3")
    );
}

#[test]
fn decodes_nif_start_at_the_entry_of_the_function_of_the_func_info_before_it() {
    let code = |instructions: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 11]);
        code.push(2); // label 1 only
        code.extend([0; 3]);
        code.push(1); // one function
        code.extend_from_slice(instructions);
        code.push(0x03);
        code
    };
    let mut table = AtomTable::new();
    let atoms = ["prim_tty", "write_nif"].map(|name| table.put(name).unwrap());
    let (instructions, _) = parse_code(
        &code(&[
            0x02, 0x12, 0x22, 0x20, // func_info prim_tty write_nif 2
            0xB3, // nif_start
        ]),
        &atoms,
        &[],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::FunctionInfo { .. },
            Instruction::NifStart { name, arity: 2 },
            Instruction::End,
        ] if name == "write_nif"
    ));
    // ERTS loads `nif_start` after another instruction of the function,
    // and a NIF replaces the function from its entry. So the dispatch
    // goes before the `move`, which does not run for a bound MFA. The
    // `TestYield` of the entry comes first: a NIF call costs one reduction
    // as `i_test_yield` does (`jit/arm/instr_bif.cpp:924-952`).
    let (instructions, _) = parse_code(
        &code(&[
            0x02, 0x12, 0x22, 0x20, // func_info prim_tty write_nif 2
            0x01, 0x10, // label 1
            0x40, 0x12, 0x03, // move prim_tty x0
            0xB3, // nif_start
        ]),
        &atoms,
        &[],
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::FunctionInfo { .. },
            Instruction::Label(1),
            Instruction::TestYield,
            Instruction::NifStart { name, arity: 2 },
            Instruction::Move(..),
            Instruction::End,
        ] if name == "write_nif"
    ));
    assert_eq!(
        parse_code(&code(&[0xB3]), &atoms, &[], &[], &[], &[])
            .unwrap_err()
            .to_string(),
        "BEAM nif_start has no func_info before it"
    );
}

#[test]
fn decodes_bif0_and_bif1_into_one_bif_instruction_with_its_fail_label() {
    let code = |instructions: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend_from_slice(instructions);
        code.push(0x03);
        code
    };
    let imports = vec![
        ("erlang".to_owned(), "node".to_owned(), 1),
        ("erlang".to_owned(), "node".to_owned(), 0),
    ];
    // The bytes of `io:io_request/2` in the pinned `io.beam`, with the
    // import indexes 14 and 15 changed to 0 and 1.
    let (instructions, _) = parse_code(
        &code(&[
            0x0A, 0x05, 0x00, 0x57, 0x03, 0x40, 0x23, // bif1 f0 node/1 {tr,x0,pid} x2
            0x09, 0x10, 0x53, // bif0 node/0 x5
            0x0A, 0x35, 0x00, 0x03, 0x13, // bif1 f3 node/1 x0 x1
        ]),
        &[],
        &imports,
        &[],
        &[],
        &[],
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Bif {
                fail_label: 0,
                module: node1_module,
                name: node1,
                arguments: node1_arguments,
                destination: Operand::X(2),
            },
            Instruction::Bif {
                fail_label: 0,
                module: node0_module,
                name: node0,
                arguments: node0_arguments,
                destination: Operand::X(5),
            },
            Instruction::Bif {
                fail_label: 3,
                module: guard_module,
                name: guard,
                arguments: guard_arguments,
                destination: Operand::X(1),
            },
            Instruction::End,
        ] if node1_module == "erlang"
            && node1 == "node"
            && matches!(node1_arguments.as_slice(), [Operand::X(0)])
            && node0_module == "erlang"
            && node0 == "node"
            && node0_arguments.is_empty()
            && guard_module == "erlang"
            && guard == "node"
            && matches!(guard_arguments.as_slice(), [Operand::X(0)])
    ));
    for bif in [&[0x09, 0x20, 0x03][..], &[0x0A, 0x05, 0x20, 0x03, 0x03]] {
        assert!(
            parse_code(&code(bif), &[], &imports, &[], &[], &[])
                .unwrap_err()
                .to_string()
                .contains("BEAM BIF import index 2 out of range")
        );
    }
}

#[test]
fn decodes_bif2_into_a_bif_instruction_with_its_fail_label() {
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    // Bytes of the pinned OTP 29.1.1 `gen_statem.beam` and `gen.beam`,
    // with the import indexes 33 and 8 changed to 0 and 1.
    code.extend([
        // gen_statem STACKTRACE(): {bif,element,{f,0},[{integer,2},{x,0}],{x,0}}
        0x0B, 0x05, 0x00, 0x21, 0x03, 0x03,
        // gen:do_call: {bif,element,{f,46},[{integer,2},{x,0}],{x,4}}
        0x0B, 0x0D, 0x2E, 0x10, 0x21, 0x03, 0x43, 0x03,
    ]);
    let imports = vec![
        ("erlang".to_owned(), "element".to_owned(), 2),
        ("erlang".to_owned(), "element".to_owned(), 2),
    ];
    let (instructions, _) = parse_code(&code, &[], &imports, &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Bif {
                fail_label: 0,
                module,
                name,
                arguments,
                destination: Operand::X(0),
            },
            Instruction::Bif {
                fail_label: 46,
                module: call_module,
                name: call_name,
                arguments: call_arguments,
                destination: Operand::X(4),
            },
            Instruction::End,
        ] if module == "erlang"
            && name == "element"
            && matches!(
                arguments.as_slice(),
                [Operand::Value(Term::Integer(2)), Operand::X(0)]
            )
            && call_module == "erlang"
            && call_name == "element"
            && matches!(
                call_arguments.as_slice(),
                [Operand::Value(Term::Integer(2)), Operand::X(0)]
            )
    ));
}

#[test]
fn decodes_is_ne_exact_with_its_fail_label() {
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    // Bytes of `user_drv:server/3` in the pinned `user_drv.beam`:
    // {test,is_ne_exact,{f,157},[{x,0},{x,3}]}
    code.extend([0x2C, 0x0D, 0x9D, 0x03, 0x33, 0x03]);
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::TestExactNotEqual {
                fail_label: 157,
                left: Operand::X(0),
                right: Operand::X(3),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_is_ge_with_a_typed_register() {
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    // Bytes of `group_probe:kind/1` in `fixtures/group/group_probe.beam`:
    // {test,is_ge,{f,22},[{tr,{x,1},{t_integer,{0,16777215}}},{integer,2}]}
    code.extend([0x28, 0x0D, 0x16, 0x57, 0x13, 0x20, 0x21, 0x03]);
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::TestGreaterEqual {
                fail_label: 22,
                left: Operand::X(1),
                right: Operand::Value(Term::Integer(2)),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_the_is_lt_is_integer_and_is_boolean_tests_of_the_compare_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/compare/compare_probe.beam` has these bytes.
    let next = [0x27, 0x75, 0x01, 0x57, 0x13, 0x10]; // next/1: is_lt f7 0 {tr,x1,1}
    let width = [0x27, 0xF5, 0x03, 0x01]; // width/1: is_lt f15 x0 0
    let is_integer = [0x2D, 0xB5, 0x03]; // kind/1: is_integer f11 x0
    let is_boolean = [0x72, 0xC5, 0x03]; // kind/1: is_boolean f12 x0
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&next[..], &width, &is_integer, &is_boolean, &[0x03]].concat());
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::TestLess {
                fail_label: 7,
                left: Operand::Value(Term::Integer(0)),
                right: Operand::X(1),
            },
            Instruction::TestLess {
                fail_label: 15,
                left: Operand::X(0),
                right: Operand::Value(Term::Integer(0)),
            },
            Instruction::Test {
                kind: TestKind::Integer,
                fail_label: 11,
                value: Operand::X(0),
            },
            Instruction::Test {
                kind: TestKind::Boolean,
                fail_label: 12,
                value: Operand::X(0),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_the_is_eq_test_of_the_compare_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/compare/compare_probe.beam` has these bytes.
    let is_eq = [0x29, 0x0D, 0x19, 0x03, 0x13]; // eq/2: is_eq f25 x0 x1
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&is_eq[..], &[0x03]].concat());
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::TestEqual {
                fail_label: 25,
                left: Operand::X(0),
                right: Operand::X(1),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_the_bs_start_match3_test_of_the_binary_match_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/binary_match/binary_match_probe.beam` has these bytes.
    // kind/1: bs_start_match3 f5 x0 1 x0
    let bs_start_match3 = [0xA6, 0x55, 0x03, 0x10, 0x03];
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&bs_start_match3[..], &[0x03]].concat());
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::BinaryMatchStart {
                fail_label: 5,
                source: Operand::X(0),
                destination: Operand::X(0),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_the_binary_match_instructions_of_is_usascii() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/binary_match/binary_match_probe.beam` has these bytes.
    // is_usascii/1 has the instructions of prim_tty:is_usascii/1.
    let parts: [&[u8]; 6] = [
        // bs_start_match3 f17 x0 1 x0
        &[0xA6, 0x0D, 0x11, 0x03, 0x10, 0x03],
        // bs_get_position x0 x1 1
        &[0xA7, 0x03, 0x13, 0x10],
        // bs_get_utf8 f16 {tr,x0,1} 2 0 x2
        &[0x8A, 0x0D, 0x10, 0x57, 0x03, 0x10, 0x20, 0x00, 0x23],
        // bs_match f16 x0 {list,[ensure_at_least,0,8]}
        &[0xB6, 0x0D, 0x10, 0x03, 0x17, 0x30, 0x82, 0x00, 0x80],
        // bs_set_position x0 x1
        &[0xA8, 0x03, 0x13],
        // bs_match f17 x0 {list,[ensure_exactly,0]}
        &[0xB6, 0x0D, 0x11, 0x03, 0x17, 0x20, 0xF2, 0x00],
    ];
    // Atom 8 is `ensure_at_least` and atom 15 is `ensure_exactly`.
    let mut table = AtomTable::new();
    let atoms: Vec<Atom> = (1..=15)
        .map(|index| match index {
            8 => table.put("ensure_at_least").unwrap(),
            15 => table.put("ensure_exactly").unwrap(),
            _ => table.put(&format!("atom{index}")).unwrap(),
        })
        .collect();
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend(parts.concat());
    code.push(0x03);
    let (instructions, _) = parse_code(&code, &atoms, &[], &[], &[], &[]).unwrap();
    assert!(
        matches!(
            instructions.as_slice(),
            [
                Instruction::BinaryMatchStart {
                    fail_label: 17,
                    source: Operand::X(0),
                    destination: Operand::X(0),
                },
                Instruction::BinaryGetPosition {
                    context: Operand::X(0),
                    destination: Operand::X(1),
                },
                Instruction::BinaryGetUtf8 {
                    fail_label: 16,
                    context: Operand::X(0),
                    destination: Operand::X(2),
                },
                Instruction::BinaryMatch {
                    fail_label: 16,
                    context: Operand::X(0),
                    commands: at_least,
                },
                Instruction::BinarySetPosition {
                    context: Operand::X(0),
                    position: Operand::X(1),
                },
                Instruction::BinaryMatch {
                    fail_label: 17,
                    context: Operand::X(0),
                    commands: exactly,
                },
                Instruction::End,
            ] if matches!(
                at_least.as_slice(),
                [BinaryMatchCommand::EnsureAtLeast { size: 0, unit }] if unit.get() == 8
            ) && matches!(
                exactly.as_slice(),
                [BinaryMatchCommand::EnsureExactly { size: 0 }]
            )
        ),
        "{instructions:?}"
    );
}

#[test]
fn reads_each_bs_match_command_and_rejects_bad_command_lists() {
    let mut table = AtomTable::new();
    let names = [
        "ensure_at_least",
        "ensure_exactly",
        "binary",
        "integer",
        "get_tail",
        "=:=",
        "skip",
        "other",
    ];
    let atoms: Vec<Atom> = names.iter().map(|name| table.put(name).unwrap()).collect();
    // {list,[binary,1,[],8,1,{x,1}, integer,1,[],8,1,{x,1},
    // get_tail,1,8,{x,1}, '=:=',1,8,0, skip,8]}: 22 items.
    let list = [
        0x17, 0x08, 0x16, 0x32, 0x10, 0x02, 0x80, 0x10, 0x13, 0x42, 0x10, 0x02, 0x80, 0x10, 0x13,
        0x52, 0x10, 0x80, 0x13, 0x62, 0x10, 0x80, 0x00, 0x72, 0x80,
    ];
    let mut cursor = 0;
    let commands = read_binary_match_commands(&list, &mut cursor, &atoms).unwrap();
    assert_eq!(cursor, list.len());
    assert!(
        matches!(
            commands.as_slice(),
            [
                BinaryMatchCommand::Unsupported(binary),
                BinaryMatchCommand::Unsupported(integer),
                BinaryMatchCommand::Unsupported(get_tail),
                BinaryMatchCommand::Unsupported(equal),
                BinaryMatchCommand::Unsupported(skip),
            ] if [binary, integer, get_tail, equal, skip].map(Atom::name)
                == ["binary", "integer", "get_tail", "=:=", "skip"]
        ),
        "{commands:?}"
    );
    let failure = |list: &[u8]| {
        read_binary_match_commands(list, &mut 0, &atoms)
            .unwrap_err()
            .to_string()
    };
    // {ensure_at_least,0,0}
    assert_eq!(
        failure(&[0x17, 0x30, 0x12, 0x00, 0x00]),
        "BEAM bs_match ensure_at_least has unit 0"
    );
    // {other,0}: the ERTS loader also refuses it (`bad_bs_match`).
    assert_eq!(
        failure(&[0x17, 0x20, 0x82, 0x00]),
        "unsupported BEAM bs_match command other"
    );
    // A list of 2 items with `{ensure_at_least,0,8}`.
    assert_eq!(
        failure(&[0x17, 0x20, 0x12, 0x00, 0x80]),
        "BEAM bs_match command list has an incomplete command"
    );
}

#[test]
fn decodes_the_fconv_instruction_of_the_float_test_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/float_test/float_test_probe.beam` has these bytes.
    let fconv = [OPCODE_FCONV, 0x03, 0x27, 0x00]; // half/1: fconv x0 fr0
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&fconv[..], &[0x03]].concat());
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::FloatConvert {
                source: Operand::X(0),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_the_is_float_test_of_the_float_test_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/float_test/float_test_probe.beam` has these bytes.
    let is_float = [0x2E, 0x55, 0x03]; // kind/1: is_float f5 x0
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&is_float[..], &[0x03]].concat());
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Test {
                kind: TestKind::Float,
                fail_label: 5,
                value: Operand::X(0),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_the_is_bitstr_test_of_the_bitstr_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/bitstr/bitstr_probe.beam` has these bytes.
    let is_bitstr = [0x81, 0x55, 0x03]; // kind/1: is_bitstr f5 x0
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&is_bitstr[..], &[0x03]].concat());
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Test {
                kind: TestKind::Bitstring,
                fail_label: 5,
                value: Operand::X(0),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_the_get_tl_instruction_of_the_io_start_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/io_start/io_start_probe.beam` has these bytes.
    let get_tl = [OPCODE_GET_TL, 0x03, 0x03]; // rest/1: {get_tl,{x,0},{x,0}}
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&get_tl[..], &[0x03]].concat());
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::GetTail {
                source: Operand::X(0),
                destination: Operand::X(0),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn decodes_the_put_map_exact_instruction_of_the_io_start_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/io_start/io_start_probe.beam` has these bytes.
    // update_guard/1: {put_map_exact,{f,38},{x,0},{x,0},1,
    // {list,[{atom,output},{atom,cooked}]}}, with output as atom 18 and
    // cooked as atom 19 of the module.
    let put_map_exact = [
        OPCODE_PUT_MAP_EXACT,
        0x0D,
        0x26,
        0x03,
        0x03,
        0x10,
        0x17,
        0x20,
        0x0A,
        0x12,
        0x0A,
        0x13,
    ];
    let mut table = AtomTable::new();
    let atoms: Vec<Atom> = (1..=17)
        .map(|index| format!("atom{index}"))
        .chain(["output".into(), "cooked".into()])
        .map(|name| table.put(&name).unwrap())
        .collect();
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&put_map_exact[..], &[0x03]].concat());
    let (instructions, _) = parse_code(&code, &atoms, &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::PutMapExact {
                fail_label: 38,
                map: Operand::X(0),
                destination: Operand::X(0),
                pairs,
            },
            Instruction::End,
        ] if matches!(
            pairs.as_slice(),
            [(Operand::Value(Term::Atom(key)), Operand::Value(Term::Atom(value)))]
                if key.name() == "output" && value.name() == "cooked"
        )
    ));
}

#[test]
fn decodes_the_has_map_fields_tests_of_the_map_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/map/map_probe.beam` has these bytes.
    // pair/1: {test,has_map_fields,{f,17},{x,0},{list,[{atom,input},{atom,output}]}},
    // with output as atom 4 and input as atom 17 of the module.
    let pair = [
        OPCODE_HAS_MAP_FIELDS,
        0x0D,
        0x11,
        0x03,
        0x17,
        0x20,
        0x0A,
        0x11,
        0x42,
    ];
    // key/2: {test,has_map_fields,{f,20},{x,1},{list,[{x,0}]}}.
    let key = [OPCODE_HAS_MAP_FIELDS, 0x0D, 0x14, 0x13, 0x17, 0x10, 0x03];
    let mut table = AtomTable::new();
    let atoms: Vec<Atom> = (1..=17)
        .map(|index| match index {
            4 => "output".to_owned(),
            17 => "input".to_owned(),
            _ => format!("atom{index}"),
        })
        .map(|name| table.put(&name).unwrap())
        .collect();
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&pair[..], &key[..], &[0x03]].concat());
    let (instructions, _) = parse_code(&code, &atoms, &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::HasMapFields {
                fail_label: 17,
                map: Operand::X(0),
                keys,
            },
            Instruction::HasMapFields {
                fail_label: 20,
                map: Operand::X(1),
                keys: register_keys,
            },
            Instruction::End,
        ] if matches!(
            keys.as_slice(),
            [Operand::Value(Term::Atom(first)), Operand::Value(Term::Atom(second))]
                if first.name() == "input" && second.name() == "output"
        ) && matches!(register_keys.as_slice(), [Operand::X(0)])
    ));
}

#[test]
fn rejects_malformed_has_map_fields_key_lists() {
    let error = |operands: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend([&[OPCODE_HAS_MAP_FIELDS, 0x35, 0x03], operands].concat());
        parse_code(&code, &[], &[], &[], &[], &[])
            .unwrap_err()
            .to_string()
    };
    // The key list must be an extended list.
    assert!(error(&[0x27, 0x10, 0x03]).contains("unsupported BEAM list operand"));
    // Eight keys with one byte left.
    assert!(error(&[0x17, 0x80, 0x03]).contains("exceeds code chunk"));
    // A label is not a key operand.
    assert!(error(&[0x17, 0x10, 0x05, 0x03]).contains("unsupported move operand tag 5"));
}

#[test]
fn decodes_the_list_record_and_receive_wait_instructions_of_pinned_gen_statem() {
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    // Bytes of the pinned OTP 29.1.1 `gen_statem.beam`.
    code.extend([
        0x38, 0x0D, 0x75, 0x03, // replies/1: {test,is_nonempty_list,{f,117},[{x,0}]}
        0x41, 0x03, 0x13, 0x23, // replies/1: {get_list,{x,0},{x,1},{x,2}}
        0x34, 0x0D, 0x73, 0x03, // replies/1: {test,is_nil,{f,115},[{x,0}]}
        0xA2, 0x53, 0x73, // system_code_change/4: {get_hd,{x,5},{x,7}}
        // enter/8: {update_record,{atom,copy},7,{tr,{y,9},_},{x,3},{list,[5,{tr,{y,2},_}]}}
        0xB5, 0x0A, 0x31, 0x70, 0x57, 0x94, 0x10, 0x33, 0x17, 0x20, 0x50, 0x57, 0x24, 0x30,
        // loop_t0q/5: {update_record,{atom,inplace},6,{tr,{x,2},_},{x,2},{list,[4,{x,3},5,nil]}}
        0xB5, 0x0A, 0x9F, 0x60, 0x57, 0x23, 0x10, 0x23, 0x17, 0x40, 0x40, 0x33, 0x50, 0x02, 0x1A,
        0x0D, 0xCC, 0x04, // loop/3: {wait_timeout,{f,204},{y,0}}
        0xA1, // call/3: raw_raise
        0x03,
    ]);
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Test {
                kind: TestKind::NonEmptyList,
                fail_label: 117,
                value: Operand::X(0),
            },
            Instruction::GetList {
                source: Operand::X(0),
                head: Operand::X(1),
                tail: Operand::X(2),
            },
            Instruction::Test {
                kind: TestKind::Nil,
                fail_label: 115,
                value: Operand::X(0),
            },
            Instruction::GetHead {
                source: Operand::X(5),
                destination: Operand::X(7),
            },
            Instruction::UpdateRecord {
                source: Operand::Y(9),
                destination: Operand::X(3),
                updates: copy,
            },
            Instruction::UpdateRecord {
                source: Operand::X(2),
                destination: Operand::X(2),
                updates: inplace,
            },
            Instruction::WaitTimeout {
                label: 204,
                source: Operand::Y(0),
            },
            Instruction::Unsupported(OPCODE_RAW_RAISE),
            Instruction::End,
        ] if matches!(copy.as_slice(), [(5, Operand::Y(2))])
            && matches!(
                inplace.as_slice(),
                [(4, Operand::X(3)), (5, Operand::Value(Term::List(nil)))] if nil.is_empty()
            )
    ));
}

#[test]
fn decodes_the_apply_instructions_of_pinned_gen_statem() {
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    // Bytes of the pinned OTP 29.1.1 `gen_statem.beam`.
    code.extend([
        0x70, 0x00, // get_callback_mode/2: {apply,0}
        0x70, 0x30, // terminate/7: {apply,3}
        0x71, 0x10, 0xB0, // terminate/7: {apply_last,1,11}
        0x03,
    ]);
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Apply {
                arity: 0,
                tail: false,
                deallocate: 0,
            },
            Instruction::Apply {
                arity: 3,
                tail: false,
                deallocate: 0,
            },
            Instruction::Apply {
                arity: 1,
                tail: true,
                deallocate: 11,
            },
            Instruction::End,
        ]
    ));
    // `apply_last` without its deallocate operand is truncated.
    code.truncate(code.len() - 2);
    assert!(parse_code(&code, &[], &[], &[], &[], &[]).is_err());
}

#[test]
fn decodes_gc_bif1_and_gc_bif2_of_the_arith_probe_fixture_into_a_bif_instruction() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/arith/arith_probe.beam` has these bytes.
    // up/1: gc_bif2 f58 live 1 '+'/2 {tr,x0,1} 1 x0
    let up = [0x7D, 0x0D, 0x3A, 0x10, 0x00, 0x57, 0x03, 0x10, 0x11, 0x03];
    // minus_overflow/0: gc_bif2 f0 live 1 '-'/2 {tr,y0,1} {tr,x0,1} x0
    let minus = [
        0x7D, 0x05, 0x10, 0x10, 0x57, 0x04, 0x10, 0x57, 0x03, 0x10, 0x03,
    ];
    // neg/1: gc_bif1 f55 live 1 '-'/1 x0 x0
    let neg = [0x7C, 0x0D, 0x37, 0x10, 0x50, 0x03, 0x03];
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&up[..], &minus, &neg, &[0x03]].concat());
    let imports = vec![
        ("erlang".to_owned(), "+".to_owned(), 2),
        ("erlang".to_owned(), "-".to_owned(), 2),
        ("erlang".to_owned(), "self".to_owned(), 0),
        ("erlang".to_owned(), "*".to_owned(), 2),
        ("erlang".to_owned(), "div".to_owned(), 2),
        ("erlang".to_owned(), "-".to_owned(), 1),
    ];
    let (instructions, _) = parse_code(&code, &[], &imports, &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Bif {
                fail_label: 58,
                module: plus_module,
                name: plus,
                arguments: plus_arguments,
                destination: Operand::X(0),
            },
            Instruction::Bif {
                fail_label: 0,
                module: minus_module,
                name: minus,
                arguments: minus_arguments,
                destination: Operand::X(0),
            },
            Instruction::Bif {
                fail_label: 55,
                module: negate_module,
                name: negate,
                arguments: negate_arguments,
                destination: Operand::X(0),
            },
            Instruction::End,
        ] if plus_module == "erlang"
            && plus == "+"
            && matches!(
                plus_arguments.as_slice(),
                [Operand::X(0), Operand::Value(Term::Integer(1))]
            )
            && minus_module == "erlang"
            && minus == "-"
            && matches!(minus_arguments.as_slice(), [Operand::Y(0), Operand::X(0)])
            && negate_module == "erlang"
            && negate == "-"
            && matches!(negate_arguments.as_slice(), [Operand::X(0)])
    ));
    // Without its destination and the end, the instruction is truncated.
    code.truncate(code.len() - 2);
    assert!(parse_code(&code, &[], &imports, &[], &[], &[]).is_err());
}

#[test]
fn decodes_the_fun_instructions_of_the_fun_probe_fixture() {
    let code = |instructions: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend_from_slice(instructions);
        code.push(0x03);
        code
    };
    let atoms = vec![Atom::from_static("safe").unwrap()];
    let module = AtomTable::new().put("fun_probe").unwrap();
    let lambda = |index, label, arity| Lambda::new(module.clone(), index, 31_263_486, label, arity);
    // The first five `FunT` entries of the fixture, with their free
    // variable counts.
    let lambdas = vec![
        (lambda(0, 127, 1), 1),
        (lambda(1, 125, 0), 0),
        (lambda(2, 123, 1), 1),
        (lambda(3, 121, 0), 0),
        (lambda(4, 112, 1), 1),
    ];
    let parse =
        |instructions: &[u8]| parse_code(&code(instructions), &atoms, &[], &[], &lambdas, &[]);
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/funs/fun_probe.beam` has these bytes.
    let make_pair = [0xAB, 0x40, 0x03, 0x17, 0x10, 0x03]; // make_pair/1: make_fun3 4 x0 [x0]
    let make_const = [0xAB, 0x10, 0xA4, 0x17, 0x00]; // locals/0: make_fun3 1 y10 []
    let call_fun = [0x4B, 0x00]; // locals/0: call_fun 0
    let tail_call = [0xB2, 0x00, 0x10, 0x57, 0x13, 0x20]; // tail_call/1: call_fun2 0 1 x1
    // The fixture bytes, then a `call_fun2` with the atom tag `safe` and
    // the fun in `y0`.
    let (instructions, _) = parse(
        &[
            &make_pair[..],
            &make_const,
            &call_fun,
            &tail_call,
            &[0xB2, 0x12, 0x00, 0x04],
        ]
        .concat(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::MakeFun { lambda: first, free, destination: Operand::X(0) },
            Instruction::MakeFun { lambda: second, free: empty, destination: Operand::Y(10) },
            Instruction::CallFun { arity: 0, function: Operand::X(0), tail: false, .. },
            Instruction::CallFun { arity: 1, function: Operand::X(1), tail: false, .. },
            Instruction::CallFun { arity: 0, function: Operand::Y(0), tail: false, .. },
            Instruction::End,
        ] if *first == lambdas[4].0
            && matches!(free.as_slice(), [Operand::X(0)])
            && *second == lambdas[1].0
            && empty.is_empty()
    ));
    let error = |instructions: &[u8]| parse(instructions).unwrap_err().to_string();
    // A lambda number outside the table, or an environment of another
    // length than the free variables of the entry.
    for make_fun3 in [
        [0xAB, 0x50, 0x03, 0x17, 0x00],
        [0xAB, 0x40, 0x03, 0x17, 0x00],
    ] {
        assert!(error(&make_fun3).contains("does not match the lambda table"));
    }
    assert_eq!(
        error(&[0xB2, 0x02, 0x00, 0x03]),
        "BEAM atom index zero is invalid"
    );
    assert_eq!(
        error(&[0xB2, 0x22, 0x00, 0x03]),
        "BEAM atom index 2 out of range"
    );
    assert_eq!(
        error(&[0xB2, 0x00, 0x00, 0x12]),
        "BEAM call_fun2 function is not a register"
    );
    assert!(error(&[0xB2, 0x01, 0x00, 0x03]).contains("unexpected BEAM operand tag 1"));
}

#[test]
fn joins_a_fun_call_in_a_tail_position_as_the_erts_loader() {
    // `call_fun Arity | deallocate D | return => i_call_fun_last Arity D`
    // and the same for `call_fun2` (`jit/arm/ops.tab:868-880`). Only
    // the three adjacent instructions join.
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend_from_slice(&[
        0x4B, 0x10, 0x12, 0x20, 0x13, // call_fun 1, deallocate 2, return
        0xB2, 0x00, 0x00, 0x04, // call_fun2 0 0 y0
        0x12, 0x00, 0x13, // deallocate 0, return
        0x4B, 0x00, 0x13, // call_fun 0, return
        0x12, 0x00, 0x13, // deallocate 0, return
        0x03,
    ]);
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::CallFun {
                arity: 1,
                function: Operand::X(1),
                tail: true,
                deallocate: 2
            },
            Instruction::CallFun {
                arity: 0,
                function: Operand::Y(0),
                tail: true,
                deallocate: 0
            },
            Instruction::CallFun {
                arity: 0,
                function: Operand::X(0),
                tail: false,
                deallocate: 0
            },
            Instruction::Return,
            Instruction::Deallocate { stack_size: 0 },
            Instruction::Return,
            Instruction::End,
        ]
    ));
}

#[test]
fn turns_a_call_of_erlang_hibernate_into_the_erts_hibernate_instruction() {
    // `call_ext`, `call_ext_last`, and `call_ext_only` of
    // `erlang:hibernate/0` (`jit/arm/ops.tab:669-673`).
    let decode = |bytes: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend_from_slice(bytes);
        code.push(0x03);
        let imports = [("erlang".to_owned(), "hibernate".to_owned(), 0)];
        parse_code(&code, &[], &imports, &[], &[], &[]).unwrap().0
    };
    assert!(matches!(
        decode(&[0x07, 0x00, 0x00]).as_slice(), // call_ext 0 import 0
        [Instruction::Hibernate, Instruction::End]
    ));
    assert!(matches!(
        decode(&[0x08, 0x00, 0x00, 0x20]).as_slice(), // call_ext_last 0 import 0 2
        [
            Instruction::Hibernate,
            Instruction::Deallocate { stack_size: 2 },
            Instruction::Return,
            Instruction::End
        ]
    ));
    assert!(matches!(
        decode(&[0x4E, 0x00, 0x00]).as_slice(), // call_ext_only 0 import 0
        [
            Instruction::Hibernate,
            Instruction::Return,
            Instruction::End
        ]
    ));
}

#[test]
fn turns_a_call_of_erlang_yield_into_the_erts_yield_instruction() {
    // `call_ext`, `call_ext_last`, and `call_ext_only` of `erlang:yield/0`
    // (`jit/arm/ops.tab:664-666`).
    let decode = |bytes: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend_from_slice(bytes);
        code.push(0x03);
        let imports = [("erlang".to_owned(), "yield".to_owned(), 0)];
        parse_code(&code, &[], &imports, &[], &[], &[]).unwrap().0
    };
    assert!(matches!(
        decode(&[0x07, 0x00, 0x00]).as_slice(), // call_ext 0 import 0
        [Instruction::Yield, Instruction::End]
    ));
    assert!(matches!(
        decode(&[0x08, 0x00, 0x00, 0x20]).as_slice(), // call_ext_last 0 import 0 2
        [
            Instruction::Yield,
            Instruction::Deallocate { stack_size: 2 },
            Instruction::Return,
            Instruction::End
        ]
    ));
    assert!(matches!(
        decode(&[0x4E, 0x00, 0x00]).as_slice(), // call_ext_only 0 import 0
        [Instruction::Yield, Instruction::Return, Instruction::End]
    ));
}

#[test]
fn puts_a_test_yield_after_the_entry_label_of_each_function() {
    // `func_prologue` ends with `i_test_yield` (`jit/arm/ops.tab:929-938`).
    // A label that does not follow `func_info` gets no `TestYield`.
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 11]);
    code.push(3); // labels 1 and 2
    code.extend([0; 3]);
    code.push(1); // one function
    code.extend([
        0x02, 0x12, 0x22, 0x00, // func_info preempt_probe body 0
        0x01, 0x10, // label 1
        0x01, 0x20, // label 2
        0x13, // return
        0x03,
    ]);
    let mut table = AtomTable::new();
    let atoms = ["preempt_probe", "body"].map(|name| table.put(name).unwrap());
    let (instructions, _) = parse_code(&code, &atoms, &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::FunctionInfo { .. },
            Instruction::Label(1),
            Instruction::TestYield,
            Instruction::Label(2),
            Instruction::Return,
            Instruction::End,
        ]
    ));
}

#[test]
fn puts_the_deallocate_and_the_return_of_a_tail_call_after_call_ext() {
    // A tail call of a BIF returns to `deallocate D | return`
    // (`jit/arm/ops.tab:687-714`). A tail call of Erlang code replaces the
    // frame and does not reach them.
    let decode = |bytes: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend_from_slice(bytes);
        code.push(0x03);
        let imports = [("erlang".to_owned(), "whereis".to_owned(), 1)];
        parse_code(&code, &[], &imports, &[], &[], &[]).unwrap().0
    };
    let whereis = |instruction: &Instruction, tail: bool, size: u32| {
        matches!(
            instruction,
            Instruction::CallExternal { arity: 1, module, name, tail: found, deallocate }
                if module == "erlang" && name == "whereis" && *found == tail && *deallocate == size
        )
    };
    let call = decode(&[0x07, 0x10, 0x00]); // call_ext 1 import 0
    assert!(whereis(&call[0], false, 0));
    assert!(matches!(call[1..], [Instruction::End]));
    let last = decode(&[0x08, 0x10, 0x00, 0x20]); // call_ext_last 1 import 0 2
    assert!(whereis(&last[0], true, 2));
    assert!(matches!(
        last[1..],
        [
            Instruction::Deallocate { stack_size: 2 },
            Instruction::Return,
            Instruction::End
        ]
    ));
    let only = decode(&[0x4E, 0x10, 0x00]); // call_ext_only 1 import 0
    assert!(whereis(&only[0], true, 0));
    assert!(matches!(only[1..], [Instruction::Return, Instruction::End]));
}

#[test]
fn decodes_the_is_function_tests_of_the_fun_probe_fixture() {
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    // Bytes of `fun_probe:kind/1` in `fixtures/funs/fun_probe.beam`, then
    // an `is_function2` with the arity in `x1`. `tests/integration/fixture_opcodes.rs`
    // checks that the fixture has these bytes.
    let is_function2 = [0x73, 0x0D, 0x64, 0x57, 0x03, 0x40, 0x11]; // is_function2 f100 x0 1
    let is_function = [0x4D, 0x0D, 0x65, 0x57, 0x03, 0x40]; // is_function f101 x0
    code.extend(is_function2);
    code.extend(is_function);
    code.extend([
        0x73, 0x0D, 0x64, 0x03, 0x13, // is_function2 f100 x0 x1
        0x03,
    ]);
    let (instructions, _) = parse_code(&code, &[], &[], &[], &[], &[]).unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::TestFunctionArity {
                fail_label: 100,
                value: Operand::X(0),
                arity: Operand::Value(Term::Integer(1)),
            },
            Instruction::Test {
                kind: TestKind::Function,
                fail_label: 101,
                value: Operand::X(0),
            },
            Instruction::TestFunctionArity {
                fail_label: 100,
                value: Operand::X(0),
                arity: Operand::X(1),
            },
            Instruction::End,
        ]
    ));
}

#[test]
fn rejects_malformed_record_update_lists() {
    let error = |bytes: &[u8]| {
        read_record_updates(bytes, &mut 0, &[], &[])
            .unwrap_err()
            .to_string()
    };
    // The shared header reader rejects a list that is not a pair list.
    assert!(error(&[0x17, 0x10, 0x50]).contains("incomplete pair"));
    // A position must be an unsigned operand.
    assert!(error(&[0x17, 0x20, 0x03, 0x03]).contains("unexpected BEAM operand tag 3"));
}

#[test]
fn rejects_a_put_tuple2_without_an_element_list() {
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([OPCODE_PUT_TUPLE2, 0x03, 0x27]);
    assert!(parse_code(&code, &[], &[], &[], &[], &[]).is_err());
}

#[test]
fn maps_supported_type_tests_and_rejects_unknown_opcodes() {
    assert!(test_kind(OPCODE_TEST_IS_ATOM).is_ok());
    assert!(test_kind(OPCODE_TEST_IS_PID).is_ok());
    assert!(matches!(test_kind(OPCODE_TEST_IS_PORT), Ok(TestKind::Port)));
    assert_eq!(unsupported_opcode_operand_count(OPCODE_TEST_IS_PORT), None);
    assert!(test_kind(OPCODE_TEST_IS_LIST).is_ok());
    assert!(test_kind(OPCODE_TEST_IS_BINARY).is_ok());
    assert!(test_kind(255).is_err());
}

#[test]
fn reads_pair_list_headers_and_rejects_malformed_forms() {
    let mut cursor = 0;
    let list = [0x17, 0x40, 0x10, 0x15, 0x20, 0x25];
    assert_eq!(read_pair_count(&list, &mut cursor).unwrap(), 2);
    assert_eq!(cursor, 2);
    assert_eq!(read_pair_count(&[0x17, 0x00], &mut 0).unwrap(), 0);
    assert!(read_pair_count(&[], &mut 0).is_err());
    assert!(read_pair_count(&[0x17], &mut 0).is_err());
    assert!(read_pair_count(&[0x27, 0x00], &mut 0).is_err());
    assert!(read_pair_count(&[0x17, 0x10, 0x00], &mut 0).is_err());
    // Eight items with no bytes left: rejected before any allocation.
    assert!(
        read_pair_count(&[0x17, 0x80], &mut 0)
            .unwrap_err()
            .to_string()
            .contains("exceeds code chunk")
    );
}

#[test]
fn skips_pinned_module_instruction_operands_and_rejects_truncation() {
    for (opcode, count) in [
        (OPCODE_TEST_HEAP, 2),
        (OPCODE_BIF0, 2),
        (OPCODE_TEST_NE, 3),
        (OPCODE_TEST_TAGGED_TUPLE, 4),
        (OPCODE_TEST_IS_MAP, 2),
        (OPCODE_BADMATCH, 1),
        (OPCODE_TRY_CASE_END, 1),
        (OPCODE_PUT_MAP_ASSOC, 5),
        (OPCODE_GET_RECORD_ELEMENTS, 3),
        (OPCODE_PUT_RECORD, 6),
        (OPCODE_PUT_TUPLE2, 2),
        (OPCODE_INIT_YREGS, 1),
        (OPCODE_BS_INIT_WRITABLE, 0),
        (OPCODE_BS_START_MATCH4, 4),
        (OPCODE_RAW_RAISE, 0),
        (OPCODE_BS_GET_UTF16, 5),
        (OPCODE_BS_GET_UTF32, 5),
        (OPCODE_BS_GET_INTEGER2, 7),
        (OPCODE_GC_BIF3, 7),
        (OPCODE_IS_ANY_NATIVE_RECORD, 2),
        (OPCODE_FMOVE, 2),
        (OPCODE_FADD, 4),
        (OPCODE_FSUB, 4),
        (OPCODE_FMUL, 4),
        (OPCODE_FDIV, 4),
    ] {
        assert_eq!(unsupported_opcode_operand_count(opcode), Some(count));
    }
    assert_eq!(unsupported_opcode_operand_count(255), None);

    let mut cursor = 0;
    assert!(skip_unsupported_instruction(255, &[], &mut cursor).is_err());
    assert!(skip_unsupported_instruction(OPCODE_BS_GET_UTF16, &[0], &mut cursor).is_err());
    assert!(skip_counted_operands(&[0x10], &mut 0).is_err());
    assert!(skip_counted_operands(&[0x20], &mut 0).is_err());
}

#[test]
fn skips_extended_operands_and_reports_invalid_or_truncated_forms() {
    for (operand, expected_cursor) in [
        (vec![0x07, 0, 0, 0, 0, 0, 0, 0, 0], 9),
        (vec![0x17], 1),
        (vec![0x27, 0x10], 2),
        (vec![0x37, 0x10, 0x10, 0x10], 4),
        (vec![0x47, 0x10], 2),
    ] {
        let mut cursor = 0;
        skip_operand(&operand, &mut cursor).unwrap();
        assert_eq!(cursor, expected_cursor);
    }
    assert!(skip_operand(&[0x07], &mut 0).is_err());
    assert!(skip_operand(&[0x37, 0x10], &mut 0).is_err());
    assert!(skip_operand(&[0x57], &mut 0).is_err());
    assert!(skip_operand(&[0x87], &mut 0).is_err());
    assert!(skip_operand(&[0x0f], &mut 0).is_err());

    let mut cursor = 0;
    assert!(skip_integer_payload(&[0], &mut cursor, 0xf8).is_err());
    assert!(skip_integer_payload(&[0, 0], &mut 0, 0x19).is_ok());
    assert!(read_integer_after_first(&[0], &mut 0, 0xf9).is_err());
}

#[test]
fn skips_bs_create_bin_allocations_that_erts_loads() {
    // Each operand loads on the pinned ERTS in bs_create_bin_alloc_probe
    // (beam_asm:module/4): 7, {alloc,[{words,1},{floats,0},{funs,1}]},
    // {alloc,[]}, {alloc,[{words,715827882}]}, and
    // {alloc,[{words,715827881},{floats,715827882},{funs,1}]}, with a sum
    // of i32::MAX - 2 words.
    let skip = |bytes: &[u8]| {
        let mut cursor = 0;
        skip_allocation(bytes, &mut cursor).map(|()| cursor)
    };
    assert_eq!(skip(&[0x70]).unwrap(), 1);
    assert_eq!(
        skip(&[0x37, 0x30, 0x00, 0x10, 0x10, 0x00, 0x20, 0x10]).unwrap(),
        8
    );
    assert_eq!(skip(&[0x37, 0x00]).unwrap(), 2);
    assert_eq!(
        skip(&[0x37, 0x10, 0x00, 0x58, 0x2A, 0xAA, 0xAA, 0xAA]).unwrap(),
        8
    );
    assert_eq!(
        skip(&[
            0x37, 0x30, 0x00, 0x58, 0x2A, 0xAA, 0xAA, 0xA9, 0x10, 0x58, 0x2A, 0xAA, 0xAA, 0xAA,
            0x20, 0x10,
        ])
        .unwrap(),
        16
    );
}

#[test]
fn rejects_bs_create_bin_allocations_that_erts_rejects() {
    // The pinned ERTS gives {error,badfile} for each of these operands of
    // bs_create_bin_alloc_probe: four items, a number of 715827883 words,
    // floats, or funs, a sum over i32::MAX, {atom,foo}, and {x,0}. The
    // assembler cannot write kind 3, which marshal_allocation_list rejects
    // (beam_file.c:2272-2273), or another extended operand.
    let reject = |bytes: &[u8]| skip_allocation(bytes, &mut 0).is_err();
    assert!(reject(&[]));
    assert!(reject(&[
        0x37, 0x40, 0x00, 0x10, 0x00, 0x00, 0x10, 0x00, 0x20, 0x10
    ]));
    for kind in [0x00, 0x10, 0x20] {
        assert!(reject(&[0x37, 0x10, kind, 0x58, 0x2A, 0xAA, 0xAA, 0xAB]));
    }
    assert!(reject(&[
        0x37, 0x30, 0x00, 0x58, 0x2A, 0xAA, 0xAA, 0xAA, 0x10, 0x58, 0x2A, 0xAA, 0xAA, 0xAA, 0x20,
        0x10,
    ]));
    assert!(reject(&[0x37, 0x10, 0x30, 0x10]));
    assert!(reject(&[0x12]));
    assert!(reject(&[0x03]));
    assert!(reject(&[0x17, 0x00]));
}

#[test]
fn reads_bs_create_bin_segments_and_rejects_malformed_lists() {
    let atoms: Vec<Atom> = ["string", "binary", "all", "integer"]
        .into_iter()
        .map(|name| Atom::from_static(name).unwrap())
        .collect();
    let read =
        |bytes: &[u8], strings: &[u8]| read_binary_segments(bytes, &mut 0, &atoms, &[], strings);
    // {list,[{atom,string},0,8,nil,{string,<<"count: ">>},{integer,7},
    //        {atom,binary},2,8,nil,{x,0},{atom,all}]}
    let segments = read(
        &[
            0x17, 0xC0, 0x12, 0x00, 0x80, 0x02, 0x00, 0x71, 0x22, 0x20, 0x80, 0x02, 0x03, 0x32,
        ],
        b"count: ",
    )
    .unwrap();
    assert!(matches!(
        segments.as_slice(),
        [
            BinarySegment::String(text),
            BinarySegment::BinaryAll { segment: 2, source: Operand::X(0) },
        ] if text == b"count: "
    ));
    // {atom,integer},2,1,nil,{x,0},{integer,8}; {atom,binary} of unit 1
    // and size all; {atom,binary} of unit 8 and size 8.
    for segment in [
        [0x42, 0x20, 0x10, 0x02, 0x03, 0x81],
        [0x22, 0x20, 0x10, 0x02, 0x03, 0x32],
        [0x22, 0x20, 0x80, 0x02, 0x03, 0x81],
    ] {
        let mut bytes = vec![0x17, 0x60];
        bytes.extend(segment);
        assert!(matches!(
            read(&bytes, b"").unwrap().as_slice(),
            [BinarySegment::Unsupported(kind)] if kind == &atoms[segment[0] as usize / 16 - 1]
        ));
    }
    // A list of one item, and a string of 7 bytes in a StrT chunk of 5.
    assert!(read(&[0x17, 0x10, 0x12], b"").is_err());
    assert!(read(&[0x17, 0x60, 0x12, 0x00, 0x80, 0x02, 0x00, 0x71], b"short").is_err());
}
