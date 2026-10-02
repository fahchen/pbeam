use std::collections::HashMap;

use super::{
    error::{BeamError, error},
    module::BeamModule,
    term::Term,
};

const WORD_SIZE: usize = 4;
const CHUNK_ALIGNMENT: usize = WORD_SIZE;
const CHUNK_HEADER_SIZE: usize = WORD_SIZE * 2;
const CONTAINER_HEADER_SIZE: usize = WORD_SIZE * 3;
const CODE_HEADER_SIZE: usize = WORD_SIZE * 5;
const CODE_HEADER_SUB_SIZE: u32 = 16;
const INSTRUCTION_SET_ERLANG: u32 = 0;
const EXPORT_ENTRY_SIZE: usize = WORD_SIZE * 3;
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
const OPCODE_LABEL: u8 = 1;
const OPCODE_FUNCTION_INFO: u8 = 2;
const OPCODE_INT_CODE_END: u8 = 3;
const OPCODE_RETURN: u8 = 19;
const OPCODE_MOVE: u8 = 64;
const FIRST_ATOM_INDEX: usize = 1;
const FIRST_LABEL_NUMBER: u32 = 1;
const CODE_LABEL_COUNT_OFFSET: usize = WORD_SIZE * 3;
const CODE_FUNCTION_COUNT_OFFSET: usize = WORD_SIZE * 4;

#[derive(Debug, Clone)]
/// An instruction that the parser can read from a BEAM `Code` chunk.
///
/// Byte examples use hexadecimal values. Compact operands use the short form
/// in the Code chunk grammar. `End` represents the stream terminator; the
/// executor does not run it.
pub(super) enum Instruction {
    /// Starts a code label. The label must be in `1..=label_count`.
    ///
    /// Encoding: opcode, then a compact unsigned label. Example: `[0x01, 0x10]`
    /// encodes label 1.
    Label(u32),
    /// Declares the module, function, and arity for the following function.
    /// Atom indexes refer to the one-based `AtU8` table.
    ///
    /// Encoding: opcode, module atom, function atom, and arity. Example:
    /// `[0x02, 0x12, 0x22, 0x00]` refers to atom indexes 1 and 2 with arity 0.
    FunctionInfo {
        module: String,
        name: String,
        arity: u32,
    },
    /// Moves an immediate atom or integer into a register.
    ///
    /// Encoding: opcode, value, and destination register. Example:
    /// `[0x40, 0x32, 0x03]` moves atom index 3 into `x0`. Execution supports
    /// `x0` only.
    Move(Term, u32),
    /// Returns the value in `x0` from the active function.
    ///
    /// Encoding: `[0x13]`. Execution reports an error if the function has not
    /// set `x0`.
    Return,
    /// Marks the end of the `Code` instruction stream.
    ///
    /// Encoding: `[0x03]`. The parser adds this internal sentinel after it
    /// reads the terminator. The executor stops at this sentinel.
    End,
}

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
/// executable_instruction ::= label | function_info | move | return
/// label       ::= 1 compact(0)
/// function_info ::= 2 compact(2) compact(2) compact(0)
/// move        ::= 64 value compact(3)
/// return      ::= 19
/// int_code_end ::= 3
/// value       ::= integer_value | atom_value
/// integer_value ::= compact(1)
/// atom_value  ::= compact(2)
///
/// compact(tag) ::= short(tag) | extended(tag)
/// short(tag)   ::= value:4 0 tag:3
/// extended(tag) ::= high:3 1 0 tag:3 low:8
/// ```
///
/// `container_size` counts bytes after its field, including `BEAM` and chunks.
/// `chunk_size` counts payload bytes and excludes padding. The parser requires
/// the `AtU8`, `ExpT`, and `Code` chunks. `u32be` is an unsigned big-endian
/// integer. `i32be` is a signed big-endian integer. `byte[n]` means exactly
/// `n` bytes. `AtU8` stores a non-positive count; its absolute value gives the
/// number of atoms. Atom names use UTF-8. The module name is the first atom.
/// Atom indices are one-based. `ExpT` contains exactly the declared number of
/// 12-byte entries. Export and code labels must be in `1..=label_count`.
/// `function_count` must match the number of `function_info` instructions.
/// `opcode_max` is not checked. The parser rejects duplicate chunk IDs. It
/// supports only the listed instructions and compact operand forms. Compact
/// tags occupy the low three bits. The parser accepts the short and one-byte
/// extension forms only. Bit fields are shown from high bit to low bit. Other
/// chunk payloads and padding byte values are ignored. Execution supports
/// `move` into register `x0` only.
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
    let (instructions, number_of_labels) = parse_code(code, &atoms)?;
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

fn parse_code(data: &[u8], atoms: &[String]) -> Result<(Vec<Instruction>, u32), BeamError> {
    if data.len() < CODE_HEADER_SIZE
        || read_u32(&data[..WORD_SIZE])? != CODE_HEADER_SUB_SIZE
        || read_u32(&data[WORD_SIZE..WORD_SIZE * 2])? != INSTRUCTION_SET_ERLANG
    {
        return Err(error("unsupported BEAM code header"));
    }
    let number_of_labels =
        read_u32(&data[CODE_LABEL_COUNT_OFFSET..CODE_LABEL_COUNT_OFFSET + WORD_SIZE])?;
    let declared_function_count =
        read_u32(&data[CODE_FUNCTION_COUNT_OFFSET..CODE_HEADER_SIZE])? as usize;
    let mut cursor = CODE_HEADER_SIZE;
    let mut instructions = Vec::new();
    let mut function_info_count = 0;
    loop {
        let opcode = *data
            .get(cursor)
            .ok_or_else(|| error("truncated BEAM instruction stream"))?;
        cursor += 1;
        let instruction = match opcode {
            OPCODE_LABEL => {
                let label = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
                if label < FIRST_LABEL_NUMBER || label > number_of_labels {
                    return Err(error(format!(
                        "BEAM label {label} is outside declared range 1..={number_of_labels}"
                    )));
                }
                Instruction::Label(label)
            }
            OPCODE_FUNCTION_INFO => {
                function_info_count += 1;
                let module =
                    atom_at(atoms, read_operand(data, &mut cursor, TAG_ATOM)? as usize)?.to_owned();
                let name =
                    atom_at(atoms, read_operand(data, &mut cursor, TAG_ATOM)? as usize)?.to_owned();
                let arity = read_operand(data, &mut cursor, TAG_UNSIGNED)? as u32;
                Instruction::FunctionInfo {
                    module,
                    name,
                    arity,
                }
            }
            OPCODE_RETURN => Instruction::Return,
            OPCODE_MOVE => {
                let value = read_value(data, &mut cursor, atoms)?;
                let destination = read_operand(data, &mut cursor, TAG_REGISTER)? as u32;
                Instruction::Move(value, destination)
            }
            OPCODE_INT_CODE_END => {
                if cursor != data.len() || function_info_count != declared_function_count {
                    return Err(error("invalid BEAM code terminator or function count"));
                }
                instructions.push(Instruction::End);
                return Ok((instructions, number_of_labels));
            }
            _ => return Err(error(format!("unsupported BEAM opcode {opcode}"))),
        };
        instructions.push(instruction);
    }
}

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
            Ok(Term::Atom(atom_at(atoms, index)?.to_owned()))
        }
        unsupported_tag => Err(error(format!(
            "unsupported constant operand tag {unsupported_tag}"
        ))),
    }
}

fn atom_at(atoms: &[String], index: usize) -> Result<&str, BeamError> {
    if index < FIRST_ATOM_INDEX {
        return Err(error("BEAM atom index zero is invalid"));
    }
    atoms
        .get(index - FIRST_ATOM_INDEX)
        .map(String::as_str)
        .ok_or_else(|| error(format!("BEAM atom index {index} out of range")))
}

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
    Err(error("extended BEAM operands are unsupported"))
}

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
    use super::{
        atom_at, parse_atoms, parse_code, parse_exports, read_integer, read_integer_after_first,
        read_operand, read_u32, read_value,
    };

    #[test]
    fn rejects_invalid_operands_and_code_headers() {
        let mut cursor = 0;
        assert!(read_integer(&[0x12], &mut cursor, 1).is_err());
        assert!(read_integer_after_first(&[], &mut 0, 0x19).is_err());
        assert!(read_integer_after_first(&[], &mut 0, 0x09).is_err());
        assert!(read_operand(&[], &mut 0, 0).is_err());
        assert!(read_value(&[0x03], &mut 0, &[]).is_err());
        assert!(read_value(&[0x12], &mut 0, &[]).is_err());
        assert!(parse_code(&[], &[]).is_err());
        assert!(parse_code(&[0; 20], &[]).is_err());
        assert!(
            parse_code(
                &[0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                &[]
            )
            .is_err()
        );
    }

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
}
