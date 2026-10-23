use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;

/// Return a scheduler whose current process is a new root process.
fn scheduler() -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
}

/// Return the atom of `name` in the table of `scheduler`.
fn atom(scheduler: &mut Scheduler, name: &str) -> Term {
    Term::Atom(scheduler.atoms.put(name).unwrap())
}

/// Return the binary bytes of a `BifResult::Value`.
fn value_bytes(result: BifResult) -> Vec<u8> {
    match result {
        BifResult::Value(Term::Binary(ref bytes)) => bytes.bit_bytes().to_vec(),
        other => panic!("{other:?}"),
    }
}

/// Return the value of a `BifResult::Value`.
fn value(result: BifResult) -> Term {
    match result {
        BifResult::Value(term) => term,
        other => panic!("{other:?}"),
    }
}

/// Return the tuple elements of a `BifResult::Raise` of class `error`.
fn raised_reason(result: BifResult) -> Vec<Term> {
    match result {
        BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::Tuple(ref elements),
        } => elements.to_vec(),
        other => panic!("{other:?}"),
    }
}

/// Return the value of the map key `name`, read through `atoms`.
fn map_field<'a>(term: &'a Term, name: &str, atoms: &AtomTable) -> Option<&'a Term> {
    let Term::Map(map) = term else {
        return None;
    };
    map.iter().find_map(|(key, value)| match key {
        Term::Atom(found) if atoms.name_of(found) == name => Some(value),
        _ => None,
    })
}

/// Assert the raw error tuple `{Tag, #{c_file_name => File, c_file_line_num
/// => Line, c_function_arg_num => Argument}, Message}` of a raise.
fn assert_c_error(result: BifResult, atoms: &AtomTable, expected: (&str, &str, i64, i64, &str)) {
    let (tag, file, line, argument, message) = expected;
    let elements = raised_reason(result);
    assert_eq!(elements.len(), 3);
    assert!(matches!(&elements[0], Term::Atom(atom) if atoms.name_of(atom) == tag));
    let map = &elements[1];
    assert_eq!(
        map_field(map, "c_file_name", atoms)
            .map(|file| match file {
                Term::Nil | Term::Cons(_) => {
                    let mut text = String::new();
                    for byte in file.list_iter().unwrap() {
                        text.push(char::from(
                            u8::try_from(byte.small_integer().unwrap()).unwrap(),
                        ));
                    }
                    text
                }
                other => panic!("{other:?}"),
            })
            .as_deref(),
        Some(file)
    );
    assert_eq!(
        map_field(map, "c_file_line_num", atoms),
        Some(&Term::Integer(line))
    );
    assert_eq!(
        map_field(map, "c_function_arg_num", atoms),
        Some(&Term::Integer(argument))
    );
    let bytes: Vec<u8> = elements[2]
        .list_iter()
        .expect("the message is a list")
        .map(|byte| u8::try_from(byte.small_integer().unwrap()).unwrap())
        .collect();
    assert_eq!(String::from_utf8(bytes).unwrap(), message);
}

/// The pinned ERTS digests of `abc` for each digest that PBeam computes.
const DIGESTS: [(&str, &[u8]); 7] = [
    (
        "sha",
        &[
            169, 153, 62, 54, 71, 6, 129, 106, 186, 62, 37, 113, 120, 80, 194, 108, 156, 208, 216,
            157,
        ],
    ),
    (
        "sha224",
        &[
            35, 9, 125, 34, 52, 5, 216, 34, 134, 66, 164, 119, 189, 162, 85, 179, 42, 173, 188,
            228, 189, 160, 179, 247, 227, 108, 157, 167,
        ],
    ),
    (
        "sha256",
        &[
            186, 120, 22, 191, 143, 1, 207, 234, 65, 65, 64, 222, 93, 174, 34, 35, 176, 3, 97, 163,
            150, 23, 122, 156, 180, 16, 255, 97, 242, 0, 21, 173,
        ],
    ),
    (
        "sha384",
        &[
            203, 0, 117, 63, 69, 163, 94, 139, 181, 160, 61, 105, 154, 198, 80, 7, 39, 44, 50, 171,
            14, 222, 209, 99, 26, 139, 96, 90, 67, 255, 91, 237, 128, 134, 7, 43, 161, 231, 204,
            35, 88, 186, 236, 161, 52, 200, 37, 167,
        ],
    ),
    (
        "sha512",
        &[
            221, 175, 53, 161, 147, 97, 122, 186, 204, 65, 115, 73, 174, 32, 65, 49, 18, 230, 250,
            78, 137, 169, 126, 162, 10, 158, 238, 230, 75, 85, 211, 154, 33, 146, 153, 42, 39, 79,
            193, 168, 54, 186, 60, 35, 163, 254, 235, 189, 69, 77, 68, 35, 100, 60, 232, 14, 42,
            154, 201, 79, 165, 76, 164, 159,
        ],
    ),
    (
        "sha512_224",
        &[
            70, 52, 39, 15, 112, 123, 106, 84, 218, 174, 117, 48, 70, 8, 66, 226, 14, 55, 237, 38,
            92, 238, 233, 164, 62, 137, 36, 170,
        ],
    ),
    (
        "sha512_256",
        &[
            83, 4, 142, 38, 129, 148, 30, 249, 155, 46, 41, 183, 107, 76, 125, 171, 228, 194, 208,
            198, 52, 252, 109, 70, 224, 226, 241, 49, 7, 231, 175, 35,
        ],
    ),
];

/// The pinned ERTS PBKDF2-HMAC values of `password`/`salt` with 1 and 2
/// iterations and a 20-byte key, for each digest that PBeam computes.
const PBKDF2_ONE: [&[u8]; 7] = [
    &[
        12, 96, 200, 15, 150, 31, 14, 113, 243, 169, 181, 36, 175, 96, 18, 6, 47, 224, 55, 166,
    ],
    &[
        60, 25, 140, 189, 185, 70, 75, 120, 87, 150, 107, 208, 91, 123, 201, 43, 193, 204, 78, 110,
    ],
    &[
        18, 15, 182, 207, 252, 248, 179, 44, 67, 231, 34, 82, 86, 196, 248, 55, 168, 101, 72, 201,
    ],
    &[
        192, 225, 79, 6, 228, 158, 50, 215, 63, 159, 82, 221, 241, 208, 197, 199, 25, 22, 9, 35,
    ],
    &[
        134, 127, 112, 207, 26, 222, 2, 207, 243, 117, 37, 153, 163, 165, 61, 196, 175, 52, 199,
        166,
    ],
    &[
        179, 74, 182, 38, 39, 106, 97, 206, 25, 210, 236, 180, 199, 225, 95, 129, 152, 162, 152,
        154,
    ],
    &[
        75, 106, 99, 17, 125, 62, 192, 3, 38, 36, 97, 96, 130, 193, 193, 145, 47, 86, 250, 95,
    ],
];

const PBKDF2_TWO: [&[u8]; 7] = [
    &[
        234, 108, 1, 77, 199, 45, 111, 140, 205, 30, 217, 42, 206, 29, 65, 240, 216, 222, 137, 87,
    ],
    &[
        147, 32, 15, 250, 150, 197, 119, 109, 56, 250, 16, 171, 223, 143, 91, 252, 0, 84, 185, 113,
    ],
    &[
        174, 77, 12, 149, 175, 107, 70, 211, 45, 10, 223, 249, 40, 240, 109, 208, 42, 48, 63, 142,
    ],
    &[
        84, 247, 117, 198, 215, 144, 242, 25, 48, 69, 145, 98, 252, 83, 93, 191, 4, 169, 57, 24,
    ],
    &[
        225, 217, 193, 106, 166, 129, 112, 138, 69, 245, 199, 196, 226, 21, 206, 182, 110, 1, 26,
        46,
    ],
    &[
        184, 135, 138, 197, 228, 80, 156, 22, 92, 27, 80, 137, 97, 250, 60, 58, 252, 239, 63, 55,
    ],
    &[
        252, 253, 16, 140, 153, 204, 136, 142, 192, 175, 159, 24, 72, 133, 175, 245, 240, 45, 25,
        169,
    ],
];

#[test]
fn gives_the_pinned_erts_digest_for_each_supported_type() {
    let mut scheduler = scheduler();
    for (name, expected) in DIGESTS {
        let digest = atom(&mut scheduler, name);
        let data = Term::binary(b"abc".to_vec());
        let result = hash_nif(&mut scheduler, &digest, &data).unwrap();
        assert_eq!(value_bytes(result), expected, "{name}");
    }
}

#[test]
fn raises_the_erts_error_for_a_bad_digest_type_or_data() {
    let mut scheduler = scheduler();
    let data = Term::binary(b"abc".to_vec());
    let nosuch = atom(&mut scheduler, "nosuch");
    let result = hash_nif(&mut scheduler, &nosuch, &data).unwrap();
    assert_c_error(
        result,
        &scheduler.atoms.clone(),
        ("badarg", "hash.c", 111, 0, "Bad digest type"),
    );
    let result = hash_nif(&mut scheduler, &Term::Integer(1), &data).unwrap();
    assert_c_error(
        result,
        &scheduler.atoms.clone(),
        ("badarg", "hash.c", 111, 0, "Bad digest type"),
    );
    let sha = atom(&mut scheduler, "sha");
    let result = hash_nif(&mut scheduler, &sha, &Term::Integer(1)).unwrap();
    assert_c_error(
        result,
        &scheduler.atoms.clone(),
        ("badarg", "hash.c", 118, 1, "Not iolist"),
    );
}

#[test]
fn raises_not_iolist_for_hash_update_data_that_is_not_a_binary() {
    // crypto:hash_update/2 converts the data first (crypto.erl:1186), so
    // only a direct NIF call reaches hash.c:221-222.
    let mut scheduler = scheduler();
    let sha = atom(&mut scheduler, "sha");
    let context = value(hash_init_nif(&mut scheduler, &sha).unwrap());
    let result = hash_update_nif(&mut scheduler, &context, &Term::Integer(1)).unwrap();
    assert_c_error(
        result,
        &scheduler.atoms.clone(),
        ("badarg", "hash.c", 222, 1, "Not iolist"),
    );
}

#[test]
fn gives_the_pinned_erts_pbkdf2_value_for_each_eligible_digest() {
    let mut scheduler = scheduler();
    let password = Term::binary(b"password".to_vec());
    let salt = Term::binary(b"salt".to_vec());
    for (index, (name, _)) in DIGESTS.iter().enumerate() {
        let digest = atom(&mut scheduler, name);
        for (iterations, expected) in [(1, PBKDF2_ONE[index]), (2, PBKDF2_TWO[index])] {
            let result = pbkdf2_hmac_nif(
                &mut scheduler,
                &digest,
                &password,
                &salt,
                &Term::Integer(iterations),
                &Term::Integer(20),
            )
            .unwrap();
            assert_eq!(value_bytes(result), expected, "{name} {iterations}");
        }
    }
}

#[test]
fn checks_the_pbkdf2_arguments_in_the_erts_order() {
    let mut scheduler = scheduler();
    let sha = atom(&mut scheduler, "sha");
    let password = Term::binary(b"password".to_vec());
    let salt = Term::binary(b"salt".to_vec());
    let text = Term::list_from(
        b"password"
            .iter()
            .map(|&byte| Term::Integer(i64::from(byte))),
    );
    type Case = (
        Term,
        Term,
        Term,
        Term,
        Term,
        (&'static str, &'static str, i64, i64, &'static str),
    );
    let cases: [Case; 11] = [
        (
            sha.clone(),
            password.clone(),
            salt.clone(),
            Term::Float(1.0),
            Term::Integer(16),
            ("badarg", "pbkdf2_hmac.c", 77, 3, "Not integer"),
        ),
        (
            sha.clone(),
            password.clone(),
            salt.clone(),
            Term::Integer(-1),
            Term::Integer(16),
            ("badarg", "pbkdf2_hmac.c", 77, 3, "Not integer"),
        ),
        (
            sha.clone(),
            password.clone(),
            salt.clone(),
            Term::Integer(0),
            Term::Integer(16),
            ("badarg", "pbkdf2_hmac.c", 79, 3, "Must be > 0"),
        ),
        (
            sha.clone(),
            password.clone(),
            salt.clone(),
            Term::Integer(1),
            Term::Float(1.0),
            ("badarg", "pbkdf2_hmac.c", 82, 4, "Not integer"),
        ),
        (
            sha.clone(),
            password.clone(),
            salt.clone(),
            Term::Integer(1),
            Term::Integer(0),
            ("badarg", "pbkdf2_hmac.c", 84, 4, "Must be > 0"),
        ),
        (
            atom(&mut scheduler, "md4"),
            password.clone(),
            salt.clone(),
            Term::Integer(1),
            Term::Integer(16),
            ("badarg", "pbkdf2_hmac.c", 38, 0, "md.p is not NULL"),
        ),
        (
            atom(&mut scheduler, "md5"),
            password.clone(),
            salt.clone(),
            Term::Integer(1),
            Term::Integer(16),
            ("badarg", "pbkdf2_hmac.c", 40, 0, "Not eligible digest type"),
        ),
        (
            atom(&mut scheduler, "nosuch"),
            password.clone(),
            salt.clone(),
            Term::Integer(1),
            Term::Integer(16),
            ("badarg", "pbkdf2_hmac.c", 36, 0, "Bad digest type"),
        ),
        (
            Term::Integer(1),
            password.clone(),
            salt.clone(),
            Term::Integer(1),
            Term::Integer(16),
            ("badarg", "pbkdf2_hmac.c", 36, 0, "Bad digest type"),
        ),
        (
            sha.clone(),
            text.clone(),
            salt.clone(),
            Term::Integer(1),
            Term::Integer(16),
            ("badarg", "pbkdf2_hmac.c", 43, 1, "Not binary"),
        ),
        (
            sha.clone(),
            password.clone(),
            text.clone(),
            Term::Integer(1),
            Term::Integer(16),
            ("badarg", "pbkdf2_hmac.c", 46, 2, "Not binary"),
        ),
    ];
    for (digest, pass, salt_term, iterations, key_length, expected) in cases {
        let result = pbkdf2_hmac_nif(
            &mut scheduler,
            &digest,
            &pass,
            &salt_term,
            &iterations,
            &key_length,
        )
        .unwrap();
        assert_c_error(result, &scheduler.atoms.clone(), expected);
    }
}

#[test]
fn truncates_the_pbkdf2_iterations_to_a_c_int_as_erts() {
    let mut scheduler = scheduler();
    let digest = atom(&mut scheduler, "sha");
    let password = Term::binary(b"password".to_vec());
    let salt = Term::binary(b"salt".to_vec());
    let one = pbkdf2_hmac_nif(
        &mut scheduler,
        &digest,
        &password,
        &salt,
        &Term::Integer(1),
        &Term::Integer(20),
    )
    .unwrap();
    let wrapped = pbkdf2_hmac_nif(
        &mut scheduler,
        &digest,
        &password,
        &salt,
        &Term::Integer(i64::from(u32::MAX) + 2),
        &Term::Integer(20),
    )
    .unwrap();
    assert_eq!(value_bytes(wrapped), value_bytes(one));
    let zero = pbkdf2_hmac_nif(
        &mut scheduler,
        &digest,
        &password,
        &salt,
        &Term::Integer(1 << 32),
        &Term::Integer(20),
    )
    .unwrap();
    assert_c_error(
        zero,
        &scheduler.atoms.clone(),
        ("error", "pbkdf2_hmac.c", 63, -1, "Low-level call failed"),
    );
    let negative = pbkdf2_hmac_nif(
        &mut scheduler,
        &digest,
        &password,
        &salt,
        &Term::Integer(1 << 31),
        &Term::Integer(20),
    )
    .unwrap();
    assert_c_error(
        negative,
        &scheduler.atoms.clone(),
        ("error", "pbkdf2_hmac.c", 63, -1, "Low-level call failed"),
    );
}

#[test]
fn bounds_the_key_length_to_a_c_int() {
    let mut scheduler = scheduler();
    let sha = atom(&mut scheduler, "sha256");
    let password = Term::binary(b"p".to_vec());
    let salt = Term::binary(b"s".to_vec());
    // The reservation is virtual: no page of it is written.
    let negative = pbkdf2_hmac_nif(
        &mut scheduler,
        &sha,
        &password,
        &salt,
        &Term::Integer(1),
        &Term::Integer(1 << 31),
    )
    .unwrap();
    assert_c_error(
        negative,
        &scheduler.atoms.clone(),
        ("error", "pbkdf2_hmac.c", 63, -1, "Low-level call failed"),
    );
    let wrapped = pbkdf2_hmac_nif(
        &mut scheduler,
        &sha,
        &password,
        &salt,
        &Term::Integer(1),
        &Term::Integer((1 << 32) + 1),
    );
    assert_int_max_error(wrapped);
    let zero = pbkdf2_hmac_nif(
        &mut scheduler,
        &sha,
        &password,
        &salt,
        &Term::Integer(1),
        &Term::Integer(1 << 32),
    );
    assert_int_max_error(zero);
}

fn assert_int_max_error(result: Result<BifResult, BeamError>) {
    let Err(error) = result else {
        panic!("a key length above INT_MAX stops the run");
    };
    assert!(
        format!("{error:?}").contains("longer than INT_MAX bytes"),
        "{error:?}"
    );
}

#[test]
fn gives_the_allocation_error_of_a_key_length_that_does_not_fit() {
    let mut scheduler = scheduler();
    let sha = atom(&mut scheduler, "sha");
    let result = pbkdf2_hmac_nif(
        &mut scheduler,
        &sha,
        &Term::binary(b"password".to_vec()),
        &Term::binary(b"salt".to_vec()),
        &Term::Integer(1 << 32),
        &Term::BigInt(std::sync::Arc::new(num_bigint::BigInt::from(u64::MAX))),
    )
    .unwrap();
    assert_c_error(
        result,
        &scheduler.atoms.clone(),
        ("error", "pbkdf2_hmac.c", 56, -1, "Can't allocate binary"),
    );
}

#[test]
fn hashes_keys_longer_than_the_digest_block() {
    let mut long = Vec::new();
    Hash::Sha256
        .pbkdf2(&[7; 100], b"salt", 1, 32, &mut long)
        .unwrap();
    let mut short = Vec::new();
    Hash::Sha256
        .pbkdf2(b"password", b"salt", 1, 32, &mut short)
        .unwrap();
    assert_eq!(long.len(), 32);
    assert_ne!(long, short);
}

#[test]
fn returns_a_value_below_the_range() {
    for range in [1u8, 2, 10, 255] {
        let value = random_below(&[range]).unwrap();
        let number = value
            .iter()
            .fold(0u16, |number, byte| (number << 8) | u16::from(*byte));
        assert!(number < u16::from(range), "{range} {value:?}");
    }
    let range = [1, 0, 0, 0, 0, 0, 0, 0, 0];
    for _ in 0..100 {
        let value = random_below(&range).unwrap();
        assert!(value.len() <= 9);
        let mut padded = vec![0; range.len() - value.len()];
        padded.extend_from_slice(&value);
        assert!(padded.as_slice() < range.as_slice());
    }
    assert_eq!(random_below(&[0, 0]), None);
    assert_eq!(random_below(&[]), None);
}

#[test]
fn strong_rand_range_gives_a_binary_or_false_as_erts() {
    assert_eq!(
        value(strong_rand_range_nif(&Term::binary(vec![1]))),
        Term::binary(vec![])
    );
    assert_eq!(
        value(strong_rand_range_nif(&Term::binary(vec![0]))),
        Term::static_atom("false")
    );
    assert_eq!(
        value(strong_rand_range_nif(&Term::binary(vec![]))),
        Term::static_atom("false")
    );
    for range in [2u8, 10, 255] {
        let Term::Binary(ref value) = value(strong_rand_range_nif(&Term::binary(vec![range])))
        else {
            panic!("the range {range} gives a binary");
        };
        let number = value
            .bit_bytes()
            .iter()
            .fold(0u16, |number, byte| (number << 8) | u16::from(*byte));
        assert!(number < u16::from(range), "{range} {value:?}");
    }
    assert!(matches!(
        strong_rand_range_nif(&Term::Integer(10)),
        BifResult::Raise { .. }
    ));
}

#[test]
fn gives_the_pinned_erts_info() {
    let mut scheduler = scheduler();
    let BifResult::Value(info) = info_nif(&mut scheduler).unwrap() else {
        panic!("info_nif gives a map");
    };
    let atoms = scheduler.atoms.clone();
    assert_eq!(
        map_field(&info, "compile_type", &atoms),
        Some(&Term::Atom(Atom::from_static("normal").unwrap()))
    );
    assert_eq!(
        map_field(&info, "link_type", &atoms).and_then(|term| match term {
            Term::Atom(atom) => Some(atoms.name_of(atom).to_owned()),
            _ => None,
        }),
        Some("static".to_owned())
    );
    assert_eq!(
        map_field(&info, "cryptolib_version_compiled", &atoms),
        Some(&string(openssl_version()))
    );
    assert_eq!(
        map_field(&info, "cryptolib_version_linked", &atoms),
        Some(&string(openssl_version()))
    );
    assert_eq!(
        map_field(&info, "fips_provider_available", &atoms),
        Some(&Term::static_atom("false"))
    );
    assert!(openssl_version().starts_with("OpenSSL 3.5.5 "));
    let BifResult::Value(lib) = info_lib() else {
        panic!("info_lib gives a list");
    };
    let expected = Term::list_from([Term::tuple(vec![
        Term::binary(b"OpenSSL".to_vec()),
        Term::Integer(0x3050_0050),
        Term::binary(openssl_version().as_bytes().to_vec()),
    ])]);
    assert_eq!(lib, expected);
}

#[test]
fn reads_unsigned_values_and_text_as_erts_nif_arguments() {
    assert_eq!(unsigned(&Term::Integer(0)), Some(0));
    assert_eq!(unsigned(&Term::Integer(-1)), None);
    assert_eq!(unsigned(&Term::Float(1.0)), None);
    assert_eq!(
        unsigned(&Term::BigInt(std::sync::Arc::new(
            num_bigint::BigInt::from(u64::MAX)
        ))),
        Some(u64::MAX)
    );
    assert_eq!(
        unsigned(&Term::BigInt(std::sync::Arc::new(
            num_bigint::BigInt::from(u64::MAX) + 1
        ))),
        None
    );
    assert_eq!(
        string("ab"),
        Term::list_from(vec![Term::Integer(97), Term::Integer(98)])
    );
}
