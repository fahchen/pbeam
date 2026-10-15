//! Bodies of the integer BIFs `+`, `-`, `*`, `div`, `rem`, `band`, `bor`,
//! `bxor`, `bnot`, `bsl`, `bsr`, `-/1`, and `abs/1`, as ERTS `erl_arith.c`
//! and `erl_bif_guard.c` give them for small integers and bignums.
//!
//! ERTS keeps an integer from -(2^59) to 2^59 - 1 as a small integer and
//! each other integer as a bignum of 64-bit digits. PBeam keeps each integer
//! in the i64 range as [`Term::Integer`], so the cost of an operation reads
//! the ERTS form of each operand: [`Term::small_integer`] and [`digits`]. A
//! bignum operation costs reductions by the heap words of its result, as
//! `bump_reds_linear` and `bump_reds_quadratic` (`erl_arith.c:56-86`) give
//! them. Each body returns its result and its reductions as a [`Charged`]
//! pair, so the leaf lane of the interpreter, which holds no scheduler, can
//! charge them after the call.

use std::cmp::Ordering;

use num_bigint::BigInt;
use num_traits::Signed;

use super::BifResult::Value;
use super::*;
use crate::runtime::scheduler::CONTEXT_REDS;
use crate::term::{MAX_BIG_DIGITS, fits_big_arity};

/// A BIF result and the reductions that it uses.
pub(super) type Charged = (BifResult, i32);

/// `ERTS_ARITH_WORDS_PER_REDUCTION` (`global.h:1041`).
const WORDS_PER_REDUCTION: usize = 16;
/// `SMALL_BITS - 1` on a 64-bit system (`erl_term.h`): a small integer
/// shifted by this many bits or more leaves the small range.
const SMALL_SHIFT_LIMIT: i64 = 59;
/// The smallest and the largest ERTS small integers, `MIN_SMALL` and
/// `MAX_SMALL` (`erl_term.h`).
const MIN_SMALL: i64 = -(1 << 59);
const MAX_SMALL: i64 = (1 << 59) - 1;
/// The bits of one bignum digit, `D_EXP` (`big.h:55`).
const DIGIT_BITS: i64 = 64;

/// An arithmetic BIF of two numbers that also takes floats.
#[derive(Clone, Copy)]
pub(super) enum Arith {
    Plus,
    Minus,
    Times,
}

/// A bitwise BIF of two integers.
#[derive(Clone, Copy)]
pub(super) enum Bitwise {
    And,
    Or,
    Xor,
}

/// Charge the reductions of a [`Charged`] result to the current process
/// of `scheduler` and return the result. A result with no reductions reads
/// no process.
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 10;
/// charge(&mut scheduler, (BifResult::badarith(), 2));
/// assert_eq!(scheduler.current_process_mut().fcalls, 8);
/// ```
pub(super) fn charge(scheduler: &mut Scheduler, (result, reductions): Charged) -> BifResult {
    if reductions > 0 {
        scheduler.current_process_mut().bump_reductions(reductions);
    }
    result
}

/// Write the reductions of a [`Charged`] result to `reductions` for the
/// leaf lane, which charges them after the call, and return the result.
///
/// # Examples
///
/// ```rust,ignore
/// let mut reductions = 0;
/// charged(&mut reductions, (BifResult::badarith(), 2));
/// assert_eq!(reductions, 2);
/// ```
pub(super) fn charged(reductions: &mut i32, (result, cost): Charged) -> BifResult {
    *reductions = cost;
    result
}

/// Return the `error:system_limit` exception of a bignum result with more
/// than `BIG_ARITY_MAX` digits (`erl_arith.c`).
fn system_limit() -> BifResult {
    BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::static_atom("system_limit"),
    }
}

/// Return the number of 64-bit digits of an integer in its ERTS bignum
/// form, as `big_size` gives it after `small_to_big` for a small integer.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(digits(&Term::Integer(5)), 1);
/// assert_eq!(digits(&Term::from_bigint(BigInt::from(1) << 64)), 2);
/// ```
fn digits(term: &Term) -> usize {
    match term {
        Term::BigInt(value) => value.bits().div_ceil(64) as usize,
        _ => 1,
    }
}

/// Return whether a term is an ERTS small integer.
fn is_small(term: &Term) -> bool {
    term.small_integer().is_some()
}

/// Return the reductions of `bump_reds_linear` (`erl_arith.c:56-72`) for
/// `words` heap words: all reductions from `CONTEXT_REDS * 16` words, else
/// one for each 16 words and one more.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(linear(3), 1);
/// assert_eq!(linear(64_000), CONTEXT_REDS);
/// ```
fn linear(words: usize) -> i32 {
    // From `CONTEXT_REDS * 16` words, the count is above `CONTEXT_REDS`,
    // so the bump empties the budget as `BUMP_ALL_REDS` does.
    (words / WORDS_PER_REDUCTION + 1).min(CONTEXT_REDS as usize) as i32
}

/// Return the reductions of `bump_reds_quadratic` (`erl_arith.c:74-86`)
/// for two sizes in digits.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(quadratic(2, 2), 1);
/// assert_eq!(quadratic(8, 8), 5);
/// ```
fn quadratic(left: usize, right: usize) -> i32 {
    // A size of `CONTEXT_REDS * 16` digits or more gives a count above
    // `CONTEXT_REDS`, so the bump empties the budget as `BUMP_ALL_REDS`
    // does. Each size is at most 2^16 - 1, so the product fits.
    (left * right / WORDS_PER_REDUCTION + 1).min(CONTEXT_REDS as usize) as i32
}

/// Return the integer `value` as a BIF result in its canonical form, or
/// `system_limit` when it has more digits than an ERTS bignum can hold.
fn big_result(value: BigInt) -> BifResult {
    if fits_big_arity(&value) {
        Value(Term::from_bigint(value))
    } else {
        system_limit()
    }
}

/// Run `erlang:'+'/2`, `erlang:'-'/2`, or `erlang:'*'/2`, as
/// `erts_mixed_plus`, `erts_mixed_minus`, and `erts_mixed_times`
/// (`erl_arith.c:358-508`, `571-714`, `716-913`) do. Two integers give an
/// integer, and a float operand gives a float through [`float_arith`]: a
/// bignum that `big_to_double` cannot convert, a result that is not finite,
/// and an operand that is not a number raise `badarith`. A result with more
/// than `BIG_ARITY_MAX` digits raises `system_limit`.
///
/// Two small integers cost no reduction, also when the result is a bignum
/// (`erl_arith.c:378-385`). With a bignum operand, `+` and `-` cost
/// [`linear`] of `max(size) + 2` and `max(size) + 1` words
/// (`erl_arith.c:434-447`, `636-648`), and `*` costs [`quadratic`] of the two sizes (`erl_arith.c:856`). An
/// operand that ERTS returns at once costs nothing: `0 + B`, `B + 0`,
/// `B - 0`, `0 * B`, `1 * B`, and their mirrors for `*`
/// (`erl_arith.c:396-423`, `631-633`, `785-817`).
///
/// # Examples
///
/// ```rust,ignore
/// let big = Term::from_bigint(BigInt::from(1) << 64);
/// let (sum, reductions) = arith(Arith::Plus, &big, &Term::Integer(1));
/// assert!(matches!(sum, BifResult::Value(Term::BigInt(_))));
/// assert_eq!(reductions, 1);
/// let (sum, reductions) = arith(Arith::Plus, &Term::Integer(i64::MAX >> 4), &Term::Integer(i64::MAX >> 4));
/// assert_eq!(reductions, 0);
/// let (sum, _) = arith(Arith::Times, &Term::Integer(2), &Term::Float(1.5));
/// assert!(matches!(sum, BifResult::Value(Term::Float(3.0))));
/// ```
pub(super) fn arith(op: Arith, left: &Term, right: &Term) -> Charged {
    if let (Term::Integer(left), Term::Integer(right)) = (left, right) {
        let small = match op {
            Arith::Plus => left.checked_add(*right),
            Arith::Minus => left.checked_sub(*right),
            Arith::Times => left.checked_mul(*right),
        };
        if let Some(value) = small {
            let (left, right) = (&Term::Integer(*left), &Term::Integer(*right));
            return (
                Value(Term::Integer(value)),
                arith_reductions(op, left, right),
            );
        }
    }
    let (Some(left_value), Some(right_value)) = (left.bigint(), right.bigint()) else {
        let float = match op {
            Arith::Plus => |left, right| left + right,
            Arith::Minus => |left, right| left - right,
            Arith::Times => |left, right| left * right,
        };
        return (erlang::float_arith(left, right, float), 0);
    };
    let value = match op {
        Arith::Plus => left_value.as_ref() + right_value.as_ref(),
        Arith::Minus => left_value.as_ref() - right_value.as_ref(),
        Arith::Times => left_value.as_ref() * right_value.as_ref(),
    };
    (big_result(value), arith_reductions(op, left, right))
}

/// Return the reductions of [`arith`] for two integers.
fn arith_reductions(op: Arith, left: &Term, right: &Term) -> i32 {
    if is_small(left) && is_small(right) {
        return 0;
    }
    let small = |term: &Term, values: &[i64]| {
        term.small_integer()
            .is_some_and(|value| values.contains(&value))
    };
    match op {
        Arith::Plus if small(left, &[0]) || small(right, &[0]) => 0,
        Arith::Minus if small(right, &[0]) => 0,
        // `+` charges the heap words `max(size) + 2`, and `-` charges
        // `max(size) + 1` (`erl_arith.c:437-442`, `639-644`).
        Arith::Plus => linear(digits(left).max(digits(right)) + 2),
        Arith::Minus => linear(digits(left).max(digits(right)) + 1),
        Arith::Times if small(left, &[0, 1]) || small(right, &[0, 1]) => 0,
        Arith::Times => quadratic(digits(left), digits(right)),
    }
}

/// Run `erlang:'div'/2` or `erlang:'rem'/2`: the quotient truncated toward
/// zero, or the remainder with the sign of `left`, as `intdiv_2`, `rem_2`,
/// and `erts_int_div_rem` (`erl_arith.c:127-161`, `1172-1250`) give them.
/// A `right` of `0` raises `badarith` before the operand types are checked,
/// so `a div 0` raises `badarith` too. An operand that is not an integer,
/// also a float, raises `badarith`.
///
/// Two small integers cost no reduction, except `-(2^59) div -1`, whose
/// quotient is a bignum. A bignum division costs [`quadratic`] of
/// `size(left) - size(right) + 1` and `size(right)`, but only when the
/// magnitude of `left` is above the magnitude of `right`; a small `left`
/// with a bignum `right` gives `0` and `left` with no cost, except
/// `-(2^59)`.
///
/// # Examples
///
/// ```rust,ignore
/// let (quotient, _) = div_rem(&Term::Integer(-7), &Term::Integer(2), false);
/// assert!(matches!(quotient, BifResult::Value(Term::Integer(-3))));
/// let (remainder, _) = div_rem(&Term::Integer(-7), &Term::Integer(3), true);
/// assert!(matches!(remainder, BifResult::Value(Term::Integer(-1))));
/// assert!(matches!(div_rem(&Term::static_atom("a"), &Term::Integer(0), false).0, BifResult::Raise { .. }));
/// ```
pub(super) fn div_rem(left: &Term, right: &Term, remainder: bool) -> Charged {
    if *right == Term::Integer(0) {
        return (BifResult::badarith(), 0);
    }
    if let (Term::Integer(left_value), Term::Integer(right_value)) = (left, right) {
        // `i64::MIN rem -1` is 0 and `i64::MIN div -1` is the only quotient
        // outside the i64 range.
        let value = if remainder {
            Some(left_value.wrapping_rem(*right_value))
        } else {
            left_value.checked_div(*right_value)
        };
        if let Some(value) = value {
            let value = Term::Integer(value);
            let reductions = if is_small(left) && is_small(right) && (remainder || is_small(&value))
            {
                0
            } else {
                div_rem_reductions(left, right)
            };
            return (Value(value), reductions);
        }
    }
    let (Some(left_value), Some(right_value)) = (left.bigint(), right.bigint()) else {
        return (BifResult::badarith(), 0);
    };
    let value = if remainder {
        left_value.as_ref() % right_value.as_ref()
    } else {
        left_value.as_ref() / right_value.as_ref()
    };
    (
        Value(Term::from_bigint(value)),
        div_rem_reductions(left, right),
    )
}

/// Return the reductions of `erts_int_div_rem` (`erl_arith.c:1172-1250`)
/// for two integers that are not both small, or for `-(2^59) div -1`.
fn div_rem_reductions(left: &Term, right: &Term) -> i32 {
    match (left.small_integer(), right.small_integer()) {
        (Some(value), None) if value != MIN_SMALL => 0,
        _ => match magnitude_order(left, right) {
            Ordering::Greater => quadratic(digits(left) - digits(right) + 1, digits(right)),
            Ordering::Less | Ordering::Equal => 0,
        },
    }
}

/// Compare the magnitudes of two integers, as `big_ucomp` does.
fn magnitude_order(left: &Term, right: &Term) -> Ordering {
    match (left, right) {
        (Term::Integer(left), Term::Integer(right)) => {
            left.unsigned_abs().cmp(&right.unsigned_abs())
        }
        _ => {
            let (left, right) = (left.bigint(), right.bigint());
            let magnitude = |value: Option<std::borrow::Cow<'_, BigInt>>| {
                value.map(|value| value.magnitude().clone())
            };
            magnitude(left).cmp(&magnitude(right))
        }
    }
}

/// Run `erlang:'band'/2`, `erlang:'bor'/2`, or `erlang:'bxor'/2` on the
/// two's complement forms of two integers, as `band_2`, `bor_2`, `bxor_2`,
/// `erts_band`, `erts_bor`, and `erts_bxor` (`erl_arith.c:164-188`,
/// `1252-1337`) do. Two small integers cost no reduction. A bignum operand
/// costs [`linear`] of `max(size) + 2` words, also when the result is a
/// small integer. An operand that is not an integer raises `badarith`, and
/// a result above `BIG_ARITY_MAX` digits raises `system_limit`
/// (`erl_arith.c:1332-1335`).
///
/// # Examples
///
/// ```rust,ignore
/// let (result, reductions) = bitwise(Bitwise::And, &Term::Integer(12), &Term::Integer(10));
/// assert!(matches!(result, BifResult::Value(Term::Integer(8))));
/// assert_eq!(reductions, 0);
/// let (_, reductions) = bitwise(Bitwise::Or, &Term::Integer(1 << 59), &Term::Integer(1));
/// assert_eq!(reductions, 1);
/// ```
pub(super) fn bitwise(op: Bitwise, left: &Term, right: &Term) -> Charged {
    let reductions = || {
        if is_small(left) && is_small(right) {
            0
        } else {
            linear(digits(left).max(digits(right)) + 2)
        }
    };
    if let (Term::Integer(left_value), Term::Integer(right_value)) = (left, right) {
        let value = match op {
            Bitwise::And => left_value & right_value,
            Bitwise::Or => left_value | right_value,
            Bitwise::Xor => left_value ^ right_value,
        };
        return (Value(Term::Integer(value)), reductions());
    }
    let (Some(left_value), Some(right_value)) = (left.bigint(), right.bigint()) else {
        return (BifResult::badarith(), 0);
    };
    let value = match op {
        Bitwise::And => left_value.as_ref() & right_value.as_ref(),
        Bitwise::Or => left_value.as_ref() | right_value.as_ref(),
        Bitwise::Xor => left_value.as_ref() ^ right_value.as_ref(),
    };
    (big_result(value), reductions())
}

/// Run `erlang:'bnot'/1`: the bitwise complement `-N - 1`, as `bnot_1`
/// (`erl_arith.c:329-349`) does. A small integer costs no reduction, and a
/// bignum costs [`linear`] of `size + 2` words. An operand that is not an
/// integer raises `badarith`, and a result above `BIG_ARITY_MAX` digits
/// raises `system_limit` (`erl_arith.c:1350-1353`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(bnot(&Term::Integer(5)).0, BifResult::Value(Term::Integer(-6))));
/// assert!(matches!(bnot(&Term::Float(1.0)).0, BifResult::Raise { .. }));
/// ```
pub(super) fn bnot(term: &Term) -> Charged {
    if let Term::Integer(value) = term {
        let reductions = if is_small(term) { 0 } else { linear(3) };
        return (Value(Term::Integer(!value)), reductions);
    }
    let Some(value) = term.bigint() else {
        return (BifResult::badarith(), 0);
    };
    (big_result(!value.into_owned()), linear(digits(term) + 2))
}

/// Run `erlang:'bsl'/2` (`right_shift` false) or `erlang:'bsr'/2`
/// (`right_shift` true), as `erts_shift` (`erl_arith.c:190-309`) does. The
/// shift is arithmetic: a right shift rounds toward negative infinity.
///
/// A bignum shift amount is never small: `N bsl` a negative bignum and
/// `N bsr` a positive bignum shift right by 2^59 bits, and `N bsl` a
/// positive bignum and `N bsr` a negative bignum give `0` for `N = 0` and
/// raise `system_limit` for each other integer. A shift of a small integer
/// whose result is small costs no reduction. Each other shift costs
/// [`linear`] of the result size of `erts_shift` plus 2 words, and a result
/// size above `BIG_ARITY_MAX + 8` digits raises `system_limit` before the
/// work (`erl_arith.c:246-265`). An operand that is not an integer raises
/// `badarith`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(shift(&Term::Integer(1), &Term::Integer(4), false).0, BifResult::Value(Term::Integer(16))));
/// assert!(matches!(shift(&Term::Integer(-16), &Term::Integer(2), true).0, BifResult::Value(Term::Integer(-4))));
/// let (big, reductions) = shift(&Term::Integer(1), &Term::Integer(64), false);
/// assert!(matches!(big, BifResult::Value(Term::BigInt(_))));
/// assert_eq!(reductions, 1);
/// ```
pub(super) fn shift(left: &Term, right: &Term, right_shift: bool) -> Charged {
    let amount = match right.small_integer() {
        Some(amount) if right_shift => -amount,
        Some(amount) => amount,
        None if right.is_integer() => {
            let negative = right.bigint().is_some_and(|value| value.is_negative());
            if negative == right_shift {
                // A shift to the left by a bignum.
                return match left {
                    _ if *left == Term::Integer(0) => (Value(left.clone()), 0),
                    _ if left.is_integer() => (system_limit(), 0),
                    _ => (BifResult::badarith(), 0),
                };
            }
            MIN_SMALL
        }
        None => return (BifResult::badarith(), 0),
    };
    if !left.is_integer() {
        return (BifResult::badarith(), 0);
    }
    if amount == 0 || *left == Term::Integer(0) {
        return (Value(left.clone()), 0);
    }
    if let Some(small) = left.small_integer() {
        if amount < 0 {
            let bits = (-amount).min(SMALL_SHIFT_LIMIT);
            return (Value(Term::Integer(small >> bits)), 0);
        }
        if amount < SMALL_SHIFT_LIMIT {
            let shifted = i128::from(small) << amount;
            if (i128::from(MIN_SMALL)..=i128::from(MAX_SMALL)).contains(&shifted) {
                return (Value(Term::Integer(shifted as i64)), 0);
            }
        }
    }
    let size = digits(left) as i64;
    let result_size = if amount > 0 {
        size + amount / DIGIT_BITS
    } else if size <= -amount / DIGIT_BITS {
        3
    } else {
        size - (-amount / DIGIT_BITS)
    };
    if result_size - 8 > MAX_BIG_DIGITS as i64 {
        return (system_limit(), 0);
    }
    let reductions = linear(result_size as usize + 2);
    let value = left
        .bigint()
        .expect("the shifted term is an integer")
        .into_owned();
    let shifted = if amount > 0 {
        value << amount as u64
    } else {
        value >> (-amount) as u64
    };
    (big_result(shifted), reductions)
}

/// Run `erlang:'-'/1`: the negation of a number, as `sminus_1` and
/// `erts_unary_minus` (`erl_arith.c:107-110`, `509-567`) do. A small
/// integer costs no reduction, also when its negation is the bignum 2^59.
/// A bignum costs [`linear`] of `size + 1` words (`erl_arith.c:540-551`).
/// A float negates with no reduction. An operand that is not a number
/// raises `badarith`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(negate(&Term::Integer(5)).0, BifResult::Value(Term::Integer(-5))));
/// assert!(matches!(negate(&Term::Integer(i64::MIN)).0, BifResult::Value(Term::BigInt(_))));
/// ```
pub(super) fn negate(term: &Term) -> Charged {
    match term {
        Term::Float(value) => return (Value(Term::Float(-value)), 0),
        Term::Integer(value) if let Some(negation) = value.checked_neg() => {
            let reductions = if is_small(term) { 0 } else { linear(2) };
            return (Value(Term::Integer(negation)), reductions);
        }
        _ => {}
    }
    let Some(value) = term.bigint() else {
        return (BifResult::badarith(), 0);
    };
    (
        Value(Term::from_bigint(-value.into_owned())),
        linear(digits(term) + 1),
    )
}

/// Run `erlang:abs/1`: the absolute value of a number, as `abs_1`
/// (`erl_bif_guard.c:57-105`) does, with no reduction. `abs(-0.0)` is
/// `0.0`. Each other term raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(abs(&Term::Integer(-3)), BifResult::Value(Term::Integer(3))));
/// assert!(matches!(abs(&Term::Integer(i64::MIN)), BifResult::Value(Term::BigInt(_))));
/// assert!(matches!(abs(&Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn abs(term: &Term) -> BifResult {
    match term {
        Term::Float(value) => Value(Term::Float(value.abs())),
        Term::Integer(value) if let Some(value) = value.checked_abs() => {
            Value(Term::Integer(value))
        }
        _ => match term.bigint() {
            Some(value) => Value(Term::from_bigint(value.abs())),
            None => BifResult::badarg(),
        },
    }
}
