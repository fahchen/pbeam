//! The text of a float, as ERTS OTP 29.1.1 writes it.
//!
//! ERTS has three float text forms:
//!
//! - [`shortest`]: the shortest text that reads back as the same float, as
//!   `d2s_buffered_n` and `to_chars` of the bundled Ryu write it
//!   (`erts/emulator/ryu/d2s.c:336-378`, `erts/emulator/ryu/to_chars.h`).
//!   `float_to_list/2` and `float_to_binary/2` use it for the `short`
//!   option, and `io_lib` prints `~p` and `~w` with it;
//! - [`scientific`]: the C text `%.*e`, as `sys_double_to_chars_ext`
//!   (`sys/unix/sys_float.c:55-68`) writes it;
//! - [`fixed`]: the text of `sys_double_to_chars_fast`
//!   (`sys/common/erl_sys_common_misc.c:161-273`) for the `decimals`
//!   option.
//!
//! The Rust standard library gives the digits. Its shortest form `{:e}`
//! gives the shortest digits that read back as the float, and the closest
//! such digits to the exact value, as Ryu does. Its exact forms `{:.N}` and
//! `{:.Ne}` round the exact decimal value half to even, as the C library
//! does. This module only lays the digits out as ERTS does.

/// The largest text that ERTS writes: the BIFs use a buffer of 256 bytes,
/// and a text of 256 bytes or more fails (`bif.c:3449-3456`).
const BUFFER_SIZE: usize = 256;

/// The largest `fixed` value that the fast path of
/// `sys_double_to_chars_fast` converts: `MAX_FLOAT`, 2^53
/// (`erl_sys_common_misc.c:170`).
const MAX_FAST_FLOAT: f64 = 9_007_199_254_740_992.0;

/// The powers of ten of the fast path, `pow10v` (`erl_sys_common_misc.c:173-176`).
const POW10: [f64; 19] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18,
];

/// Return the shortest text of a float that reads back as the same float,
/// as ERTS Ryu writes it. The text is fixed notation when the exponent is
/// near the digits, else scientific notation with at least one digit after
/// the point and no `+` in the exponent (`to_chars.h`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(shortest(0.1), b"0.1");
/// assert_eq!(shortest(1.0e20), b"1.0e20");
/// assert_eq!(shortest(123456789.0), b"123456789.0");
/// assert_eq!(shortest(1.0e-5), b"1.0e-5");
/// assert_eq!(shortest(-0.0), b"-0.0");
/// ```
pub(crate) fn shortest(value: f64) -> Vec<u8> {
    let mut text = Vec::new();
    if value.is_sign_negative() {
        text.push(b'-');
    }
    if value == 0.0 {
        // `copy_special_str` (`d2s.c`).
        text.extend_from_slice(b"0.0");
        return text;
    }
    let (digits, scientific_exponent) = shortest_digits(value.abs());
    let length = digits.len() as i32;
    let exponent = scientific_exponent - (length - 1);
    let output = digits
        .iter()
        .fold(0_u64, |output, digit| output * 10 + u64::from(digit - b'0'));
    let (lower, upper) = if length == 1 {
        (-4, 2)
    } else if scientific_exponent >= 10 {
        (-(length + 2), 2)
    } else {
        (-(length + 2), 1)
    };
    let fixed = lower <= exponent
        && exponent <= upper
        && !((output >= 1 << 53 && exponent == 0)
            || (output > (1 << 52) / 5 && exponent == 1)
            || (output > (1 << 51) / 25 && exponent == 2));
    if fixed {
        let whole = length + exponent;
        if exponent >= 0 {
            text.extend_from_slice(&digits);
            text.extend(std::iter::repeat_n(b'0', exponent as usize));
            text.extend_from_slice(b".0");
        } else if whole > 0 {
            text.extend_from_slice(&digits[..whole as usize]);
            text.push(b'.');
            text.extend_from_slice(&digits[whole as usize..]);
        } else {
            text.extend_from_slice(b"0.");
            text.extend(std::iter::repeat_n(b'0', (-whole) as usize));
            text.extend_from_slice(&digits);
        }
    } else {
        text.push(digits[0]);
        text.push(b'.');
        if length == 1 {
            text.push(b'0');
        } else {
            text.extend_from_slice(&digits[1..]);
        }
        text.push(b'e');
        text.extend_from_slice(scientific_exponent.to_string().as_bytes());
    }
    text
}

/// Return the shortest digits of a positive float and the exponent of the
/// first digit, as Ryu `d2d` gives them (`ryu/d2s.c:100-292`).
///
/// The Rust shortest form gives the same digits, except for an exact tie:
/// when the float is exactly halfway between two shortest candidates,
/// Rust takes the upper one, and Ryu takes the even one ("Round even if the
/// exact number is .....50..0", `ryu/d2s.c:246-249`). A tie is rare: the
/// exact value of the float has one digit more than the shortest digits,
/// and that digit is 5. The function checks the tie with the exact digits
/// of the float, which have at most 767 significant digits.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(shortest_digits(0.1), (b"1".to_vec(), -1));
/// // 2012773988401152.25 is a tie between ...152.2 and ...152.3.
/// assert_eq!(shortest_digits(2012773988401152.25), (b"20127739884011522".to_vec(), 15));
/// ```
fn shortest_digits(value: f64) -> (Vec<u8>, i32) {
    let (mut digits, exponent) = scientific_digits(&format!("{value:e}"));
    let length = digits.len();
    let last = digits[length - 1];
    if (last - b'0') % 2 == 1 {
        let mut tie = digits.clone();
        tie[length - 1] = last - 1;
        tie.push(b'5');
        // The text with one digit more rounds to the tie only near a tie.
        // Only then does the function write all exact digits.
        if scientific_digits(&format!("{value:.length$e}")) == (tie.clone(), exponent) {
            let (exact, _) = scientific_digits(&format!("{value:.766e}"));
            let mut even = digits.clone();
            even[length - 1] = last - 1;
            // Ryu takes the even digits only when they are in the interval
            // of the float (`ryu/d2s.c:251`), that is, when they read back
            // as the float.
            let text = format!("0.{}e{}", String::from_utf8_lossy(&even), exponent + 1);
            if exact[length + 1..].iter().all(|&digit| digit == b'0')
                && text.parse::<f64>() == Ok(value)
            {
                digits = even;
            }
        }
    }
    (digits, exponent)
}

/// Return the digits and the exponent of a Rust `{:e}` text.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(scientific_digits("1.25e-3"), (b"125".to_vec(), -3));
/// ```
fn scientific_digits(text: &str) -> (Vec<u8>, i32) {
    let (mantissa, exponent) = text.split_once('e').expect("`{:e}` writes an exponent");
    let digits = mantissa.bytes().filter(|&byte| byte != b'.').collect();
    (
        digits,
        exponent.parse().expect("`{:e}` writes a decimal exponent"),
    )
}

/// Return the C text `%.*e` of a float with `precision` digits after the
/// point: the exponent has a sign and at least two digits.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(scientific(1.5, 6), b"1.500000e+00");
/// assert_eq!(scientific(1.0e-100, 2), b"1.00e-100");
/// assert_eq!(scientific(2.5, 0), b"2e+00");
/// ```
pub(crate) fn scientific(value: f64, precision: usize) -> Vec<u8> {
    let text = format!("{value:.precision$e}");
    let (mantissa, exponent) = text.split_once('e').expect("`{:e}` writes an exponent");
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(digits) => ('-', digits),
        None => ('+', exponent),
    };
    format!("{mantissa}e{sign}{digits:0>2}").into_bytes()
}

/// Return the text of `float_to_list/2` with `{scientific, Decimals}`, or
/// `None` for a text of 256 bytes or more, as `sys_double_to_chars_ext`
/// (`sys/unix/sys_float.c:55-68`) gives it. ERTS passes the C `int`
/// `decimals` as the precision of `%.*e`, and C reads a negative precision
/// as no precision, which is 6 digits.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(scientific_option(1.5, 3), Some(b"1.500e+00".to_vec()));
/// assert_eq!(scientific_option(1.5, -1), Some(b"1.500000e+00".to_vec()));
/// assert_eq!(scientific_option(1.5, 251), None);
/// ```
pub(crate) fn scientific_option(value: f64, decimals: i32) -> Option<Vec<u8>> {
    let precision = usize::try_from(decimals).unwrap_or(6);
    // The text has the precision digits and at least five other bytes, so
    // a larger precision cannot fit. The check keeps a large precision from
    // allocating a large text.
    if precision >= BUFFER_SIZE {
        return None;
    }
    let text = scientific(value, precision);
    (text.len() < BUFFER_SIZE).then_some(text)
}

/// Return the text of `float_to_list/2` with `{decimals, Decimals}`, and
/// with `compact` when `compact` is true, or `None` when ERTS fails, as
/// `sys_double_to_chars_fast` (`erl_sys_common_misc.c:161-273`) gives it.
/// A negative `decimals` fails. A float above 2^53 or 19 decimals or more
/// takes the C text `%.*f`, which fails at 256 bytes or more. Else the
/// float rounds with C `round` half away from zero, so `2.5` with 0
/// decimals gives `3`, and `0.125` with 2 decimals gives `0.13`. `compact`
/// removes the trailing zeros, but keeps one zero after the point.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(fixed(1.5, 0, false), Some(b"2".to_vec()));
/// assert_eq!(fixed(0.125, 2, false), Some(b"0.13".to_vec()));
/// assert_eq!(fixed(1.0, 3, true), Some(b"1.0".to_vec()));
/// assert_eq!(fixed(-0.0, 1, false), Some(b"-0.0".to_vec()));
/// assert_eq!(fixed(1.0, -1, false), None);
/// assert_eq!(fixed(1.0e300, 2, false), None);
/// ```
pub(crate) fn fixed(value: f64, decimals: i32, compact: bool) -> Option<Vec<u8>> {
    let decimals = usize::try_from(decimals).ok()?;
    let magnitude = value.abs();
    if magnitude > MAX_FAST_FLOAT || decimals >= POW10.len() {
        // The text has the decimals and at least two other bytes.
        if decimals >= BUFFER_SIZE {
            return None;
        }
        let mut text = format!("{value:.decimals$}").into_bytes();
        if text.len() >= BUFFER_SIZE {
            return None;
        }
        if compact {
            trim_trailing_zeros(&mut text);
        }
        return Some(text);
    }
    let mut text = Vec::new();
    if value.is_sign_negative() {
        text.push(b'-');
    }
    if decimals == 0 {
        // `round_int64` is C `lround`, which rounds half away from zero.
        text.extend_from_slice((magnitude.round() as u64).to_string().as_bytes());
        return Some(text);
    }
    let whole = magnitude.floor();
    let fraction = ((magnitude - whole) * POW10[decimals]).round();
    let (whole, fraction) = if fraction >= POW10[decimals] {
        (whole as u64 + 1, 0)
    } else {
        (whole as u64, fraction as u64)
    };
    text.extend_from_slice(format!("{whole}.{fraction:0decimals$}").as_bytes());
    if compact {
        trim_trailing_zeros(&mut text);
    }
    Some(text)
}

/// Remove the trailing zeros of a text, but keep one zero after a point,
/// as `find_first_trailing_zero` (`erl_sys_common_misc.c:134-139`) does.
/// A text with no point loses its trailing zeros too, as in ERTS.
///
/// # Examples
///
/// ```rust,ignore
/// let mut text = b"1.500".to_vec();
/// trim_trailing_zeros(&mut text);
/// assert_eq!(text, b"1.5");
/// let mut text = b"1.000".to_vec();
/// trim_trailing_zeros(&mut text);
/// assert_eq!(text, b"1.0");
/// ```
fn trim_trailing_zeros(text: &mut Vec<u8>) {
    let mut end = text.len();
    while end > 0 && text[end - 1] == b'0' {
        end -= 1;
    }
    if end > 0 && text[end - 1] == b'.' {
        end += 1;
    }
    text.truncate(end);
}
