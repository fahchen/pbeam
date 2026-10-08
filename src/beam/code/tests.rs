use super::{
    atom_at, atom_chunk_name, parse_atoms, parse_exports, parse_lambdas, parse_literals,
    read_literal, read_u32,
};
use crate::term::{AtomTable, Fun, Lambda, Map, Term};

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
    let mut table = AtomTable::new();
    assert!(read_u32(&[0, 1]).is_err());
    assert!(parse_atoms(&[], &mut table).is_err());
    assert!(parse_atoms(&(-1i32).to_be_bytes(), &mut table).is_err());
    assert!(atom_at(&[], 0).is_err());

    let mut malformed_atoms = (-2i32).to_be_bytes().to_vec();
    malformed_atoms.extend([0x30, b'a', b'b', b'c', 0xa0]);
    assert!(parse_atoms(&malformed_atoms, &mut table).is_err());
    assert!(parse_atoms(&[0xff, 0xff, 0xff, 0xff, 0x10, 0xff], &mut table).is_err());
    assert!(
        parse_atoms(&0i32.to_be_bytes(), &mut table)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn puts_atu8_atoms_into_the_table_in_chunk_order() {
    let mut table = AtomTable::new();
    // AtU8 with zeta, ok, and alpha: ok keeps its static index 506.
    let mut chunk = (-3i32).to_be_bytes().to_vec();
    chunk.extend([0x40, b'z', b'e', b't', b'a', 0x20, b'o', b'k']);
    chunk.extend([0x50, b'a', b'l', b'p', b'h', b'a']);
    let atoms = parse_atoms(&chunk, &mut table).unwrap();
    let indices: Vec<_> = atoms.iter().map(|atom| atom.index()).collect();
    assert_eq!(indices, [1150, 506, 1151]);
    assert_eq!(atoms[2].name(), "alpha");
    assert_eq!(table.get("zeta"), Some(atoms[0].clone()));
    // A second module gets the same index for an existing atom.
    assert_eq!(parse_atoms(&chunk, &mut table).unwrap(), atoms);
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
    let mut table = AtomTable::new();
    let mut chunk = (-2i32).to_be_bytes().to_vec();
    for last in *b"bd" {
        chunk.extend([0x28, 0]);
        chunk.extend(a255.as_bytes());
        chunk.push(last);
    }
    let atoms = parse_atoms(&chunk, &mut table).unwrap();
    assert_eq!(atoms[0].name(), a255);
    assert_eq!(atoms[0].index(), 1150);
    assert_eq!(atoms[1], atoms[0]);
    assert_eq!(table.get(&a255), Some(atoms[0].clone()));
}

#[test]
fn rejects_invalid_export_tables_and_indices() {
    let atoms = [AtomTable::new().put("constant").unwrap()];
    assert!(parse_exports(&[], &[]).is_err());
    assert!(parse_exports(&[0, 0, 0, 1], &[]).is_err());
    let exports = [0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 1];
    assert!(parse_exports(&exports, &atoms).is_err());
}

#[test]
fn reads_uncompressed_binary_literals() {
    let mut atoms = AtomTable::new();
    let mut table = 0u32.to_be_bytes().to_vec();
    table.extend(1u32.to_be_bytes());
    table.extend(8u32.to_be_bytes());
    table.extend([131, 109, 0, 0, 0, 2, b'o', b'k']);
    assert_eq!(
        parse_literals(&table, &mut atoms).unwrap(),
        vec![Some(Term::Binary(b"ok".to_vec()))]
    );
    assert_eq!(
        parse_literals(&[0, 0, 0, 0, 0, 0, 0, 0], &mut atoms).unwrap(),
        Vec::<Option<Term>>::new()
    );
    assert_eq!(read_literal(&[1], &mut atoms).unwrap(), None);
    assert!(read_literal(&[131, 109], &mut atoms).is_err());
    assert_eq!(
        read_literal(&[131, 107, 0, 1, b'\n'], &mut atoms).unwrap(),
        Some(Term::List(vec![Term::Integer(10)]))
    );
    assert!(read_literal(&[131, 107], &mut atoms).is_err());
    assert!(read_literal(&[131, 107, 0, 2, b'x'], &mut atoms).is_err());
}

#[test]
fn puts_literal_atoms_into_the_table_in_byte_order() {
    let mut atoms = AtomTable::new();
    // {pbeam_lit_b, #{pbeam_lit_a => 1, pbeam_lit_b => 2}}: ERTS 29.1.1 creates pbeam_lit_b
    // first, so maps:keys/1 of the map is [pbeam_lit_b,pbeam_lit_a].
    let mut bytes = vec![131, 104, 2, 119, 11];
    bytes.extend(b"pbeam_lit_b");
    bytes.extend([116, 0, 0, 0, 2, 119, 11]);
    bytes.extend(b"pbeam_lit_a");
    bytes.extend([97, 1, 119, 11]);
    bytes.extend(b"pbeam_lit_b");
    bytes.extend([97, 2]);
    let literal = read_literal(&bytes, &mut atoms).unwrap().unwrap();
    assert_eq!(atoms.get("pbeam_lit_b").unwrap().index(), 1150);
    assert_eq!(atoms.get("pbeam_lit_a").unwrap().index(), 1151);
    let (lit_a, lit_b) = (atom(&atoms, "pbeam_lit_a"), atom(&atoms, "pbeam_lit_b"));
    let pairs = vec![(lit_a, Term::Integer(1)), (lit_b.clone(), Term::Integer(2))];
    let map = Term::Map(Map::from_pairs(pairs).unwrap());
    assert_eq!(literal, Term::Tuple(vec![lit_b, map]));
    assert_eq!(
        format!("{literal:?}"),
        "Tuple([Atom(\"pbeam_lit_b\"), Map([(Atom(\"pbeam_lit_b\"), Integer(2)), \
         (Atom(\"pbeam_lit_a\"), Integer(1))])])"
    );
    // [pbeam_x, 1 bsl 64, pbeam_y]: the atoms after the unsupported SMALL_BIG_EXT are not
    // created.
    let mut unsupported = vec![131, 108, 0, 0, 0, 3, 119, 7];
    unsupported.extend(b"pbeam_x");
    unsupported.extend([110, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 119, 7]);
    unsupported.extend(b"pbeam_y");
    unsupported.push(106);
    assert_eq!(read_literal(&unsupported, &mut atoms).unwrap(), None);
    assert_eq!(atoms.get("pbeam_x").unwrap().index(), 1152);
    assert_eq!(atoms.get("pbeam_y"), None);
}

#[test]
fn reads_nested_literals_and_marks_unsupported_tags_at_any_depth() {
    let mut atoms = AtomTable::new();
    let mut read = |bytes: &[u8]| read_literal(bytes, &mut atoms);
    // [a|[b]] gives the same list as [a,b].
    let nested = read(&[
        131, 108, 0, 0, 0, 1, 119, 1, b'a', 108, 0, 0, 0, 1, 119, 1, b'b', 106,
    ])
    .unwrap();
    assert_eq!(
        read(&[131, 104, 2, 119, 2, b'o', b'k', 106]).unwrap(),
        Some(Term::Tuple(vec![
            Term::static_atom("ok"),
            Term::List(vec![])
        ]))
    );
    // NEW_FLOAT_EXT (70) 1.0 inside a tuple, inside a list, and as a list tail.
    let float = [70, 63, 240, 0, 0, 0, 0, 0, 0];
    assert_eq!(read(&[&[131, 104, 1][..], &float].concat()).unwrap(), None);
    assert_eq!(
        read(&[&[131, 108, 0, 0, 0, 1][..], &float, &[106]].concat()).unwrap(),
        None
    );
    assert_eq!(
        read(&[&[131, 108, 0, 0, 0, 1, 106][..], &float].concat()).unwrap(),
        None
    );
    // [[]|t] has an atom tail, and [a|[b|t]] keeps the tail of its list tail.
    let nil_head = read(&[131, 108, 0, 0, 0, 1, 106, 119, 1, b't']).unwrap();
    let tail = [108, 0, 0, 0, 1, 119, 1, b'b', 119, 1, b't'];
    let list_tail = read(&[&[131, 108, 0, 0, 0, 1, 119, 1, b'a'][..], &tail].concat()).unwrap();
    // A LIST_EXT of length 0 gives its tail (external.c:4581-4588).
    let empty = read(&[131, 108, 0, 0, 0, 0, 119, 1, b't']).unwrap();
    assert!(read(&[131, 119, 1, 0xff]).is_err());
    assert!(read(&[131, 106, 106]).is_err());
    assert!(read(&[131, 104]).is_err());
    // [a, b] with length 2, and a zero-length LIST_EXT whose tail is [].
    let proper = read(&[131, 108, 0, 0, 0, 2, 119, 1, b'a', 119, 1, b'b', 106]).unwrap();
    assert_eq!(
        read(&[131, 108, 0, 0, 0, 0, 106]).unwrap(),
        Some(Term::List(vec![]))
    );

    // #{b => [], a => {}} keeps its pairs in flatmap key order. The first literal made a
    // before b, so a has the lower atom index.
    let map = read(&[
        131, 116, 0, 0, 0, 2, 119, 1, b'b', 106, 119, 1, b'a', 104, 0,
    ])
    .unwrap()
    .unwrap();
    assert_eq!(
        format!("{map:?}"),
        "Map([(Atom(\"a\"), Tuple([])), (Atom(\"b\"), List([]))])"
    );
    assert_eq!(
        read(&[131, 116, 0, 0, 0, 0]).unwrap(),
        Some(Term::Map(Map::from_pairs(vec![]).unwrap()))
    );
    // A map key that `Map` does not support yet makes the literal unsupported: #{{} => []}.
    assert_eq!(read(&[131, 116, 0, 0, 0, 1, 104, 0, 106]).unwrap(), None);
    // ERTS rejects a MAP_EXT with a duplicate key: #{1 => [], 1 => []}.
    assert_eq!(
        read(&[131, 116, 0, 0, 0, 2, 97, 1, 106, 97, 1, 106])
            .unwrap_err()
            .to_string(),
        "duplicate key in BEAM map literal"
    );

    // SMALL_INTEGER_EXT: 1 and 255.
    assert_eq!(read(&[131, 97, 1]).unwrap(), Some(Term::Integer(1)));
    assert_eq!(read(&[131, 97, 255]).unwrap(), Some(Term::Integer(255)));
    // INTEGER_EXT: 256, -1, and the i32 limits, as term_to_binary/1 writes them.
    for (bytes, value) in [
        ([0, 0, 1, 0], 256),
        ([255, 255, 255, 255], -1),
        ([127, 255, 255, 255], i64::from(i32::MAX)),
        ([128, 0, 0, 0], i64::from(i32::MIN)),
    ] {
        let literal = [&[131, 98][..], &bytes].concat();
        assert_eq!(read(&literal).unwrap(), Some(Term::Integer(value)));
    }
    assert!(read(&[131, 98, 0, 0, 1]).is_err());

    // erlc writes #{1 => v, ..., 33 => v} as 131,116,0,0,0,33,97,1,119,1,118,...;
    // 33 keys give a hashmap in the ERTS order, from either input order.
    let mut literal = |keys: &mut dyn Iterator<Item = u8>| {
        let mut bytes = vec![131, 116, 0, 0, 0, 33];
        for key in keys {
            bytes.extend([97, key, 119, 1, b'v']);
        }
        read(&bytes).unwrap().unwrap()
    };
    let forward = literal(&mut (1..=33));
    assert_eq!(forward, literal(&mut (1..=33).rev()));
    assert!(
        format!("{forward:?}").starts_with(
            "Map([(Integer(18), Atom(\"v\")), (Integer(4), Atom(\"v\")), (Integer(12), "
        )
    );

    // ERTS decodes a term nested 1000000 levels deep; so does PBeam.
    let mut deep = vec![131];
    for _ in 0..1_000_000 {
        deep.extend([104, 1]);
    }
    deep.push(106);
    let term = read(&deep).unwrap().unwrap();
    assert!(format!("{term:?}").starts_with("Tuple([Tuple(["));

    let list = Some(Term::List(vec![atom(&atoms, "a"), atom(&atoms, "b")]));
    assert_eq!(nested, list);
    let t = || Box::new(atom(&atoms, "t"));
    assert_eq!(
        nil_head,
        Some(Term::ImproperList(vec![Term::List(vec![])], t()))
    );
    assert_eq!(
        list_tail,
        Some(Term::ImproperList(
            vec![atom(&atoms, "a"), atom(&atoms, "b")],
            t()
        ))
    );
    assert_eq!(empty, Some(*t()));
    assert_eq!(proper, list);
}

#[test]
fn reads_external_fun_literals_as_export_ext() {
    let mut atoms = AtomTable::new();
    let mut read = |bytes: &[u8]| read_literal(bytes, &mut atoms);
    // erlc writes `fun lists:reverse/1` as EXPORT_EXT (113), two atoms,
    // and SMALL_INTEGER_EXT.
    let mut reverse = vec![131, 113, 119, 5];
    reverse.extend(b"lists");
    reverse.extend([119, 7]);
    reverse.extend(b"reverse");
    reverse.extend([97, 1]);
    let reverse = read(&reverse).unwrap();
    // An atom tag that PBeam does not decode, such as ATOM_EXT (100),
    // makes the literal unsupported.
    assert_eq!(read(&[131, 113, 100, 0, 1, b'm']).unwrap(), None);
    // ERTS `dec_term` (`external.c:5006-5041`) fails for a module or
    // function that is not an atom, or an arity that is not an integer.
    for bytes in [
        &[131, 113, 106, 119, 1, b'f', 97, 1][..],
        &[131, 113, 119, 1, b'm', 106, 97, 1],
        &[131, 113, 119, 1, b'm', 119, 1, b'f', 106],
    ] {
        assert_eq!(
            read(bytes).unwrap_err().to_string(),
            "invalid BEAM external fun literal"
        );
    }
    // ERTS 29.1.1 loads an INTEGER_EXT arity in `0..=255`, as
    // `[98, 0, 0, 0, 1]` and `[98, 0, 0, 0, 255]`. It rejects -1 as a
    // corrupt literal table, and it has no defined result above 255.
    let export = |arity: &[u8]| [&[131, 113, 119, 1, b'm', 119, 1, b'f'][..], arity].concat();
    let integer_one = read(&export(&[98, 0, 0, 0, 1])).unwrap();
    let integer_max = read(&export(&[98, 0, 0, 0, 255])).unwrap();
    for arity in [&[98, 255, 255, 255, 255][..], &[98, 0, 0, 1, 0]] {
        assert_eq!(
            read(&export(arity)).unwrap_err().to_string(),
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
    let mut atoms = AtomTable::new();
    let mut parse = |bytes: &[u8]| parse_literals(bytes, &mut atoms);
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
    let mut table = AtomTable::new();
    let atoms = vec![table.put("fun_probe").unwrap(), table.put("f").unwrap()];
    let module = atoms[0].clone();
    // An entry of the form that erlc writes for `fun(X) -> {Tag, X} end`:
    // atom 2, total arity 2, label 127, index 0, one free variable, old
    // unique value 31263486. A count below the number of entries reads
    // only the counted entries, and atom index 0 is the ERTS non-value
    // slot.
    let entries = [[2, 2, 127, 0, 1, 31_263_486], [0, 0, 51, 1, 0, 7]];
    assert_eq!(
        parse_lambdas(&lambda_table(2, &entries), &atoms).unwrap(),
        vec![
            (Lambda::new(module.clone(), 0, 31_263_486, 127, 1), 1),
            (Lambda::new(module, 1, 7, 51, 0), 0),
        ]
    );
    assert_eq!(
        parse_lambdas(&lambda_table(1, &entries), &atoms)
            .unwrap()
            .len(),
        1
    );
    let error = |bytes: Vec<u8>| parse_lambdas(&bytes, &atoms).unwrap_err().to_string();
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
