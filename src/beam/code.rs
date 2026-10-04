use std::collections::HashMap;

use super::module::BeamModule;
use crate::{
    error::{BeamError, error},
    term::Term,
};

const WORD_SIZE: usize = 4;
mod instruction;
pub(crate) use instruction::{Instruction, Operand, TestKind};

const CHUNK_ALIGNMENT: usize = WORD_SIZE;
const CHUNK_HEADER_SIZE: usize = WORD_SIZE * 2;
const CONTAINER_HEADER_SIZE: usize = WORD_SIZE * 3;
const EXPORT_ENTRY_SIZE: usize = WORD_SIZE * 3;
const IMPORT_ENTRY_SIZE: usize = WORD_SIZE * 3;
const COMPACT_TAG_MASK: u8 = 0b111;
const COMPACT_EXTENDED_FLAG: u8 = 0b1000;
const COMPACT_SIGN_FLAG: u8 = 0b1_0000;
const COMPACT_HIGH_BITS_MASK: u8 = 0b1110_0000;
const COMPACT_SHORT_VALUE_SHIFT: u8 = 4;
const COMPACT_HIGH_VALUE_SHIFT: u8 = 3;
const TAG_UNSIGNED: u8 = 0;
const TAG_INTEGER: u8 = 1;
const TAG_ATOM: u8 = 2;
const TAG_REGISTER: u8 = 3;
const TAG_Y_REGISTER: u8 = 4;
const TAG_LABEL: u8 = 5;
const TAG_LITERAL: u8 = 7;
const LIST_SUBTAG: i128 = 1;
const ETF_VERSION: u8 = 131;
const ETF_SMALL_TUPLE_EXT: u8 = 104;
const ETF_NIL_EXT: u8 = 106;
const ETF_STRING_EXT: u8 = 107;
const ETF_LIST_EXT: u8 = 108;
const ETF_BINARY_EXT: u8 = 109;
const ETF_SMALL_ATOM_UTF8_EXT: u8 = 119;
const FIRST_ATOM_INDEX: usize = 1;
const FIRST_LABEL_NUMBER: u32 = 1;

/// Parse the BEAM subset used by this runtime.
///
/// Container grammar:
///
/// ```text
/// beam_file ::= "FOR1" container_size "BEAM" chunk*
/// container_size ::= u32be
/// chunk     ::= chunk_id chunk_size payload padding
/// chunk_id  ::= byte[4]
/// chunk_size ::= u32be
/// payload   ::= byte[chunk_size]
/// padding   ::= byte[(4 - chunk_size mod 4) mod 4]
/// u32be     ::= byte[4]
/// i32be     ::= byte[4]
///
/// atoms_chunk ::= "AtU8" chunk_size atom_table padding
/// atom_table  ::= atom_count atom_entry[abs(atom_count)]
/// atom_count  ::= i32be
/// atom_entry  ::= atom_name_length utf8_name[atom_name_length]
/// atom_name_length ::= compact(0)
///
/// exports_chunk ::= "ExpT" chunk_size export_table padding
/// export_table  ::= export_count export[export_count]
/// export_count  ::= u32be
/// export        ::= atom_index:u32be arity:u32be label:u32be
///
/// code_chunk  ::= "Code" chunk_size code_header executable_instruction* int_code_end padding
/// code_header ::= sub_size instruction_set opcode_max label_count function_count
/// sub_size    ::= 16:u32be
/// instruction_set ::= 0:u32be
/// opcode_max ::= u32be
/// label_count ::= u32be
/// function_count ::= u32be
/// executable_instruction ::= label | function_info | move | call | call_only | call_ext | jump | select_val | test | put_list | allocate | deallocate | return | noop | generated_instruction
/// label       ::= 1 compact(0)
/// function_info ::= 2 compact(2) compact(2) compact(0)
/// move        ::= 64 move_source (x_register | y_register)
/// move_source ::= value | x_register | y_register
/// move_source ::= literal_value
/// literal_value ::= compact(7) compact(0) ; compact(7) value is 4
/// x_register  ::= compact(3)
/// y_register  ::= compact(4)
/// call        ::= 4 compact(0) compact(5)
/// call_only   ::= 6 compact(0) compact(5)
/// call_ext    ::= 7 compact(0) compact(0)
/// call_ext_last ::= 8 compact(0) compact(0) compact(0)
/// call_ext_only ::= 78 compact(0) compact(0)
/// allocate    ::= 12 compact(0) compact(0)
/// deallocate  ::= 18 compact(0)
/// return      ::= 19
/// line        ::= 153 compact(0)
/// jump       ::= 61 compact(5)
/// select_val ::= 59 select_source compact(5) selection_list
/// select_source ::= register | typed_register
/// typed_register ::= compact(7, short=5) register compact(0)
/// selection_list ::= compact(7, short=1) compact(0) selection_pair[select_count/2]
/// selection_pair ::= atom_value compact(5)
/// type_test ::= test_opcode compact(5) register
/// test_opcode ::= 48 ; is_atom | 49 ; is_pid | 50 ; is_reference | 53 ; is_binary | 55 ; is_list | 57 ; is_tuple
/// put_list ::= 69 move_source move_source register
/// noop ::= 16 test_heap_operands | 172 init_yregs_operands
/// int_code_end ::= 3
/// imports_chunk ::= "ImpT" chunk_size import_table padding
/// import_table ::= import_count import[import_count]
/// import_count ::= u32be
/// import ::= module_atom_index:u32be function_atom_index:u32be arity:u32be
/// value       ::= integer_value | atom_value
/// integer_value ::= compact(1)
/// atom_value  ::= compact(2)
/// nil_value ::= compact(2) ; atom index zero
/// literals_chunk ::= "LitT" chunk_size 0:u32be literal_count literal_entry*
/// literal_count ::= u32be
/// literal_entry ::= term_size:u32be 131 external_term ; term_size bytes in total
/// external_term ::= binary_term | string_term | nil_term | atom_term | tuple_term | list_term
/// binary_term ::= 109 binary_size:u32be byte[binary_size]
/// string_term ::= 107 string_size:u16be byte[string_size]
/// nil_term    ::= 106
/// atom_term   ::= 119 name_size:u8 utf8_byte[name_size]
/// tuple_term  ::= 104 arity:u8 external_term[arity]
/// list_term   ::= 108 length:u32be external_term[length] tail:external_term
///
/// compact(tag) ::= short(tag) | extended(tag)
/// short(tag)   ::= value:4 0 tag:3
/// extended(tag) ::= high:3 1 0 tag:3 low:8
/// ```
///
/// `container_size` counts bytes after its field, including `BEAM` and chunks.
/// `chunk_size` counts payload bytes and excludes padding. The parser requires
/// the `AtU8`, `ExpT`, and `Code` chunks. `ImpT` is required for external
/// calls. `u32be` is an unsigned big-endian
/// integer. `i32be` is a signed big-endian integer. `byte[n]` means exactly
/// `n` bytes. `AtU8` stores a non-positive count; its absolute value gives the
/// number of atoms. Atom names use UTF-8. The module name is the first atom.
/// Atom indices are one-based. `ExpT` contains exactly the declared number of
/// 12-byte entries. Export and code labels must be in `1..=label_count`.
/// `function_count` must match the number of `function_info` instructions.
/// `opcode_max` is not checked. The parser rejects duplicate chunk IDs. It
/// supports only the listed instructions and compact operand forms. `LitT`
/// supports uncompressed tables. A literal loads as a value only when every
/// nested tag is listed; another tag, such as a map, integer, or improper list
/// tail, makes the literal unsupported, and execution fails only if an
/// instruction uses it. `[a|[b]]` and `[a,b]` give the same list. The
/// parser retains generated instructions that it does not execute. Execution
/// fails if it reaches one. Compact
/// tags occupy the low three bits. The parser accepts the short and one-byte
/// extension forms only. Bit fields are shown from high bit to low bit. Other
/// chunk payloads and padding byte values are ignored. The `ImpT` chunk maps
/// external call indexes to module/function/arity. The executor supports only
/// the listed instructions.
/// # Examples
///
/// ```rust,ignore
/// let module = parse(&beam_bytes)?;
/// assert_eq!(module.name, "Elixir.Hello");
/// ```
pub(super) fn parse(bytes: &[u8]) -> Result<BeamModule, BeamError> {
    if bytes.len() < CONTAINER_HEADER_SIZE || &bytes[..WORD_SIZE] != b"FOR1" {
        return Err(error("invalid BEAM container header"));
    }
    let container_size = read_u32(&bytes[WORD_SIZE..WORD_SIZE * 2])? as usize;
    if container_size != bytes.len() - WORD_SIZE * 2
        || &bytes[WORD_SIZE * 2..CONTAINER_HEADER_SIZE] != b"BEAM"
    {
        return Err(error("invalid BEAM container size or identity"));
    }

    let mut chunks = HashMap::new();
    let mut cursor = CONTAINER_HEADER_SIZE;
    while cursor < bytes.len() {
        if bytes.len() - cursor < CHUNK_HEADER_SIZE {
            return Err(error("truncated BEAM chunk header"));
        }
        let header_end = cursor + CHUNK_HEADER_SIZE;
        let chunk_id = &bytes[cursor..cursor + WORD_SIZE];
        let chunk_size = read_u32(&bytes[cursor + WORD_SIZE..header_end])? as usize;
        let padding = (CHUNK_ALIGNMENT - (chunk_size % CHUNK_ALIGNMENT)) % CHUNK_ALIGNMENT;
        if chunk_size > bytes.len() - header_end || padding > bytes.len() - header_end - chunk_size
        {
            return Err(error("BEAM chunk exceeds container bounds"));
        }
        let data_end = header_end + chunk_size;
        let padded_end = data_end + padding;
        if chunks
            .insert(chunk_id.to_vec(), &bytes[header_end..data_end])
            .is_some()
        {
            return Err(error("duplicate BEAM chunk"));
        }
        cursor = padded_end;
    }

    let atoms = parse_atoms(required_chunk(&chunks, b"AtU8")?)?;
    let exports = parse_exports(required_chunk(&chunks, b"ExpT")?, &atoms)?;
    let code = required_chunk(&chunks, b"Code")?;
    let name = atoms
        .first()
        .ok_or_else(|| error("empty BEAM atom table"))?
        .clone();
    let imports = chunks
        .get(b"ImpT".as_slice())
        .map(|data| parse_imports(data, &atoms))
        .transpose()?
        .unwrap_or_default();
    let literals = chunks
        .get(b"LitT".as_slice())
        .map(|data| parse_literals(data))
        .transpose()?
        .unwrap_or_default();
    let (instructions, number_of_labels) =
        instruction::parse_code(code, &atoms, &imports, &literals)?;
    if let Some(label) = exports
        .values()
        .find(|&&label| label < FIRST_LABEL_NUMBER || label > number_of_labels)
    {
        return Err(error(format!(
            "BEAM export label {label} is outside declared range 1..={number_of_labels}"
        )));
    }
    Ok(BeamModule {
        name,
        exports,
        instructions,
    })
}

/// Read the atom names from an `AtU8` chunk.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = parse_atoms(&[0xff, 0xff, 0xff, 0xff, 0x10, b'a'])?;
/// assert_eq!(atoms, ["a"]);
/// ```
fn parse_atoms(data: &[u8]) -> Result<Vec<String>, BeamError> {
    let signed_atom_count = read_u32(data)? as i32;
    let atom_count = usize::try_from(
        signed_atom_count
            .checked_neg()
            .ok_or_else(|| error("unsupported atom table encoding"))?,
    )
    .map_err(|_| error("invalid atom count"))?;
    if atom_count > data.len() - WORD_SIZE {
        return Err(error("atom count exceeds chunk size"));
    }
    let mut atoms = Vec::with_capacity(atom_count);
    let mut cursor = WORD_SIZE;
    for _ in 0..atom_count {
        let atom_name_length = read_operand(data, &mut cursor, TAG_UNSIGNED)? as usize;
        if atom_name_length > data.len() - cursor {
            return Err(error("truncated atom name"));
        }
        let atom_name_end = cursor + atom_name_length;
        let atom = std::str::from_utf8(
            data.get(cursor..atom_name_end)
                .ok_or_else(|| error("truncated atom name"))?,
        )
        .map_err(|_| error("invalid UTF-8 atom name"))?;
        atoms.push(atom.to_owned());
        cursor = atom_name_end;
    }
    if cursor != data.len() {
        return Err(error("unexpected bytes after atom table"));
    }
    Ok(atoms)
}

/// Read exported function names, arities, and labels from an `ExpT` chunk.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = vec!["main".to_owned()];
/// let table = &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
/// let exports = parse_exports(table, &atoms)?;
/// assert_eq!(exports.get(&("main".to_owned(), 0)), Some(&1));
/// ```
fn parse_exports(data: &[u8], atoms: &[String]) -> Result<HashMap<(String, u32), u32>, BeamError> {
    let export_count = read_u32(data)? as usize;
    let expected_table_size = WORD_SIZE
        .checked_add(
            export_count
                .checked_mul(EXPORT_ENTRY_SIZE)
                .ok_or_else(|| error("export count overflow"))?,
        )
        .ok_or_else(|| error("export table size overflow"))?;
    if expected_table_size != data.len() {
        return Err(error("invalid export table size"));
    }
    let mut exports = HashMap::new();
    for entry in data[WORD_SIZE..].chunks(EXPORT_ENTRY_SIZE) {
        let atom_index = read_u32(&entry[..WORD_SIZE])? as usize;
        let arity = read_u32(&entry[WORD_SIZE..WORD_SIZE * 2])?;
        let label = read_u32(&entry[WORD_SIZE * 2..])?;
        let name = atom_at(atoms, atom_index)?.to_owned();
        if exports.insert((name, arity), label).is_some() {
            return Err(error("duplicate BEAM export"));
        }
    }
    Ok(exports)
}

/// Read external module, function, and arity entries from an `ImpT` chunk.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = vec!["IO".to_owned(), "puts".to_owned()];
/// let table = &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1];
/// assert_eq!(parse_imports(table, &atoms)?, [("IO".into(), "puts".into(), 1)]);
/// ```
fn parse_imports(data: &[u8], atoms: &[String]) -> Result<Vec<(String, String, u32)>, BeamError> {
    let import_count = read_u32(data)? as usize;
    let expected_size = WORD_SIZE
        .checked_add(
            import_count
                .checked_mul(IMPORT_ENTRY_SIZE)
                .ok_or_else(|| error("import count overflow"))?,
        )
        .ok_or_else(|| error("import table size overflow"))?;
    if expected_size != data.len() {
        return Err(error("invalid import table size"));
    }
    data[WORD_SIZE..]
        .chunks(IMPORT_ENTRY_SIZE)
        .map(|entry| {
            Ok((
                atom_at(atoms, read_u32(&entry[..WORD_SIZE])? as usize)?.to_owned(),
                atom_at(atoms, read_u32(&entry[WORD_SIZE..WORD_SIZE * 2])? as usize)?.to_owned(),
                read_u32(&entry[WORD_SIZE * 2..])?,
            ))
        })
        .collect()
}

/// Read supported binary terms from an uncompressed `LitT` chunk.
///
/// # Examples
///
/// ```rust,ignore
/// let literals = parse_literals(&[0; 8])?;
/// assert!(literals.is_empty());
/// ```
fn parse_literals(data: &[u8]) -> Result<Vec<Option<Term>>, BeamError> {
    if data.len() < WORD_SIZE * 2 {
        return Err(error("truncated BEAM literal table header"));
    }
    if read_u32(&data[..WORD_SIZE])? != 0 {
        return Err(error("compressed BEAM literal tables are unsupported"));
    }
    let literal_count = read_u32(&data[WORD_SIZE..WORD_SIZE * 2])? as usize;
    if literal_count > (data.len() - WORD_SIZE * 2) / WORD_SIZE {
        return Err(error("BEAM literal count exceeds chunk size"));
    }

    let mut cursor = WORD_SIZE * 2;
    let mut literals = Vec::with_capacity(literal_count);
    for _ in 0..literal_count {
        let literal_size = read_u32(&data[cursor..])? as usize;
        cursor += WORD_SIZE;
        let literal_end = cursor
            .checked_add(literal_size)
            .filter(|&end| end <= data.len())
            .ok_or_else(|| error("BEAM literal exceeds chunk size"))?;
        literals.push(read_literal(&data[cursor..literal_end])?);
        cursor = literal_end;
    }
    if cursor != data.len() {
        return Err(error("unexpected bytes after BEAM literal table"));
    }
    Ok(literals)
}

/// Read one `LitT` entry. Return `None` when the term contains a tag that
/// `literal_entry` does not list, at any depth.
///
/// # Examples
///
/// ```rust,ignore
/// let term = read_literal(&[131, 109, 0, 0, 0, 2, b'o', b'k'])?;
/// assert_eq!(term, Some(Term::Binary(b"ok".to_vec())));
/// // [flush]
/// let list = read_literal(&[131, 108, 0, 0, 0, 1, 119, 5, b'f', b'l', b'u', b's', b'h', 106])?;
/// assert_eq!(list, Some(Term::List(vec![Term::Atom("flush".into())])));
/// ```
fn read_literal(data: &[u8]) -> Result<Option<Term>, BeamError> {
    if data.first() != Some(&ETF_VERSION) {
        return Ok(None);
    }
    let mut cursor = 1;
    let Some(term) = read_external_term(data, &mut cursor)? else {
        return Ok(None);
    };
    if cursor != data.len() {
        return Err(error("unexpected bytes after BEAM literal"));
    }
    Ok(Some(term))
}

/// A list or tuple that `read_external_term` has started but not finished.
enum PartialTerm {
    /// A tuple that still needs `remaining` elements.
    Tuple { remaining: u8, elements: Vec<Term> },
    /// A list that still needs `remaining` elements before its tail.
    List { remaining: u32, elements: Vec<Term> },
    /// A list whose elements are read and whose tail is next.
    ListTail { elements: Vec<Term> },
}

/// Read one external term at the cursor and advance past it. Return `None`
/// for a tag or list tail that `literal_entry` does not list. Nested lists
/// and tuples use a work stack, so nesting has no fixed depth limit, as in
/// the ERTS decoder.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// // {}
/// assert_eq!(read_external_term(&[104, 0], &mut cursor)?, Some(Term::Tuple(vec![])));
/// assert_eq!(cursor, 2);
/// ```
fn read_external_term(data: &[u8], cursor: &mut usize) -> Result<Option<Term>, BeamError> {
    let mut partial = Vec::new();
    loop {
        let mut term = match take(data, cursor, 1)?[0] {
            ETF_BINARY_EXT => {
                let size = read_u32(take(data, cursor, WORD_SIZE)?)? as usize;
                Term::Binary(take(data, cursor, size)?.to_vec())
            }
            ETF_STRING_EXT => {
                let size_bytes = take(data, cursor, 2)?;
                let size = u16::from_be_bytes([size_bytes[0], size_bytes[1]]) as usize;
                let characters = take(data, cursor, size)?
                    .iter()
                    .map(|&byte| Term::Integer(i64::from(byte)))
                    .collect();
                Term::List(characters)
            }
            ETF_NIL_EXT => Term::List(Vec::new()),
            ETF_SMALL_ATOM_UTF8_EXT => {
                let size = take(data, cursor, 1)?[0] as usize;
                let name = std::str::from_utf8(take(data, cursor, size)?)
                    .map_err(|_| error("invalid UTF-8 in BEAM atom literal"))?;
                Term::Atom(name.to_owned())
            }
            ETF_SMALL_TUPLE_EXT => match take(data, cursor, 1)?[0] {
                0 => Term::Tuple(Vec::new()),
                remaining => {
                    let elements = Vec::new();
                    partial.push(PartialTerm::Tuple {
                        remaining,
                        elements,
                    });
                    continue;
                }
            },
            ETF_LIST_EXT => {
                let elements = Vec::new();
                partial.push(match read_u32(take(data, cursor, WORD_SIZE)?)? {
                    0 => PartialTerm::ListTail { elements },
                    remaining => PartialTerm::List {
                        remaining,
                        elements,
                    },
                });
                continue;
            }
            _ => return Ok(None),
        };
        // Give the finished term to the innermost partial term, and finish
        // each partial term that it completes.
        loop {
            match partial.pop() {
                None => return Ok(Some(term)),
                Some(PartialTerm::Tuple {
                    remaining,
                    mut elements,
                }) => {
                    elements.push(term);
                    if remaining == 1 {
                        term = Term::Tuple(elements);
                        continue;
                    }
                    let remaining = remaining - 1;
                    partial.push(PartialTerm::Tuple {
                        remaining,
                        elements,
                    });
                }
                Some(PartialTerm::List {
                    remaining,
                    mut elements,
                }) => {
                    elements.push(term);
                    partial.push(match remaining {
                        1 => PartialTerm::ListTail { elements },
                        _ => PartialTerm::List {
                            remaining: remaining - 1,
                            elements,
                        },
                    });
                }
                Some(PartialTerm::ListTail { mut elements }) => {
                    let Term::List(tail) = &mut term else {
                        return Ok(None);
                    };
                    elements.append(tail);
                    term = Term::List(elements);
                    continue;
                }
            }
            break;
        }
    }
}

/// Return the next `length` bytes at the cursor and advance past them.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 1;
/// assert_eq!(take(&[1, 2, 3], &mut cursor, 2)?, &[2, 3]);
/// assert!(take(&[1], &mut 0, 2).is_err());
/// ```
fn take<'a>(data: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], BeamError> {
    let end = cursor
        .checked_add(length)
        .filter(|&end| end <= data.len())
        .ok_or_else(|| error("truncated BEAM literal"))?;
    let bytes = &data[*cursor..end];
    *cursor = end;
    Ok(bytes)
}

/// Read a supported integer or atom term and advance the cursor.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// let term = read_value(&[0x01], &mut cursor, &[])?;
/// assert_eq!(term, Term::Integer(0));
/// assert_eq!(cursor, 1);
/// ```
fn read_value(data: &[u8], cursor: &mut usize, atoms: &[String]) -> Result<Term, BeamError> {
    let first_byte = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    match first_byte & COMPACT_TAG_MASK {
        TAG_INTEGER => {
            let value = read_integer(data, cursor, TAG_INTEGER)?;
            Ok(Term::Integer(
                i64::try_from(value).map_err(|_| error("integer constant exceeds i64"))?,
            ))
        }
        TAG_ATOM => {
            let index = read_operand(data, cursor, TAG_ATOM)? as usize;
            if index == 0 {
                return Ok(Term::List(Vec::new()));
            }
            Ok(Term::Atom(atom_at(atoms, index)?.to_owned()))
        }
        unsupported_tag => Err(error(format!(
            "unsupported constant operand tag {unsupported_tag}"
        ))),
    }
}

/// Return the atom at a one-based BEAM atom index.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = vec!["hello".to_owned()];
/// assert_eq!(atom_at(&atoms, 1)?, "hello");
/// ```
fn atom_at(atoms: &[String], index: usize) -> Result<&str, BeamError> {
    if index < FIRST_ATOM_INDEX {
        return Err(error("BEAM atom index zero is invalid"));
    }
    atoms
        .get(index - FIRST_ATOM_INDEX)
        .map(String::as_str)
        .ok_or_else(|| error(format!("BEAM atom index {index} out of range")))
}

/// Return a required chunk payload by its four-byte ID.
///
/// # Examples
///
/// ```rust,ignore
/// let payload = [0x03];
/// let chunks = HashMap::from([(b"Code".to_vec(), &payload[..])]);
/// assert_eq!(required_chunk(&chunks, b"Code")?, &payload);
/// ```
fn required_chunk<'a>(
    chunks: &HashMap<Vec<u8>, &'a [u8]>,
    chunk_id: &[u8; WORD_SIZE],
) -> Result<&'a [u8], BeamError> {
    chunks.get(chunk_id.as_slice()).copied().ok_or_else(|| {
        error(format!(
            "missing {} chunk",
            String::from_utf8_lossy(chunk_id)
        ))
    })
}

/// Read a compact unsigned operand with the expected tag.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// assert_eq!(read_operand(&[0x10], &mut cursor, TAG_UNSIGNED)?, 1);
/// assert_eq!(cursor, 1);
/// ```
fn read_operand(data: &[u8], cursor: &mut usize, expected_tag: u8) -> Result<u64, BeamError> {
    let first = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    *cursor += 1;
    if first & COMPACT_TAG_MASK != expected_tag {
        return Err(error(format!(
            "unexpected BEAM operand tag {}",
            first & COMPACT_TAG_MASK
        )));
    }
    read_integer_after_first(data, cursor, first)
}

/// Read a compact integer payload and advance the cursor.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// assert_eq!(read_integer(&[0x21], &mut cursor, TAG_INTEGER)?, 2);
/// assert_eq!(cursor, 1);
/// ```
fn read_integer(data: &[u8], cursor: &mut usize, expected_tag: u8) -> Result<i128, BeamError> {
    let first = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    *cursor += 1;
    if first & COMPACT_TAG_MASK != expected_tag {
        return Err(error(format!(
            "unexpected BEAM operand tag {}",
            first & COMPACT_TAG_MASK
        )));
    }
    let value = read_integer_after_first(data, cursor, first)?;
    Ok(value as i128)
}

/// Read the integer payload after its first compact-operand byte.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// assert_eq!(read_integer_after_first(&[0, 7], &mut cursor, 0x19)?, 7);
/// assert_eq!(cursor, 2);
/// ```
fn read_integer_after_first(
    data: &[u8],
    cursor: &mut usize,
    first_byte: u8,
) -> Result<u64, BeamError> {
    if first_byte & COMPACT_EXTENDED_FLAG == 0 {
        return Ok((first_byte >> COMPACT_SHORT_VALUE_SHIFT) as u64);
    }
    if first_byte & COMPACT_SIGN_FLAG == 0 {
        let low_value_byte = *data
            .get(*cursor)
            .ok_or_else(|| error("truncated BEAM operand"))?;
        *cursor += 1;
        return Ok(
            (((first_byte & COMPACT_HIGH_BITS_MASK) as u64) << COMPACT_HIGH_VALUE_SHIFT)
                | low_value_byte as u64,
        );
    }
    let size_code = (first_byte >> 5) as usize;
    let byte_count = if size_code == 7 {
        usize::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
            .map_err(|_| error("BEAM integer operand is too large"))?
            .checked_add(9)
            .ok_or_else(|| error("BEAM integer operand size overflow"))?
    } else {
        size_code + 2
    };
    if byte_count > size_of::<u64>() {
        return Err(error("BEAM integer operand exceeds u64"));
    }
    let end = cursor
        .checked_add(byte_count)
        .filter(|&end| end <= data.len())
        .ok_or_else(|| error("truncated BEAM integer operand"))?;
    let value = data[*cursor..end]
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
    *cursor = end;
    Ok(value)
}

/// Read one unsigned, big-endian 32-bit field.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(read_u32(&[0, 0, 0, 2])?, 2);
/// ```
fn read_u32(field_bytes: &[u8]) -> Result<u32, BeamError> {
    let field_bytes = field_bytes.get(..WORD_SIZE).ok_or_else(|| {
        error(format!(
            "truncated 32-bit BEAM field: got {} bytes",
            field_bytes.len()
        ))
    })?;
    let field: [u8; WORD_SIZE] = field_bytes
        .try_into()
        .expect("four-byte BEAM field was checked above");
    Ok(u32::from_be_bytes(field))
}

#[cfg(test)]
mod tests {
    use super::{atom_at, parse_atoms, parse_exports, parse_literals, read_literal, read_u32};
    use crate::term::Term;

    #[test]
    fn rejects_invalid_atom_tables_and_indices() {
        assert!(read_u32(&[0, 1]).is_err());
        assert!(parse_atoms(&[]).is_err());
        assert!(parse_atoms(&(-1i32).to_be_bytes()).is_err());
        assert!(atom_at(&[], 0).is_err());

        let mut malformed_atoms = (-2i32).to_be_bytes().to_vec();
        malformed_atoms.extend([0x30, b'a', b'b', b'c', 0xa0]);
        assert!(parse_atoms(&malformed_atoms).is_err());
        assert!(parse_atoms(&[0xff, 0xff, 0xff, 0xff, 0x10, 0xff]).is_err());
        assert!(parse_atoms(&0i32.to_be_bytes()).unwrap().is_empty());
    }

    #[test]
    fn rejects_invalid_export_tables_and_indices() {
        assert!(parse_exports(&[], &[]).is_err());
        assert!(parse_exports(&[0, 0, 0, 1], &[]).is_err());
        let exports = [0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 1];
        assert!(parse_exports(&exports, &["constant".into()]).is_err());
    }

    #[test]
    fn reads_uncompressed_binary_literals() {
        let mut table = 0u32.to_be_bytes().to_vec();
        table.extend(1u32.to_be_bytes());
        table.extend(8u32.to_be_bytes());
        table.extend([131, 109, 0, 0, 0, 2, b'o', b'k']);
        assert_eq!(
            parse_literals(&table).unwrap(),
            vec![Some(Term::Binary(b"ok".to_vec()))]
        );
        assert_eq!(
            parse_literals(&[0, 0, 0, 0, 0, 0, 0, 0]).unwrap(),
            Vec::<Option<Term>>::new()
        );
        assert_eq!(read_literal(&[1]).unwrap(), None);
        assert!(read_literal(&[131, 109]).is_err());
        assert_eq!(
            read_literal(&[131, 107, 0, 1, b'\n']).unwrap(),
            Some(Term::List(vec![Term::Integer(10)]))
        );
        assert!(read_literal(&[131, 107]).is_err());
        assert!(read_literal(&[131, 107, 0, 2, b'x']).is_err());
    }

    #[test]
    fn reads_nested_literals_and_marks_unsupported_tags_at_any_depth() {
        let atom = |name: &str| Term::Atom(name.into());
        // [a|[b]] gives the same list as [a,b].
        assert_eq!(
            read_literal(&[
                131, 108, 0, 0, 0, 1, 119, 1, b'a', 108, 0, 0, 0, 1, 119, 1, b'b', 106
            ])
            .unwrap(),
            Some(Term::List(vec![atom("a"), atom("b")]))
        );
        assert_eq!(
            read_literal(&[131, 104, 2, 119, 2, b'o', b'k', 106]).unwrap(),
            Some(Term::Tuple(vec![atom("ok"), Term::List(vec![])]))
        );
        // A small integer (97) inside a tuple or a list, and an improper tail.
        assert_eq!(read_literal(&[131, 104, 1, 97, 1]).unwrap(), None);
        assert_eq!(
            read_literal(&[131, 108, 0, 0, 0, 1, 97, 1, 106]).unwrap(),
            None
        );
        assert_eq!(
            read_literal(&[131, 108, 0, 0, 0, 1, 106, 119, 1, b't']).unwrap(),
            None
        );
        assert!(read_literal(&[131, 119, 1, 0xff]).is_err());
        assert!(read_literal(&[131, 106, 106]).is_err());
        assert!(read_literal(&[131, 104]).is_err());
        // [a, b] with length 2, and a zero-length LIST_EXT whose tail is [].
        assert_eq!(
            read_literal(&[131, 108, 0, 0, 0, 2, 119, 1, b'a', 119, 1, b'b', 106]).unwrap(),
            Some(Term::List(vec![atom("a"), atom("b")]))
        );
        assert_eq!(
            read_literal(&[131, 108, 0, 0, 0, 0, 106]).unwrap(),
            Some(Term::List(vec![]))
        );

        // ERTS decodes a term nested 1000000 levels deep; so does PBeam.
        let mut deep = vec![131];
        for _ in 0..1_000_000 {
            deep.extend([104, 1]);
        }
        deep.push(106);
        let term = read_literal(&deep).unwrap().unwrap();
        assert!(format!("{term:?}").starts_with("Tuple([Tuple(["));
    }

    #[test]
    fn rejects_invalid_literal_tables() {
        assert!(parse_literals(&[0, 0, 0, 0]).is_err());
        assert!(parse_literals(&[0, 0, 0, 1, 0, 0, 0, 0]).is_err());
        assert!(parse_literals(&[0, 0, 0, 0, 0, 0, 0, 1]).is_err());
        assert!(parse_literals(&[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1]).is_err());
        assert!(parse_literals(&[0, 0, 0, 0, 0, 0, 0, 0, 1]).is_err());

        let mut malformed_binary = vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 7];
        malformed_binary.extend([131, 109, 0, 0, 0, 2, b'x']);
        assert!(parse_literals(&malformed_binary).is_err());
    }
}
