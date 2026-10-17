//! The ERTS term order, as `erts_cmp_compound` (`utils.c`, OTP 29.1.1)
//! gives it to `is_lt` and `is_ge` with arithmetic comparison (`exact = 0`),
//! and to the map key order with exact comparison (`exact = 1`).
//!
//! Terms of different types compare by type, as the `*_DEF` values in
//! `erl_term.h` order them:
//!
//! `number < atom < reference < fun < port < pid < tuple < map < [] < list < bitstring`
//!
//! In arithmetic comparison, numbers compare by their mathematical value:
//! `1 == 1.0`, `0.0 == -0.0`, and `9007199254740993 > 9007199254740992.0`
//! (`utils.c:2366-2456`). In exact comparison, each integer is less than
//! each float (`exact_fall_through`, `utils.c:2457-2459`), and two floats
//! compare by `erts_float_exact_sortable` (`utils.c:2131-2137`,
//! `erl_utils.h:227-237`), so `-0.0` is less than `0.0`. Map keys always
//! compare exactly, and map values compare in the mode of the map
//! (`utils.c:2506-2550`). Terms of one type compare as follows:
//!
//! - atom: by the bytes of the UTF-8 name, as `erts_cmp_atoms` does, not by
//!   the atom index;
//! - fun: a local fun is less than an external fun. Local funs compare by
//!   the module name, the lambda index, and the old unique value of the fun
//!   entry, then by the number of free variables, then free variable by
//!   free variable (`utils.c:2157-2187`). External funs compare by the
//!   module name, the function name, and the arity (`utils.c:2189-2207`);
//! - port: by the port number, which is the ERTS port data
//!   (`utils.c:1751-1755`);
//! - pid: by the process counter, which gives the order of creation, as the
//!   ERTS pid data does until the process table uses a slot again;
//! - tuple: by size, then element by element;
//! - map: by size, then by the keys in term order, then by the values in key
//!   order;
//! - list: element by element, then by the tails, where a proper list has
//!   the tail `[]` (`utils.c:1827-1851`). With equal lengths, the two tails
//!   compare. When one list is shorter, its tail compares with the rest of
//!   the other list, which is a list that is not empty. Thus a shorter
//!   proper list is less when it is a prefix of the other, because `[]` is
//!   less than a list, `[1|x] < [1,2]`, and `[1|<<>>] > [1,2]`;
//! - binary: by the bytes, then by the size.
//!
//! Two references compare by their number words, from the highest word
//! down, as in ERTS (`erts_cmp_compound`, `utils.c:2283-2286`). Two
//! different alias references have no PBeam order: ERTS holds the pid of
//! the owner in the higher words, and a PBeam reference does not keep it.

use std::cmp::Ordering;

use num_bigint::{BigInt, Sign};
use num_traits::FromPrimitive;

use super::{AtomTable, Term, match_context::MATCH_CONTEXT_ESCAPE};
use crate::error::{BeamError, error};

/// The rank of `[]` in the ERTS term order (`NIL_DEF`).
const NIL_RANK: u8 = 8;
/// The rank of a list that is not empty in the ERTS term order (`LIST_DEF`).
const LIST_RANK: u8 = 9;

/// One step of a comparison on the work stack: compare two terms, exactly
/// when the flag is true.
struct Step<'a>(&'a Term, &'a Term, bool);

impl Term {
    /// Compare two terms in the ERTS term order with a work stack, so a deep
    /// term does not recurse.
    ///
    /// The pair of the two terms is compared first with no work stack, so a
    /// pair that the comparison decides allocates nothing. Only a pair that
    /// the comparison does not decide pushes the pairs of its elements: two
    /// cons cells, two tuples of equal size, two maps of equal size, and two
    /// funs that the own entries do not order. The loop then drains the stack
    /// in the order that ERTS compares the pairs, and returns the result of
    /// the first pair that is not equal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use std::cmp::Ordering;
    ///
    /// // The types decide this pair, so the comparison allocates nothing.
    /// assert_eq!(Term::Integer(1).compare(&Term::static_atom("a"), &atoms)?, Ordering::Less);
    /// let short = Term::list_from([Term::static_atom("b")]);
    /// let long = Term::list_from([Term::static_atom("a"), Term::static_atom("z")]);
    /// assert_eq!(short.compare(&long, &atoms)?, Ordering::Greater);
    /// // Two equal tuples push the pair of each element, and the loop then
    /// // returns Equal.
    /// let tuple = Term::tuple(vec![Term::static_atom("a"), Term::Integer(1)]);
    /// assert_eq!(tuple.compare(&tuple, &atoms)?, Ordering::Equal);
    /// assert_eq!(Term::Reference(Reference(1)).compare(&Term::Reference(Reference(2)), &atoms)?, Ordering::Less);
    /// ```
    ///
    /// # Errors
    ///
    /// Return a runtime error when the result depends on the order of an
    /// alias reference.
    pub(crate) fn compare(&self, other: &Term, atoms: &AtomTable) -> Result<Ordering, BeamError> {
        self.compare_in(other, false, atoms)
    }

    /// Compare two terms in the ERTS term order with exact comparison, as
    /// `erts_cmp` with `exact = 1` orders map keys (`erts_cmp_flatmap_keys`,
    /// `erl_utils.h:267-272`). Each integer is less than each float, and
    /// `-0.0` is less than `0.0`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use std::cmp::Ordering;
    ///
    /// assert_eq!(Term::Float(1.0).compare_exact(&Term::Integer(2), &atoms)?, Ordering::Greater);
    /// assert_eq!(Term::Float(-0.0).compare_exact(&Term::Float(0.0), &atoms)?, Ordering::Less);
    /// ```
    ///
    /// # Errors
    ///
    /// Return a runtime error when the result depends on the order of an
    /// alias reference.
    pub(crate) fn compare_exact(
        &self,
        other: &Term,
        atoms: &AtomTable,
    ) -> Result<Ordering, BeamError> {
        self.compare_in(other, true, atoms)
    }

    /// Compare two terms, exactly when `exact` is true.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::Integer(1).compare_in(&Term::Float(1.0), false, &atoms)?.is_eq());
    /// ```
    fn compare_in(
        &self,
        other: &Term,
        exact: bool,
        atoms: &AtomTable,
    ) -> Result<Ordering, BeamError> {
        // The pair of the two terms is compared first, with no work stack. A
        // pair that the comparison decides needs no stack and allocates
        // nothing; only a pair that the comparison does not decide pushes the
        // pairs of its elements, and the loop then drains them in the order of
        // the comparison.
        let mut steps = Vec::new();
        let order = compare_pair(self, other, exact, &mut steps, atoms)?;
        if order.is_ne() {
            return Ok(order);
        }
        while let Some(Step(left, right, exact)) = steps.pop() {
            let order = compare_pair(left, right, exact, &mut steps, atoms)?;
            if order.is_ne() {
                return Ok(order);
            }
        }
        Ok(Ordering::Equal)
    }
}

/// Compare two terms without their nested terms. For two lists, tuples,
/// maps, or funs that the sizes and fun entries do not order, push the
/// nested pairs onto `steps` and return `Equal`. Two cons cells push the
/// pair of their heads and then the pair of their tails, as
/// `erts_cmp_compound` compares each head and then goes on with the tails
/// (`utils.c:1827-1851`).
///
/// # Examples
///
/// ```rust,ignore
/// let mut steps = Vec::new();
/// let left = Term::tuple(vec![Term::Integer(1)]);
/// let right = Term::tuple(vec![Term::Integer(2)]);
/// let atoms = AtomTable::new();
/// assert_eq!(compare_pair(&left, &right, false, &mut steps, &atoms)?, Ordering::Equal);
/// assert_eq!(steps.len(), 1);
/// ```
fn compare_pair<'a>(
    left: &'a Term,
    right: &'a Term,
    exact: bool,
    steps: &mut Vec<Step<'a>>,
    atoms: &AtomTable,
) -> Result<Ordering, BeamError> {
    let order = match (left, right) {
        (Term::Cons(left), Term::Cons(right)) => {
            // The tails go on the stack first, so the heads compare first.
            steps.extend([
                Step(&left.tail, &right.tail, exact),
                Step(&left.head, &right.head, exact),
            ]);
            Ordering::Equal
        }
        (Term::Integer(left), Term::Integer(right)) => left.cmp(right),
        // A big integer is outside the i64 range, so its sign orders it
        // against an i64 (`SMALL_BIG`, `BIG_SMALL`, `utils.c`).
        (Term::BigInt(left), Term::BigInt(right)) => left.cmp(right),
        (Term::BigInt(big), Term::Integer(_)) => big_sign_order(big),
        (Term::Integer(_), Term::BigInt(big)) => big_sign_order(big).reverse(),
        (Term::BigInt(big), Term::Float(float)) if !exact => compare_big_float(big, *float),
        (Term::Float(float), Term::BigInt(big)) if !exact => {
            compare_big_float(big, *float).reverse()
        }
        (Term::BigInt(_), Term::Float(_)) => Ordering::Less,
        (Term::Float(_), Term::BigInt(_)) => Ordering::Greater,
        (Term::Float(left), Term::Float(right)) if exact => {
            exact_sortable(*left).cmp(&exact_sortable(*right))
        }
        // `0.0` and `-0.0` are equal in arithmetic comparison.
        (Term::Float(left), Term::Float(right)) => {
            left.partial_cmp(right).expect("a float term is finite")
        }
        (Term::Integer(integer), Term::Float(float)) if !exact => {
            compare_integer_float(*integer, *float)
        }
        (Term::Float(float), Term::Integer(integer)) if !exact => {
            compare_integer_float(*integer, *float).reverse()
        }
        (Term::Integer(_), Term::Float(_)) => Ordering::Less,
        (Term::Float(_), Term::Integer(_)) => Ordering::Greater,
        (Term::Atom(left), Term::Atom(right)) => atoms.name_of(left).cmp(atoms.name_of(right)),
        (Term::Binary(left), Term::Binary(right)) => left.cmp(right),
        (Term::Pid(left), Term::Pid(right)) => left.0.cmp(&right.0),
        (Term::Port(left), Term::Port(right)) => left.0.cmp(&right.0),
        // ERTS compares two references on one node by their number words,
        // from the highest word down (`erts_cmp_compound`,
        // utils.c:2283-2286), and the words below match that layout (row
        // "Term order" of docs/erts-differences.md). An alias reference
        // holds the pid of its owner in the higher ERTS words, which a PBeam
        // reference does not keep, so its order is not supported.
        (Term::Reference(left), Term::Reference(right)) if left == right => Ordering::Equal,
        (Term::Reference(left), Term::Reference(right))
            if left.is_pid_reference() || right.is_pid_reference() =>
        {
            return Err(error(
                "unsupported term order of an alias reference (see \"Term order\" in docs/erts-differences.md)",
            ));
        }
        (Term::Reference(left), Term::Reference(right)) => left
            .numbers()
            .iter()
            .rev()
            .cmp(right.numbers().iter().rev()),
        (Term::Tuple(left), Term::Tuple(right)) if left.len() == right.len() => {
            steps.extend(
                left.iter()
                    .zip(right.iter())
                    .rev()
                    .map(|(l, r)| Step(l, r, exact)),
            );
            Ordering::Equal
        }
        (Term::Tuple(left), Term::Tuple(right)) => left.len().cmp(&right.len()),
        (Term::Map(left), Term::Map(right)) if left.len() == right.len() => {
            // ERTS compares all keys first, exactly, then the values in the
            // mode of the map (`erts_cmp_compound`, `utils.c:2506-2550`).
            let (left, right) = (sorted_pairs(left, atoms), sorted_pairs(right, atoms));
            let pairs = || left.iter().zip(&right).rev();
            steps.extend(pairs().map(|((_, l), (_, r))| Step(l, r, exact)));
            steps.extend(pairs().map(|((l, _), (r, _))| Step(l, r, true)));
            Ordering::Equal
        }
        (Term::Map(left), Term::Map(right)) => left.len().cmp(&right.len()),
        (Term::Fun(left), Term::Fun(right)) => {
            let order = left
                .target()
                .compare(right.target(), atoms)
                .then(left.free().len().cmp(&right.free().len()));
            if order.is_eq() {
                let pairs = left.free().iter().zip(right.free()).rev();
                steps.extend(pairs.map(|(l, r)| Step(l, r, exact)));
            }
            order
        }
        _ => type_rank(left).cmp(&type_rank(right)),
    };
    Ok(order)
}

/// Return the bits of a float as an unsigned value whose order is the exact
/// float order, as `erts_float_exact_sortable` (`erl_utils.h:227-237`)
/// does: a negative float inverts its bits, and a positive float sets the
/// sign bit.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(exact_sortable(-0.0) < exact_sortable(0.0));
/// assert!(exact_sortable(-2.0) < exact_sortable(-1.0));
/// ```
fn exact_sortable(value: f64) -> u64 {
    const SIGN_BIT: u64 = 1 << 63;
    let bits = value.to_bits();
    if bits & SIGN_BIT != 0 {
        !bits
    } else {
        bits | SIGN_BIT
    }
}

/// Compare an integer with a float by their mathematical values, as
/// `erts_cmp_compound` does for `SMALL_FLOAT` and `BIG_FLOAT` in arithmetic
/// comparison (`utils.c:2366-2430`). A float term is finite. The
/// comparison is exact: `9007199254740993` is greater than
/// `9007199254740992.0`.
///
/// # Examples
///
/// ```rust,ignore
/// use std::cmp::Ordering;
///
/// assert_eq!(compare_integer_float(1, 1.0), Ordering::Equal);
/// assert_eq!(compare_integer_float(9007199254740993, 9007199254740992.0), Ordering::Greater);
/// assert_eq!(compare_integer_float(i64::MAX, 9.3e18), Ordering::Less);
/// assert_eq!(compare_integer_float(-1, -0.5), Ordering::Less);
/// ```
pub(crate) fn compare_integer_float(integer: i64, float: f64) -> Ordering {
    // 2^63 as a float: each float at or above it is greater than each i64,
    // and each float below -2^63 is less than each i64.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if float >= LIMIT {
        return Ordering::Less;
    }
    if float < -LIMIT {
        return Ordering::Greater;
    }
    let whole = float.trunc();
    // The whole part is in the i64 range, so the cast is exact.
    integer.cmp(&(whole as i64)).then_with(|| {
        let fraction = float - whole;
        0.0.partial_cmp(&fraction).expect("a float term is finite")
    })
}

/// Return the order of a big integer against each `i64`: greater for a
/// positive one and less for a negative one.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(big_sign_order(&(BigInt::from(1) << 64)), Ordering::Greater);
/// ```
fn big_sign_order(big: &BigInt) -> Ordering {
    if big.sign() == Sign::Minus {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

/// Compare a big integer with a float by their mathematical values, as
/// `erts_cmp_compound` does for `BIG_FLOAT` (`utils.c:2401-2430`). ERTS
/// compares the sizes first and then the float as a bignum
/// (`double_to_big`), which gives the exact order. The big integer is
/// outside the i64 range, so a float with a fractional part is inside it
/// and the whole part of the float decides the order.
///
/// # Examples
///
/// ```rust,ignore
/// let big = BigInt::from(1) << 64;
/// assert_eq!(compare_big_float(&big, 18446744073709551616.0), Ordering::Equal);
/// assert_eq!(compare_big_float(&big, 1.5), Ordering::Greater);
/// assert_eq!(compare_big_float(&-big, 1.0e300), Ordering::Less);
/// ```
pub(crate) fn compare_big_float(big: &BigInt, float: f64) -> Ordering {
    let whole = BigInt::from_f64(float.trunc()).expect("a float term is finite");
    big.cmp(&whole)
}

/// Return the pairs of a map, sorted by the term order of the keys.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Map::from_pairs(vec![(Term::static_atom("b"), Term::Integer(1)), (Term::Integer(2), Term::Integer(3))], &atoms)?;
/// let keys: Vec<_> = sorted_pairs(&map, &atoms).into_iter().map(|(key, _)| key.clone()).collect();
/// assert_eq!(keys, [Term::Integer(2), Term::static_atom("b")]);
/// ```
fn sorted_pairs<'a>(map: &'a super::Map, atoms: &AtomTable) -> Vec<(&'a Term, &'a Term)> {
    let mut pairs: Vec<_> = map.iter().collect();
    pairs.sort_unstable_by(|(left, _), (right, _)| {
        left.compare_exact(right, atoms)
            .expect("a map key is a binary, a number, an atom, or a list")
    });
    pairs
}

/// Return the rank of the type of a term in the ERTS term order. `[]` and a
/// cons cell have different ranks, as `NIL_DEF` and `LIST_DEF` do. An
/// improper list has the rank of a cons cell.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(type_rank(&Term::Integer(1)) < type_rank(&Term::static_atom("a")));
/// assert!(type_rank(&Term::nil()) < type_rank(&Term::list_from([Term::Integer(1)])));
/// ```
fn type_rank(term: &Term) -> u8 {
    match term {
        Term::Integer(_) | Term::BigInt(_) | Term::Float(_) => 0,
        Term::Atom(_) => 1,
        Term::Reference(_) => 2,
        Term::Fun(_) => 3,
        Term::Port(_) => 4,
        Term::Pid(_) => 5,
        Term::Tuple(_) => 6,
        Term::Map(_) => 7,
        Term::Nil => NIL_RANK,
        Term::Cons(_) => LIST_RANK,
        Term::Binary(_) => 10,
        Term::MatchContext(_) => unreachable!("{MATCH_CONTEXT_ESCAPE}"),
    }
}

#[cfg(test)]
#[path = "../../tests/term/order.rs"]
mod tests;
