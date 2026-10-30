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
//! | float | `%.6e`, for example `1.500000e+00` |
//! | proper list of bytes that are not control characters | `"..."` with `\n` and `"` escaped |
//! | other list | `[1,2|x]` |
//! | tuple | `{a,{b}}` |
//! | binary of bytes in 32..=126 | `<<"a\"b">>` |
//! | other binary | `<<>>`, `<<1,2,200>>`, and a bitstring as `<<1,2:3>>` |
//! | map | `#{a=>1}`, in the ERTS key order |
//! | local fun | `#Fun<Module.OldIndex.OldUniq>` |
//! | external fun | `fun lists:map/2` |
//! | pid | `<0.Number.Serial>` |
//! | reference | `#Ref<0.N2.N1.N0>` |

use super::external_id::{ExternalData, PID_NUMBER_BITS, PID_NUMBER_MASK};
use super::{AtomTable, Reference, Target, Term, match_context::MATCH_CONTEXT_ESCAPE};

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
/// data, and a reference prints its `Reference::numbers`. `atoms` is the
/// table of the run that owns `term`, so each atom name comes from the table.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// let term = Term::tuple(vec![Term::static_atom("undef"), Term::nil()]);
/// assert_eq!(print_term(&term, &atoms), b"{undef,[]}");
/// let string = Term::list_from(b"ab\n".iter().map(|&byte| Term::Integer(byte.into())));
/// assert_eq!(print_term(&string, &atoms), b"\"ab\\n\"");
/// assert_eq!(print_term(&Term::binary(vec![1, 2]), &atoms), b"<<1,2>>");
/// ```
pub(crate) fn print_term(term: &Term, atoms: &AtomTable) -> Vec<u8> {
    let mut count = i64::MAX;
    print_term_counted(term, atoms, &mut count)
}

/// Return the `%.*T` text of a term with the precision `limit`, as
/// `erts_printf_term` (`erl_printf_term.c:813-824`) prints it: the text of
/// [`print_term`] up to the point where the count runs out, then `... ` when
/// the count is at most 0 at the end.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// assert_eq!(print_term_limited(&Term::static_atom("ab"), &atoms, 2), b"ab... ");
/// ```
pub(crate) fn print_term_limited(term: &Term, atoms: &AtomTable, limit: i64) -> Vec<u8> {
    let mut count = limit;
    let mut text = print_term_counted(term, atoms, &mut count);
    if count <= 0 {
        text.extend_from_slice(b"... ");
    }
    text
}

/// Return the text of `print_term` (`erl_printf_term.c:352-811`) with the
/// count `dcount`. Each term takes 1 from the count before it prints, and
/// the print stops when the count was at most 0 (`:428-429`). An atom also
/// takes the byte length of its name (`:263`), and the print stops after
/// an atom when the count is at most 0 (`:106-114`), and an external fun the
/// lengths of its module and function names (`:698-718`). Each character
/// of a string takes 1 before it prints (`:537-538`), and each byte of a
/// binary takes 1 after it prints (`:631-632`, `:667-668`).
#[expect(
    clippy::too_many_lines,
    reason = "mirrors `print_term` of `erl_printf_term.c` with its `dcount` accounting: one arm for each term type, and the arms share the work stack"
)]
fn print_term_counted(term: &Term, atoms: &AtomTable, dcount: &mut i64) -> Vec<u8> {
    let mut text = Vec::new();
    let mut items = vec![Item::Term(term)];
    while let Some(item) = items.pop() {
        let term = match item {
            Item::Text(fixed) => {
                text.extend_from_slice(fixed.as_bytes());
                continue;
            }
            Item::Term(_) if take(dcount) => break,
            Item::Term(term) => term,
        };
        match term {
            Term::Atom(atom) => {
                let name = atoms.name_of(atom);
                *dcount -= len(name);
                print_atom(name, &mut text);
                // `PRINT_ATOM` stops when the count is at most 0
                // (`erl_printf_term.c:106-114`).
                if *dcount <= 0 {
                    break;
                }
            }
            Term::Integer(value) => text.extend_from_slice(value.to_string().as_bytes()),
            Term::BigInt(value) => text.extend_from_slice(value.to_string().as_bytes()),
            // `PRINT_DOUBLE(res, fn, arg, 'e', 6, 0, ff.fd)`
            // (`erl_printf_term.c:596-600`): six digits after the point.
            Term::Float(value) => {
                text.extend_from_slice(&crate::term::float_text::scientific(*value, 6));
            }
            Term::Nil => text.extend_from_slice(b"[]"),
            Term::Cons(_)
                if let Some(elements) = term.proper_list()
                    && let Some(bytes) = printable_string(elements) =>
            {
                text.push(b'"');
                for byte in bytes {
                    if take(dcount) {
                        return text;
                    }
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
            Term::Binary(binary)
                if binary.bytes().is_some_and(|bytes| {
                    !bytes.is_empty() && bytes.iter().all(|byte| (32..127).contains(byte))
                }) =>
            {
                text.extend_from_slice(b"<<\"");
                for &byte in binary.bit_bytes() {
                    if byte == b'"' {
                        text.push(b'\\');
                    }
                    text.push(byte);
                    if take(dcount) {
                        return text;
                    }
                }
                text.extend_from_slice(b"\">>");
            }
            Term::Binary(binary) => {
                // A bitstring prints its whole bytes, then its tail bits as
                // `Value:Size` (`erl_printf_term.c:615-653`).
                let size = binary.bit_size();
                let bits = binary.bit_bytes();
                let tail = size % 8;
                text.extend_from_slice(b"<<");
                for (index, byte) in bits[..size / 8].iter().enumerate() {
                    if index > 0 {
                        text.push(b',');
                    }
                    text.extend_from_slice(byte.to_string().as_bytes());
                    if take(dcount) {
                        return text;
                    }
                }
                if tail != 0 {
                    if size >= 8 {
                        text.push(b',');
                    }
                    let value = bits[size / 8] >> (8 - tail);
                    text.extend_from_slice(format!("{value}:{tail}").as_bytes());
                }
                text.extend_from_slice(b">>");
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
            Term::Fun(fun) => match fun.target() {
                Target::Local(lambda) => text.extend_from_slice(
                    format!(
                        "#Fun<{}.{}.{}>",
                        atoms.name_of(lambda.module()),
                        lambda.old_index,
                        lambda.old_unique
                    )
                    .as_bytes(),
                ),
                Target::External {
                    module,
                    function,
                    arity,
                } => {
                    text.extend_from_slice(b"fun ");
                    let (module, function) = (atoms.name_of(module), atoms.name_of(function));
                    *dcount -= len(module) + len(function);
                    print_atom(module, &mut text);
                    text.push(b':');
                    print_atom(function, &mut text);
                    text.extend_from_slice(format!("/{arity}").as_bytes());
                }
            },
            Term::Pid(pid) => {
                let data = pid.0;
                text.extend_from_slice(
                    format!("<0.{}.{}>", data & PID_NUMBER_MASK, data >> PID_NUMBER_BITS)
                        .as_bytes(),
                );
            }
            Term::Port(port) => text.extend_from_slice(format!("#Port<0.{}>", port.0).as_bytes()),
            // The channel number of a node of another node is the atom
            // index of its name (`dist_entry_channel_no`,
            // `erl_node_container_utils.h:109-116`, `erl_printf_term.c:497-531`).
            Term::External(id) => {
                // An old incarnation of the local node has the dist entry of
                // the local node, whose channel number is 0.
                let channel = if id.node == atoms.this_node().0 {
                    0
                } else {
                    id.node.index()
                };
                let printed = match &id.data {
                    ExternalData::Pid { number, serial } => {
                        format!("<{channel}.{number}.{serial}>")
                    }
                    ExternalData::Port(number) => format!("#Port<{channel}.{number}>"),
                    ExternalData::Reference(numbers) => {
                        let parts: Vec<String> = std::iter::once(channel)
                            .chain(numbers.iter().rev().copied())
                            .map(|part| part.to_string())
                            .collect();
                        format!("#Ref<{}>", parts.join("."))
                    }
                };
                text.extend_from_slice(printed.as_bytes());
            }
            Term::Reference(_) | Term::Resource(_) => {
                let [n0, n1, n2] = term.reference().map_or([0; 3], Reference::numbers);
                text.extend_from_slice(format!("#Ref<0.{n2}.{n1}.{n0}>").as_bytes());
            }
            // `#Module:Name{Key=Value,...}` in the sorted key order
            // (`erl_printf_term.c:558-585`).
            Term::Record(record) => {
                let def = record.def();
                text.push(b'#');
                for (index, atom) in [&def.module, &def.name].into_iter().enumerate() {
                    let name = atoms.name_of(atom);
                    *dcount -= len(name);
                    print_atom(name, &mut text);
                    if *dcount <= 0 {
                        return text;
                    }
                    text.push(if index == 0 { b':' } else { b'{' });
                }
                items.push(Item::Text("}"));
                let pairs = def.key_terms().iter().zip(record.values());
                for (index, (key, value)) in pairs.enumerate().rev() {
                    items.push(Item::Term(value));
                    items.push(Item::Text("="));
                    items.push(Item::Term(key));
                    if index > 0 {
                        items.push(Item::Text(","));
                    }
                }
            }
            Term::MatchContext(_) => unreachable!("{MATCH_CONTEXT_ESCAPE}"),
        }
    }
    text
}

/// Take 1 from the count, as `(*dcount)-- <= 0` does, and return whether
/// the count was at most 0 before, which stops the print.
fn take(dcount: &mut i64) -> bool {
    let before = *dcount;
    *dcount = before.saturating_sub(1);
    before <= 0
}

/// Return the byte length of an atom name as the count type, as
/// `entry->len` (`erl_printf_term.c:261-263`).
fn len(name: &str) -> i64 {
    i64::try_from(name.len()).expect("an atom name is short")
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
