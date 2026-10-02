use super::{
    error::{BeamError, error},
    format,
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

pub(super) fn decode(data: &[u8], atoms: &[String]) -> Result<Vec<Instruction>, BeamError> {
    if data.len() < 20 || format::read_u32(&data[..4])? != 16 || format::read_u32(&data[4..8])? != 0
    {
        return Err(error("unsupported BEAM code header"));
    }
    let function_count = format::read_u32(&data[16..20])? as usize;
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
                let module = format::atom_at(atoms, read_operand(data, &mut cursor, 2)? as usize)?
                    .to_owned();
                let name = format::atom_at(atoms, read_operand(data, &mut cursor, 2)? as usize)?
                    .to_owned();
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
            Ok(Term::Atom(format::atom_at(atoms, index)?.to_owned()))
        }
        tag => Err(error(format!("unsupported constant operand tag {tag}"))),
    }
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

#[cfg(test)]
mod tests {
    use super::{decode, read_integer, read_integer_after_first, read_operand, read_value};

    #[test]
    fn rejects_invalid_operands_and_code_headers() {
        let mut cursor = 0;
        assert!(read_integer(&[0x12], &mut cursor, 1).is_err());
        assert!(read_integer_after_first(&[], &mut 0, 0x19).is_err());
        assert!(read_integer_after_first(&[], &mut 0, 0x09).is_err());
        assert!(read_operand(&[], &mut 0, 0).is_err());
        assert!(read_value(&[0x03], &mut 0, &[]).is_err());
        assert!(read_value(&[0x12], &mut 0, &[]).is_err());
        assert!(decode(&[], &[]).is_err());
        assert!(decode(&[0; 20], &[]).is_err());
        assert!(
            decode(
                &[0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                &[]
            )
            .is_err()
        );
    }
}
