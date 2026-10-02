use std::collections::HashMap;

use super::{
    error::{BeamError, error},
    term::Term,
};

#[derive(Debug, Clone)]
pub(super) enum Instruction {
    Label(u32),
    FunctionInfo {
        module: String,
        name: String,
        arity: u32,
    },
    Move(Term, u32),
    Return,
    End,
}

pub(super) struct DecodedModule {
    pub(super) name: String,
    pub(super) exports: HashMap<(String, u32), u32>,
    pub(super) instructions: Vec<Instruction>,
}

/// Parse the BEAM subset used by this runtime.
///
/// Container grammar:
///
/// ```text
/// beam_file ::= "FOR1" container_size "BEAM" chunk*
/// chunk     ::= chunk_id chunk_size payload padding
/// chunk_id  ::= byte[4]
/// payload   ::= byte[chunk_size]
/// padding   ::= byte[(4 - chunk_size mod 4) mod 4]
/// ```
///
/// `container_size` counts bytes after its field, including `BEAM` and chunks.
/// `chunk_size` counts payload bytes and excludes padding. The parser requires
/// the `AtU8`, `ExpT`, and `Code` chunks.
pub(super) fn parse(bytes: &[u8]) -> Result<DecodedModule, BeamError> {
    if bytes.len() < 12 || &bytes[..4] != b"FOR1" {
        return Err(error("invalid BEAM container header"));
    }
    let declared = read_u32(&bytes[4..8])? as usize;
    if declared != bytes.len() - 8 || &bytes[8..12] != b"BEAM" {
        return Err(error("invalid BEAM container size or identity"));
    }

    let mut chunks = HashMap::new();
    let mut cursor = 12;
    while cursor < bytes.len() {
        if bytes.len() - cursor < 8 {
            return Err(error("truncated BEAM chunk header"));
        }
        let header_end = cursor + 8;
        let id = &bytes[cursor..cursor + 4];
        let length = read_u32(&bytes[cursor + 4..header_end])? as usize;
        let padding = (4 - (length % 4)) % 4;
        if length > bytes.len() - header_end || padding > bytes.len() - header_end - length {
            return Err(error("BEAM chunk exceeds container bounds"));
        }
        let data_end = header_end + length;
        let padded_end = data_end + padding;
        if chunks
            .insert(id.to_vec(), &bytes[header_end..data_end])
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
    let (instructions, number_of_labels) = decode_code(code, &atoms)?;
    if let Some(label) = exports
        .values()
        .find(|&&label| label == 0 || label > number_of_labels)
    {
        return Err(error(format!(
            "BEAM export label {label} is outside declared range 1..={number_of_labels}"
        )));
    }
    Ok(DecodedModule {
        name,
        exports,
        instructions,
    })
}

fn decode_code(data: &[u8], atoms: &[String]) -> Result<(Vec<Instruction>, u32), BeamError> {
    if data.len() < 20 || read_u32(&data[..4])? != 16 || read_u32(&data[4..8])? != 0 {
        return Err(error("unsupported BEAM code header"));
    }
    let number_of_labels = read_u32(&data[12..16])?;
    let function_count = read_u32(&data[16..20])? as usize;
    let mut cursor = 20;
    let mut instructions = Vec::new();
    let mut functions = 0;
    loop {
        let opcode = *data
            .get(cursor)
            .ok_or_else(|| error("truncated BEAM instruction stream"))?;
        cursor += 1;
        let instruction = match opcode {
            1 => {
                let label = read_operand(data, &mut cursor, 0)? as u32;
                if label == 0 || label > number_of_labels {
                    return Err(error(format!(
                        "BEAM label {label} is outside declared range 1..={number_of_labels}"
                    )));
                }
                Instruction::Label(label)
            }
            2 => {
                functions += 1;
                let module =
                    atom_at(atoms, read_operand(data, &mut cursor, 2)? as usize)?.to_owned();
                let name = atom_at(atoms, read_operand(data, &mut cursor, 2)? as usize)?.to_owned();
                let arity = read_operand(data, &mut cursor, 0)? as u32;
                Instruction::FunctionInfo {
                    module,
                    name,
                    arity,
                }
            }
            19 => Instruction::Return,
            64 => {
                let value = read_value(data, &mut cursor, atoms)?;
                let destination = read_operand(data, &mut cursor, 3)? as u32;
                Instruction::Move(value, destination)
            }
            3 => {
                if cursor != data.len() || functions != function_count {
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
    let count = read_u32(data)? as i32;
    let count = usize::try_from(
        count
            .checked_neg()
            .ok_or_else(|| error("unsupported atom table encoding"))?,
    )
    .map_err(|_| error("invalid atom count"))?;
    if count > data.len() - 4 {
        return Err(error("atom count exceeds chunk size"));
    }
    let mut atoms = Vec::with_capacity(count);
    let mut cursor = 4;
    for _ in 0..count {
        let length = read_operand(data, &mut cursor, 0)? as usize;
        if length > data.len() - cursor {
            return Err(error("truncated atom name"));
        }
        let end = cursor + length;
        let atom = std::str::from_utf8(
            data.get(cursor..end)
                .ok_or_else(|| error("truncated atom name"))?,
        )
        .map_err(|_| error("invalid UTF-8 atom name"))?;
        atoms.push(atom.to_owned());
        cursor = end;
    }
    if cursor != data.len() {
        return Err(error("unexpected bytes after atom table"));
    }
    Ok(atoms)
}

fn parse_exports(data: &[u8], atoms: &[String]) -> Result<HashMap<(String, u32), u32>, BeamError> {
    let count = read_u32(data)? as usize;
    let expected = 4usize
        .checked_add(
            count
                .checked_mul(12)
                .ok_or_else(|| error("export count overflow"))?,
        )
        .ok_or_else(|| error("export table size overflow"))?;
    if expected != data.len() {
        return Err(error("invalid export table size"));
    }
    let mut exports = HashMap::new();
    for entry in data[4..].chunks(12) {
        let name = atom_at(atoms, read_u32(&entry[..4])? as usize)?.to_owned();
        let arity = read_u32(&entry[4..8])?;
        let label = read_u32(&entry[8..])?;
        if exports.insert((name, arity), label).is_some() {
            return Err(error("duplicate BEAM export"));
        }
    }
    Ok(exports)
}

fn read_value(data: &[u8], cursor: &mut usize, atoms: &[String]) -> Result<Term, BeamError> {
    let byte = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    match byte & 7 {
        1 => {
            let value = read_integer(data, cursor, 1)?;
            Ok(Term::Integer(
                i64::try_from(value).map_err(|_| error("integer constant exceeds i64"))?,
            ))
        }
        2 => {
            let index = read_operand(data, cursor, 2)? as usize;
            Ok(Term::Atom(atom_at(atoms, index)?.to_owned()))
        }
        tag => Err(error(format!("unsupported constant operand tag {tag}"))),
    }
}

fn atom_at(atoms: &[String], index: usize) -> Result<&str, BeamError> {
    if index == 0 {
        return Err(error("BEAM atom index zero is invalid"));
    }
    atoms
        .get(index - 1)
        .map(String::as_str)
        .ok_or_else(|| error(format!("BEAM atom index {index} out of range")))
}

fn required_chunk<'a>(
    chunks: &HashMap<Vec<u8>, &'a [u8]>,
    id: &[u8; 4],
) -> Result<&'a [u8], BeamError> {
    chunks
        .get(id.as_slice())
        .copied()
        .ok_or_else(|| error(format!("missing {} chunk", String::from_utf8_lossy(id))))
}

fn read_operand(data: &[u8], cursor: &mut usize, tag: u8) -> Result<u64, BeamError> {
    let first = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    *cursor += 1;
    if first & 7 != tag {
        return Err(error(format!("unexpected BEAM operand tag {}", first & 7)));
    }
    read_integer_after_first(data, cursor, first)
}

fn read_integer(data: &[u8], cursor: &mut usize, tag: u8) -> Result<i128, BeamError> {
    let first = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    *cursor += 1;
    if first & 7 != tag {
        return Err(error(format!("unexpected BEAM operand tag {}", first & 7)));
    }
    let value = read_integer_after_first(data, cursor, first)?;
    Ok(value as i128)
}

fn read_integer_after_first(data: &[u8], cursor: &mut usize, first: u8) -> Result<u64, BeamError> {
    if first & 8 == 0 {
        return Ok((first >> 4) as u64);
    }
    if first & 16 == 0 {
        let low = *data
            .get(*cursor)
            .ok_or_else(|| error("truncated BEAM operand"))?;
        *cursor += 1;
        return Ok((((first & 0xe0) as u64) << 3) | low as u64);
    }
    Err(error("extended BEAM operands are unsupported"))
}

fn read_u32(bytes: &[u8]) -> Result<u32, BeamError> {
    let bytes = bytes.get(..4).ok_or_else(|| {
        error(format!(
            "truncated 32-bit BEAM field: got {} bytes",
            bytes.len()
        ))
    })?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::{
        atom_at, decode_code, parse_atoms, parse_exports, read_integer, read_integer_after_first,
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
        assert!(decode_code(&[], &[]).is_err());
        assert!(decode_code(&[0; 20], &[]).is_err());
        assert!(
            decode_code(
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
