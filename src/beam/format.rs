use std::collections::HashMap;

use super::{
    code,
    error::{BeamError, error},
};

pub(super) struct ParsedBeam<'a> {
    pub(super) name: String,
    pub(super) atoms: Vec<String>,
    pub(super) exports: HashMap<(String, u32), u32>,
    pub(super) code: &'a [u8],
}

pub(super) fn parse(bytes: &[u8]) -> Result<ParsedBeam<'_>, BeamError> {
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
    Ok(ParsedBeam {
        name,
        atoms,
        exports,
        code,
    })
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
        let length = code::read_operand(data, &mut cursor, 0)? as usize;
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

pub(super) fn atom_at(atoms: &[String], index: usize) -> Result<&str, BeamError> {
    if index == 0 {
        return Err(error("BEAM atom index zero is invalid"));
    }
    atoms
        .get(index - 1)
        .map(String::as_str)
        .ok_or_else(|| error(format!("BEAM atom index {index} out of range")))
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

#[cfg(test)]
mod tests {
    use super::{atom_at, parse_atoms, parse_exports, read_u32};

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
