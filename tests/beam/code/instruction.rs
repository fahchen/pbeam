use super::super::{LineTable, read_integer, read_integer_after_first, read_operand, read_value};
use super::{
    FloatOperation, OPCODE_BIF0, OPCODE_FADD, OPCODE_FCONV, OPCODE_FDIV, OPCODE_FMOVE, OPCODE_FMUL,
    OPCODE_FNEGATE, OPCODE_FSUB, OPCODE_GET_MAP_ELEMENTS, OPCODE_GET_TL, OPCODE_HAS_MAP_FIELDS,
    OPCODE_INIT_YREGS, OPCODE_PUT_MAP_ASSOC, OPCODE_PUT_MAP_EXACT, OPCODE_PUT_TUPLE2,
    OPCODE_TEST_EXACT_EQ, OPCODE_TEST_HEAP, OPCODE_TEST_IS_ATOM, OPCODE_TEST_IS_BINARY,
    OPCODE_TEST_IS_LIST, OPCODE_TEST_IS_MAP, OPCODE_TEST_IS_NUMBER, OPCODE_TEST_IS_PID,
    OPCODE_TEST_IS_PORT, OPCODE_TEST_TAGGED_TUPLE, SELECT_VALUE_LINEAR_LIMIT, parse_code,
    prepare_select_value_choices, read_binary_match_commands, read_binary_segments,
    read_move_operand, read_operand_pairs, read_pair_count, read_record_updates,
    read_segment_flags, read_segment_string, skip_allocation, skip_counted_operands,
    skip_integer_payload, skip_operand, skip_unsupported_instruction, sort_map_key_pairs,
    test_kind, unsupported_opcode_operand_count,
};
use crate::{
    beam::code::{
        BinaryMatchCommand, BinarySegment, Instruction, Operand, SelectValueChoices, TestKind,
        TestOperand,
    },
    runtime::BifId,
    term::{Atom, AtomTable, Lambda, Reference, Term},
};

#[test]
fn reads_operand_pairs_and_rejects_other_lists() {
    let atoms = vec![Atom::from_static("output").unwrap()];
    // {list,[{atom,output},{x,1}]}
    let pairs = read_operand_pairs(&[0x17, 0x20, 0x12, 0x13], &mut 0, &atoms, &[]).unwrap();
    assert!(matches!(
        pairs.as_slice(),
        [(Operand::Value(Term::Atom(key)), Operand::X(1))] if key.is_static("output")
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
    assert!(
        parse_code(
            &[],
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
    assert!(
        parse_code(
            &[0; 20],
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
    assert!(
        parse_code(
            &[0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
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
        Term::Integer(-101_714_049_538_469_044)
    );
    // A leading zero byte keeps a large payload positive.
    assert_eq!(integer(&[0x39, 0x00, 0xFF, 0xFF]), Term::Integer(65535));
    // The short and extended forms are never negative.
    assert_eq!(integer(&[0xF1]), Term::Integer(15));
    assert_eq!(integer(&[0xE9, 0xFF]), Term::Integer(2047));
}

#[test]
fn reads_only_supported_move_literals() {
    let literals = vec![Some(Term::binary(b"ok".to_vec())), None];
    assert!(matches!(
        &read_move_operand(&[0x47, 0x00], &mut 0, &[], &literals).unwrap(),
        Operand::Value(Term::Binary(value)) if value.bit_bytes() == b"ok"
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
        Operand::Value(values) if values.is_nil()
    ));
    // erlc 29.1.1 writes {integer,18446744073709551616} as 0xF9, the
    // extra byte count 0, and nine payload bytes. It loads as a bignum,
    // as ERTS `marshal_integer` (`beam_file.c:2134-2215`) loads it; a
    // short payload is malformed, and so is a payload with two leading
    // zero bytes of magnitude.
    let mut cursor = 0;
    let huge = [0xF9, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(matches!(
        read_move_operand(&huge, &mut cursor, &[], &literals).unwrap(),
        Operand::Value(value) if value == Term::from_bigint(num_bigint::BigInt::from(1) << 64)
    ));
    assert_eq!(cursor, huge.len());
    let negative = [0xF9, 0x00, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(matches!(
        read_move_operand(&negative, &mut 0, &[], &literals).unwrap(),
        Operand::Value(value) if value == Term::from_bigint(-(num_bigint::BigInt::from(1) << 64u32))
    ));
    assert!(read_move_operand(&huge[..10], &mut 0, &[], &literals).is_err());
    let padded = [0xF9, 0x00, 0, 0, 0xFF, 0, 0, 0, 0, 0, 0];
    assert!(read_move_operand(&padded, &mut 0, &[], &literals).is_err());
    assert!(read_move_operand(&[0xF9, 0x7F], &mut 0, &[], &literals).is_err());
    // `1 bsl 4194240` needs 524281 two's complement bytes, so the extra
    // byte count is 524272: the unsigned operand [0x38, 0x07, 0xFF, 0xF0].
    // It has one bit more than `BIG_ARITY_MAX` digits hold, and
    // `bytes_to_big` fails (`beam_file.c:2210-2211`). `1 bsl 4194240 - 1`
    // fits and loads.
    let operand = |first: u8, rest: u8| {
        let mut bytes = vec![0xF9, 0x38, 0x07, 0xFF, 0xF0, first];
        bytes.resize(bytes.len() + 524_280, rest);
        bytes
    };
    let error = read_move_operand(&operand(0x01, 0x00), &mut 0, &[], &literals).unwrap_err();
    assert!(error.to_string().contains("BIG_ARITY_MAX"), "{error}");
    let limit = (num_bigint::BigInt::from(1) << 4_194_240) - 1;
    assert!(matches!(
        read_move_operand(&operand(0x00, 0xFF), &mut 0, &[], &literals).unwrap(),
        Operand::Value(value) if value == Term::from_bigint(limit)
    ));
    assert!(read_move_operand(&[0x47, 0x20], &mut 0, &[], &literals).is_err());
    assert!(read_move_operand(&[0x27, 0x00], &mut 0, &[], &literals).is_err());
    assert!(read_move_operand(&[0x06], &mut 0, &[], &literals).is_err());
    assert_eq!(read_value(&[0x02], &mut 0, &[]).unwrap(), Term::nil());
}

#[test]
fn rejects_unsupported_select_val_forms() {
    let code = |instructions: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend_from_slice(instructions);
        code
    };

    assert!(
        parse_code(
            &code(&[59, 3, 21, 39]),
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
    assert!(
        parse_code(
            &code(&[59, 3, 21, 23, 16]),
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
    // A register as a selection value, with two items present.
    assert!(
        parse_code(
            &code(&[59, 3, 21, 23, 32, 3, 21]),
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .unwrap_err()
        .to_string()
        .contains("unsupported BEAM selection value")
    );
    // A count larger than the remaining bytes fails before any allocation.
    for opcode in [59, 60] {
        assert!(
            parse_code(
                &code(&[opcode, 3, 21, 23, 32, 3]),
                &AtomTable::new(),
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                &mut LineTable::default()
            )
            .unwrap_err()
            .to_string()
            .contains("exceeds code chunk")
        );
    }
}

/// Check the prepared form of a `select_val` choice list: a small list keeps
/// the encoded order, a large distinct list takes the term order, and a
/// duplicate or an incomparable choice keeps the linear form with the
/// encoded order.
#[test]
fn prepares_the_select_value_choices_by_shape() {
    let atoms = AtomTable::new();
    let limit_signed = i64::try_from(SELECT_VALUE_LINEAR_LIMIT).unwrap();
    let limit_unsigned = u64::try_from(SELECT_VALUE_LINEAR_LIMIT).unwrap();
    let choices = |values: &[i64]| -> Vec<(Term, u32)> {
        values
            .iter()
            .enumerate()
            .map(|(index, value)| (Term::Integer(*value), u32::try_from(index).unwrap()))
            .collect()
    };

    // The boundary: a list of exactly the limit keeps the linear walk.
    let limit: Vec<i64> = (0..limit_signed).collect();
    assert!(matches!(
        prepare_select_value_choices(choices(&limit), &atoms),
        SelectValueChoices::Linear(_)
    ));

    // One choice more takes the term order.
    let mut values: Vec<i64> = (0..=limit_signed).collect();
    values.reverse();
    let large = choices(&values);
    let SelectValueChoices::Ordered(ordered) = prepare_select_value_choices(large.clone(), &atoms)
    else {
        panic!(
            "{} distinct choices must take the ordered form",
            values.len()
        );
    };
    let mut expected = large;
    expected.sort_by_key(|(term, _)| match term {
        Term::Integer(value) => *value,
        _ => unreachable!("the probe holds integers only"),
    });
    assert_eq!(ordered, expected);

    // A duplicate choice keeps the linear form and its encoded order, so the
    // interpreter still gives the first match.
    let mut duplicate_values: Vec<i64> = (0..limit_signed).collect();
    duplicate_values.push(0);
    let duplicate = choices(&duplicate_values);
    let SelectValueChoices::Linear(kept) = prepare_select_value_choices(duplicate.clone(), &atoms)
    else {
        panic!("a duplicate choice must keep the linear form");
    };
    assert_eq!(kept, duplicate);

    // Two different references have a term order, so a large list of
    // distinct references takes the ordered form.
    let references: Vec<(Term, u32)> = (0..=limit_unsigned)
        .map(|index| {
            (
                Term::Reference(Reference(index)),
                u32::try_from(index).unwrap(),
            )
        })
        .collect();
    assert!(matches!(
        prepare_select_value_choices(references, &atoms),
        SelectValueChoices::Ordered(_)
    ));
    // Two different alias references have a PBeam term order
    // (`src/term/order.rs`), so a large list of distinct aliases also takes
    // the ordered form.
    let aliases: Vec<(Term, u32)> = (0..=limit_unsigned)
        .map(|index| {
            (
                Term::Reference(Reference(index).pid_reference()),
                u32::try_from(index).unwrap(),
            )
        })
        .collect();
    assert!(matches!(
        prepare_select_value_choices(aliases, &atoms),
        SelectValueChoices::Ordered(_)
    ));
}

#[test]
fn decodes_build_stacktrace_without_operands() {
    // `try f() catch C:R:S -> S end` compiles to `{move,{x,2},{x,0}}`
    // and `build_stacktrace`.
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([0x40, 0x23, 0x03, 0xA0, 0x03]);
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Raise {
                trace: Operand::X(2),
                value: Operand::X(1),
            },
            Instruction::Raise {
                trace: Operand::Y(0),
                value: Operand::Value(values),
            },
            Instruction::End,
        ] if values.is_nil()
    ));
}

#[test]
fn decodes_the_error_path_instructions() {
    // Bytes of `fixtures/error_path/error_path_probe.beam`: `badmatch x0`,
    // `badmatch y0`, `case_end x0`, `if_end`, `try_case_end x0`,
    // `badrecord x0`, and `raw_raise`. ERTS also loads a literal source,
    // such as `case_end nil` (`emu/ops.tab:273-282`).
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([
        0x48, 0x03, 0x48, 0x04, 0x4A, 0x03, 0x49, 0x6B, 0x03, 0xB4, 0x03, 0xA1, 0x4A, 0x02, 0x03,
    ]);
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::TaggedError {
                tag: "badmatch",
                value: Operand::X(0),
            },
            Instruction::TaggedError {
                tag: "badmatch",
                value: Operand::Y(0),
            },
            Instruction::TaggedError {
                tag: "case_clause",
                value: Operand::X(0),
            },
            Instruction::IfEnd,
            Instruction::TaggedError {
                tag: "try_clause",
                value: Operand::X(0),
            },
            Instruction::TaggedError {
                tag: "badrecord",
                value: Operand::X(0),
            },
            Instruction::RawRaise,
            Instruction::TaggedError {
                tag: "case_clause",
                value: Operand::Value(values),
            },
            Instruction::End,
        ] if values.is_nil()
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
    let (instructions, _, _) = parse_code(
        &code(&[
            0x68, 0x04, 0x75, // try y0 f7
            0x68, 0x14, 0x0D, 0x11, // try y1 f17
            0x3E, 0x04, 0xA5, // catch y0 f10
            0x69, 0x04, // try_end y0
            0x6A, 0x04, // try_case y0
            0x3F, 0x04, // catch_end y0
        ]),
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
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
        parse_code(
            &code(&[0x68, 0x03, 0x75]),
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
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
    let table = AtomTable::new();
    let atoms = ["prim_tty", "write_nif"].map(|name| table.put(name).unwrap());
    let (instructions, _, _) = parse_code(
        &code(&[
            0x02, 0x12, 0x22, 0x20, // func_info prim_tty write_nif 2
            0xB3, // nif_start
        ]),
        &table,
        &atoms,
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
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
    let (instructions, _, _) = parse_code(
        &code(&[
            0x02, 0x12, 0x22, 0x20, // func_info prim_tty write_nif 2
            0x01, 0x10, // label 1
            0x40, 0x12, 0x03, // move prim_tty x0
            0xB3, // nif_start
        ]),
        &table,
        &atoms,
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
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
        parse_code(
            &code(&[0xB3]),
            &table,
            &atoms,
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
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
    let (instructions, _, _) = parse_code(
        &code(&[
            0x0A, 0x05, 0x00, 0x57, 0x03, 0x40, 0x23, // bif1 f0 node/1 {tr,x0,pid} x2
            0x09, 0x10, 0x53, // bif0 node/0 x5
            0x0A, 0x35, 0x00, 0x03, 0x13, // bif1 f3 node/1 x0 x1
        ]),
        &AtomTable::new(),
        &[],
        &imports,
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Bif {
                fail_label: 0,
                bif: BifId::ErlangNode1,
                arguments: node1_arguments,
                destination: Operand::X(2),
            },
            Instruction::Bif {
                fail_label: 0,
                bif: BifId::ErlangNode0,
                arguments: node0_arguments,
                destination: Operand::X(5),
            },
            Instruction::Bif {
                fail_label: 3,
                bif: BifId::ErlangNode1,
                arguments: guard_arguments,
                destination: Operand::X(1),
            },
            Instruction::End,
        ] if matches!(node1_arguments.as_slice(), [Operand::X(0)])
            && node0_arguments.is_empty()
            && matches!(guard_arguments.as_slice(), [Operand::X(0)])
    ));
    for bif in [&[0x09, 0x20, 0x03][..], &[0x0A, 0x05, 0x20, 0x03, 0x03]] {
        assert!(
            parse_code(
                &code(bif),
                &AtomTable::new(),
                &[],
                &imports,
                &[],
                &[],
                &[],
                &[],
                &mut LineTable::default()
            )
            .unwrap_err()
            .to_string()
            .contains("BEAM BIF import index 2 out of range")
        );
    }
    // `lists:node/0` is not a `bif.tab` entry, so the loader rejects the
    // `bif` instruction of a malformed module with the error that the run
    // gave before the instruction carried an identity.
    let imports = vec![("lists".to_owned(), "node".to_owned(), 0)];
    assert!(
        parse_code(
            &code(&[0x09, 0x00, 0x53]),
            &AtomTable::new(),
            &[],
            &imports,
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .unwrap_err()
        .to_string()
        .contains("unsupported BIF lists:node/0")
    );
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &imports,
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Bif {
                fail_label: 0,
                bif: BifId::ErlangElement2,
                arguments,
                destination: Operand::X(0),
            },
            Instruction::Bif {
                fail_label: 46,
                bif: BifId::ErlangElement2,
                arguments: call_arguments,
                destination: Operand::X(4),
            },
            Instruction::End,
        ] if matches!(
                arguments.as_slice(),
                [Operand::Value(Term::Integer(2)), Operand::X(0)]
            )
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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

/// The loader selects the classified operand form of `test is_eq_exact` when
/// both operands are a register or an immediate and at least one is a
/// register. It keeps the generic form for two immediates and for an operand
/// that it cannot classify, so the run loop keeps its error text.
#[test]
fn selects_the_classified_operand_form_of_is_eq_exact() {
    let atoms = vec![
        Atom::from_static("ok").unwrap(),
        Atom::from_static("output").unwrap(),
    ];
    let parse = |stream: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend_from_slice(stream);
        code.push(0x03);
        parse_code(
            &code,
            &AtomTable::new(),
            &atoms,
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default(),
        )
        .unwrap()
        .0
    };
    // {test,is_eq_exact,{f,3},[{x,0},{integer,0}]}: the floor-loop shape.
    assert!(matches!(
        parse(&[0x2B, 0x35, 0x03, 0x01]).as_slice(),
        [
            Instruction::TestExactEqualOperands {
                fail_label: 3,
                left: TestOperand::X(0),
                right: TestOperand::Value(Term::Integer(0)),
            },
            Instruction::End,
        ]
    ));
    // {test,is_eq_exact,{f,3},[{x,0},{x,1}]}: two registers.
    assert!(matches!(
        parse(&[0x2B, 0x35, 0x03, 0x13]).as_slice(),
        [
            Instruction::TestExactEqualOperands {
                fail_label: 3,
                left: TestOperand::X(0),
                right: TestOperand::X(1),
            },
            Instruction::End,
        ]
    ));
    // {test,is_eq_exact,{f,3},[{y,1},{atom,2}]}: a Y register and an atom.
    assert!(matches!(
        parse(&[0x2B, 0x35, 0x14, 0x22]).as_slice(),
        [
            Instruction::TestExactEqualOperands {
                fail_label: 3,
                left: TestOperand::Y(1),
                right: TestOperand::Value(Term::Atom(_)),
            },
            Instruction::End,
        ]
    ));
    // {test,is_eq_exact,{f,3},[{atom,1},{x,0}]}: the immediate comes first.
    assert!(matches!(
        parse(&[0x2B, 0x35, 0x12, 0x03]).as_slice(),
        [
            Instruction::TestExactEqualOperands {
                fail_label: 3,
                left: TestOperand::Value(Term::Atom(_)),
                right: TestOperand::X(0),
            },
            Instruction::End,
        ]
    ));
    // Two immediates keep the generic form.
    assert!(matches!(
        parse(&[0x2B, 0x35, 0x12, 0x22]).as_slice(),
        [
            Instruction::TestExactEqual {
                fail_label: 3,
                left: Operand::Value(Term::Atom(_)),
                right: Operand::Value(Term::Atom(_)),
            },
            Instruction::End,
        ]
    ));
    // An unsupported literal keeps the generic form.
    assert!(super::classify_test_operand(&Operand::UnsupportedLiteral(0)).is_none());
    // A bignum operand is a value, so the form is classified.
    assert!(matches!(
        parse(&[0x2B, 0x35, 0x03, 0xF9, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0]).as_slice(),
        [
            Instruction::TestExactEqualOperands {
                fail_label: 3,
                left: TestOperand::X(0),
                right: TestOperand::Value(Term::BigInt(_)),
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
fn decodes_is_any_native_record_with_a_register_source_as_the_erts_loader() {
    // `is_any_native_record Fail Src` (genop.tab:722-724) loads only with an
    // x or a y register source (emu `ops.tab:1631`; JIT
    // `jit/arm/ops.tab:1400`). A typed register (`[0x57, 0x03, 0x00]`) is
    // its register.
    let parse = |stream: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend(stream);
        code.push(0x03);
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default(),
        )
    };
    let (instructions, _, _) = parse(&[
        0xBA, 0x55, 0x03, 0xBA, 0x65, 0x14, 0xBA, 0x75, 0x57, 0x23, 0x00,
    ])
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Test {
                kind: TestKind::AnyNativeRecord,
                fail_label: 5,
                value: Operand::X(0),
            },
            Instruction::Test {
                kind: TestKind::AnyNativeRecord,
                fail_label: 6,
                value: Operand::Y(1),
            },
            Instruction::Test {
                kind: TestKind::AnyNativeRecord,
                fail_label: 7,
                value: Operand::X(2),
            },
            Instruction::End,
        ]
    ));
    assert_eq!(
        parse(&[0xBA, 0x55, 0x01]).unwrap_err().to_string(),
        "BEAM is_any_native_record source is not a register"
    );
}

#[test]
fn decodes_the_is_eq_test_of_the_compare_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/compare/compare_probe.beam` has these bytes.
    let is_eq = [0x29, 0x0D, 0x19, 0x03, 0x13]; // eq/2: is_eq f25 x0 x1
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&is_eq[..], &[0x03]].concat());
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
fn decodes_the_is_ne_test_of_the_compare_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/compare/compare_probe.beam` has these bytes.
    let is_ne = [0x2A, 0x0D, 0x1E, 0x03, 0x13]; // ne/2: is_ne f30 x0 x1
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&is_ne[..], &[0x03]].concat());
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::TestNotEqual {
                fail_label: 30,
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
    let table = AtomTable::new();
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
    let (instructions, _, _) = parse_code(
        &code,
        &table,
        &atoms,
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
                    destination: Some(Operand::X(2)),
                },
                Instruction::BinaryMatch {
                    fail_label: 16,
                    context: Operand::X(0),
                    commands: at_least,
                    test_heap: false,
                },
                Instruction::BinarySetPosition {
                    context: Operand::X(0),
                    position: Operand::X(1),
                },
                Instruction::BinaryMatch {
                    fail_label: 17,
                    context: Operand::X(0),
                    commands: exactly,
                    test_heap: false,
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
fn decodes_the_bs_match_commands_of_the_bs_match_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/binary_match/bs_match_probe.beam` has these bytes.
    let parts: [&[u8]; 2] = [
        // tail/1: bs_match f28 x0 {list,[ensure_at_least,8,8,skip,8,
        // get_tail,1,8,{x,0}]}
        &[
            0xB6, 0x0D, 0x1C, 0x03, 0x17, 0x90, 0x92, 0x80, 0x80, 0x0A, 0x11, 0x80, 0xB2, 0x10,
            0x80, 0x03,
        ],
        // prefix/1: bs_match f15 x0 {list,[ensure_at_least,32,1,
        // '=:=',nil,31,850802231,'=:=',nil,1,1]}
        &[
            0xB6, 0xF5, 0x03, 0x17, 0xB0, 0x92, 0x08, 0x20, 0x10, 0xA2, 0x02, 0x08, 0x1F, 0x58,
            0x32, 0xB6, 0x36, 0x37, 0xA2, 0x02, 0x10, 0x10,
        ],
    ];
    // Atom 9 is `ensure_at_least`, 10 is `=:=`, 11 is `get_tail`, and 17
    // is `skip`.
    let table = AtomTable::new();
    let atoms: Vec<Atom> = (1..=17)
        .map(|index| match index {
            9 => table.put("ensure_at_least").unwrap(),
            10 => table.put("=:=").unwrap(),
            11 => table.put("get_tail").unwrap(),
            17 => table.put("skip").unwrap(),
            _ => table.put(&format!("atom{index}")).unwrap(),
        })
        .collect();
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend(parts.concat());
    code.push(0x03);
    let (instructions, _, _) = parse_code(
        &code,
        &table,
        &atoms,
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(
        matches!(
            instructions.as_slice(),
            [
                Instruction::BinaryMatch {
                    fail_label: 28,
                    context: Operand::X(0),
                    commands: tail,
                    test_heap: false,
                },
                Instruction::BinaryMatch {
                    fail_label: 15,
                    context: Operand::X(0),
                    commands: prefix,
                    test_heap: false,
                },
                Instruction::End,
            ] if matches!(
                tail.as_slice(),
                [
                    BinaryMatchCommand::EnsureAtLeast { size: 8, unit },
                    BinaryMatchCommand::Skip { size: 8 },
                    BinaryMatchCommand::GetTail { destination: Operand::X(0) },
                ] if unit.get() == 8
            ) && matches!(
                prefix.as_slice(),
                [
                    BinaryMatchCommand::EnsureAtLeast { size: 32, unit },
                    BinaryMatchCommand::Equal { size: 31, value: 850_802_231 },
                    BinaryMatchCommand::Equal { size: 1, value: 1 },
                ] if unit.get() == 1
            )
        ),
        "{instructions:?}"
    );
}

#[test]
fn decodes_each_operand_of_the_utf16_and_utf32_instructions_of_bs_utf_probe() {
    // `bs_utf_probe` has `bs_get_utf16` and `bs_get_utf32` with the flags 0,
    // 2 (`little`), and 16 (`native`), and `bs_skip_utf16` and
    // `bs_skip_utf32` with the flags 0 and 2 (genop.tab:504-508). A native
    // segment has the byte order of the host (`$NativeEndian`,
    // generators.tab). The fixture has the typed register `x0`.
    let parts: [&[u8]; 10] = [
        &[0x8C, 0x35, 0x57, 0x03, 0x10, 0x10, 0x00, 0x13],
        &[0x8C, 0x65, 0x57, 0x03, 0x10, 0x10, 0x20, 0x13],
        &[0x8C, 0x95, 0x57, 0x03, 0x10, 0x10, 0x08, 0x10, 0x13],
        &[0x8E, 0xC5, 0x57, 0x03, 0x10, 0x10, 0x00, 0x13],
        &[0x8E, 0xF5, 0x57, 0x03, 0x10, 0x10, 0x20, 0x13],
        &[0x8E, 0x0D, 0x12, 0x57, 0x03, 0x10, 0x10, 0x08, 0x10, 0x13],
        &[0x8D, 0x0D, 0x15, 0x57, 0x03, 0x10, 0x10, 0x00],
        &[0x8D, 0x0D, 0x18, 0x57, 0x03, 0x10, 0x10, 0x20],
        &[0x8F, 0x0D, 0x1B, 0x57, 0x03, 0x10, 0x10, 0x00],
        &[0x8F, 0x0D, 0x1E, 0x57, 0x03, 0x10, 0x10, 0x20],
    ];
    let table = AtomTable::new();
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend(parts.concat());
    code.push(0x03);
    let (instructions, _, _) = parse_code(
        &code,
        &table,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    let decoded: Vec<_> = instructions
        .iter()
        .filter(|instruction| !matches!(instruction, Instruction::End))
        .map(|instruction| match instruction {
            Instruction::BinaryGetUtf16 {
                fail_label,
                context: Operand::X(0),
                little,
                destination,
            } => (16, *fail_label, *little, destination),
            Instruction::BinaryGetUtf32 {
                fail_label,
                context: Operand::X(0),
                little,
                destination,
            } => (32, *fail_label, *little, destination),
            other => panic!("{other:?}"),
        })
        // `Operand` has no `PartialEq`: a destination is `Some(true)` for
        // `x1`.
        .map(|(width, label, little, destination)| {
            let x1 = destination
                .as_ref()
                .map(|operand| matches!(operand, Operand::X(1)));
            (width, label, little, x1)
        })
        .collect();
    let native = cfg!(target_endian = "little");
    assert_eq!(
        decoded,
        [
            (16, 3, false, Some(true)),
            (16, 6, true, Some(true)),
            (16, 9, native, Some(true)),
            (32, 12, false, Some(true)),
            (32, 15, true, Some(true)),
            (32, 18, native, Some(true)),
            (16, 21, false, None),
            (16, 24, true, None),
            (32, 27, false, None),
            (32, 30, true, None),
        ]
    );
}

#[test]
fn decodes_each_operand_of_the_bs_get_float2_instructions_of_bs_float_probe() {
    // `bs_float_probe:big/2`, `little/2`, `native/2`, `units/2`, and
    // `half/1` have `bs_get_float2` with the flags 0, 2 (`little`), and 16
    // (`native`), the unit 16, and the literal size 16. The fixture has the
    // typed register `x0`; the plain `x0` decodes the same way. A native
    // segment has the byte order of the host (`$NativeEndian`,
    // generators.tab:57-71).
    let parts: [&[u8]; 5] = [
        &[0x76, 0x35, 0x03, 0x20, 0x13, 0x10, 0x00, 0x13],
        &[0x76, 0x65, 0x03, 0x20, 0x13, 0x10, 0x20, 0x13],
        &[0x76, 0x95, 0x03, 0x20, 0x13, 0x10, 0x08, 0x10, 0x13],
        &[0x76, 0xC5, 0x03, 0x20, 0x13, 0x08, 0x10, 0x00, 0x13],
        &[0x76, 0xF5, 0x03, 0x10, 0x09, 0x10, 0x10, 0x00, 0x13],
    ];
    let table = AtomTable::new();
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend(parts.concat());
    code.push(0x03);
    let (instructions, _, _) = parse_code(
        &code,
        &table,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    let floats: Vec<_> = instructions
        .iter()
        .filter(|instruction| !matches!(instruction, Instruction::End))
        .map(|instruction| match instruction {
            Instruction::BinaryGetFloat {
                fail_label,
                context: Operand::X(0),
                size,
                unit,
                little,
                destination: Operand::X(1),
            } => (*fail_label, size.clone(), *unit, *little),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(floats.len(), 5);
    let sizes: Vec<_> = floats
        .iter()
        .map(|(label, size, unit, little)| (*label, matches!(size, Operand::X(1)), *unit, *little))
        .collect();
    assert_eq!(
        sizes,
        [
            (3, true, 1, false),
            (6, true, 1, true),
            (9, true, 1, cfg!(target_endian = "little")),
            (12, true, 16, false),
            (15, false, 1, false),
        ]
    );
    assert!(matches!(&floats[4].1, Operand::Value(Term::Integer(16))));
    // A literal size that is not a small integer, such as 2^64, stays a
    // `bs_get_float2` operand, as ERTS loads it (jit/arm/ops.tab:1009-1010);
    // the run fails it.
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([
        0x76, 0x35, 0x03, 0x20, 0xF9, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    code.extend([0x10, 0x00, 0x13, 0x03]);
    let (instructions, _, _) = parse_code(
        &code,
        &table,
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(
        matches!(
            instructions.as_slice(),
            [
                Instruction::BinaryGetFloat {
                    fail_label: 3,
                    size: Operand::Value(Term::BigInt(size)),
                    unit: 1,
                    little: false,
                    ..
                },
                Instruction::End
            ] if size.to_string() == "18446744073709551616"
        ),
        "{instructions:?}"
    );
}

#[test]
fn decodes_the_bs_start_match4_bs_get_binary2_and_bs_get_tail_instructions_of_bs_get_probe() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/binary_match/bs_get_probe.beam` has the first four parts.
    let parts: [&[u8]; 6] = [
        // split/2: bs_start_match4 no_fail 2 x0 x0
        &[0xAA, 0x32, 0x20, 0x03, 0x03],
        // split/2: bs_get_binary2 f3 {tr,x0,1} 2 x1 8 0 x1
        &[0x77, 0x35, 0x57, 0x03, 0x10, 0x20, 0x13, 0x80, 0x00, 0x13],
        // split/2: bs_get_tail x0 x0 2
        &[0xA5, 0x03, 0x03, 0x20],
        // nibbles/2: bs_start_match4 resume 2 x0 x0
        &[0xAA, 0xB2, 0x20, 0x03, 0x03],
        // bs_start_match4 f5 2 x0 x1, which the ERTS loader also accepts
        &[0xAA, 0x55, 0x20, 0x03, 0x13],
        // bs_get_binary2 f3 x0 2 all 8 0 x1
        &[0x77, 0x35, 0x03, 0x20, 0x42, 0x80, 0x00, 0x13],
    ];
    // Atom 3 is `no_fail`, 4 is `all`, and 11 is `resume`.
    let table = AtomTable::new();
    let atoms: Vec<Atom> = (1..=11)
        .map(|index| match index {
            3 => table.put("no_fail").unwrap(),
            4 => table.put("all").unwrap(),
            11 => table.put("resume").unwrap(),
            _ => table.put(&format!("atom{index}")).unwrap(),
        })
        .collect();
    let code = |parts: &[&[u8]]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend(parts.concat());
        code.push(0x03);
        code
    };
    let parse = |parts: &[&[u8]]| {
        parse_code(
            &code(parts),
            &table,
            &atoms,
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default(),
        )
    };
    let (instructions, _, _) = parse(&parts).unwrap();
    assert!(
        matches!(
            instructions.as_slice(),
            [
                Instruction::BinaryMatchStart {
                    fail_label: 0,
                    source: Operand::X(0),
                    destination: Operand::X(0),
                },
                Instruction::BinaryGetBinary {
                    fail_label: 3,
                    context: Operand::X(0),
                    size: Operand::X(1),
                    unit: 8,
                    destination: Operand::X(1),
                },
                Instruction::BinaryGetTail {
                    context: Operand::X(0),
                    destination: Operand::X(0),
                },
                Instruction::BinaryMatchResume {
                    source: Operand::X(0),
                    destination: Operand::X(0),
                },
                Instruction::BinaryMatchStart {
                    fail_label: 5,
                    source: Operand::X(0),
                    destination: Operand::X(1),
                },
                Instruction::BinaryMatch {
                    fail_label: 3,
                    context: Operand::X(0),
                    commands: all,
                    test_heap: false,
                },
                Instruction::End,
            ] if matches!(
                all.as_slice(),
                [
                    BinaryMatchCommand::EnsureAtLeast { size: 0, unit },
                    BinaryMatchCommand::GetTail { destination: Operand::X(1) },
                ] if unit.get() == 8
            )
        ),
        "{instructions:?}"
    );
    // A size `all` with unit 0 has no `ensure_at_least` command.
    assert_eq!(
        parse(&[&[0x77, 0x35, 0x03, 0x20, 0x42, 0x00, 0x00, 0x13]])
            .unwrap_err()
            .to_string(),
        "BEAM bs_get_binary2 has unit 0"
    );
    // The ERTS loader has no form of `bs_start_match4` with another atom
    // (jit/arm/ops.tab:1036-1052).
    assert_eq!(
        parse(&[&[0xAA, 0x12, 0x20, 0x03, 0x03]])
            .unwrap_err()
            .to_string(),
        "unsupported BEAM bs_start_match4 fail operand atom1"
    );
}

#[test]
fn reads_each_bs_match_command_and_rejects_bad_command_lists() {
    let table = AtomTable::new();
    let names = [
        "ensure_at_least",
        "ensure_exactly",
        "binary",
        "integer",
        "get_tail",
        "=:=",
        "skip",
        "other",
        "little",
        "signed",
    ];
    let atoms: Vec<Atom> = names.iter().map(|name| table.put(name).unwrap()).collect();
    let literals = vec![Some(Term::list_from(vec![
        Term::Atom(atoms[8].clone()),
        Term::Atom(atoms[9].clone()),
    ]))];
    // {list,[binary,1,nil,2,8,{x,1}, integer,1,{literal,0},12,1,{x,2},
    // get_tail,1,8,{x,3}, '=:=',nil,8,97, skip,8]}: 22 items. Literal 0
    // is [little,signed].
    let list = [
        0x17, 0x08, 0x16, 0x32, 0x10, 0x02, 0x20, 0x80, 0x23, 0x42, 0x10, 0x47, 0x00, 0xC0, 0x10,
        0x23, 0x52, 0x10, 0x80, 0x33, 0x62, 0x02, 0x80, 0x08, 0x61, 0x72, 0x80,
    ];
    let mut cursor = 0;
    let commands =
        read_binary_match_commands(&list, &mut cursor, &table, &atoms, &literals).unwrap();
    assert_eq!(cursor, list.len());
    assert!(
        matches!(
            commands.as_slice(),
            [
                BinaryMatchCommand::Binary {
                    size: 16,
                    destination: Operand::X(2),
                },
                BinaryMatchCommand::Integer {
                    size: 12,
                    little: true,
                    signed: true,
                    destination: Operand::X(2),
                },
                BinaryMatchCommand::GetTail {
                    destination: Operand::X(3),
                },
                BinaryMatchCommand::Equal { size: 8, value: 97 },
                BinaryMatchCommand::Skip { size: 8 },
            ]
        ),
        "{commands:?}"
    );
    // {list,[integer,1,nil,8,1,{x,1}]}: nil flags read as unsigned
    // big-endian.
    let commands = read_binary_match_commands(
        &[0x17, 0x60, 0x42, 0x10, 0x02, 0x80, 0x10, 0x13],
        &mut 0,
        &table,
        &atoms,
        &literals,
    )
    .unwrap();
    assert!(
        matches!(
            commands.as_slice(),
            [BinaryMatchCommand::Integer {
                size: 8,
                little: false,
                signed: false,
                destination: Operand::X(1),
            }]
        ),
        "{commands:?}"
    );
    let failure = |list: &[u8]| {
        read_binary_match_commands(list, &mut 0, &table, &atoms, &literals)
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
    // {integer,1,{x,1},8,1,{x,1}}: the flags are a register.
    assert_eq!(
        failure(&[0x17, 0x60, 0x42, 0x10, 0x13, 0x80, 0x10, 0x13]),
        "unsupported BEAM bs_match integer flags X(1)"
    );
    // {binary,1,nil,2^62,4,{x,1}}: the size in bits is 2^64.
    assert_eq!(
        failure(&[
            0x17, 0x60, 0x32, 0x10, 0x02, 0xD8, 0x40, 0, 0, 0, 0, 0, 0, 0, 0x40, 0x13
        ]),
        "BEAM bs_match binary size overflow"
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::FloatConvert {
                source: Operand::X(0),
                register: 0,
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
fn decodes_the_is_number_test_of_the_type_test_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/type_test/type_test_probe.beam` has these bytes.
    let is_number = [0x2F, 0x0D, 0x1B, 0x03]; // number/1: is_number f27 x0
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&is_number[..], &[0x03]].concat());
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Test {
                kind: TestKind::Number,
                fail_label: 27,
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
    let table = AtomTable::new();
    let atoms: Vec<Atom> = (1..=17)
        .map(|index| format!("atom{index}"))
        .chain(["output".into(), "cooked".into()])
        .map(|name| table.put(&name).unwrap())
        .collect();
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&put_map_exact[..], &[0x03]].concat());
    let (instructions, _, _) = parse_code(
        &code,
        &table,
        &atoms,
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
                if table.name_of(key) == "output" && table.name_of(value) == "cooked"
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
    let table = AtomTable::new();
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
    let (instructions, _, _) = parse_code(
        &code,
        &table,
        &atoms,
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
                if first.is_static("input") && second.is_static("output")
        ) && matches!(register_keys.as_slice(), [Operand::X(0)])
    ));
}

#[test]
fn sorts_get_map_elements_keys_in_flatmap_key_order_like_the_erts_loader() {
    let atoms = AtomTable::new();
    let destinations = |pairs: Vec<(Operand, Operand)>| {
        sort_map_key_pairs(pairs, &atoms).map(|pairs| {
            pairs
                .into_iter()
                .map(|(_, destination)| match destination {
                    Operand::X(index) => index,
                    _ => panic!("each destination is an X register"),
                })
                .collect::<Vec<_>>()
        })
    };
    let key = Operand::Value;
    // pick/1 of `fixtures/map/map_elements_probe.erl`:
    // {list,[nil,{x,0},{atom,true},{x,3},{atom,abs_path},{x,2},{integer,1},{x,1}]}.
    // An integer comes before an atom, an atom before `[]`, and `true` has a
    // lower atom index than `abs_path`.
    assert_eq!(
        destinations(vec![
            (key(Term::nil()), Operand::X(0)),
            (key(Term::static_atom("true")), Operand::X(3)),
            (key(Term::static_atom("abs_path")), Operand::X(2)),
            (key(Term::Integer(1)), Operand::X(1)),
        ])
        .unwrap(),
        [1, 3, 2, 0]
    );
    // One pair can have a register key.
    assert_eq!(
        destinations(vec![(Operand::X(1), Operand::X(0))]).unwrap(),
        [0]
    );
    // A literal key that PBeam does not support yet comes first.
    assert_eq!(
        destinations(vec![
            (key(Term::Integer(1)), Operand::X(0)),
            (Operand::UnsupportedLiteral(0), Operand::X(1)),
        ])
        .unwrap(),
        [1, 0]
    );
    // ERTS loads no register key among other keys.
    assert!(
        destinations(vec![
            (key(Term::Integer(1)), Operand::X(0)),
            (Operand::Y(0), Operand::X(1)),
        ])
        .unwrap_err()
        .to_string()
        .contains("a register key among other keys")
    );
    // {get_map_elements,{f,3},{x,0},{list,[{x,1},{x,0},{integer,2},{x,1}]}}
    // does not load.
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([
        OPCODE_GET_MAP_ELEMENTS,
        0x35,
        0x03,
        0x17,
        0x40,
        0x13,
        0x03,
        0x21,
        0x13,
        0x03,
    ]);
    assert!(
        parse_code(
            &code,
            &atoms,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .unwrap_err()
        .to_string()
        .contains("a register key among other keys")
    );
}

#[test]
fn rejects_malformed_has_map_fields_key_lists() {
    let error = |operands: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend([&[OPCODE_HAS_MAP_FIELDS, 0x35, 0x03], operands].concat());
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default(),
        )
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
            Instruction::RawRaise,
            Instruction::End,
        ] if matches!(copy.as_slice(), [(5, Operand::Y(2))])
            && matches!(
                inplace.as_slice(),
                [(4, Operand::X(3)), (5, Operand::Value(nil))] if nil.is_nil()
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
    assert!(
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &imports,
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Bif {
                fail_label: 58,
                bif: BifId::ErlangPlus2,
                arguments: plus_arguments,
                destination: Operand::X(0),
            },
            Instruction::Bif {
                fail_label: 0,
                bif: BifId::ErlangMinus2,
                arguments: minus_arguments,
                destination: Operand::X(0),
            },
            Instruction::Bif {
                fail_label: 55,
                bif: BifId::ErlangMinus1,
                arguments: negate_arguments,
                destination: Operand::X(0),
            },
            Instruction::End,
        ] if matches!(
                plus_arguments.as_slice(),
                [Operand::X(0), Operand::Value(Term::Integer(1))]
            )
            && matches!(minus_arguments.as_slice(), [Operand::Y(0), Operand::X(0)])
            && matches!(negate_arguments.as_slice(), [Operand::X(0)])
    ));
    // Without its destination and the end, the instruction is truncated.
    code.truncate(code.len() - 2);
    assert!(
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &imports,
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
}

#[test]
fn decodes_gc_bif3_of_the_binary_part_probe_fixture_into_a_bif_instruction() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/binary_part/binary_part_probe.beam` has these bytes.
    // guard/3: gc_bif3 f13 live 3 binary_part/3 x0 x1 x2 x0
    let guard = [0x98, 0xD5, 0x30, 0x00, 0x03, 0x13, 0x23, 0x03];
    // main/0: gc_bif3 f0 live 1 binary_part/3 y23 y22 x0 y22
    let body = [
        0x98, 0x05, 0x10, 0x00, 0x0C, 0x17, 0x0C, 0x16, 0x03, 0x0C, 0x16,
    ];
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&guard[..], &body, &[0x03]].concat());
    let imports = vec![("erlang".to_owned(), "binary_part".to_owned(), 3)];
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &imports,
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Bif {
                fail_label: 13,
                bif: BifId::ErlangBinaryPart3,
                arguments: guard_arguments,
                destination: Operand::X(0),
            },
            Instruction::Bif {
                fail_label: 0,
                bif: BifId::ErlangBinaryPart3,
                arguments: body_arguments,
                destination: Operand::Y(22),
            },
            Instruction::End,
        ] if matches!(
                guard_arguments.as_slice(),
                [Operand::X(0), Operand::X(1), Operand::X(2)]
            )
            && matches!(
                body_arguments.as_slice(),
                [Operand::Y(23), Operand::Y(22), Operand::X(0)]
            )
    ));
    // Without its destination and the end, the instruction is truncated.
    code.truncate(code.len() - 3);
    assert!(
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &imports,
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
}

#[test]
fn decodes_bif3_of_the_is_integer_probe_fixture_into_a_bif_instruction() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/is_integer/is_integer_probe.beam` has these bytes.
    // guard/3: bif3 f12 is_integer/3 x0 x1 x2 x0
    let guard = [0xB9, 0xC5, 0x10, 0x03, 0x13, 0x23, 0x03];
    // body/3: bif3 f0 is_integer/3 y0 y2 x0 x0
    let body = [0xB9, 0x05, 0x10, 0x04, 0x24, 0x03, 0x03];
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend([&guard[..], &body, &[0x03]].concat());
    // The instructions name import 1, as in the fixture.
    let imports = vec![
        ("erlang".to_owned(), "binary_part".to_owned(), 3),
        ("erlang".to_owned(), "is_integer".to_owned(), 3),
    ];
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &imports,
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::Bif {
                fail_label: 12,
                bif: BifId::ErlangIsInteger3,
                arguments: guard_arguments,
                destination: Operand::X(0),
            },
            Instruction::Bif {
                fail_label: 0,
                bif: BifId::ErlangIsInteger3,
                arguments: body_arguments,
                destination: Operand::X(0),
            },
            Instruction::End,
        ] if matches!(
                guard_arguments.as_slice(),
                [Operand::X(0), Operand::X(1), Operand::X(2)]
            )
            && matches!(
                body_arguments.as_slice(),
                [Operand::Y(0), Operand::Y(2), Operand::X(0)]
            )
    ));
    // ERTS loads the destination of `i_bif3` only as an X or Y register
    // (`d` in jit/arm/ops.tab:836), so the integer 0 as destination fails.
    let mut literal_destination = code.clone();
    literal_destination[20 + 6] = 0x01;
    assert!(
        parse_code(
            &literal_destination,
            &AtomTable::new(),
            &[],
            &imports,
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
    // Without its destination and the end, the instruction is truncated.
    code.truncate(code.len() - 2);
    assert!(
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &imports,
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
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
    let lambda =
        |index, label, arity| Lambda::new(module.clone(), index, index, 31_263_486, label, arity);
    // The first five `FunT` entries of the fixture, with their free
    // variable counts.
    let lambdas = vec![
        (std::sync::Arc::new(lambda(0, 127, 1)), 1),
        (std::sync::Arc::new(lambda(1, 125, 0)), 0),
        (std::sync::Arc::new(lambda(2, 123, 1)), 1),
        (std::sync::Arc::new(lambda(3, 121, 0)), 0),
        (std::sync::Arc::new(lambda(4, 112, 1)), 1),
    ];
    let parse = |instructions: &[u8]| {
        parse_code(
            &code(instructions),
            &AtomTable::new(),
            &atoms,
            &[],
            &[],
            &lambdas,
            &[],
            &[],
            &mut LineTable::default(),
        )
    };
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/funs/fun_probe.beam` has these bytes.
    let make_pair = [0xAB, 0x40, 0x03, 0x17, 0x10, 0x03]; // make_pair/1: make_fun3 4 x0 [x0]
    let make_const = [0xAB, 0x10, 0xA4, 0x17, 0x00]; // locals/0: make_fun3 1 y10 []
    let call_fun = [0x4B, 0x00]; // locals/0: call_fun 0
    let tail_call = [0xB2, 0x00, 0x10, 0x57, 0x13, 0x20]; // tail_call/1: call_fun2 0 1 x1
    // The fixture bytes, then a `call_fun2` with the atom tag `safe` and
    // the fun in `y0`.
    let (instructions, _, _) = parse(
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
        ] if **first == *lambdas[4].0
            && matches!(free.as_slice(), [Operand::X(0)])
            && **second == *lambdas[1].0
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &imports,
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default(),
        )
        .unwrap()
        .0
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
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &imports,
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default(),
        )
        .unwrap()
        .0
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
    let table = AtomTable::new();
    let atoms = ["preempt_probe", "body"].map(|name| table.put(name).unwrap());
    let (instructions, _, _) = parse_code(
        &code,
        &table,
        &atoms,
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &imports,
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default(),
        )
        .unwrap()
        .0
    };
    let whereis = |instruction: &Instruction, tail: bool, size: u32| {
        matches!(
            instruction,
            Instruction::CallExternal {
                import: 0,
                tail: found,
                deallocate
            } if *found == tail && *deallocate == size
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
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
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
    assert!(
        parse_code(
            &code,
            &AtomTable::new(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default()
        )
        .is_err()
    );
}

#[test]
fn maps_supported_type_tests_and_rejects_unknown_opcodes() {
    assert!(test_kind(OPCODE_TEST_IS_ATOM).is_ok());
    assert!(test_kind(OPCODE_TEST_IS_PID).is_ok());
    assert!(matches!(test_kind(OPCODE_TEST_IS_PORT), Ok(TestKind::Port)));
    assert_eq!(unsupported_opcode_operand_count(OPCODE_TEST_IS_PORT), None);
    assert!(matches!(
        test_kind(OPCODE_TEST_IS_NUMBER),
        Ok(TestKind::Number)
    ));
    assert_eq!(
        unsupported_opcode_operand_count(OPCODE_TEST_IS_NUMBER),
        None
    );
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
        (OPCODE_TEST_EXACT_EQ, 3),
        (OPCODE_TEST_TAGGED_TUPLE, 4),
        (OPCODE_TEST_IS_MAP, 2),
        (OPCODE_PUT_MAP_ASSOC, 5),
        (OPCODE_PUT_TUPLE2, 2),
        (OPCODE_INIT_YREGS, 1),
    ] {
        assert_eq!(unsupported_opcode_operand_count(opcode), Some(count));
    }
    assert_eq!(unsupported_opcode_operand_count(255), None);

    let mut cursor = 0;
    assert!(skip_unsupported_instruction(255, &[], &mut cursor).is_err());
    assert!(skip_unsupported_instruction(OPCODE_TEST_HEAP, &[0], &mut cursor).is_err());
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
    let atoms: Vec<Atom> = ["string", "binary", "all", "integer", "append"]
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
            BinarySegment::BinaryAll {
                segment: 2,
                unit: 8,
                source: Operand::X(0)
            },
        ] if text == b"count: "
    ));
    // {atom,binary} of unit 3 and size all; {atom,binary} of unit 8 and
    // size 8.
    assert!(matches!(
        read(&[0x17, 0x60, 0x22, 0x20, 0x30, 0x02, 0x03, 0x32], b"")
            .unwrap()
            .as_slice(),
        [BinarySegment::BinaryAll {
            segment: 2,
            unit: 3,
            source: Operand::X(0)
        }]
    ));
    assert!(matches!(
        read(&[0x17, 0x60, 0x22, 0x20, 0x80, 0x02, 0x03, 0x81], b"")
            .unwrap()
            .as_slice(),
        [BinarySegment::Binary {
            segment: 2,
            unit: 8,
            source: Operand::X(0),
            size: Operand::Value(Term::Integer(8)),
        }]
    ));
    // {atom,append},1,16,nil,{x,0},{atom,all} as the first segment.
    let segments = read(
        &[
            0x17, 0xC0, 0x52, 0x10, 0x08, 0x10, 0x02, 0x03, 0x32, 0x22, 0x20, 0x80, 0x02, 0x13,
            0x32,
        ],
        b"",
    )
    .unwrap();
    assert!(matches!(
        segments.as_slice(),
        [
            BinarySegment::Append {
                segment: 1,
                unit: 16,
                source: Operand::X(0)
            },
            BinarySegment::BinaryAll {
                segment: 2,
                unit: 8,
                source: Operand::X(1)
            },
        ]
    ));
    // An append segment after another segment, and one with size 8.
    for bytes in [
        [
            0x17, 0xC0, 0x22, 0x10, 0x80, 0x02, 0x03, 0x32, 0x52, 0x20, 0x80, 0x02, 0x13, 0x32,
        ]
        .as_slice(),
        [0x17, 0x60, 0x52, 0x10, 0x80, 0x02, 0x03, 0x81].as_slice(),
    ] {
        let segments = read(bytes, b"").unwrap();
        assert!(matches!(
            segments.last(),
            Some(BinarySegment::Unsupported(kind)) if kind.is_static("append")
        ));
    }
}

#[test]
fn reads_bs_create_bin_integer_and_utf_segments_and_rejects_short_lists() {
    let atoms: Vec<Atom> = ["string", "binary", "all", "integer", "append"]
        .into_iter()
        .map(|name| Atom::from_static(name).unwrap())
        .collect();
    let read =
        |bytes: &[u8], strings: &[u8]| read_binary_segments(bytes, &mut 0, &atoms, &[], strings);
    // {atom,integer},1,1,nil,{x,0},{integer,8},
    // {atom,integer},2,8,{literal,[little]},{x,1},{x,2},
    // {atom,integer},3,1,{literal,[signed,native]},{x,0},{integer,16},
    // {atom,utf8},4,0,nil,{x,3},{atom,undefined}, and
    // {atom,utf16},5,0,nil,{x,0},{atom,undefined}.
    let atoms: Vec<Atom> = ["integer", "utf8", "undefined", "utf16"]
        .into_iter()
        .map(|name| Atom::from_static(name).unwrap())
        .collect();
    let literals = [
        Some(Term::list_from(vec![Term::static_atom("little")])),
        Some(Term::list_from(vec![
            Term::static_atom("signed"),
            Term::static_atom("native"),
        ])),
    ];
    let segments = read_binary_segments(
        &[
            0x17, 0x08, 0x1E, 0x12, 0x10, 0x10, 0x02, 0x03, 0x81, 0x12, 0x20, 0x80, 0x47, 0x00,
            0x13, 0x23, 0x12, 0x30, 0x10, 0x47, 0x10, 0x03, 0x09, 0x10, 0x22, 0x40, 0x00, 0x02,
            0x33, 0x32, 0x42, 0x50, 0x00, 0x02, 0x03, 0x32,
        ],
        &mut 0,
        &atoms,
        &literals,
        b"",
    )
    .unwrap();
    assert!(matches!(
        segments.as_slice(),
        [
            BinarySegment::Integer {
                segment: 1,
                unit: 1,
                little: false,
                source: Operand::X(0),
                size: Operand::Value(Term::Integer(8)),
            },
            BinarySegment::Integer {
                segment: 2,
                unit: 8,
                little: true,
                source: Operand::X(1),
                size: Operand::X(2),
            },
            BinarySegment::Integer {
                segment: 3,
                unit: 1,
                little: true,
                source: Operand::X(0),
                size: Operand::Value(Term::Integer(16)),
            },
            BinarySegment::Utf8 {
                segment: 4,
                source: Operand::X(3),
            },
            BinarySegment::Utf16 {
                segment: 5,
                little: false,
                source: Operand::X(0),
            },
        ]
    ));
    // A list of one item, and a string of 7 bytes in a StrT chunk of 5.
    assert!(read(&[0x17, 0x10, 0x12], b"").is_err());
    assert!(read(&[0x17, 0x60, 0x12, 0x00, 0x80, 0x02, 0x00, 0x71], b"short").is_err());
}

/// Return the bytes of the fields of the arm of `instruction`.
///
/// Each pattern names every field of its variant, and no pattern uses `..`.
/// Thus the compiler refuses this function when a field is added, removed, or
/// renamed, and the layout table of [`records_the_decoded_form_layout`]
/// cannot go stale without a compile error.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(arm_field_bytes(&Instruction::Label(1)), size_of::<u32>());
/// ```
#[expect(
    clippy::match_same_arms,
    reason = "each variant has its own arm that names every field, so a changed field is a compile error; merged arms would hide that"
)]
#[expect(
    clippy::too_many_lines,
    reason = "table: one arm for each instruction, and each arm sums the sizes of its fields"
)]
fn arm_field_bytes(instruction: &Instruction) -> usize {
    use std::mem::size_of_val;
    match instruction {
        Instruction::Label(field_0) => size_of_val(field_0),
        Instruction::FunctionInfo {
            module,
            name,
            arity,
        } => size_of_val(module) + size_of_val(name) + size_of_val(arity),
        Instruction::Move(field_0, field_1) => size_of_val(field_0) + size_of_val(field_1),
        Instruction::Call {
            arity,
            function,
            target,
        } => size_of_val(arity) + size_of_val(function) + size_of_val(target),
        Instruction::CallLast {
            arity,
            function,
            target,
            deallocate,
        } => {
            size_of_val(arity)
                + size_of_val(function)
                + size_of_val(target)
                + size_of_val(deallocate)
        }
        Instruction::CallOnly {
            arity,
            function,
            target,
        } => size_of_val(arity) + size_of_val(function) + size_of_val(target),
        Instruction::CallExternal {
            import,
            tail,
            deallocate,
        } => size_of_val(import) + size_of_val(tail) + size_of_val(deallocate),
        Instruction::Hibernate => 0,
        Instruction::Yield => 0,
        Instruction::IsNativeRecord {
            fail_label,
            value,
            module,
            name,
        } => size_of_val(fail_label) + size_of_val(value) + size_of_val(module) + size_of_val(name),
        Instruction::IsRecordAccessible {
            fail_label,
            value,
            external,
        } => size_of_val(fail_label) + size_of_val(value) + size_of_val(external),
        Instruction::GetRecordElements {
            fail_label,
            value,
            pairs,
        } => size_of_val(fail_label) + size_of_val(value) + size_of_val(pairs),
        Instruction::PutRecord {
            kind,
            destination,
            updates,
        } => size_of_val(kind) + size_of_val(destination) + size_of_val(updates),
        Instruction::GetRecordField {
            fail_label,
            value,
            id,
            field,
            destination,
        } => {
            size_of_val(fail_label)
                + size_of_val(value)
                + size_of_val(id)
                + size_of_val(field)
                + size_of_val(destination)
        }
        Instruction::Apply {
            arity,
            tail,
            deallocate,
        } => size_of_val(arity) + size_of_val(tail) + size_of_val(deallocate),
        Instruction::Bif {
            fail_label,
            bif,
            arguments,
            destination,
        } => {
            size_of_val(fail_label)
                + size_of_val(bif)
                + size_of_val(arguments)
                + size_of_val(destination)
        }
        Instruction::MakeFun {
            lambda,
            free,
            destination,
        } => size_of_val(lambda) + size_of_val(free) + size_of_val(destination),
        Instruction::CallFun {
            arity,
            function,
            tail,
            deallocate,
        } => {
            size_of_val(arity) + size_of_val(function) + size_of_val(tail) + size_of_val(deallocate)
        }
        Instruction::Catch { register, label } => size_of_val(register) + size_of_val(label),
        Instruction::TryEnd { register } => size_of_val(register),
        Instruction::TryCase { register } => size_of_val(register),
        Instruction::CatchEnd { register } => size_of_val(register),
        Instruction::Raise { trace, value } => size_of_val(trace) + size_of_val(value),
        // `tag` is a reference field, so `size_of_val(tag)` measures the
        // reference. Name the field and write the size of its type.
        Instruction::TaggedError { tag: _, value } => {
            std::mem::size_of::<&'static str>() + size_of_val(value)
        }
        Instruction::IfEnd => 0,
        Instruction::RawRaise => 0,
        Instruction::BuildStacktrace => 0,
        Instruction::Allocate { stack_size } => size_of_val(stack_size),
        Instruction::Deallocate { stack_size } => size_of_val(stack_size),
        Instruction::Trim { count, remaining } => size_of_val(count) + size_of_val(remaining),
        Instruction::Jump(field_0) => size_of_val(field_0),
        Instruction::Test {
            kind,
            fail_label,
            value,
        } => size_of_val(kind) + size_of_val(fail_label) + size_of_val(value),
        Instruction::TestFunctionArity {
            fail_label,
            value,
            arity,
        } => size_of_val(fail_label) + size_of_val(value) + size_of_val(arity),
        Instruction::TestArity {
            fail_label,
            value,
            arity,
        } => size_of_val(fail_label) + size_of_val(value) + size_of_val(arity),
        Instruction::SelectTupleArity {
            value,
            fail_label,
            choices,
        } => size_of_val(value) + size_of_val(fail_label) + size_of_val(choices),
        Instruction::SelectValue {
            value,
            fail_label,
            choices,
        } => size_of_val(value) + size_of_val(fail_label) + size_of_val(choices),
        Instruction::GetMapElements {
            fail_label,
            map,
            pairs,
        } => size_of_val(fail_label) + size_of_val(map) + size_of_val(pairs),
        Instruction::HasMapFields {
            fail_label,
            map,
            keys,
        } => size_of_val(fail_label) + size_of_val(map) + size_of_val(keys),
        Instruction::PutMapAssoc {
            map,
            destination,
            pairs,
        } => size_of_val(map) + size_of_val(destination) + size_of_val(pairs),
        Instruction::PutMapExact {
            fail_label,
            map,
            destination,
            pairs,
        } => {
            size_of_val(fail_label)
                + size_of_val(map)
                + size_of_val(destination)
                + size_of_val(pairs)
        }
        Instruction::Swap(field_0, field_1) => size_of_val(field_0) + size_of_val(field_1),
        Instruction::PutList {
            head,
            tail,
            destination,
        } => size_of_val(head) + size_of_val(tail) + size_of_val(destination),
        Instruction::PutTuple {
            elements,
            destination,
        } => size_of_val(elements) + size_of_val(destination),
        Instruction::UpdateRecord {
            source,
            destination,
            updates,
        } => size_of_val(source) + size_of_val(destination) + size_of_val(updates),
        Instruction::GetList { source, head, tail } => {
            size_of_val(source) + size_of_val(head) + size_of_val(tail)
        }
        Instruction::GetHead {
            source,
            destination,
        } => size_of_val(source) + size_of_val(destination),
        Instruction::GetTail {
            source,
            destination,
        } => size_of_val(source) + size_of_val(destination),
        Instruction::GetTupleElement {
            tuple,
            index,
            destination,
        } => size_of_val(tuple) + size_of_val(index) + size_of_val(destination),
        Instruction::TestExactEqual {
            fail_label,
            left,
            right,
        } => size_of_val(fail_label) + size_of_val(left) + size_of_val(right),
        Instruction::TestExactEqualOperands {
            fail_label,
            left,
            right,
        } => size_of_val(fail_label) + size_of_val(left) + size_of_val(right),
        Instruction::TestEqual {
            fail_label,
            left,
            right,
        } => size_of_val(fail_label) + size_of_val(left) + size_of_val(right),
        Instruction::TestNotEqual {
            fail_label,
            left,
            right,
        } => size_of_val(fail_label) + size_of_val(left) + size_of_val(right),
        Instruction::TestExactNotEqual {
            fail_label,
            left,
            right,
        } => size_of_val(fail_label) + size_of_val(left) + size_of_val(right),
        Instruction::TestLess {
            fail_label,
            left,
            right,
        } => size_of_val(fail_label) + size_of_val(left) + size_of_val(right),
        Instruction::TestGreaterEqual {
            fail_label,
            left,
            right,
        } => size_of_val(fail_label) + size_of_val(left) + size_of_val(right),
        Instruction::TestTaggedTuple {
            fail_label,
            value,
            arity,
            tag,
        } => size_of_val(fail_label) + size_of_val(value) + size_of_val(arity) + size_of_val(tag),
        Instruction::BinaryMatchStart {
            fail_label,
            source,
            destination,
        } => size_of_val(fail_label) + size_of_val(source) + size_of_val(destination),
        Instruction::BinaryMatchResume {
            source,
            destination,
        } => size_of_val(source) + size_of_val(destination),
        Instruction::BinaryGetTail {
            context,
            destination,
        } => size_of_val(context) + size_of_val(destination),
        Instruction::BinaryGetBinary {
            fail_label,
            context,
            size,
            unit,
            destination,
        } => {
            size_of_val(fail_label)
                + size_of_val(context)
                + size_of_val(size)
                + size_of_val(unit)
                + size_of_val(destination)
        }
        Instruction::BinaryGetFloat {
            fail_label,
            context,
            size,
            unit,
            little,
            destination,
        } => {
            size_of_val(fail_label)
                + size_of_val(context)
                + size_of_val(size)
                + size_of_val(unit)
                + size_of_val(little)
                + size_of_val(destination)
        }
        Instruction::BinaryGetInteger {
            fail_label,
            context,
            size,
            unit,
            little,
            signed,
            destination,
        } => {
            size_of_val(fail_label)
                + size_of_val(context)
                + size_of_val(size)
                + size_of_val(unit)
                + size_of_val(little)
                + size_of_val(signed)
                + size_of_val(destination)
        }
        Instruction::BinaryGetPosition {
            context,
            destination,
        } => size_of_val(context) + size_of_val(destination),
        Instruction::BinarySetPosition { context, position } => {
            size_of_val(context) + size_of_val(position)
        }
        Instruction::BinarySkipBits {
            fail_label,
            context,
            size,
            unit,
        } => size_of_val(fail_label) + size_of_val(context) + size_of_val(size) + size_of_val(unit),
        Instruction::BinaryGetUtf8 {
            fail_label,
            context,
            destination,
        } => size_of_val(fail_label) + size_of_val(context) + size_of_val(destination),
        Instruction::BinaryGetUtf16 {
            fail_label,
            context,
            little,
            destination,
        }
        | Instruction::BinaryGetUtf32 {
            fail_label,
            context,
            little,
            destination,
        } => {
            size_of_val(fail_label)
                + size_of_val(context)
                + size_of_val(little)
                + size_of_val(destination)
        }
        Instruction::BinaryMatch {
            fail_label,
            context,
            commands,
            test_heap,
        } => {
            size_of_val(fail_label)
                + size_of_val(context)
                + size_of_val(commands)
                + size_of_val(test_heap)
        }
        Instruction::BinaryMatchString {
            fail_label,
            context,
            bits,
            text,
        } => size_of_val(fail_label) + size_of_val(context) + size_of_val(bits) + size_of_val(text),
        Instruction::BinaryInitWritable => 0,
        Instruction::BinaryCreate {
            fail_label,
            destination,
            segments,
        } => size_of_val(fail_label) + size_of_val(destination) + size_of_val(segments),
        Instruction::FloatConvert { source, register } => {
            size_of_val(source) + size_of_val(register)
        }
        Instruction::FloatLoad { source, register } => size_of_val(source) + size_of_val(register),
        Instruction::FloatStore {
            register,
            destination,
        } => size_of_val(register) + size_of_val(destination),
        Instruction::FloatArith {
            operation,
            left,
            right,
            destination,
        } => {
            size_of_val(operation)
                + size_of_val(left)
                + size_of_val(right)
                + size_of_val(destination)
        }
        Instruction::FloatNegate {
            source,
            destination,
        } => size_of_val(source) + size_of_val(destination),
        Instruction::Send => 0,
        Instruction::RemoveMessage => 0,
        Instruction::LoopReceive {
            fail_label,
            destination,
        } => size_of_val(fail_label) + size_of_val(destination),
        Instruction::LoopReceiveEnd(field_0) => size_of_val(field_0),
        Instruction::Wait(field_0) => size_of_val(field_0),
        Instruction::WaitTimeout { label, source } => size_of_val(label) + size_of_val(source),
        Instruction::Timeout => 0,
        Instruction::ReceiveMarkerReserve(field_0) => size_of_val(field_0),
        Instruction::ReceiveMarkerBind { marker, reference } => {
            size_of_val(marker) + size_of_val(reference)
        }
        Instruction::ReceiveMarkerUse(field_0) => size_of_val(field_0),
        Instruction::ReceiveMarkerClear(field_0) => size_of_val(field_0),
        Instruction::NifStart { name, arity } => size_of_val(name) + size_of_val(arity),
        Instruction::TestYield => 0,
        Instruction::Noop => 0,
        Instruction::Return => 0,
        Instruction::End => 0,
    }
}

/// Record the layout of the decoded form with the pinned rustc with the pinned rustc: the size of
/// [`Instruction`], the size of the field types that dominate it, and the
/// bytes of the fields of each large arm. A change of a field type or of an
/// arm fails the assertions, and a change of the fields of any arm fails the
/// compile of [`arm_field_bytes`].
#[test]
fn records_the_decoded_form_layout() {
    use std::mem::size_of;

    let arms: [(&str, Instruction); 8] = [
        (
            "`BinaryGetBinary`",
            Instruction::BinaryGetBinary {
                fail_label: 0,
                context: Operand::X(0),
                size: Operand::X(1),
                unit: 8,
                destination: Operand::X(2),
            },
        ),
        (
            "`GetList`",
            Instruction::GetList {
                source: Operand::X(0),
                head: Operand::X(1),
                tail: Operand::X(2),
            },
        ),
        (
            "`PutList`",
            Instruction::PutList {
                head: Operand::X(0),
                tail: Operand::X(1),
                destination: Operand::X(2),
            },
        ),
        (
            "`Bif`",
            Instruction::Bif {
                fail_label: 0,
                bif: BifId::ErlangPlus2,
                arguments: vec![Operand::X(0), Operand::X(1)],
                destination: Operand::X(0),
            },
        ),
        (
            "`CallExternal`",
            Instruction::CallExternal {
                import: 0,
                tail: true,
                deallocate: 0,
            },
        ),
        (
            "`SelectValue`",
            Instruction::SelectValue {
                value: Operand::X(0),
                fail_label: 0,
                choices: SelectValueChoices::Linear(vec![(Term::Integer(1), 1)]),
            },
        ),
        (
            "`Move`",
            Instruction::Move(Operand::X(0), Operand::Value(Term::Integer(1))),
        ),
        ("`Label`", Instruction::Label(1)),
    ];

    println!("| decoded form item | bytes |");
    println!("| --- | --- |");
    println!("| `Instruction` | {} |", size_of::<Instruction>());
    println!("| `Operand` | {} |", size_of::<Operand>());
    println!("| `Term` | {} |", size_of::<Term>());
    println!("| `String` | {} |", size_of::<String>());
    println!("| `Vec<Operand>` | {} |", size_of::<Vec<Operand>>());
    println!("| `BifId` | {} |", size_of::<BifId>());
    println!("| `Option<BifId>` | {} |", size_of::<Option<BifId>>());
    println!("| `u64` | {} |", size_of::<u64>());
    println!("| `u32` | {} |", size_of::<u32>());
    println!();
    println!("| arm | bytes of the fields |");
    println!("| --- | --- |");
    for (name, instruction) in &arms {
        println!("| {name} | {} |", arm_field_bytes(instruction));
    }

    assert_eq!(size_of::<Instruction>(), 64, "the size of `Instruction`");
    assert_eq!(size_of::<Operand>(), 16, "the size of `Operand`");
    assert_eq!(size_of::<Term>(), 16, "the size of `Term`");
    assert_eq!(size_of::<String>(), 24, "the size of `String`");
    assert_eq!(size_of::<Vec<Operand>>(), 24, "the size of `Vec<Operand>`");
    // The arm that sets the size of `Instruction`, and the arm of the floor
    // loop. The plan records `BinaryGetBinary` at 132 bytes and `Bif` at 124
    // bytes at the baseline revision; step 4 removed the two `String` fields
    // from `Bif`. The boxed binary view of task [#571] takes `Term` from 40
    // bytes to 32, and the two `Operand` fields and the `Term` field of
    // `BinaryGetBinary` then lose 24 bytes. `Bif` holds the arguments of the
    // call in one `Term` slot, so it loses the 8 bytes of one `Term`. The
    // atom index of task [#596] takes `Term` from 32 bytes to 16 and
    // `Operand` from 32 bytes to 16, so `Instruction` is 64 bytes and each
    // `Operand` or `Term` field of an arm loses 16 bytes.
    //
    // [#571]: https://github.com/fahchen/pbeam/issues/571
    // [#596]: https://github.com/fahchen/pbeam/issues/596
    assert_eq!(
        arm_field_bytes(&arms[0].1),
        60,
        "the fields of `BinaryGetBinary`"
    );
    assert_eq!(arm_field_bytes(&arms[3].1), 48, "the fields of `Bif`");
    assert_eq!(arm_field_bytes(&arms[6].1), 32, "the fields of `Move`");
}

#[test]
fn decodes_the_float_register_instructions_of_the_float_probe_fixture() {
    // `float_probe:registers/2` in `fixtures/float/float_probe.beam` has
    // `{fmove,{x,0},{fr,0}}`, `{fmove,{fr,2},{x,0}}`, `fadd`, `fsub`,
    // `fmul`, `fdiv`, and `fnegate` with the fail label 0.
    let float = |opcode, operands: &[u8]| [&[opcode][..], operands].concat();
    let arith = [0x05, 0x27, 0x00, 0x27, 0x10, 0x27, 0x20];
    let mut body = [
        float(OPCODE_FMOVE, &[0x03, 0x27, 0x00]),
        float(OPCODE_FMOVE, &[0x27, 0x20, 0x03]),
        float(OPCODE_FADD, &arith),
        float(OPCODE_FSUB, &arith),
        float(OPCODE_FMUL, &arith),
        float(OPCODE_FDIV, &arith),
        float(OPCODE_FNEGATE, &[0x05, 0x27, 0x00, 0x27, 0x60]),
    ]
    .concat();
    body.push(0x03);
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend(&body);
    let (instructions, _, _) = parse_code(
        &code,
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(matches!(
        instructions.as_slice(),
        [
            Instruction::FloatLoad {
                source: Operand::X(0),
                register: 0,
            },
            Instruction::FloatStore {
                register: 2,
                destination: Operand::X(0),
            },
            Instruction::FloatArith {
                operation: FloatOperation::Add,
                left: 0,
                right: 1,
                destination: 2,
            },
            Instruction::FloatArith {
                operation: FloatOperation::Subtract,
                ..
            },
            Instruction::FloatArith {
                operation: FloatOperation::Multiply,
                ..
            },
            Instruction::FloatArith {
                operation: FloatOperation::Divide,
                ..
            },
            Instruction::FloatNegate {
                source: 0,
                destination: 6,
            },
            Instruction::End,
        ]
    ));
    // ERTS loads the float arithmetic only with the fail label 0, and each
    // operand must be a float register.
    for bad in [
        float(OPCODE_FADD, &[0x15, 0x27, 0x00, 0x27, 0x10, 0x27, 0x20]),
        float(OPCODE_FADD, &[0x05, 0x03, 0x27, 0x10, 0x27, 0x20]),
    ] {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend(&bad);
        code.push(0x03);
        assert!(
            parse_code(
                &code,
                &AtomTable::new(),
                &[],
                &[],
                &[],
                &[],
                &[],
                &[],
                &mut LineTable::default()
            )
            .is_err()
        );
    }
}

#[test]
fn decodes_the_bs_match_string_instructions_of_the_bs_match_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/binary_match/bs_match_probe.beam` has these bytes.
    // match_string/1: bs_match_string f37 x0 104 (offset 0)
    // match_string_offset/1: bs_match_string f40 x0 104 (offset 0)
    let parts: [&[u8]; 2] = [
        &[0x84, 0x0D, 0x25, 0x03, 0x08, 0x68, 0x00],
        &[0x84, 0x0D, 0x28, 0x03, 0x08, 0x68, 0x00],
    ];
    let (instructions, _, _) = parse_code(
        &code_with(&parts),
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        b"icrosoft.icon",
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(
        matches!(
            instructions.as_slice(),
            [
                Instruction::BinaryMatchString {
                    fail_label: 37,
                    context: Operand::X(0),
                    bits: 104,
                    text,
                },
                Instruction::BinaryMatchString {
                    fail_label: 40,
                    context: Operand::X(0),
                    bits: 104,
                    text: second,
                },
                Instruction::End,
            ] if text == b"icrosoft.icon" && second == b"icrosoft.icon"
        ),
        "{instructions:?}"
    );
}

#[test]
fn rejects_a_bs_match_string_outside_the_string_chunk() {
    // A `Bits` whose length byte count leaves the `StrT` chunk is an error,
    // as the ERTS loader gives.
    let mut cursor = 0;
    assert_eq!(
        read_segment_string(&[0x00], &mut 0, b"count: ", 16).unwrap(),
        b"co".to_vec()
    );
    assert!(read_segment_string(&[0x80, 0x10], &mut cursor, b"count: ", 16).is_err());
    assert!(read_segment_string(&[0x00], &mut 0, b"count: ", 8 * 8 + 1).is_err());
    assert!(read_segment_string(&[], &mut 0, b"count: ", 16).is_err());
}

#[test]
fn reads_the_little_signed_and_native_flags_of_a_segment() {
    // `bs_get_integer2` of `get_size_flagged/2` has the flags 6, and
    // `bs_skip_bits2` and the `bs_get_*` instructions have the flags 0.
    assert_eq!(read_segment_flags(&[0x60], &mut 0).unwrap(), (true, true));
    assert_eq!(read_segment_flags(&[0x00], &mut 0).unwrap(), (false, false));
    assert_eq!(read_segment_flags(&[0x20], &mut 0).unwrap(), (true, false));
    assert_eq!(read_segment_flags(&[0x40], &mut 0).unwrap(), (false, true));
    assert_eq!(
        read_segment_flags(&[0x08, 0x10], &mut 0).unwrap(),
        (cfg!(target_endian = "little"), false)
    );
    assert!(read_segment_flags(&[], &mut 0).is_err());
}

#[test]
fn decodes_the_bs_skip_bits2_instructions_of_the_bs_match_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/binary_match/bs_match_probe.beam` has these bytes.
    // skip_size/2: bs_skip_bits2 f46 {tr,x0,3} x1 8 0
    // skip_size_bits/2: bs_skip_bits2 f49 {tr,x0,3} x1 1 0
    let parts: [&[u8]; 2] = [
        &[0x78, 0x0D, 0x2E, 0x57, 0x03, 0x30, 0x13, 0x80, 0x00],
        &[0x78, 0x0D, 0x31, 0x57, 0x03, 0x30, 0x13, 0x10, 0x00],
    ];
    let (instructions, _, _) = parse_code(
        &code_with(&parts),
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(
        matches!(
            instructions.as_slice(),
            [
                Instruction::BinarySkipBits {
                    fail_label: 46,
                    context: Operand::X(0),
                    size: Operand::X(1),
                    unit: 8,
                },
                Instruction::BinarySkipBits {
                    fail_label: 49,
                    context: Operand::X(0),
                    size: Operand::X(1),
                    unit: 1,
                },
                Instruction::End,
            ]
        ),
        "{instructions:?}"
    );
}

#[test]
fn decodes_the_bs_get_integer2_instructions_of_the_bs_match_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/binary_match/bs_match_probe.beam` has these bytes.
    // get_size/2: bs_get_integer2 f52 2 {tr,x0,3} x1 1 0 x1
    // get_size_flagged/2: bs_get_integer2 f55 2 {tr,x0,3} x1 1
    // {literal,[little,signed]} x1
    let parts: [&[u8]; 2] = [
        &[
            0x75, 0x0D, 0x34, 0x57, 0x03, 0x30, 0x20, 0x13, 0x10, 0x00, 0x13,
        ],
        &[
            0x75, 0x0D, 0x37, 0x57, 0x03, 0x30, 0x20, 0x13, 0x10, 0x60, 0x13,
        ],
    ];
    let (instructions, _, _) = parse_code(
        &code_with(&parts),
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(
        matches!(
            instructions.as_slice(),
            [
                Instruction::BinaryGetInteger {
                    fail_label: 52,
                    context: Operand::X(0),
                    size: Operand::X(1),
                    unit: 1,
                    little: false,
                    signed: false,
                    destination: Operand::X(1),
                },
                Instruction::BinaryGetInteger {
                    fail_label: 55,
                    context: Operand::X(0),
                    size: Operand::X(1),
                    unit: 1,
                    little: true,
                    signed: true,
                    destination: Operand::X(1),
                },
                Instruction::End,
            ]
        ),
        "{instructions:?}"
    );
}

#[test]
fn decodes_the_bs_skip_utf8_instruction_of_the_bs_utf_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/binary_match/bs_utf_probe.beam` has these bytes.
    // skip8/1: bs_skip_utf8 f47 {tr,x0,1} 1 0
    let parts: [&[u8]; 1] = [&[0x8B, 0x0D, 0x2F, 0x57, 0x03, 0x10, 0x10, 0x00]];
    let (instructions, _, _) = parse_code(
        &code_with(&parts),
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap();
    assert!(
        matches!(
            instructions.as_slice(),
            [
                Instruction::BinaryGetUtf8 {
                    fail_label: 47,
                    context: Operand::X(0),
                    destination: None,
                },
                Instruction::End,
            ]
        ),
        "{instructions:?}"
    );
}

#[test]
fn decodes_the_utf16_and_utf32_segments_of_the_bs_create_bin_utf_probe_fixture() {
    // `tests/integration/fixture_opcodes.rs` checks that
    // `fixtures/bs_create_bin/bs_create_bin_utf_probe.beam` has these
    // bytes, which are the segment lists of `utf16/1`, `utf16_little/1`,
    // and `utf32/1`.
    // Atom 0 is `nil`, atom 2 is `utf16`, atom 3 is `undefined`, and atom
    // 6 is `utf32`. Literal 0 is `[little]`.
    let atoms: Vec<Atom> = (1..=6)
        .map(|index| {
            Atom::from_static(match index {
                2 => "utf16",
                3 => "undefined",
                6 => "utf32",
                _ => "ok",
            })
            .unwrap()
        })
        .collect();
    let literals = [Some(Term::list_from(vec![Term::static_atom("little")]))];
    let little = [0x17, 0x60, 0x22, 0x10, 0x00, 0x47, 0x00, 0x03, 0x32];
    let mut cursor = 0;
    let segments = read_binary_segments(&little, &mut cursor, &atoms, &literals, &[]).unwrap();
    assert!(
        matches!(
            segments.as_slice(),
            [BinarySegment::Utf16 {
                segment: 1,
                little: true,
                source: Operand::X(0),
            }]
        ),
        "{segments:?}"
    );
    let big = [0x17, 0x60, 0x22, 0x10, 0x00, 0x02, 0x03, 0x32];
    let segments = read_binary_segments(&big, &mut 0, &atoms, &[], &[]).unwrap();
    assert!(
        matches!(
            segments.as_slice(),
            [BinarySegment::Utf16 {
                segment: 1,
                little: false,
                source: Operand::X(0),
            }]
        ),
        "{segments:?}"
    );
    let utf32 = [0x17, 0x60, 0x62, 0x10, 0x00, 0x02, 0x03, 0x32];
    let segments = read_binary_segments(&utf32, &mut 0, &atoms, &[], &[]).unwrap();
    assert!(
        matches!(
            segments.as_slice(),
            [BinarySegment::Utf32 {
                segment: 1,
                little: false,
                source: Operand::X(0),
            }]
        ),
        "{segments:?}"
    );
}

/// The code chunk of a synthetic module with the instruction bytes `parts`,
/// the label count 16, and the `int_code_end` instruction.
fn code_with(parts: &[&[u8]]) -> Vec<u8> {
    let mut code = vec![0, 0, 0, 16];
    code.extend([0; 16]);
    code.extend(parts.concat());
    code.push(0x03);
    code
}

#[test]
fn rejects_an_opcode_that_the_loader_does_not_decode() {
    let parts: [&[u8]; 1] = [&[250, 0x10]];
    let error = parse_code(
        &code_with(&parts),
        &AtomTable::new(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
        &mut LineTable::default(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("unsupported BEAM opcode 250"),
        "{error}"
    );
}

/// The ERTS loader fuses `bs_match` and the `test_heap` right after it
/// into `i_bs_match_test_heap` (JIT jit/arm/ops.tab:962-963).
#[test]
fn marks_a_bs_match_that_a_test_heap_follows() {
    // bs_match f16 x0 {list,[ensure_at_least,0,8]}
    let bs_match: &[u8] = &[0xB6, 0x0D, 0x10, 0x03, 0x17, 0x30, 0x82, 0x00, 0x80];
    // test_heap 3 2
    let test_heap: &[u8] = &[0x10, 0x30, 0x20];
    // bs_set_position x0 x1
    let set_position: &[u8] = &[0xA8, 0x03, 0x13];
    // Atom 8 is `ensure_at_least`.
    let table = AtomTable::new();
    let atoms: Vec<Atom> = (1..=8)
        .map(|index| match index {
            8 => table.put("ensure_at_least").unwrap(),
            _ => table.put(&format!("atom{index}")).unwrap(),
        })
        .collect();
    let fused = |parts: &[&[u8]]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend(parts.concat());
        code.push(0x03);
        let (instructions, _, _) = parse_code(
            &code,
            &table,
            &atoms,
            &[],
            &[],
            &[],
            &[],
            &[],
            &mut LineTable::default(),
        )
        .unwrap();
        match instructions.first() {
            Some(Instruction::BinaryMatch { test_heap, .. }) => *test_heap,
            other => panic!("{other:?}"),
        }
    };
    assert!(fused(&[bs_match, test_heap]));
    assert!(!fused(&[bs_match, set_position, test_heap]));
}

#[test]
fn rejects_malformed_native_record_operands() {
    // Atom 1 is `point`; literal 0 is `{1}`. No `Recs` record is loaded.
    let table = AtomTable::new();
    let atoms = [table.put("point").unwrap()];
    let literals = [Some(Term::tuple(vec![Term::Integer(1)]))];
    let parse = |stream: &[u8]| {
        let mut code = vec![0, 0, 0, 16];
        code.extend([0; 16]);
        code.extend(stream);
        code.push(0x03);
        parse_code(
            &code,
            &table,
            &atoms,
            &[],
            &literals,
            &[],
            &[],
            &[],
            &mut LineTable::default(),
        )
        .unwrap_err()
        .to_string()
    };
    for (stream, message) in [
        (
            &[0xBB, 0x15, 0x01][..],
            "BEAM native record source is not a register",
        ),
        (
            &[0xBB, 0x15, 0x03, 0x01],
            "BEAM record operand is not an atom",
        ),
        (
            &[0xBD, 0x05, 0x12, 0x02, 0x03, 0x00, 0x17, 0x00],
            "BEAM put_record names no record of the module",
        ),
        (
            &[0xBD, 0x05, 0x12, 0x01, 0x03, 0x00, 0x17, 0x00],
            "unsupported BEAM put_record source",
        ),
        (
            &[0xBD, 0x05, 0x01, 0x02, 0x03, 0x00, 0x17, 0x00],
            "unsupported BEAM record identifier",
        ),
        (
            &[0xBD, 0x05, 0x47, 0x00, 0x02, 0x03, 0x00, 0x17, 0x00],
            "unsupported BEAM record identifier",
        ),
        (
            &[0xBD, 0x05, 0x12, 0x03, 0x03, 0x00, 0x17, 0x20, 0x11, 0x13],
            "BEAM record field is not an atom",
        ),
    ] {
        assert_eq!(parse(stream), message, "{stream:?}");
    }
}
