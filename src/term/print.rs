//! The text of a term, as ERTS OTP 29.1.1 prints it with `%T`
//! (`print_term` in `erl_printf_term.c:352-811`).
//!
//! `erlang:display/1` and `erts_internal:term_to_string/2` print a term with
//! `%T`. The text is a byte string: an atom gives the UTF-8 bytes of its
//! name, and a printable list or binary gives its bytes as they are. ERTS
//! prints each term kind that PBeam represents as follows:
//!
//! | Term | Text |
//! | --- | --- |
//! | atom | the name, in quotes when it is not a Latin-1 lowercase letter followed by Latin-1 letters, digits, and `_` |
//! | integer | decimal, for example `-7` |
//! | proper list of bytes that are not control characters | `"..."` with `\n` and `"` escaped |
//! | other list | `[1,2|x]` |
//! | tuple | `{a,{b}}` |
//! | binary of bytes in 32..=126 | `<<"a\"b">>` |
//! | other binary | `<<>>`, `<<1,2,200>>` |
//! | map | `#{a=>1}`, in the ERTS key order |
//! | local fun | `#Fun<Module.OldIndex.OldUniq>` |
//! | external fun | `fun lists:map/2` |
//! | pid | `<0.Number.Serial>` |
//! | reference | `#Ref<0.N2.N1.N0>` |

use super::{Fun, Target, Term, match_context::MATCH_CONTEXT_ESCAPE};

/// The number of low bits of the pid data that ERTS prints as the pid
/// number (`_PID_NUM_SIZE`, `erl_term.h:646-656`). The higher bits are the
/// serial.
const PID_NUMBER_BITS: u32 = 28;
/// The mask of the pid number in the pid data.
const PID_NUMBER_MASK: u64 = (1 << PID_NUMBER_BITS) - 1;

/// An item of the work stack of `print_term`: a term to print, or a fixed
/// text, as the `PRT_*` markers of `print_term` are.
enum Item<'a> {
    Term(&'a Term),
    Text(&'static str),
}

/// Return the `%T` text of a term, as ERTS `print_term`
/// (`erl_printf_term.c:352-811`) prints it with no length limit. A deep term
/// does not recurse on the Rust stack: the function keeps a work stack, as
/// `print_term` does. A pid prints its PBeam process number as the ERTS pid
/// data, and a reference prints its `Reference::numbers`.
///
/// # Examples
///
/// ```rust,ignore
/// let term = Term::tuple(vec![Term::static_atom("undef"), Term::nil()]);
/// assert_eq!(print_term(&term), b"{undef,[]}");
/// let string = Term::list_from(b"ab\n".iter().map(|&byte| Term::Integer(byte.into())));
/// assert_eq!(print_term(&string), b"\"ab\\n\"");
/// assert_eq!(print_term(&Term::binary(vec![1, 2])), b"<<1,2>>");
/// ```
pub(crate) fn print_term(term: &Term) -> Vec<u8> {
    let mut text = Vec::new();
    let mut items = vec![Item::Term(term)];
    while let Some(item) = items.pop() {
        let term = match item {
            Item::Text(fixed) => {
                text.extend_from_slice(fixed.as_bytes());
                continue;
            }
            Item::Term(term) => term,
        };
        match term {
            Term::Atom(atom) => print_atom(atom.name(), &mut text),
            Term::Integer(value) => text.extend_from_slice(value.to_string().as_bytes()),
            Term::Nil => text.extend_from_slice(b"[]"),
            Term::Cons(_)
                if let Some(elements) = term.proper_list()
                    && let Some(bytes) = printable_string(elements) =>
            {
                text.push(b'"');
                for byte in bytes {
                    match byte {
                        b'\n' => text.extend_from_slice(b"\\n"),
                        b'"' => text.extend_from_slice(b"\\\""),
                        byte => text.push(byte),
                    }
                }
                text.push(b'"');
            }
            Term::Cons(_) => {
                let (elements, tail) = term.list_cells().expect("the term is a list");
                text.push(b'[');
                items.push(Item::Text("]"));
                if let Some(tail) = tail {
                    items.push(Item::Term(tail));
                    items.push(Item::Text("|"));
                }
                push_separated(&mut items, elements.collect::<Vec<_>>().into_iter());
            }
            Term::Tuple(elements) => {
                text.push(b'{');
                items.push(Item::Text("}"));
                push_separated(&mut items, elements.iter());
            }
            Term::Binary(bytes)
                if !bytes.is_empty() && bytes.iter().all(|byte| (32..127).contains(byte)) =>
            {
                text.extend_from_slice(b"<<\"");
                for &byte in bytes.iter() {
                    if byte == b'"' {
                        text.push(b'\\');
                    }
                    text.push(byte);
                }
                text.extend_from_slice(b"\">>");
            }
            Term::Binary(bytes) => {
                let numbers: Vec<String> = bytes.iter().map(u8::to_string).collect();
                text.extend_from_slice(format!("<<{}>>", numbers.join(",")).as_bytes());
            }
            Term::Map(map) => {
                text.extend_from_slice(b"#{");
                items.push(Item::Text("}"));
                let pairs: Vec<_> = map.iter().collect();
                for (index, (key, value)) in pairs.into_iter().enumerate().rev() {
                    items.push(Item::Term(value));
                    items.push(Item::Text("=>"));
                    items.push(Item::Term(key));
                    if index > 0 {
                        items.push(Item::Text(","));
                    }
                }
            }
            Term::Fun(Fun {
                target: Target::Local(lambda),
                ..
            }) => text.extend_from_slice(
                format!(
                    "#Fun<{}.{}.{}>",
                    lambda.module().name(),
                    lambda.index,
                    lambda.old_unique
                )
                .as_bytes(),
            ),
            Term::Fun(Fun {
                target:
                    Target::External {
                        module,
                        function,
                        arity,
                    },
                ..
            }) => {
                text.extend_from_slice(b"fun ");
                print_atom(module.name(), &mut text);
                text.push(b':');
                print_atom(function.name(), &mut text);
                text.extend_from_slice(format!("/{arity}").as_bytes());
            }
            Term::Pid(pid) => {
                let data = pid.0;
                text.extend_from_slice(
                    format!("<0.{}.{}>", data & PID_NUMBER_MASK, data >> PID_NUMBER_BITS)
                        .as_bytes(),
                );
            }
            Term::Port(port) => text.extend_from_slice(format!("#Port<0.{}>", port.0).as_bytes()),
            Term::Reference(reference) => {
                let [n0, n1, n2] = reference.numbers();
                text.extend_from_slice(format!("#Ref<0.{n2}.{n1}.{n0}>").as_bytes());
            }
            Term::MatchContext(_) => unreachable!("{MATCH_CONTEXT_ESCAPE}"),
        }
    }
    text
}

/// Push the items of `elements` separated by commas, so that the first
/// element pops first.
///
/// # Examples
///
/// ```rust,ignore
/// let elements = [Term::Integer(1), Term::Integer(2)];
/// let mut items = Vec::new();
/// push_separated(&mut items, elements.iter());
/// // The items pop as 1, ",", 2.
/// assert_eq!(items.len(), 3);
/// ```
fn push_separated<'a>(
    items: &mut Vec<Item<'a>>,
    elements: impl DoubleEndedIterator<Item = &'a Term> + ExactSizeIterator,
) {
    for (index, element) in elements.enumerate().rev() {
        items.push(Item::Term(element));
        if index > 0 {
            items.push(Item::Text(","));
        }
    }
}

/// Return the bytes of a list that ERTS prints as a string
/// (`is_printable_string`, `erl_printf_term.c:147-168`): each element is an
/// integer in 0..=255 that is not a Latin-1 control character, or is a
/// space, `\n`, `\t`, or `\r`. Return `None` for any other list. The caller
/// gives a proper list that is not empty.
///
/// # Examples
///
/// ```rust,ignore
/// let list = |values: &[i64]| values.iter().map(|&value| Term::Integer(value)).collect::<Vec<_>>();
/// assert_eq!(printable_string(&list(&[160, 255, 9])), Some(vec![160, 255, 9]));
/// assert_eq!(printable_string(&list(&[128])), None);
/// assert_eq!(printable_string(&list(&[300])), None);
/// ```
fn printable_string<'a>(elements: impl IntoIterator<Item = &'a Term>) -> Option<Vec<u8>> {
    elements
        .into_iter()
        .map(|element| match element {
            Term::Integer(value) => u8::try_from(*value)
                .ok()
                .filter(|&byte| !is_control(byte) || matches!(byte, b' ' | b'\n' | b'\t' | b'\r')),
            _ => None,
        })
        .collect()
}

/// Return whether a Latin-1 byte is a control character for a string:
/// below 32, 127, or 128..=159 (`IS_CNTRL`, `erl_printf_term.c:136-142`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_control(127));
/// assert!(!is_control(160));
/// ```
fn is_control(byte: u8) -> bool {
    matches!(byte, 0..32 | 127..160)
}

/// Return whether a code point is a Latin-1 lowercase letter (`IS_LOWER`,
/// `erl_printf_term.c:123-124`): `a`..=`z`, or 223..=255 without 247.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_lower('é'));
/// assert!(!is_lower('Ä'));
/// ```
fn is_lower(code_point: char) -> bool {
    matches!(code_point, 'a'..='z' | '\u{df}'..='\u{f6}' | '\u{f8}'..='\u{ff}')
}

/// Return whether a code point is a Latin-1 uppercase letter (`IS_UPPER`,
/// `erl_printf_term.c:126-127`): `A`..=`Z`, or 192..=222 without 215.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_upper('Ä'));
/// assert!(!is_upper('é'));
/// ```
fn is_upper(code_point: char) -> bool {
    matches!(code_point, 'A'..='Z' | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{de}')
}

/// Write an atom name, in quotes when it needs them, as `print_atom_name`
/// (`erl_printf_term.c:235-327`) does. The name needs no quotes when its
/// first code point is a Latin-1 lowercase letter and each other code point
/// is a Latin-1 letter, a digit, or `_`. A reserved word such as `end` gets
/// no quotes. In the text, `'`, `\`, and the escapes `\n`, `\f`, `\t`, `\r`,
/// `\b`, and `\v` get a backslash, and each other code point below 32 or
/// in 128..=159 is written as `\ooo` in octal. Each other code point is
/// written as its UTF-8 bytes.
///
/// # Examples
///
/// ```rust,ignore
/// let mut text = Vec::new();
/// print_atom("it's", &mut text);
/// assert_eq!(text, b"'it\\'s'");
/// let mut text = Vec::new();
/// print_atom("\u{80}", &mut text);
/// assert_eq!(text, b"'\\200'");
/// ```
fn print_atom(name: &str, text: &mut Vec<u8>) {
    let mut code_points = name.chars();
    let plain = code_points.next().is_some_and(is_lower)
        && code_points.all(|code_point| {
            is_lower(code_point)
                || is_upper(code_point)
                || code_point.is_ascii_digit()
                || code_point == '_'
        });
    if !plain {
        text.push(b'\'');
    }
    for code_point in name.chars() {
        match code_point {
            '\'' => text.extend_from_slice(b"\\'"),
            '\\' => text.extend_from_slice(b"\\\\"),
            '\n' => text.extend_from_slice(b"\\n"),
            '\u{c}' => text.extend_from_slice(b"\\f"),
            '\t' => text.extend_from_slice(b"\\t"),
            '\r' => text.extend_from_slice(b"\\r"),
            '\u{8}' => text.extend_from_slice(b"\\b"),
            '\u{b}' => text.extend_from_slice(b"\\v"),
            '\0'..='\u{1f}' | '\u{80}'..='\u{9f}' => {
                text.extend_from_slice(format!("\\{:03o}", u32::from(code_point)).as_bytes());
            }
            _ => text.extend_from_slice(code_point.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    if !plain {
        text.push(b'\'');
    }
}

#[cfg(test)]
#[path = "../../tests/term/print.rs"]
mod tests;
