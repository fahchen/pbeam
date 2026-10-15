use std::sync::Arc;

use super::{internal_hash, mix64};
use crate::term::{Atom, Binary, Fun, Lambda, Map, MatchState, PortId, ProcessId, Reference, Term};

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
    ];
    for (term, hash) in cases {
        assert_eq!(internal_hash(&term), hash, "{term:?}");
    }
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
