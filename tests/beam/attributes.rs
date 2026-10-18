use super::{Cursor, native_declarations};

/// Return the `SMALL_ATOM_UTF8_EXT` bytes of a name.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(atom("ok"), [119, 2, b'o', b'k']);
/// ```
fn atom(name: &str) -> Vec<u8> {
    let mut bytes = vec![119, u8::try_from(name.len()).expect("a short name")];
    bytes.extend_from_slice(name.as_bytes());
    bytes
}

/// Return the `SMALL_TUPLE_EXT` bytes of elements.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(tuple(&[small_integer(1)]), [104, 1, 97, 1]);
/// ```
fn tuple(elements: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![104, u8::try_from(elements.len()).expect("a short tuple")];
    for element in elements {
        bytes.extend_from_slice(element);
    }
    bytes
}

/// Return the `LIST_EXT` bytes of elements and an empty tail.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(list(&[]), [108, 0, 0, 0, 0, 106]);
/// ```
fn list(elements: &[Vec<u8>]) -> Vec<u8> {
    list_with_tail(elements, &[106])
}

/// Return the `LIST_EXT` bytes of elements and a tail.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(
///     list_with_tail(&[small_integer(1)], &[106]),
///     [108, 0, 0, 0, 1, 97, 1, 106]
/// );
/// ```
fn list_with_tail(elements: &[Vec<u8>], tail: &[u8]) -> Vec<u8> {
    let mut bytes = vec![108];
    bytes.extend_from_slice(&u32::try_from(elements.len()).unwrap().to_be_bytes());
    for element in elements {
        bytes.extend_from_slice(element);
    }
    bytes.extend_from_slice(tail);
    bytes
}

/// Return the `SMALL_INTEGER_EXT` bytes of a value.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(small_integer(2), [97, 2]);
/// ```
fn small_integer(value: u8) -> Vec<u8> {
    vec![97, value]
}

/// Return the `SMALL_BIG_EXT` bytes of a value, as `vsn` uses them.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(small_big(&[1, 2]), [110, 2, 0, 1, 2]);
/// ```
fn small_big(magnitude: &[u8]) -> Vec<u8> {
    let mut bytes = vec![110, u8::try_from(magnitude.len()).unwrap(), 0];
    bytes.extend_from_slice(magnitude);
    bytes
}

/// Return the bytes of the `Attr` chunk of a list of name and value pairs.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(chunk(&[]), [131, 108, 0, 0, 0, 0, 106]);
/// ```
fn chunk(pairs: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let entries = pairs
        .iter()
        .map(|(name, value)| tuple(&[atom(name), value.clone()]))
        .collect::<Vec<_>>();
    let mut bytes = vec![131];
    bytes.extend_from_slice(&list(&entries));
    bytes
}

/// Return the value bytes of a `pbeam_native` attribute.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(declarations(&[("sha256", 1)]), list(&[tuple(&[atom("sha256"), small_integer(1)])]));
/// ```
fn declarations(pairs: &[(&str, u8)]) -> Vec<u8> {
    let entries = pairs
        .iter()
        .map(|(name, arity)| tuple(&[atom(name), small_integer(*arity)]))
        .collect::<Vec<_>>();
    list(&entries)
}

#[test]
fn reads_the_declaration_after_a_skipped_attribute() {
    // The compiler writes `vsn` with a big number, which the reader skips.
    let bytes = chunk(&[
        ("vsn", small_big(&[1, 2, 3])),
        (
            "pbeam_native",
            declarations(&[("sha256", 1), ("hidden", 0)]),
        ),
    ]);
    assert_eq!(
        native_declarations(&bytes).unwrap(),
        [("sha256".to_owned(), 1), ("hidden".to_owned(), 0)]
    );
}

#[test]
fn reads_a_latin1_atom_name() {
    let bytes = vec![
        131,
        // A `LIST_EXT` of one `ATOM_EXT` pair, which holds a Latin-1 name,
        // and the empty value and tail of the list.
        108, 0, 0, 0, 1, 104, 2, 100, 0, 12, b'p', b'b', b'e', b'a', b'm', b'_', b'n', b'a', b't',
        b'i', b'v', b'e', 106, 106,
    ];
    assert_eq!(native_declarations(&bytes).unwrap(), []);
}

#[test]
fn reads_each_arity_encoding_and_an_empty_declaration() {
    let bytes = chunk(&[("pbeam_native", list(&[])), ("other", small_integer(1))]);
    assert_eq!(native_declarations(&bytes).unwrap(), []);

    // A large arity takes `INTEGER_EXT`.
    let mut big = vec![98];
    big.extend_from_slice(&300u32.to_be_bytes());
    let bytes = chunk(&[(
        "pbeam_native",
        list(&[
            tuple(&[atom("wide"), big]),
            tuple(&[atom("small"), small_integer(9)]),
        ]),
    )]);
    assert_eq!(
        native_declarations(&bytes).unwrap(),
        [("wide".to_owned(), 300), ("small".to_owned(), 9)]
    );
}

#[test]
fn reads_declarations_across_list_segments() {
    // ERTS decodes the `LIST_EXT` segments of one list as that one list
    // (`external.c:4581-4614`). The `pbeam_native` attribute and the pairs of
    // its value can each follow a segment boundary.
    let value = list_with_tail(
        &[tuple(&[atom("sha256"), small_integer(1)])],
        &list_with_tail(&[tuple(&[atom("sha512"), small_integer(1)])], &[106]),
    );
    let attributes = list_with_tail(
        &[tuple(&[atom("other"), small_integer(1)])],
        &list_with_tail(&[tuple(&[atom("pbeam_native"), value])], &[106]),
    );
    let mut bytes = vec![131];
    bytes.extend_from_slice(&attributes);
    assert_eq!(
        native_declarations(&bytes).unwrap(),
        [("sha256".to_owned(), 1), ("sha512".to_owned(), 1)]
    );
}

#[test]
fn reads_an_improper_tail_and_refuses_a_missing_one() {
    // ERTS decodes an attribute list with an improper tail as that list
    // (`external.c:4581-4614`), so the reader skips the tail.
    let attributes = list_with_tail(
        &[tuple(&[
            atom("pbeam_native"),
            declarations(&[("sha256", 1)]),
        ])],
        &small_integer(1),
    );
    let mut bytes = vec![131];
    bytes.extend_from_slice(&attributes);
    assert_eq!(
        native_declarations(&bytes).unwrap(),
        [("sha256".to_owned(), 1)]
    );

    // A list that ends after its elements has no tail term, so the encoding
    // is bad.
    let attributes = list_with_tail(&[tuple(&[atom("other"), small_integer(1)])], &[]);
    let mut bytes = vec![131];
    bytes.extend_from_slice(&attributes);
    assert!(native_declarations(&bytes).is_err());
}

#[test]
fn skips_every_supported_tag() {
    // One attribute of each tag shape before the declaration.
    let mut new_pid = vec![88];
    new_pid.extend_from_slice(&atom("node"));
    new_pid.extend_from_slice(&[0; 12]);

    let mut pid = vec![103];
    pid.extend_from_slice(&atom("node"));
    pid.extend_from_slice(&[0; 9]);

    let mut v4_port = vec![120];
    v4_port.extend_from_slice(&atom("node"));
    v4_port.extend_from_slice(&[0; 12]);

    let mut new_port = vec![89];
    new_port.extend_from_slice(&atom("node"));
    new_port.extend_from_slice(&[0; 8]);

    let mut port = vec![102];
    port.extend_from_slice(&atom("node"));
    port.extend_from_slice(&[0; 5]);

    let mut newer_reference = vec![90, 0, 2];
    newer_reference.extend_from_slice(&atom("node"));
    newer_reference.extend_from_slice(&[0; 12]);

    let mut new_reference = vec![114, 0, 2];
    new_reference.extend_from_slice(&atom("node"));
    new_reference.extend_from_slice(&[0; 9]);

    let mut reference = vec![101];
    reference.extend_from_slice(&atom("node"));
    reference.extend_from_slice(&[0; 5]);

    let mut new_fun = vec![112];
    new_fun.extend_from_slice(&[0; 25]);
    new_fun.extend_from_slice(&1u32.to_be_bytes());
    new_fun.extend_from_slice(&atom("m"));
    new_fun.extend_from_slice(&small_integer(42));
    new_fun.extend_from_slice(&small_integer(7));
    new_fun.extend_from_slice(&pid_());
    new_fun.extend_from_slice(&tuple(&[small_integer(0)]));

    let mut export = vec![113];
    export.extend_from_slice(&atom("m"));
    export.extend_from_slice(&atom("f"));
    export.extend_from_slice(&small_integer(1));

    let mut structure = vec![67];
    structure.extend_from_slice(&1u32.to_be_bytes());
    structure.push(0);
    structure.extend_from_slice(&atom("m"));
    structure.extend_from_slice(&atom("f"));
    structure.extend_from_slice(&atom("field"));
    structure.extend_from_slice(&small_integer(3));

    let mut string = vec![107, 0, 2];
    string.extend_from_slice(b"hi");

    let mut bit_binary = vec![77];
    bit_binary.extend_from_slice(&1u32.to_be_bytes());
    bit_binary.push(1);
    bit_binary.push(0x80);

    // The atom references of a distribution encoding.
    let atom_cache_ref = vec![82, 0];
    let mut atom_internal_ref2 = vec![73];
    atom_internal_ref2.extend_from_slice(&[0; 2]);
    let mut atom_internal_ref3 = vec![75];
    atom_internal_ref3.extend_from_slice(&[0; 3]);

    let bytes = chunk(&[
        ("small_integer", small_integer(1)),
        ("integer", {
            let mut value = vec![98];
            value.extend_from_slice(&1u32.to_be_bytes());
            value
        }),
        ("float", {
            let mut value = vec![99];
            value.extend_from_slice(b"1.0");
            value.resize(1 + 31, b' ');
            value
        }),
        ("new_float", {
            let mut value = vec![70];
            value.extend_from_slice(&1.0f64.to_be_bytes());
            value
        }),
        ("atom_utf8", {
            let mut value = vec![118, 0, 2];
            value.extend_from_slice(b"ok");
            value
        }),
        ("small_atom_utf8", atom("ok")),
        ("atom_latin1", {
            let mut value = vec![100, 0, 2];
            value.extend_from_slice(b"ok");
            value
        }),
        ("small_atom_latin1", {
            let mut value = vec![115, 2];
            value.extend_from_slice(b"ok");
            value
        }),
        ("nil", vec![106]),
        ("string", string),
        ("binary", {
            let mut value = vec![109];
            value.extend_from_slice(&2u32.to_be_bytes());
            value.extend_from_slice(b"hi");
            value
        }),
        ("bit_binary", bit_binary),
        ("small_big", small_big(&[1, 2])),
        ("large_big", {
            let mut value = vec![111];
            value.extend_from_slice(&2u32.to_be_bytes());
            value.push(0);
            value.extend_from_slice(&[1, 2]);
            value
        }),
        ("new_pid", new_pid),
        ("pid", pid),
        ("v4_port", v4_port),
        ("new_port", new_port),
        ("port", port),
        ("newer_reference", newer_reference),
        ("new_reference", new_reference),
        ("reference", reference),
        ("new_fun", new_fun),
        ("export", export),
        ("struct", structure),
        ("atom_cache_ref", atom_cache_ref),
        ("atom_internal_ref2", atom_internal_ref2),
        ("atom_internal_ref3", atom_internal_ref3),
        ("tuple", tuple(&[small_integer(1)])),
        ("large_tuple", {
            let mut value = vec![105];
            value.extend_from_slice(&1u32.to_be_bytes());
            value.extend_from_slice(&small_integer(1));
            value
        }),
        ("map", {
            let mut value = vec![116];
            value.extend_from_slice(&1u32.to_be_bytes());
            value.extend_from_slice(&atom("k"));
            value.extend_from_slice(&small_integer(1));
            value
        }),
        (
            "nested",
            list(&[tuple(&[atom("a"), list(&[small_integer(1)])])]),
        ),
        ("pbeam_native", declarations(&[("sha256", 1)])),
    ]);
    assert_eq!(
        native_declarations(&bytes).unwrap(),
        [("sha256".to_owned(), 1)]
    );
}

/// Return the bytes of a pid of one node and twelve zero bytes.
fn pid_() -> Vec<u8> {
    let mut bytes = vec![88];
    bytes.extend_from_slice(&atom("node"));
    bytes.extend_from_slice(&[0; 12]);
    bytes
}

#[test]
fn accepts_the_attribute_shapes_that_erts_loads() {
    // ERTS decodes the one external term of the chunk and refuses a module
    // only when the encoding is bad (`asm_load.c:1086-1091`). It reads no
    // chunk of no bytes (`asm_load.c:1080`), and it loads a term of another
    // shape, so PBeam reads no declaration from one.
    let cases: [Vec<u8>; 14] = [
        // A payload of no bytes.
        Vec::new(),
        // A term that is not a list.
        vec![131, 97, 1],
        // An attribute that is not a {name, value} pair.
        vec![131, 108, 0, 0, 0, 1, 104, 1, 97, 1, 106],
        // An attribute that is not a tuple at all.
        vec![131, 108, 0, 0, 0, 1, 97, 1, 106],
        // An attribute whose name is not an atom.
        vec![131, 108, 0, 0, 0, 1, 104, 2, 97, 1, 106, 106],
        // An attribute pair of a large tuple.
        {
            let mut bytes = vec![131, 108, 0, 0, 0, 1, 105, 0, 0, 0, 2];
            bytes.extend_from_slice(&atom("vs"));
            bytes.push(106);
            bytes.push(106);
            bytes
        },
        // The four atom encodings of an attribute name.
        {
            let mut bytes = vec![131, 108, 0, 0, 0, 4];
            for name in [&[100u8, 0, 2][..], &[115, 2], &[118, 0, 2], &[119, 2]] {
                bytes.extend_from_slice(&[104, 2]);
                bytes.extend_from_slice(name);
                bytes.extend_from_slice(b"vs");
                bytes.push(106);
            }
            // The tail of the attribute list.
            bytes.push(106);
            bytes
        },
        // An attribute name that is not UTF-8, which the reader reports as
        // an attribute of another shape.
        vec![131, 108, 0, 0, 0, 1, 104, 2, 119, 2, 0xff, 0xfe, 106, 106],
        // Bytes after the term of the chunk.
        {
            let mut bytes = chunk(&[]);
            bytes.push(0);
            bytes
        },
        // A `pbeam_native` value that is not a list.
        chunk(&[("pbeam_native", small_integer(1))]),
        // A declaration pair of three elements.
        chunk(&[(
            "pbeam_native",
            list(&[tuple(&[atom("f"), small_integer(1), small_integer(2)])]),
        )]),
        // A declaration arity of a wrong tag.
        chunk(&[("pbeam_native", list(&[tuple(&[atom("f"), atom("one")])]))]),
        // A declaration arity that is negative.
        chunk(&[(
            "pbeam_native",
            list(&[tuple(&[atom("f"), {
                let mut value = vec![98];
                value.extend_from_slice(&(-1i32).to_be_bytes());
                value
            }])]),
        )]),
        // A pid inside an attribute value, whose node atom and identity
        // bytes are inside that value too.
        {
            let mut bytes = vec![131, 108, 0, 0, 0, 1, 104, 2, 119, 5];
            bytes.extend_from_slice(b"other");
            bytes.extend_from_slice(&pid_());
            bytes.push(106);
            bytes
        },
    ];
    for bytes in cases {
        assert_eq!(native_declarations(&bytes).unwrap(), [], "{bytes:?}");
    }
}

#[test]
fn reports_a_payload_that_erts_refuses() {
    // A version that is not the external term format, and a truncated term.
    assert!(native_declarations(&[130]).is_err());
    assert!(native_declarations(&[131, 108, 0, 0, 0, 1, 104, 2, 119]).is_err());

    // Each tag and each count that the ERTS walk refuses
    // (`external.c:6041-6540`), with the least bytes that reach the refusal.
    let refused: [(&str, Vec<u8>); 12] = [
        // `FUN_EXT`, which OTP 23 removed.
        ("fun", chunk(&[("other", vec![117])])),
        // The identity bytes of a pid whose node term ends at the end of the
        // attribute value, so the atom of the tail of the list must consume
        // them. The walk keeps the count across the elements of a container
        // and refuses the tail (`external.c:6194-6248`, `external.c:6249-6262`).
        (
            "pid across the value",
            vec![
                131, 108, 0, 0, 0, 1, 104, 2, 119, 5, b'o', b't', b'h', b'e', b'r', 88, 97, 0, 106,
            ],
        ),
        // `LOCAL_EXT`, which needs a matching block hash.
        ("local", chunk(&[("other", vec![121, 0, 0, 0, 0])])),
        // An atom name above 255 characters, which `pair` reads.
        ("attribute name", {
            let mut bytes = vec![131, 108, 0, 0, 0, 1, 104, 2, 100];
            bytes.extend_from_slice(&256u16.to_be_bytes());
            bytes.resize(bytes.len() + 256, b'a');
            bytes
        }),
        // An atom name above 255 characters, which the walk skips.
        (
            "atom",
            chunk(&[("other", {
                let mut value = vec![100];
                value.extend_from_slice(&256u16.to_be_bytes());
                value.resize(value.len() + 256, b'a');
                value
            })]),
        ),
        // A big number above `BIG_ARITY_MAX` digits of an `ErtsDigit`.
        (
            "big",
            chunk(&[("other", {
                let mut value = vec![111];
                value.extend_from_slice(&(65535u32 * 8 + 1).to_be_bytes());
                value.push(0);
                value
            })]),
        ),
        // A tuple above `ERTS_MAX_TUPLE_SIZE`.
        (
            "tuple",
            chunk(&[("other", {
                let mut value = vec![105];
                value.extend_from_slice(&(1u32 << 24).to_be_bytes());
                value
            })]),
        ),
        // A map above 2^31 - 1 pairs.
        (
            "map",
            chunk(&[("other", {
                let mut value = vec![116];
                value.extend_from_slice(&(1u32 << 31).to_be_bytes());
                value
            })]),
        ),
        // A reference above `ERTS_MAX_REF_NUMBERS` numbers.
        ("reference", chunk(&[("other", vec![114, 0, 6])])),
        // A fun above `MAX_ARG` free variables.
        (
            "new_fun",
            chunk(&[("other", {
                let mut value = vec![112];
                value.extend_from_slice(&[0; 25]);
                value.extend_from_slice(&256u32.to_be_bytes());
                value
            })]),
        ),
        // The node of a pid is not an atom.
        (
            "node",
            chunk(&[("other", vec![88, 106, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])]),
        ),
        // A tag that ERTS does not accept.
        ("tag", chunk(&[("other", vec![200])])),
    ];
    for (name, bytes) in refused {
        assert!(native_declarations(&bytes).is_err(), "{name}");
    }
}

#[test]
fn skips_a_deep_term_without_recursion() {
    // A list of 10000 nested tuples.
    let mut value = small_integer(1);
    for _ in 0..10_000 {
        value = tuple(&[value]);
    }
    assert_eq!(native_declarations(&chunk(&[("deep", value)])).unwrap(), []);
}

#[test]
fn reads_the_attribute_chunk_of_a_compiled_module() {
    // The compiler writes a `vsn` attribute first. The module declares no
    // native function, so the reader skips `vsn` and gives an empty list.
    let bytes = include_bytes!("../../fixtures/policy/Elixir.ElixirPolicy.beam");
    assert_eq!(
        native_declarations(attr_chunk(bytes).expect("the module has an Attr chunk")).unwrap(),
        []
    );
}

/// Return the payload of the `Attr` chunk of a BEAM file.
///
/// Each chunk head is an identifier of four bytes, a size of four bytes,
/// and a body that pads to a multiple of four bytes
/// (`beam_file.c:load_code`, `beam_file.c:beamfile_read`).
fn attr_chunk(bytes: &[u8]) -> Option<&[u8]> {
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_be_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if id == b"Attr" {
            return bytes.get(pos + 8..pos + 8 + size);
        }
        pos += 8 + ((size + 3) & !3);
    }
    None
}

#[test]
fn reads_the_cursor_directly() {
    let bytes = [119, 2, b'o', b'k'];
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(cursor.atom().unwrap(), "ok");
    assert!(cursor.byte().is_err());
}
