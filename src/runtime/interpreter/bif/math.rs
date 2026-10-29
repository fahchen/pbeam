//! Bodies of the `math` BIFs, as ERTS `math_call_1` and `math_call_2`
//! (`erl_math.c:35-141`) give them.

use super::BifResult::Value;
use super::{BifResult, Term};

/// Return the double of an argument of a `math` BIF, as `math_call_1` and
/// `math_call_2` read one (`erl_math.c:43-56`, `107-133`). A float, a small
/// integer, and a bignum that `big_to_double` converts give their nearest
/// double, with ties to even, as [`Term::number_float`] gives it. A bignum
/// that `big_to_double` cannot convert, because its nearest double is not
/// finite, gives `error:badarith`. Each other term gives `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(double(&Term::Integer(3)), Ok(3.0));
/// assert_eq!(double(&Term::from_bigint(BigInt::from(1) << 1024)), Err(BifResult::badarith()));
/// assert!(matches!(double(&Term::nil()), Err(BifResult::Raise { .. })));
/// ```
fn double(term: &Term) -> Result<f64, BifResult> {
    // `big_to_double` fails only for a bignum (`erl_math.c:47-52`), so the
    // other terms that give no double are not numbers.
    match term {
        Term::BigInt(_) => term.number_float().ok_or_else(BifResult::badarith),
        _ => term.number_float().ok_or_else(BifResult::badarg),
    }
}

/// Return the float result of a `math` BIF, or `error:badarith` for a
/// result that is not finite, as `ERTS_FP_ERROR_THOROUGH` does for the
/// result of each `math` function (`sys/unix/erl_unix_sys.h:388-398`).
/// `math_call_1` and `math_call_2` apply it after the function and before
/// they make the float term (`erl_math.c:57-58`, `135-136`), so
/// `math:sqrt(-1)` and `math:log(0)` raise `badarith` and an underflow to
/// `0.0` returns `0.0`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(finite_float(2.0), BifResult::Value(Term::Float(value)) if value == 2.0));
/// assert!(matches!(finite_float(f64::INFINITY), BifResult::Raise { .. }));
/// ```
fn finite_float(value: f64) -> BifResult {
    if value.is_finite() {
        Value(Term::Float(value))
    } else {
        BifResult::badarith()
    }
}

/// Run a `math` BIF of one argument through the function of the C library
/// that ERTS calls (`math_call_1`, `erl_math.c:35-63`). The table gives the
/// `math` BIFs of `erl_math.c:143-292` that call `sin`, `cos`, `tan`,
/// `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`, `log2`, `ceil`, and
/// `floor` to it; the `f64` method of each calls the function of the C
/// library of the platform. `math_call_errno_1` of `acos` and `asin`
/// (`erl_math.c:69-96`) adds no case: a C99 library returns NaN
/// for a domain error, and [`finite_float`] raises `badarith` for it.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(call_1(f64::exp, &Term::Integer(1)), BifResult::Value(Term::Float(value)) if value == 2.718281828459045));
/// ```
pub(super) fn call_1(function: fn(f64) -> f64, argument: &Term) -> BifResult {
    match double(argument) {
        Ok(value) => finite_float(function(value)),
        Err(error) => error,
    }
}

/// Run `math:exp/1`, as ERTS `math_exp_1` does (`erl_math.c:235-238`).
/// `f64::exp` calls the `exp` of the C library of the platform, which is
/// the function that ERTS calls.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(exp(&Term::Integer(0)), BifResult::Value(Term::Float(value)) if value == 1.0));
/// assert!(matches!(exp(&Term::Float(1000.0)), BifResult::Raise { .. }));
/// ```
pub(super) fn exp(argument: &Term) -> BifResult {
    call_1(f64::exp, argument)
}

/// Run `math:log/1`, as ERTS `math_log_1` does (`erl_math.c:240-243`). See
/// [`exp`].
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(log(&Term::Integer(1)), BifResult::Value(Term::Float(value)) if value == 0.0));
/// assert!(matches!(log(&Term::Integer(0)), BifResult::Raise { .. }));
/// ```
pub(super) fn log(argument: &Term) -> BifResult {
    call_1(f64::ln, argument)
}

/// Run `math:log10/1`, as ERTS `math_log10_1` does (`erl_math.c:264-267`).
/// See [`exp`].
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(log10(&Term::Integer(100)), BifResult::Value(Term::Float(value)) if value == 2.0));
/// assert!(matches!(log10(&Term::Integer(0)), BifResult::Raise { .. }));
/// ```
pub(super) fn log10(argument: &Term) -> BifResult {
    call_1(f64::log10, argument)
}

/// Run `math:sqrt/1`, as ERTS `math_sqrt_1` does (`erl_math.c:269-272`).
/// See [`exp`].
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(sqrt(&Term::Integer(4)), BifResult::Value(Term::Float(value)) if value == 2.0));
/// assert!(matches!(sqrt(&Term::Integer(-1)), BifResult::Raise { .. }));
/// ```
pub(super) fn sqrt(argument: &Term) -> BifResult {
    call_1(f64::sqrt, argument)
}

/// Run `math:pow/2`, as ERTS `math_pow_2` does (`erl_math.c:279-282`),
/// through the function of the C library that `math_call_2` calls
/// (`erl_math.c:98-141`). `f64::powf` calls the `pow` of the C library of
/// the platform, which is the function that ERTS calls.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(pow(&Term::Integer(2), &Term::Integer(10)), BifResult::Value(Term::Float(value)) if value == 1024.0));
/// assert!(matches!(pow(&Term::Integer(0), &Term::Integer(-1)), BifResult::Raise { .. }));
/// assert!(matches!(pow(&Term::Integer(1), &Term::nil()), BifResult::Raise { .. }));
/// ```
pub(super) fn pow(left: &Term, right: &Term) -> BifResult {
    call_2(f64::powf, left, right)
}

/// Run a `math` BIF of two arguments through the function of the C library
/// that ERTS calls (`math_call_2`, `erl_math.c:98-141`). The first argument
/// that gives no double gives the error. The table gives `math:atan2/2`
/// (`erl_math.c:274-277`, `f64::atan2`) and `math:fmod/2`
/// (`erl_math.c:294-297`, [`fmod`]) to it.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(call_2(f64::atan2, &Term::Integer(0), &Term::Integer(1)), BifResult::Value(Term::Float(value)) if value == 0.0));
/// ```
pub(super) fn call_2(function: fn(f64, f64) -> f64, left: &Term, right: &Term) -> BifResult {
    match (double(left), double(right)) {
        (Ok(left), Ok(right)) => finite_float(function(left, right)),
        (Err(error), _) | (_, Err(error)) => error,
    }
}

/// Return the C `fmod` of two doubles. The Rust `%` of `f64` is the exact
/// remainder with the sign of the dividend, as C `fmod` is, and a zero
/// divisor gives NaN, so `math:fmod(1, 0)` raises `badarith`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(fmod(-5.0, 3.0), -2.0);
/// assert!(fmod(1.0, 0.0).is_nan());
/// ```
pub(super) fn fmod(left: f64, right: f64) -> f64 {
    left % right
}
