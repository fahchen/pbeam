use std::sync::Arc;

use super::{internal_hash, make_hash, make_hash2, mix64};
use crate::term::{
    Atom, AtomTable, Binary, Fun, Lambda, Map, MatchState, PortId, ProcessId, Reference, Term,
};

#[test]
#[should_panic(expected = "a match context reached a term operation")]
fn panics_for_a_match_context() {
    // The interpreter gives a runtime error before a match context can
    // reach a term operation.
    internal_hash(&Term::MatchContext(Arc::new(MatchState::new(
        Binary::from(b"a"),
    ))));
}

fn atom(name: &str) -> Term {
    Term::static_atom(name)
}

fn integers(values: &[i64]) -> Term {
    Term::list_from(values.iter().map(|value| Term::Integer(*value)))
}

fn map(pairs: impl IntoIterator<Item = (Term, Term)>) -> Term {
    Term::Map(
        Map::from_pairs(pairs.into_iter().collect(), &crate::term::test_atom_table()).unwrap(),
    )
}

fn big(sign: i64, shift: u32) -> Term {
    Term::from_bigint(num_bigint::BigInt::from(sign) << shift)
}

fn improper(cells: Vec<Term>, tail: Term) -> Term {
    Term::list_with_tail(cells, tail)
}

#[test]
fn mixes_a_word_as_erts_ihash_mix64() {
    assert_eq!(mix64(0x1F), 7938660269479274665);
    // true has index 1: its ERTS word is (1 << 6) | 0xB = 0x4B.
    assert_eq!(mix64(0x4B), 14558535449716669319);
}

#[test]
fn hashes_each_term_kind_as_pinned_erts() {
    // Each value is erts_debug:get_internal_state({internal_hash, Term}) on ERTS 29.1.1.
    let tuple = |elements: Vec<Term>| Term::tuple(elements);
    let cases = [
        (atom("ok"), 4036026909208831136),
        (Term::Integer(1), 7938660269479274665),
        (Term::Integer(-1), 7256831767414464289),
        (Term::nil(), 16046243290777390298),
        (tuple(vec![]), 4510301282025278020),
        (
            tuple(vec![atom("ok"), Term::Integer(1)]),
            13883435329789904289,
        ),
        (
            tuple(vec![Term::Integer(1), tuple(vec![Term::Integer(2)])]),
            15167167310543595139,
        ),
        (Term::list_from(vec![atom("ok")]), 466713979249896199),
        (
            integers(b"hello".map(i64::from).as_slice()),
            5671500630603279798,
        ),
        (integers(&[300]), 15858254366034436735),
        (
            Term::list_from(vec![tuple(vec![atom("ok")])]),
            15994800526372216946,
        ),
        (Term::list_from(vec![Term::nil()]), 7939938580689537527),
        (
            Term::list_from(vec![Term::binary(vec![]), Term::Integer(2)]),
            2437268029134553953,
        ),
        (
            Term::list_from(vec![Term::Integer(1), tuple(vec![]), Term::Integer(2)]),
            11634306265473751121,
        ),
        (Term::binary(vec![]), 10873612744977862784),
        (Term::binary(vec![1, 2, 3]), 3963608694498431467),
        (
            Term::binary(b"0123456789abcde".to_vec()),
            13603490536549166064,
        ),
        (
            Term::binary(b"0123456789abcdef".to_vec()),
            8513549014846254092,
        ),
        (
            Term::binary(b"0123456789abcdefg".to_vec()),
            11292494362027136432,
        ),
        (Term::Integer(1 << 59), 10054563828903259082),
        (Term::Integer(-(1 << 59) - 1), 12317164193119007090),
        (Term::Integer(i64::MAX), 1866222494570913558),
        (Term::Integer(i64::MIN), 11874823041652698399),
        (map([]), 9376759162891905861),
        (
            map([
                (atom("ok"), Term::nil()),
                (Term::Integer(1), Term::binary(vec![])),
            ]),
            13992029790355293803,
        ),
        // maps:from_list([{I, I} || I <- lists:seq(1, 33)]): a HEAD_BITMAP root.
        (
            map((1..=33).map(|key| (Term::Integer(key), Term::Integer(key)))),
            178448403113005431,
        ),
        // maps:from_list([{I, [I]} || I <- lists:seq(1, 200)]): a HEAD_ARRAY root
        // with inner nodes.
        (
            map((1..=200).map(|key| (Term::Integer(key), integers(&[key])))),
            5428453677902343107,
        ),
        // <0.10.0> has the pid word 0x503, so its ERTS pid data is 0x50.
        (Term::Pid(ProcessId(0x50)), 8954564308403532307),
        // #Ref<0.4116665985.1537736705.33174> has the numbers 0x8196, 0x5BA80001, and
        // 0xF55F5681: the counter value 0xF55F56815BA88196 of scheduler 1.
        (
            Term::Reference(Reference(0xF55F_5681_5BA8_8196)),
            11474811932501652611,
        ),
        // Bignums of one, two, three, and four 64-bit digits.
        (Term::Integer(1 << 62), 2820654030469875806),
        (big(1, 64), 1273177175058809839),
        (big(-1, 64), 773736285751472253),
        (big(1, 128), 9267933053641204122),
        (
            Term::from_bigint((num_bigint::BigInt::from(1) << 192u32) + 7),
            15715561134050055630,
        ),
    ];
    for (term, hash) in cases {
        assert_eq!(internal_hash(&term), hash, "{term:?}");
    }
}

#[test]
fn hashes_a_bitstring_by_its_bits_as_pinned_erts() {
    // Each value is erts_debug:get_internal_state({internal_hash, Term}) on ERTS 29.1.1.
    // ERTS mixes the size in bits, the whole bytes, and the tail bits
    // (`erl_term_hashing.c:2029-2091`).
    let bits = |bytes: &[u8], size| Term::Binary(Binary::from(bytes).bits(0, size));
    let mut tail = b"0123456789abcdefg".to_vec();
    tail.push(0xA0);
    let cases = [
        (bits(&[0x20], 3), 2772974853977271676),
        (Term::binary(vec![0x20]), 10090078442021580778),
        (Term::binary(b"abc".to_vec()), 11766418651593861201),
        // <<_:4, A:12/bits, _/bits>> = <<"abc">>, A = <<22,2:4>>.
        (
            Term::Binary(Binary::from(b"abc".to_vec()).bits(4, 16)),
            12847386933134786152,
        ),
        (bits(&[0x10], 4), 5341495853110203383),
        (bits(&[0], 3), 2512535726010557823),
        (bits(&tail, 139), 6506100538193254079),
    ];
    for (term, hash) in cases {
        assert_eq!(internal_hash(&term), hash, "{term:?}");
    }
    // The bits after the size do not change the hash, as they do not change
    // the equality.
    assert_eq!(internal_hash(&bits(&[0x3F], 3)), 2772974853977271676);
}

/// Return a local fun of `lists` with a fun entry and free variables.
fn local(index: i32, old_unique: i32, arity: u32, free: Vec<Term>) -> Term {
    let module = Atom::from_static("lists").unwrap();
    Term::Fun(Fun::new(
        Lambda::new(module, index, old_unique, 53, arity),
        free,
    ))
}

fn external(function: &str, arity: u32) -> Term {
    let atom = |name| Atom::from_static(name).unwrap();
    Term::Fun(Fun::external(atom("lists"), atom(function), arity))
}

#[test]
fn hashes_local_funs_as_pinned_erts() {
    // Each value is erts_debug:get_internal_state({internal_hash, Term}) on ERTS 29.1.1,
    // for a fun that binary_to_term/1 makes from NEW_FUN_EXT with the module lists, the
    // index, the old index, the old unique value, and the free variables.
    let zero = || local(0, 7, 1, vec![]);
    let free = local(0, 7, 1, vec![Term::Integer(1), Term::tuple(vec![])]);
    let negative = || local(3, -5, 2, vec![Term::binary(b"ab".to_vec()), atom("ok")]);
    let cases = [
        (zero(), 16900316212850717680),
        (free.clone(), 15970829942560899780),
        (negative(), 12860429061993126401),
        (
            Term::tuple(vec![zero(), Term::Integer(1)]),
            9006137623669004246,
        ),
        (
            map([(atom("ok"), free), (Term::Integer(1), zero())]),
            14001711515840955162,
        ),
        (Term::list_from(vec![negative()]), 1176312308706464788),
    ];
    for (term, hash) in cases {
        assert_eq!(internal_hash(&term), hash, "{term:?}");
    }
    // The entry label only selects code, so it is not part of the hash.
    let module = Atom::from_static("lists").unwrap();
    let relabeled = Term::Fun(Fun::new(Lambda::new(module, 0, 7, 61, 1), vec![]));
    assert_eq!(internal_hash(&relabeled), internal_hash(&zero()));
}

#[test]
fn hashes_external_funs_by_their_mfa() {
    // ERTS mixes the export entry address, which changes from run to run:
    // fun lists:reverse/1 gave 638061310806519221 and 18158401298541009015.
    // PBeam mixes a word of the module, function, and arity in its place.
    let reverse = external("reverse", 1);
    assert_eq!(internal_hash(&reverse), internal_hash(&reverse.clone()));
    let funs = [
        reverse,
        external("reverse", 2),
        external("member", 1),
        Term::Fun(Fun::external(
            Atom::from_static("erlang").unwrap(),
            Atom::from_static("reverse").unwrap(),
            1,
        )),
        local(0, 7, 1, vec![]),
    ];
    for (index, term) in funs.iter().enumerate() {
        for other in &funs[index + 1..] {
            assert_ne!(internal_hash(term), internal_hash(other), "{term:?}");
        }
    }
}

#[test]
fn hashes_improper_lists_as_pinned_erts() {
    // Each value is erts_debug:get_internal_state({internal_hash, Term}) on ERTS 29.1.1.
    let cases = [
        (
            improper(vec![Term::Integer(1)], Term::Integer(2)),
            5915944085625558568,
        ),
        (
            improper(vec![Term::Integer(97), Term::Integer(98)], atom("ok")),
            6406651010506344343,
        ),
        (
            improper(vec![Term::tuple(vec![])], atom("ok")),
            10451236899705497869,
        ),
        (
            improper(vec![Term::Integer(1)], Term::tuple(vec![])),
            4641139002621691555,
        ),
        (
            improper(
                vec![improper(vec![Term::Integer(1)], Term::Integer(2))],
                Term::Integer(3),
            ),
            6434836968081835632,
        ),
        (
            improper(vec![integers(&[1])], Term::binary(vec![])),
            465555062996238064,
        ),
        (
            improper(vec![atom("ok")], atom("true")),
            12145861372843638597,
        ),
        (
            improper(
                vec![Term::Integer(1), Term::tuple(vec![])],
                Term::Integer(2),
            ),
            502573382164607531,
        ),
    ];
    for (term, hash) in cases {
        assert_eq!(internal_hash(&term), hash, "{term:?}");
    }
}

#[test]
fn hashes_collision_nodes_as_pinned_erts() {
    // The keys of each pair have the same internal hash on pinned ERTS
    // 29.1.1, so each map has one collision node (`erts_debug:map_info/1`
    // gives `{collisions,1,...}`). ERTS hashes the node as the tuple of
    // its `[Key | Value]` cells (erl_map.h:197, 205-210).
    let a = integers(&[126, 114, 233, 199, 27, 172, 209, 95]);
    let b = integers(&[57, 112, 19, 186, 16, 195, 81, 198]);
    let c = integers(&[161, 92, 65, 50, 231, 24, 153, 137]);
    let d = integers(&[246, 157, 203, 77, 19, 254, 19, 93]);
    assert_eq!(internal_hash(&a), 16578151383281136240);
    assert_eq!(internal_hash(&b), 16578151383281136240);
    assert_eq!(internal_hash(&c), 10130931470384579971);
    assert_eq!(internal_hash(&d), 10130931470384579971);
    let with = |first: (Term, Term), second: (Term, Term)| {
        map([first, second]
            .into_iter()
            .chain((1..=31).map(|key| (Term::Integer(key), Term::Integer(key)))))
    };
    let cases = [
        (
            with(
                (a.clone(), Term::Integer(100)),
                (b.clone(), Term::Integer(200)),
            ),
            9593933779384815641,
        ),
        // An improper list value continues the [Key | Value] cell as more cells.
        (
            with(
                (
                    a.clone(),
                    improper(vec![Term::Integer(1)], Term::Integer(2)),
                ),
                (b.clone(), Term::tuple(vec![])),
            ),
            243852246011044368,
        ),
        (
            with((a, integers(&[120, 121])), (b, Term::nil())),
            2185849235149492526,
        ),
        (
            with((c, Term::Integer(1000)), (d, Term::tuple(vec![]))),
            3981912085379260197,
        ),
    ];
    for (term, hash) in cases {
        assert_eq!(internal_hash(&term), hash);
    }
}

#[test]
fn hashes_a_deep_term_without_recursion() {
    let deep = |levels: usize| {
        (0..levels).fold(Term::nil(), |inner, level| match level % 5 {
            0 => Term::tuple(vec![inner, Term::binary(vec![])]),
            1 => Term::list_from(vec![inner, Term::Integer(1)]),
            2 => map([(atom("ok"), inner)]),
            3 => improper(vec![inner], Term::Integer(1)),
            _ => local(0, 7, 1, vec![inner, Term::Integer(1)]),
        })
    };
    let hash = internal_hash(&deep(1_000_000));
    assert_eq!(hash, internal_hash(&deep(1_000_000)));
    assert_ne!(hash, internal_hash(&deep(999_999)));
}

#[test]
fn hashes_a_port_as_pinned_erts() {
    // ERTS: erts_debug:get_internal_state({internal_hash, list_to_port("#Port<0.5>")})
    // after erts_debug:set_internal_state(available_internal_state, true).
    // The port has the port data 0x28, which `port_info/1` gives as its id
    // 40, so its word is 0x287.
    assert_eq!(internal_hash(&Term::Port(PortId(0x28))), 589508649189025146);
}

#[test]
fn hashes_a_float_as_pinned_erts() {
    // `make_internal_hash` mixes `IHASH_TYPE_FLOAT` and the bits of the
    // float (`erl_term_hashing.c:2193-2202`). ERTS 29.1.1 gives these values
    // with `erts_debug:get_internal_state({internal_hash, Term})`.
    let hashes = [1.0, -0.0, 0.0, 2.5].map(|value| internal_hash(&Term::Float(value)));
    assert_eq!(
        hashes,
        [
            1378904917976523826,
            12330998780720423744,
            13198493423301380071,
            3606456034818785087
        ]
    );
}

/// A table with the atoms of the phash tests, and the atoms `a` and `abc`
/// from it.
fn phash_atoms() -> (AtomTable, Atom, Atom) {
    let mut atoms = AtomTable::new();
    let a = atoms.put("a").unwrap();
    let abc = atoms.put("abc").unwrap();
    (atoms, a, abc)
}

/// The terms of the phash tests with their `make_hash` and `make_hash2`
/// values, as the pinned ERTS gives them: the value of
/// `erlang:phash(Term, 4294967296)` is the `make_hash` value plus one, and
/// the value of `erlang:phash2(Term, 4294967296)` is the `make_hash2` value
/// (`bif.c:5319-5402`). Both are measured with `mise run erts-eval` on the
/// pinned ERTS OTP 29.1.1.
fn phash_terms(atoms: &AtomTable, a: &Atom, abc: &Atom) -> Vec<(Term, u32, u32)> {
    let map = |pairs: Vec<(Term, Term)>| Term::Map(Map::from_pairs(pairs, atoms).unwrap());
    let range = |first: i64, last: i64| Term::list_from((first..=last).map(Term::Integer));
    vec![
        (Term::Integer(1 << 59), 2_147_483_672, 2_442_888_721),
        (Term::Float(1.5), 1_073_217_536, 2_023_646_235),
        (Term::Float(1.0e300), 4_130_836_896, 387_524_106),
        (Term::Float(-0.0), 0, 423_528_920),
        (
            Term::tuple(vec![
                Term::Atom(a.clone()),
                Term::Integer(1),
                Term::Integer(2),
            ]),
            1_756_186_963,
            313_221_675,
        ),
        (range(1, 3), 3_336_869_157, 4_186_538_188),
        (range(1, 100), 2_449_873_273, 1_698_018_711),
        (
            map(vec![(Term::Atom(a.clone()), Term::Integer(1))]),
            1_670_235_873,
            1_401_795_262,
        ),
        (
            map((1..=40)
                .map(|key| (Term::Integer(key), Term::Integer(key)))
                .collect()),
            2_470_972_761,
            2_202_532_150,
        ),
        (
            map((1..=100)
                .map(|key| (Term::Integer(key), Term::Integer(key + 1)))
                .collect()),
            3_637_457_306,
            3_369_016_695,
        ),
        (Term::binary(vec![1, 2, 3]), 687_692_589, 4_167_228_639),
        (Term::binary(vec![0; 1000]), 1000, 4_013_479_069),
        (Term::binary(vec![]), 0, 147_926_629),
        (Term::Atom(abc.clone()), 26_499, 26_499),
        (Term::Integer(-1), 1_680_185_269, 1_117_813_597),
        (Term::Integer(0), 0, 3_175_731_469),
        (Term::Integer(1), 2_788_898_427, 539_485_162),
        (Term::nil(), 1, 3_468_870_702),
        (Term::tuple(vec![]), 0, 221_703_996),
        (map(vec![]), 1_113_425_984, 844_985_373),
        (Term::Pid(ProcessId(7)), 1_912_891_191, 591_482_922),
        (
            Term::Reference(Reference::ordinary(7).unwrap()),
            134_293_115,
            464_301_042,
        ),
        (Term::Port(PortId(5)), 3_777_324_193, 125_905_316),
    ]
}

#[test]
fn hashes_terms_with_make_hash_as_pinned_erts() {
    // The old term hash of `erlang:phash/2` gives the same value for each
    // term as the pinned ERTS (`make_hash`, `erl_term_hashing.c:165-427`).
    let (atoms, a, abc) = phash_atoms();
    for (term, hash, _) in phash_terms(&atoms, &a, &abc) {
        assert_eq!(make_hash(&atoms, &term), hash, "{term:?}");
    }
}

#[test]
fn hashes_terms_with_make_hash2_as_pinned_erts() {
    // The term hash of `erlang:phash2/2` gives the same value for each term
    // as the pinned ERTS (`make_hash2`, `erl_term_hashing.c:858-1596`), in
    // the range 2^32, which leaves the hash as it is.
    let (atoms, a, abc) = phash_atoms();
    for (term, _, hash) in phash_terms(&atoms, &a, &abc) {
        // An ERTS port number is a 64-bit unique value, not the number of
        // `port_info(Port, id)`, so an ERTS run cannot pin the value of a
        // port term. The port branch follows `UINT32_HASH_2(low, high,
        // HCONST_6)` (`erl_term_hashing.c:1446-1452`), and the `make_hash`
        // case above pins the port number of a fabricated port.
        if matches!(term, Term::Port(_)) {
            continue;
        }
        assert_eq!(make_hash2(&atoms, &term).0, hash, "{term:?}");
    }
}

#[test]
fn hashes_a_bitstring_that_is_not_a_whole_number_of_bytes_as_pinned_erts() {
    // The binary paths of both hashes read the whole bytes and then the tail
    // bits in the high bits of the last byte (`hash_binary_bytes`,
    // `erl_term_hashing.c:117-160`; `BIN_REF_SUBTAG`, `1270-1310`). The
    // values are `erlang:phash(<<1,2:4>>, 4294967296)` minus one,
    // `erlang:phash2(<<1,2:4>>, 4294967296)`, and
    // `erlang:phash2(<<1,2:3>>, 4294967296)` on the pinned ERTS (measured
    // with `mise run erts-eval`).
    let atoms = AtomTable::new();
    let four = Term::Binary(Binary::from(vec![1, 0x20]).bits(0, 12));
    assert_eq!(make_hash(&atoms, &four), 3_378_198_522);
    assert_eq!(make_hash2(&atoms, &four).0, 915_905_169);
    let three = Term::Binary(Binary::from(vec![1, 0x40]).bits(0, 11));
    assert_eq!(make_hash2(&atoms, &three).0, 2_841_540_738);
}

#[test]
#[should_panic(expected = "a match context reached a term operation")]
fn make_hash_panics_for_a_match_context() {
    let atoms = AtomTable::new();
    make_hash(
        &atoms,
        &Term::MatchContext(Arc::new(MatchState::new(Binary::from(b"a")))),
    );
}

#[test]
#[should_panic(expected = "a match context reached a term operation")]
fn make_hash2_panics_for_a_match_context() {
    let atoms = AtomTable::new();
    make_hash2(
        &atoms,
        &Term::MatchContext(Arc::new(MatchState::new(Binary::from(b"a")))),
    );
}

#[test]
fn hashes_a_collision_node_of_a_hashmap_as_pinned_erts() {
    // The pairs of equal map-hash keys of the test above form one collision
    // node of the hashmap, which `make_hash2` reads as the pushed pairs of
    // the node (`push_children`, `erl_term_hashing.c:1224-1250`). The
    // values are `erlang:phash(Map, 4294967296)` minus one and
    // `erlang:phash2(Map, 4294967296)` on the pinned ERTS for the same map
    // (measured with `mise run erts-eval`).
    let a = integers(&[126, 114, 233, 199, 27, 172, 209, 95]);
    let b = integers(&[57, 112, 19, 186, 16, 195, 81, 198]);
    let term = map([(a, Term::Integer(100)), (b, Term::Integer(200))]
        .into_iter()
        .chain((1..=31).map(|key| (Term::Integer(key), Term::Integer(key)))));
    let atoms = AtomTable::new();
    assert_eq!(make_hash(&atoms, &term), 996_043_277);
    assert_eq!(make_hash2(&atoms, &term).0, 727_602_666);
}

#[test]
fn hashes_a_port_with_a_large_number_from_the_erts_source() {
    // A port number above 2^32 mixes its high word first in `make_hash`
    // (`erl_term_hashing.c:255-266`), and `make_hash2` mixes both words
    // (`1446-1452`). An ERTS run cannot pin a port number, so the values are
    // computed from those functions. The `make_hash` value is the value of
    // `phash` minus one.
    let atoms = AtomTable::new();
    let port = Term::Port(PortId(1 << 32));
    assert_eq!(make_hash(&atoms, &port), 3_591_747_951);
    assert_eq!(make_hash2(&atoms, &port).0, 3_633_483_707);
}

#[test]
fn hashes_only_the_pid_number_and_not_the_serial_as_pinned_erts() {
    // ERTS hashes the 28-bit pid number only (`internal_pid_number`,
    // `erl_term.h:645-656`), so `<0.0.1>` and `<0.0.0>` have the same hash.
    // The pinned ERTS gives `erlang:phash(Pid, 4294967296)` = 1 and
    // `erlang:phash2(Pid, 4294967296)` = 2858162279 for both pids (measured
    // with `mise run erts-eval`).
    let atoms = AtomTable::new();
    let zero = Term::Pid(ProcessId(0));
    let serial = Term::Pid(ProcessId(1 << 28));
    assert_eq!(make_hash(&atoms, &zero), 0);
    assert_eq!(make_hash(&atoms, &serial), 0);
    assert_eq!(make_hash2(&atoms, &zero).0, 2_858_162_279);
    assert_eq!(make_hash2(&atoms, &serial).0, 2_858_162_279);
}
