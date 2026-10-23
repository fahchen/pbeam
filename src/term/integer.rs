//! Integer terms outside the signed 64-bit range: the canonical form and
//! the conversions between a term and a [`BigInt`].

use std::{borrow::Cow, cmp::Ordering, sync::Arc};

use num_bigint::BigInt;
use num_traits::ToPrimitive;

use super::Term;

/// The largest number of 64-bit digits of an ERTS bignum on a 64-bit
/// system, `BIG_ARITY_MAX` (`erl_term.h:465-466`). A result with more
/// digits raises `system_limit`, and a literal with more digits does not
/// load (`bytes_to_big`, `big.c`).
pub(crate) const MAX_BIG_DIGITS: u64 = (1 << 16) - 1;

/// Return whether `value` fits an ERTS bignum: at most [`MAX_BIG_DIGITS`]
/// digits of 64 bits.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(fits_big_arity(&(BigInt::from(1) << 64)));
/// assert!(!fits_big_arity(&(BigInt::from(1) << (65535 * 64))));
/// ```
pub(crate) fn fits_big_arity(value: &BigInt) -> bool {
    value.bits() <= MAX_BIG_DIGITS * 64
}

impl Term {
    /// Return the integer `value` in its canonical form: [`Term::Integer`]
    /// for a value in the signed 64-bit range, else [`Term::BigInt`]. ERTS
    /// also normalizes each bignum result that fits a small integer
    /// (`big_norm`, `big.c`), so `=:=` and the hash agree.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::from_bigint(BigInt::from(7)), Term::Integer(7));
    /// assert!(matches!(Term::from_bigint(BigInt::from(1) << 64), Term::BigInt(_)));
    /// ```
    pub(crate) fn from_bigint(value: BigInt) -> Term {
        match i64::try_from(&value) {
            Ok(value) => Term::Integer(value),
            Err(_) => Term::BigInt(Arc::new(value)),
        }
    }

    /// Return the value of an integer term as a [`BigInt`], borrowed for a
    /// [`Term::BigInt`], or `None` for a term that is not an integer.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::Integer(-2).bigint().as_deref(), Some(&BigInt::from(-2)));
    /// assert_eq!(Term::nil().bigint(), None);
    /// ```
    pub(crate) fn bigint(&self) -> Option<Cow<'_, BigInt>> {
        match self {
            Term::Integer(value) => Some(Cow::Owned(BigInt::from(*value))),
            Term::BigInt(value) => Some(Cow::Borrowed(value)),
            _ => None,
        }
    }

    /// Return a number as a float, as the C cast of a small integer and
    /// `big_to_double` (`big.c:2411-2529`) give it: the nearest float, with
    /// ties to even. Return `None` for a term that is not a number, and for
    /// a bignum whose nearest float is not finite, where `big_to_double`
    /// fails.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Term::Integer(3).number_float(), Some(3.0));
    /// assert_eq!(Term::from_bigint(BigInt::from(1) << 64).number_float(), Some(18446744073709551616.0));
    /// assert_eq!(Term::from_bigint(BigInt::from(1) << 1024).number_float(), None);
    /// assert_eq!(Term::nil().number_float(), None);
    /// ```
    pub(crate) fn number_float(&self) -> Option<f64> {
        match self {
            Term::Integer(value) => Some(*value as f64),
            // `BigInt::to_f64` rounds to odd in 64 bits and then to the
            // nearest float, so it gives the value of `big_to_double`.
            Term::BigInt(value) => value.to_f64().filter(|value| value.is_finite()),
            Term::Float(value) => Some(*value),
            _ => None,
        }
    }

    /// Compare two integers by value, or return `None` when a term is not
    /// an integer. A big integer is outside the i64 range, so its sign
    /// orders it against an i64.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let big = Term::from_bigint(BigInt::from(1) << 64);
    /// assert_eq!(Term::Integer(1).integer_order(&big), Some(Ordering::Less));
    /// assert_eq!(big.integer_order(&Term::Float(1.0)), None);
    /// ```
    pub(crate) fn integer_order(&self, other: &Term) -> Option<Ordering> {
        match (self, other) {
            (Term::Integer(left), Term::Integer(right)) => Some(left.cmp(right)),
            _ if self.is_integer() && other.is_integer() => {
                Some(self.bigint()?.as_ref().cmp(other.bigint()?.as_ref()))
            }
            _ => None,
        }
    }

    /// Return whether the term is an integer, of either form.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Term::Integer(1).is_integer());
    /// assert!(!Term::Float(1.0).is_integer());
    /// ```
    pub(crate) fn is_integer(&self) -> bool {
        matches!(self, Term::Integer(_) | Term::BigInt(_))
    }
}
