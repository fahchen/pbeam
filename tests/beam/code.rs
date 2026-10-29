use super::{
    atom_at, atom_chunk_name, parse_atoms, parse_exports, parse_lambdas, parse_literals,
    parse_records, read_literal, read_u32,
};
use crate::term::{AtomTable, Binary, Fun, Lambda, Map, Term, debug_term};

/// Return a `FunT` payload with a count and entries. Each entry gives
/// its function atom index, total arity, label, index, number of free
/// variables, and old unique value.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(lambda_table(0, &[]), [0, 0, 0, 0]);
/// ```
fn lambda_table(count: i32, entries: &[[i32; 6]]) -> Vec<u8> {
    let fields = std::iter::once(count).chain(entries.iter().flatten().copied());
    fields.flat_map(i32::to_be_bytes).collect()
}

/// Return the atom term of a name that exists in a table.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(atom(&AtomTable::new(), "ok"), Term::static_atom("ok"));
/// ```
fn atom(table: &AtomTable, name: &str) -> Term {
    Term::Atom(table.get(name).unwrap())
}

#[test]
fn rejects_invalid_atom_tables_and_indices() {
    let table = AtomTable::new();
    assert!(read_u32(&[0, 1]).is_err());
    assert!(parse_atoms(&[], &table).is_err());
    assert!(parse_atoms(&(-1i32).to_be_bytes(), &table).is_err());
    assert!(atom_at(&[], 0).is_err());

    let mut malformed_atoms = (-2i32).to_be_bytes().to_vec();
    malformed_atoms.extend([0x30, b'a', b'b', b'c', 0xa0]);
    assert!(parse_atoms(&malformed_atoms, &table).is_err());
    assert!(parse_atoms(&[0xff, 0xff, 0xff, 0xff, 0x10, 0xff], &table).is_err());
    assert_eq!(
        parse_atoms(&0i32.to_be_bytes(), &table).unwrap(),
        [] as [crate::term::Atom; 0]
    );
}

#[test]
fn puts_atu8_atoms_into_the_table_in_chunk_order() {
    let table = AtomTable::new();
    // AtU8 with zeta, ok, and alpha: ok keeps its static index 506.
    let mut chunk = (-3i32).to_be_bytes().to_vec();
    chunk.extend([0x40, b'z', b'e', b't', b'a', 0x20, b'o', b'k']);
    chunk.extend([0x50, b'a', b'l', b'p', b'h', b'a']);
    let atoms = parse_atoms(&chunk, &table).unwrap();
    let indices: Vec<_> = atoms.iter().map(crate::term::Atom::index).collect();
    assert_eq!(indices, [1167, 506, 1168]);
    assert_eq!(table.name_of(&atoms[2]), "alpha");
    assert_eq!(table.get("zeta"), Some(atoms[0].clone()));
    // A second module gets the same index for an existing atom.
    assert_eq!(parse_atoms(&chunk, &table).unwrap(), atoms);
}

#[test]
fn truncates_a_long_atu8_name_to_255_characters_like_erts() {
    // OTP 29.1.1 loads 255 `a` and `b`, or 255 `a` and 0xff, as the atom of
    // 255 `a`. It rejects 254 `a` and 0xff as a corrupt atom table.
    let a255 = "a".repeat(255);
    assert_eq!(atom_chunk_name(a255.as_bytes()).unwrap(), a255);
    assert_eq!(
        atom_chunk_name(format!("{a255}b").as_bytes()).unwrap(),
        a255
    );
    assert_eq!(
        atom_chunk_name(&[a255.as_bytes(), &[0xff]].concat()).unwrap(),
        a255
    );
    assert!(atom_chunk_name(&[&a255.as_bytes()[1..], &[0xff]].concat()).is_err());
    // ERTS counts characters, not bytes: 255 `é` and `x` is 255 `é`.
    let e255 = "é".repeat(255);
    assert_eq!(
        atom_chunk_name(format!("{e255}x").as_bytes()).unwrap(),
        e255
    );

    // AtU8 with 255 `a` and `b`, then 255 `a` and `d`: one atom.
    let table = AtomTable::new();
    let mut chunk = (-2i32).to_be_bytes().to_vec();
    for last in *b"bd" {
        chunk.extend([0x28, 0]);
        chunk.extend(a255.as_bytes());
        chunk.push(last);
    }
    let atoms = parse_atoms(&chunk, &table).unwrap();
    assert_eq!(table.name_of(&atoms[0]), a255);
    assert_eq!(atoms[0].index(), 1167);
    assert_eq!(atoms[1], atoms[0]);
    assert_eq!(table.get(&a255), Some(atoms[0].clone()));
}

#[test]
fn rejects_invalid_export_tables_and_indices() {
    let table = AtomTable::new();
    let atoms = [table.put("constant").unwrap()];
    assert!(parse_exports(&[], &[], &table).is_err());
    assert!(parse_exports(&[0, 0, 0, 1], &[], &table).is_err());
    let exports = [0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 1];
    assert!(parse_exports(&exports, &atoms, &table).is_err());
}

#[test]
fn reads_uncompressed_binary_literals() {
    let atoms = AtomTable::new();
    let mut table = 0u32.to_be_bytes().to_vec();
    table.extend(1u32.to_be_bytes());
    table.extend(8u32.to_be_bytes());
    table.extend([131, 109, 0, 0, 0, 2, b'o', b'k']);
    assert_eq!(
        parse_literals(&table, &atoms).unwrap(),
        vec![Some(Term::binary(b"ok".to_vec()))]
    );
    assert_eq!(
        parse_literals(&[0, 0, 0, 0, 0, 0, 0, 0], &atoms).unwrap(),
        Vec::<Option<Term>>::new()
    );
    assert_eq!(read_literal(&[1], &atoms).unwrap(), None);
    assert!(read_literal(&[131, 109], &atoms).is_err());
    assert_eq!(
        read_literal(&[131, 107, 0, 1, b'\n'], &atoms).unwrap(),
        Some(Term::list_from(vec![Term::Integer(10)]))
    );
    assert!(read_literal(&[131, 107], &atoms).is_err());
    assert!(read_literal(&[131, 107, 0, 2, b'x'], &atoms).is_err());
}

#[test]
fn reads_bit_binary_literals() {
    // BIT_BINARY_EXT (77): a byte count, the number of bits of the last byte
    // (1 to 8), and the bytes (`external.c:4895-4923`). <<1:3>> is
    // [77, 0, 0, 0, 1, 3, 32]. The low bits of the last byte are not part of
    // the bitstring, and a count of 8 gives a binary.
    let atoms = AtomTable::new();
    let bits = Term::Binary(Binary::from(vec![32]).bits(0, 3));
    assert_eq!(
        read_literal(&[131, 77, 0, 0, 0, 1, 3, 32], &atoms).unwrap(),
        Some(bits.clone())
    );
    assert_eq!(
        read_literal(&[131, 77, 0, 0, 0, 1, 3, 0x3F], &atoms).unwrap(),
        Some(bits)
    );
    assert_eq!(
        read_literal(&[131, 77, 0, 0, 0, 2, 8, 1, 2], &atoms).unwrap(),
        Some(Term::binary(vec![1, 2]))
    );
    // ERTS rejects a count outside 1 to 8 and an empty byte list
    // (`external.c:4913-4917`).
    for bad in [
        &[131, 77, 0, 0, 0, 1, 0, 32][..],
        &[131, 77, 0, 0, 0, 1, 9, 32],
        &[131, 77, 0, 0, 0, 0, 1],
        &[131, 77, 0, 0, 0, 1],
    ] {
        assert!(read_literal(bad, &atoms).is_err(), "{bad:?}");
    }
}

#[test]
fn reads_large_tuple_literals() {
    // `LARGE_TUPLE_EXT` (tag 105) holds a four-byte element count
    // (`external.c:4542-4549`), as the literal of a tuple above 255
    // elements does.
    let atoms = AtomTable::new();
    let bytes = [131, 105, 0, 0, 0, 3, 97, 1, 97, 2, 97, 3];
    assert_eq!(
        read_literal(&bytes, &atoms).unwrap(),
        Some(Term::tuple(vec![
            Term::Integer(1),
            Term::Integer(2),
            Term::Integer(3),
        ]))
    );
    // The empty large tuple is the empty tuple.
    assert_eq!(
        read_literal(&[131, 105, 0, 0, 0, 0], &atoms).unwrap(),
        Some(Term::tuple(Vec::new()))
    );
    // ERTS rejects a count above `ERTS_MAX_TUPLE_SIZE` (`external.c:6284-6289`).
    assert_eq!(
        read_literal(&[131, 105, 1, 0, 0, 0], &atoms)
            .unwrap_err()
            .to_string(),
        "BEAM tuple literal exceeds the maximum tuple size"
    );
}

#[test]
fn puts_literal_atoms_into_the_table_in_byte_order() {
    let atoms = AtomTable::new();
    // {pbeam_lit_b, #{pbeam_lit_a => 1, pbeam_lit_b => 2}}: ERTS 29.1.1 creates pbeam_lit_b
    // first, so maps:keys/1 of the map is [pbeam_lit_b,pbeam_lit_a].
    let mut bytes = vec![131, 104, 2, 119, 11];
    bytes.extend(b"pbeam_lit_b");
    bytes.extend([116, 0, 0, 0, 2, 119, 11]);
    bytes.extend(b"pbeam_lit_a");
    bytes.extend([97, 1, 119, 11]);
    bytes.extend(b"pbeam_lit_b");
    bytes.extend([97, 2]);
    let literal = read_literal(&bytes, &atoms).unwrap().unwrap();
    assert_eq!(atoms.get("pbeam_lit_b").unwrap().index(), 1167);
    assert_eq!(atoms.get("pbeam_lit_a").unwrap().index(), 1168);
    let (lit_a, lit_b) = (atom(&atoms, "pbeam_lit_a"), atom(&atoms, "pbeam_lit_b"));
    let pairs = vec![(lit_a, Term::Integer(1)), (lit_b.clone(), Term::Integer(2))];
    let map = Term::Map(Map::from_pairs(pairs, &atoms).unwrap());
    assert_eq!(literal, Term::tuple(vec![lit_b, map]));
    assert_eq!(
        debug_term(&literal, &atoms),
        "Tuple([Atom(\"pbeam_lit_b\"), Map([(Atom(\"pbeam_lit_b\"), Integer(2)), \
         (Atom(\"pbeam_lit_a\"), Integer(1))])])"
    );
    // [pbeam_x, <pid>, pbeam_y]: the atoms after the unsupported
    // NEW_PID_EXT (88) are not created.
    let mut unsupported = vec![131, 108, 0, 0, 0, 3, 119, 7];
    unsupported.extend(b"pbeam_x");
    unsupported.extend([88, 119, 7]);
    unsupported.extend(b"pbeam_y");
    unsupported.push(106);
    assert_eq!(read_literal(&unsupported, &atoms).unwrap(), None);
    assert_eq!(atoms.get("pbeam_x").unwrap().index(), 1169);
    assert_eq!(atoms.get("pbeam_y"), None);
}

#[test]
fn reads_nested_literals_and_marks_unsupported_tags_at_any_depth() {
    let atoms = AtomTable::new();
    let read = |atoms: &AtomTable, bytes: &[u8]| read_literal(bytes, atoms);
    // [a|[b]] gives the same list as [a,b].
    let nested = read(
        &atoms,
        &[
            131, 108, 0, 0, 0, 1, 119, 1, b'a', 108, 0, 0, 0, 1, 119, 1, b'b', 106,
        ],
    )
    .unwrap();
    assert_eq!(
        read(&atoms, &[131, 104, 2, 119, 2, b'o', b'k', 106]).unwrap(),
        Some(Term::tuple(vec![Term::static_atom("ok"), Term::nil()]))
    );
    // NEW_FLOAT_EXT (70) 1.0 loads; an infinity is an error, as `dec_term`
    // rejects it (`external.c:4666-4668`).
    assert_eq!(
        read(&atoms, &[131, 70, 63, 240, 0, 0, 0, 0, 0, 0]).unwrap(),
        Some(Term::Float(1.0))
    );
    assert!(read(&atoms, &[131, 70, 127, 240, 0, 0, 0, 0, 0, 0]).is_err());
    // SMALL_BIG_EXT (110) and LARGE_BIG_EXT (111) load as integers in
    // their canonical form (`external.c:4457-4497`): 5 is a small integer,
    // each sign byte but 0 is negative, leading zero bytes are stripped, and
    // a value of more than `BIG_ARITY_MAX` digits is an error.
    let big = |sign: i32, shift: u32| Term::from_bigint(num_bigint::BigInt::from(sign) << shift);
    assert_eq!(
        read(&atoms, &[131, 110, 1, 0, 5]).unwrap(),
        Some(Term::Integer(5))
    );
    assert_eq!(
        read(&atoms, &[131, 110, 0, 1]).unwrap(),
        Some(Term::Integer(0))
    );
    let two_64 = [131, 110, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0];
    assert_eq!(read(&atoms, &two_64).unwrap(), Some(big(1, 64)));
    let negative = [131, 110, 9, 2, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    assert_eq!(read(&atoms, &negative).unwrap(), Some(big(-1, 64)));
    let mut large = vec![131, 111, 0, 0, 1, 1, 0];
    large.extend([0; 256]);
    large.push(1);
    assert_eq!(read(&atoms, &large).unwrap(), Some(big(1, 2048)));
    let mut huge = vec![131, 111, 0, 8, 0, 1, 0];
    huge.extend(vec![0; 524_288]);
    huge.push(1);
    assert!(read(&atoms, &huge).is_err());
    // NEW_PID_EXT (88) inside a tuple, inside a list, and as a list tail.
    let bignum = [88, 1, 0, 5];
    assert_eq!(
        read(&atoms, &[&[131, 104, 1][..], &bignum].concat()).unwrap(),
        None
    );
    assert_eq!(
        read(
            &atoms,
            &[&[131, 108, 0, 0, 0, 1][..], &bignum, &[106]].concat()
        )
        .unwrap(),
        None
    );
    assert_eq!(
        read(
            &atoms,
            &[&[131, 108, 0, 0, 0, 1, 106][..], &bignum].concat()
        )
        .unwrap(),
        None
    );
    // [[]|t] has an atom tail, and [a|[b|t]] keeps the tail of its list tail.
    let nil_head = read(&atoms, &[131, 108, 0, 0, 0, 1, 106, 119, 1, b't']).unwrap();
    let tail = [108, 0, 0, 0, 1, 119, 1, b'b', 119, 1, b't'];
    let list_tail = read(
        &atoms,
        &[&[131, 108, 0, 0, 0, 1, 119, 1, b'a'][..], &tail].concat(),
    )
    .unwrap();
    // A LIST_EXT of length 0 gives its tail (external.c:4581-4588).
    let empty = read(&atoms, &[131, 108, 0, 0, 0, 0, 119, 1, b't']).unwrap();
    assert!(read(&atoms, &[131, 119, 1, 0xff]).is_err());
    assert!(read(&atoms, &[131, 106, 106]).is_err());
    assert!(read(&atoms, &[131, 104]).is_err());
    // [a, b] with length 2, and a zero-length LIST_EXT whose tail is [].
    let proper = read(
        &atoms,
        &[131, 108, 0, 0, 0, 2, 119, 1, b'a', 119, 1, b'b', 106],
    )
    .unwrap();
    assert_eq!(
        read(&atoms, &[131, 108, 0, 0, 0, 0, 106]).unwrap(),
        Some(Term::nil())
    );

    // #{b => [], a => {}} keeps its pairs in flatmap key order. The first literal made a
    // before b, so a has the lower atom index.
    let map = read(
        &atoms,
        &[
            131, 116, 0, 0, 0, 2, 119, 1, b'b', 106, 119, 1, b'a', 104, 0,
        ],
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        debug_term(&map, &atoms),
        "Map([(Atom(\"a\"), Tuple([])), (Atom(\"b\"), List([]))])"
    );
    assert_eq!(
        read(&atoms, &[131, 116, 0, 0, 0, 0]).unwrap(),
        Some(Term::Map(
            Map::from_pairs(vec![], &AtomTable::new()).unwrap()
        ))
    );
    // A fun key loads: #{{fun a:b/0} => []}.
    let fun_key = read(
        &atoms,
        &[
            131, 116, 0, 0, 0, 1, 104, 1, 113, 119, 1, b'a', 119, 1, b'b', 97, 0, 106,
        ],
    )
    .unwrap()
    .unwrap();
    assert!(matches!(&fun_key, Term::Map(map) if map.len() == 1));
    // ERTS rejects a MAP_EXT with a duplicate key: #{1 => [], 1 => []}.
    assert_eq!(
        read(&atoms, &[131, 116, 0, 0, 0, 2, 97, 1, 106, 97, 1, 106])
            .unwrap_err()
            .to_string(),
        "duplicate key in BEAM map literal"
    );

    // SMALL_INTEGER_EXT: 1 and 255.
    assert_eq!(read(&atoms, &[131, 97, 1]).unwrap(), Some(Term::Integer(1)));
    assert_eq!(
        read(&atoms, &[131, 97, 255]).unwrap(),
        Some(Term::Integer(255))
    );
    // INTEGER_EXT: 256, -1, and the i32 limits, as term_to_binary/1 writes them.
    for (bytes, value) in [
        ([0, 0, 1, 0], 256),
        ([255, 255, 255, 255], -1),
        ([127, 255, 255, 255], i64::from(i32::MAX)),
        ([128, 0, 0, 0], i64::from(i32::MIN)),
    ] {
        let literal = [&[131, 98][..], &bytes].concat();
        assert_eq!(read(&atoms, &literal).unwrap(), Some(Term::Integer(value)));
    }
    assert!(read(&atoms, &[131, 98, 0, 0, 1]).is_err());

    // erlc writes #{1 => v, ..., 33 => v} as 131,116,0,0,0,33,97,1,119,1,118,...;
    // 33 keys give a hashmap in the ERTS order, from either input order.
    let literal = |keys: &mut dyn Iterator<Item = u8>| {
        let mut bytes = vec![131, 116, 0, 0, 0, 33];
        for key in keys {
            bytes.extend([97, key, 119, 1, b'v']);
        }
        read(&atoms, &bytes).unwrap().unwrap()
    };
    let forward = literal(&mut (1..=33));
    assert_eq!(forward, literal(&mut (1..=33).rev()));
    assert!(
        debug_term(&forward, &atoms).starts_with(
            "Map([(Integer(18), Atom(\"v\")), (Integer(4), Atom(\"v\")), (Integer(12), "
        )
    );

    // ERTS decodes a term nested 1000000 levels deep; so does PBeam.
    let mut deep = vec![131];
    for _ in 0..1_000_000 {
        deep.extend([104, 1]);
    }
    deep.push(106);
    let term = read(&atoms, &deep).unwrap().unwrap();
    assert!(format!("{term:?}").starts_with("Tuple([Tuple(["));

    let list = Some(Term::list_from(vec![atom(&atoms, "a"), atom(&atoms, "b")]));
    assert_eq!(nested, list);
    let t = || Box::new(atom(&atoms, "t"));
    assert_eq!(
        nil_head,
        Some(Term::list_with_tail(vec![Term::nil()], *t()))
    );
    assert_eq!(
        list_tail,
        Some(Term::list_with_tail(
            vec![atom(&atoms, "a"), atom(&atoms, "b")],
            *t()
        ))
    );
    assert_eq!(empty, Some(*t()));
    assert_eq!(proper, list);
}

#[test]
fn reads_external_fun_literals_as_export_ext() {
    let atoms = AtomTable::new();
    let read = |atoms: &AtomTable, bytes: &[u8]| read_literal(bytes, atoms);
    // erlc writes `fun lists:reverse/1` as EXPORT_EXT (113), two atoms,
    // and SMALL_INTEGER_EXT.
    let mut reverse = vec![131, 113, 119, 5];
    reverse.extend(b"lists");
    reverse.extend([119, 7]);
    reverse.extend(b"reverse");
    reverse.extend([97, 1]);
    let reverse = read(&atoms, &reverse).unwrap();
    // An atom tag that PBeam does not decode, such as ATOM_EXT (100),
    // makes the literal unsupported.
    assert_eq!(read(&atoms, &[131, 113, 100, 0, 1, b'm']).unwrap(), None);
    // ERTS `dec_term` (`external.c:5006-5041`) fails for a module or
    // function that is not an atom, or an arity that is not an integer.
    for bytes in [
        &[131, 113, 106, 119, 1, b'f', 97, 1][..],
        &[131, 113, 119, 1, b'm', 106, 97, 1],
        &[131, 113, 119, 1, b'm', 119, 1, b'f', 106],
    ] {
        assert_eq!(
            read(&atoms, bytes).unwrap_err().to_string(),
            "invalid BEAM external fun literal"
        );
    }
    // ERTS 29.1.1 loads an INTEGER_EXT arity in `0..=255`, as
    // `[98, 0, 0, 0, 1]` and `[98, 0, 0, 0, 255]`. It rejects -1 as a
    // corrupt literal table, and it has no defined result above 255.
    let export = |arity: &[u8]| [&[131, 113, 119, 1, b'm', 119, 1, b'f'][..], arity].concat();
    let integer_one = read(&atoms, &export(&[98, 0, 0, 0, 1])).unwrap();
    let integer_max = read(&atoms, &export(&[98, 0, 0, 0, 255])).unwrap();
    for arity in [&[98, 255, 255, 255, 255][..], &[98, 0, 0, 1, 0]] {
        assert_eq!(
            read(&atoms, &export(arity)).unwrap_err().to_string(),
            "invalid BEAM external fun literal"
        );
    }
    let atom = |name| atoms.get(name).unwrap();
    let fun = Fun::external(atom("lists"), atom("reverse"), 1);
    assert_eq!(reverse, Some(Term::Fun(fun)));
    for (fun, arity) in [(integer_one, 1), (integer_max, 255)] {
        assert_eq!(
            fun,
            Some(Term::Fun(Fun::external(atom("m"), atom("f"), arity)))
        );
    }
}

#[test]
fn rejects_invalid_literal_tables() {
    let atoms = AtomTable::new();
    let parse = |bytes: &[u8]| parse_literals(bytes, &atoms);
    assert!(parse(&[0, 0, 0, 0]).is_err());
    assert!(parse(&[0, 0, 0, 1, 0, 0, 0, 0]).is_err());
    assert!(parse(&[0, 0, 0, 0, 0, 0, 0, 1]).is_err());
    assert!(parse(&[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1]).is_err());
    assert!(parse(&[0, 0, 0, 0, 0, 0, 0, 0, 1]).is_err());

    let mut malformed_binary = vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 7];
    malformed_binary.extend([131, 109, 0, 0, 0, 2, b'x']);
    assert!(parse(&malformed_binary).is_err());
}

#[test]
fn reads_the_lambda_table_as_parse_lambda_chunk() {
    let table = AtomTable::new();
    let atoms = vec![table.put("fun_probe").unwrap(), table.put("f").unwrap()];
    let module = atoms[0].clone();
    // An entry of the form that erlc writes for `fun(X) -> {Tag, X} end`:
    // atom 2, total arity 2, label 127, index 0, one free variable, old
    // unique value 31263486. A count below the number of entries reads
    // only the counted entries, and atom index 0 is the ERTS non-value
    // slot.
    let entries = [[2, 2, 127, 0, 1, 31_263_486], [0, 0, 51, 1, 0, 7]];
    assert_eq!(
        parse_lambdas(&lambda_table(2, &entries), &atoms, [9; 16]).unwrap(),
        vec![
            (
                std::sync::Arc::new(
                    Lambda::new(module.clone(), 0, 0, 31_263_486, 127, 1).with_uniq([9; 16])
                ),
                1
            ),
            (
                std::sync::Arc::new(Lambda::new(module, 1, 1, 7, 51, 0).with_uniq([9; 16])),
                0
            ),
        ]
    );
    assert_eq!(
        parse_lambdas(&lambda_table(1, &entries), &atoms, [9; 16])
            .unwrap()
            .len(),
        1
    );
    let error = |bytes: Vec<u8>| {
        parse_lambdas(&bytes, &atoms, [9; 16])
            .unwrap_err()
            .to_string()
    };
    assert_eq!(error(lambda_table(-1, &[])), "negative BEAM lambda count");
    assert_eq!(
        error(lambda_table(2, &entries[..1])),
        "invalid BEAM lambda table size"
    );
    let mut partial = lambda_table(1, &entries[..1]);
    partial.push(0);
    assert_eq!(error(partial), "invalid BEAM lambda table size");
    assert!(error(vec![0, 0]).contains("truncated"));
    for atom_index in [3, -1] {
        assert_eq!(
            error(lambda_table(1, &[[atom_index, 0, 1, 0, 0, 0]])),
            "BEAM lambda function atom index out of range"
        );
    }
    for (total_arity, free_count) in [(1, 2), (1, -1)] {
        assert_eq!(
            error(lambda_table(1, &[[1, total_arity, 1, 0, free_count, 0]])),
            "BEAM lambda has more free variables than arguments"
        );
    }
}

/// Return a `Recs` chunk of version `version` with `count` records and
/// the bytes `body`.
fn recs(version: i32, count: i32, body: &[u8]) -> Vec<u8> {
    let mut data: Vec<u8> = [version, count, 0]
        .iter()
        .flat_map(|word| word.to_be_bytes())
        .collect();
    data.extend_from_slice(body);
    data
}

#[test]
fn parses_the_records_chunk_as_pinned_erts() {
    let table = AtomTable::new();
    // Atom 1 is the module, 2 the name, 3 `true`, 4 the key, 5 not a flag.
    let atoms: Vec<_> = ["nr_def", "priv", "true", "z", "other"]
        .iter()
        .map(|name| table.put(name).unwrap())
        .collect();
    let literals = [Some(Term::tuple(vec![]))];
    // Op 5, name, `true`, a list of 2 elements, the key, and the default.
    let record = |flag: u8, length: u8, default: &[u8]| {
        let mut body = vec![5, 0x22, flag, 0x17, length, 0x42];
        body.extend_from_slice(default);
        recs(0, 1, &body)
    };
    let parse = |data: &[u8]| parse_records(data, &atoms, &literals, &table);
    for bad in [
        recs(1, 0, &[]),
        recs(0, -1, &[]),
        recs(0, 1, &[6]),
        recs(0, 1, &[5, 0x02]),
        recs(0, 1, &[5, 0x01]),
        record(0x52, 0x20, &[0x00]),
        record(0x32, 0x10, &[0x00]),
        record(0x32, 0x20, &[0x03]),
        record(0x32, 0x20, &[0x47, 0x10]),
    ] {
        assert_eq!(
            parse(&bad).unwrap_err().to_string(),
            "malformed BEAM Recs chunk"
        );
    }
    let defs = parse(&record(0x32, 0x20, &[0x47, 0x00])).unwrap();
    assert!(defs[0].is_exported);
    assert_eq!(defs[0].defaults[..], [Some(Term::tuple(vec![]))]);
}
