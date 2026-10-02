use std::collections::HashMap;

use super::{BeamError, Instruction, Term, error};

pub(super) fn load(bytes: &[u8]) -> Result<super::BeamModule, BeamError> {
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
    let code = parse_code(required_chunk(&chunks, b"Code")?, &atoms)?;
    let exports = parse_exports(required_chunk(&chunks, b"ExpT")?, &atoms)?;
    let name = atoms
        .first()
        .ok_or_else(|| error("empty BEAM atom table"))?
        .clone();
    Ok(super::BeamModule {
        name,
        exports,
        instructions: code,
    })
}
fn required_chunk<'a>(
    chunks: &'a HashMap<Vec<u8>, &'a [u8]>,
    id: &[u8; 4],
) -> Result<&'a [u8], BeamError> {
    chunks
        .get(id.as_slice())
        .copied()
        .ok_or_else(|| error(format!("missing {} chunk", String::from_utf8_lossy(id))))
}

pub(super) fn parse_atoms(data: &[u8]) -> Result<Vec<String>, BeamError> {
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

pub(super) fn parse_exports(
    data: &[u8],
    atoms: &[String],
) -> Result<HashMap<(String, u32), u32>, BeamError> {
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

pub(super) fn parse_code(data: &[u8], atoms: &[String]) -> Result<Vec<Instruction>, BeamError> {
    if data.len() < 20 || read_u32(&data[..4])? != 16 || read_u32(&data[4..8])? != 0 {
        return Err(error("unsupported BEAM code header"));
    }
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
            1 => Instruction::Label(read_operand(data, &mut cursor, 0)? as u32),
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
                return Ok(instructions);
            }
            _ => return Err(error(format!("unsupported BEAM opcode {opcode}"))),
        };
        instructions.push(instruction);
    }
}

pub(super) fn read_value(
    data: &[u8],
    cursor: &mut usize,
    atoms: &[String],
) -> Result<Term, BeamError> {
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

pub(super) fn atom_at(atoms: &[String], index: usize) -> Result<&str, BeamError> {
    if index == 0 {
        return Err(error("BEAM atom index zero is invalid"));
    }
    atoms
        .get(index - 1)
        .map(String::as_str)
        .ok_or_else(|| error(format!("BEAM atom index {index} out of range")))
}

pub(super) fn read_operand(data: &[u8], cursor: &mut usize, tag: u8) -> Result<u64, BeamError> {
    let first = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    *cursor += 1;
    if first & 7 != tag {
        return Err(error(format!("unexpected BEAM operand tag {}", first & 7)));
    }
    read_integer_after_first(data, cursor, first)
}

pub(super) fn read_integer(data: &[u8], cursor: &mut usize, tag: u8) -> Result<i128, BeamError> {
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

pub(super) fn read_integer_after_first(
    data: &[u8],
    cursor: &mut usize,
    first: u8,
) -> Result<u64, BeamError> {
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

pub(super) fn read_u32(bytes: &[u8]) -> Result<u32, BeamError> {
    let bytes = bytes.get(..4).ok_or_else(|| {
        error(format!(
            "truncated 32-bit BEAM field: got {} bytes",
            bytes.len()
        ))
    })?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
