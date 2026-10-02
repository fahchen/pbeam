use std::sync::OnceLock;

use pbeam_runtime::{BeamModule, Term};

fn constant_fixture() -> &'static [u8] {
    static BEAM: OnceLock<Vec<u8>> = OnceLock::new();
    BEAM.get_or_init(|| {
        std::fs::read("fixtures/beam/constant.beam").expect("read checked-in BEAM fixture")
    })
}

#[test]
fn executes_exported_atom_and_small_integer_constants() {
    let module = BeamModule::load(constant_fixture()).unwrap();
    assert_eq!(
        module.execute("main", &[]).unwrap(),
        Term::Atom("ok".into())
    );
    assert_eq!(module.execute("number", &[]).unwrap(), Term::Integer(7));
    assert_eq!(module.execute("larger", &[]).unwrap(), Term::Integer(1000));
}

#[test]
fn rejects_truncated_container() {
    assert!(BeamModule::load(b"FOR1").is_err());
}

#[test]
fn rejects_wrong_container_identity() {
    let mut beam = constant_fixture().to_vec();
    beam[8..12].copy_from_slice(b"NOPE");
    assert!(BeamModule::load(&beam).is_err());
}

#[test]
fn rejects_invalid_magic_and_declared_size() {
    let mut wrong_magic = constant_fixture().to_vec();
    wrong_magic[..4].copy_from_slice(b"NOPE");
    assert!(BeamModule::load(&wrong_magic).is_err());

    let mut wrong_size = constant_fixture().to_vec();
    wrong_size[4..8].copy_from_slice(&0u32.to_be_bytes());
    assert!(BeamModule::load(&wrong_size).is_err());
}

#[test]
fn rejects_unsupported_entry_instruction() {
    let mut beam = constant_fixture().to_vec();
    let opcode = beam.windows(3).position(|w| w == [64, 50, 3]).unwrap();
    beam[opcode] = 255;
    assert!(BeamModule::load(&beam).is_err());
}

#[test]
fn reports_missing_export() {
    let module = BeamModule::load(constant_fixture()).unwrap();
    assert!(module.execute("missing", &[]).is_err());
}

fn beam_chunks(beam: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut chunks = Vec::new();
    let mut cursor = 12;
    while cursor < beam.len() {
        let id: [u8; 4] = beam[cursor..cursor + 4].try_into().unwrap();
        let length = u32::from_be_bytes(beam[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let start = cursor + 8;
        chunks.push((id, beam[start..start + length].to_vec()));
        cursor = start + length + (4 - length % 4) % 4;
    }
    chunks
}

fn assemble_chunks(chunks: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    let mut body = b"BEAM".to_vec();
    for (id, data) in chunks {
        body.extend(id);
        body.extend((data.len() as u32).to_be_bytes());
        body.extend(data.iter());
        body.resize(body.len() + (4 - data.len() % 4) % 4, 0);
    }
    let mut beam = b"FOR1".to_vec();
    beam.extend((body.len() as u32).to_be_bytes());
    beam.extend(body);
    beam
}

fn edit_chunk(beam: &[u8], name: &[u8; 4], edit: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut edit = Some(edit);
    let chunks = beam_chunks(beam)
        .into_iter()
        .map(|(id, mut data)| {
            if &id == name {
                edit.take().unwrap()(&mut data);
            }
            (id, data)
        })
        .collect();
    assemble_chunks(chunks)
}

#[test]
fn rejects_truncated_chunk_header_and_out_of_bounds_chunk() {
    let mut truncated = constant_fixture().to_vec();
    truncated.push(0);
    let size = (truncated.len() - 8) as u32;
    truncated[4..8].copy_from_slice(&size.to_be_bytes());
    assert!(
        BeamModule::load(&truncated)
            .unwrap_err()
            .to_string()
            .contains("chunk header")
    );

    let mut oversized = constant_fixture().to_vec();
    oversized[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(
        BeamModule::load(&oversized)
            .unwrap_err()
            .to_string()
            .contains("container bounds")
    );
}

#[test]
fn rejects_duplicate_and_missing_required_chunks() {
    let source = constant_fixture();
    let mut chunks = beam_chunks(source);
    chunks.push(chunks[0].clone());
    assert!(
        BeamModule::load(&assemble_chunks(chunks))
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    for missing in [b"AtU8", b"Code", b"ExpT"] {
        let chunks = beam_chunks(source)
            .into_iter()
            .filter(|(id, _)| id != missing)
            .collect();
        assert!(BeamModule::load(&assemble_chunks(chunks)).is_err());
    }
}

#[test]
fn rejects_bad_atom_table_count_encoding_and_names() {
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"AtU8", |data| {
            data[0..4].copy_from_slice(&(-100i32).to_be_bytes())
        }))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"AtU8", |data| data[0..4]
            .copy_from_slice(&i32::MIN.to_be_bytes())))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"AtU8", |data| data[0..4]
            .copy_from_slice(&3i32.to_be_bytes())))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"AtU8", |data| *data
            .last_mut()
            .unwrap() =
            255))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"AtU8", |data| data.push(0))).is_err()
    );
}

#[test]
fn rejects_invalid_exports_and_code_header() {
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"ExpT", |data| data.truncate(4)))
            .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"ExpT", |data| {
            let first = data[4..16].to_vec();
            data[16..28].copy_from_slice(&first);
        }))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"Code", |data| data[0..4]
            .copy_from_slice(&1u32.to_be_bytes())))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"Code", |data| data[4..8]
            .copy_from_slice(&1u32.to_be_bytes())))
        .is_err()
    );
}

#[test]
fn rejects_code_labels_outside_declared_range() {
    let beam = edit_chunk(constant_fixture(), b"Code", |data| {
        data[12..16].copy_from_slice(&0u32.to_be_bytes());
    });
    assert!(
        BeamModule::load(&beam)
            .unwrap_err()
            .to_string()
            .contains("label")
    );
}

#[test]
fn rejects_zero_code_label() {
    let beam = edit_chunk(constant_fixture(), b"Code", |data| {
        let label = data
            .windows(2)
            .position(|bytes| bytes == [1, 0x10])
            .unwrap();
        data[label + 1] = 0;
    });
    assert!(
        BeamModule::load(&beam)
            .unwrap_err()
            .to_string()
            .contains("outside declared range")
    );
}

#[test]
fn rejects_invalid_code_stream_and_operands() {
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"Code", |data| data
            [16..20]
            .copy_from_slice(&99u32.to_be_bytes())))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"Code", |data| *data
            .last_mut()
            .unwrap() =
            255))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"Code", |data| {
            let i = data.windows(4).position(|w| w == [2, 18, 34, 0]).unwrap();
            data[i + 1] = 19;
        }))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"Code", |data| {
            let i = data.windows(2).position(|w| w == [64, 50]).unwrap();
            data[i + 1] = 52;
        }))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"Code", |data| {
            let i = data.windows(3).position(|w| w == [64, 50, 3]).unwrap();
            data[i + 2] = 20;
        }))
        .is_err()
    );
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"Code", |data| {
            let i = data.windows(3).position(|w| w == [64, 0x69, 0xe8]).unwrap();
            data[i + 1] = 0x19;
        }))
        .is_err()
    );
}

#[test]
fn rejects_truncated_atom_and_zero_atom_index() {
    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"AtU8", |data| {
            *data.last_mut().unwrap() = 0xf0;
        }))
        .is_err()
    );

    assert!(
        BeamModule::load(&edit_chunk(constant_fixture(), b"Code", |data| {
            let i = data.windows(4).position(|w| w == [2, 18, 34, 0]).unwrap();
            data[i + 1] = 2;
        }))
        .is_err()
    );
}

#[test]
fn rejects_export_labels_outside_declared_range() {
    let beam = edit_chunk(constant_fixture(), b"ExpT", |data| {
        for entry in data[4..].chunks_mut(12) {
            entry[8..12].copy_from_slice(&99u32.to_be_bytes());
        }
    });
    assert!(
        BeamModule::load(&beam)
            .unwrap_err()
            .to_string()
            .contains("export label")
    );
}
