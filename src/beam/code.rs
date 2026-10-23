use std::{collections::HashMap, sync::Arc};

use num_bigint::{BigInt, Sign};

use super::attributes;
use super::module::{BeamModule, Import};
use crate::{
    error::{BeamError, error},
    runtime::import_kind,
    term::{
        Atom, AtomTable, Binary, Fun, Lambda, MAX_ATOM_CHARACTERS, MAX_TUPLE_SIZE, Map, Term,
        fits_big_arity,
    },
};

const WORD_SIZE: usize = 4;
mod instruction;
mod line;
pub(crate) use instruction::{
    BinaryMatchCommand, BinarySegment, FloatOperation, Instruction, Operand, SelectValueChoices,
    TestKind, TestOperand,
};
pub(crate) use line::LineTable;

const CHUNK_ALIGNMENT: usize = WORD_SIZE;
const CHUNK_HEADER_SIZE: usize = WORD_SIZE * 2;
const CONTAINER_HEADER_SIZE: usize = WORD_SIZE * 3;
const EXPORT_ENTRY_SIZE: usize = WORD_SIZE * 3;
const IMPORT_ENTRY_SIZE: usize = WORD_SIZE * 3;
/// The size of a `FunT` entry: six 32-bit fields.
const LAMBDA_ENTRY_SIZE: usize = WORD_SIZE * 6;
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
/// The first byte of an integer operand with 9 or more payload bytes: size
/// code 7, the sign and extended flags, and `TAG_INTEGER`. Its value is
/// outside the 64-bit signed range, so ERTS loads it as a bignum.
const HUGE_INTEGER_FIRST_BYTE: u8 =
    0b1110_0000 | COMPACT_SIGN_FLAG | COMPACT_EXTENDED_FLAG | TAG_INTEGER;
const LIST_SUBTAG: i128 = 1;
const ETF_VERSION: u8 = 131;
/// `NEW_FLOAT_EXT`, an IEEE 754 binary64 float in 8 big-endian bytes, such
/// as `1.5` as `[70, 63, 248, 0, 0, 0, 0, 0, 0]` (`external.c:4651-4673`).
const ETF_NEW_FLOAT_EXT: u8 = 70;
const ETF_SMALL_INTEGER_EXT: u8 = 97;
/// `INTEGER_EXT`, a 32-bit big-endian signed integer, such as `-1` as
/// `[98, 255, 255, 255, 255]`.
const ETF_INTEGER_EXT: u8 = 98;
const ETF_SMALL_TUPLE_EXT: u8 = 104;
/// `LARGE_TUPLE_EXT`, a tuple with a four-byte big-endian element count,
/// such as a 300-element tuple (`external.c:4542-4549`).
const ETF_LARGE_TUPLE_EXT: u8 = 105;
/// `SMALL_BIG_EXT`: a bignum with a one-byte digit count, a sign byte, and
/// little-endian bytes, such as `1 bsl 64` as
/// `[110, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]` (`external.c:4457-4497`).
const ETF_SMALL_BIG_EXT: u8 = 110;
/// `LARGE_BIG_EXT`: a bignum with a four-byte big-endian byte count.
const ETF_LARGE_BIG_EXT: u8 = 111;
const ETF_NIL_EXT: u8 = 106;
const ETF_STRING_EXT: u8 = 107;
const ETF_LIST_EXT: u8 = 108;
const ETF_BINARY_EXT: u8 = 109;
/// `BIT_BINARY_EXT`, a bitstring such as `<<1:3>>` as `[77, 0, 0, 0, 1, 3,
/// 32]`: a four-byte big-endian byte count, the number of bits (1 to 8) of
/// the last byte that are part of the bitstring, and the bytes
/// (`external.c:4895-4923`).
const ETF_BIT_BINARY_EXT: u8 = 77;
/// `EXPORT_EXT`, an external fun such as `fun lists:reverse/1`.
const ETF_EXPORT_EXT: u8 = 113;
const ETF_MAP_EXT: u8 = 116;
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
/// executable_instruction ::= label | function_info | move | call | call_only | call_ext | jump | select_val | test | put_list | allocate | deallocate | return | noop | make_fun3 | call_fun | call_fun2 | is_function2 | generated_instruction
/// label       ::= 1 compact(0)
/// function_info ::= 2 compact(2) compact(2) compact(0)
/// move        ::= 64 move_source (x_register | y_register)
/// move_source ::= value | x_register | y_register
/// move_source ::= literal_value
/// move_source ::= bignum_value ; loads, and fails when the instruction reads it
/// bignum_value ::= huge(1) ; PBeam has no bignum term
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
/// test_opcode ::= 45 ; is_integer | 48 ; is_atom | 49 ; is_pid | 50 ; is_reference | 53 ; is_binary | 55 ; is_list | 57 ; is_tuple | 77 ; is_function | 114 ; is_boolean | 129 ; is_bitstr | 156 ; is_map
/// is_function2 ::= 115 compact(5) move_source move_source ; the arity is a literal or a register
/// put_list ::= 69 move_source move_source register
/// make_fun3 ::= 171 lambda_number:compact(0) register environment
/// environment ::= compact(7, short=1) count:compact(0) move_source[count]
/// call_fun ::= 75 arity:compact(0)
/// call_fun2 ::= 178 call_fun2_tag arity:compact(0) (register | typed_register)
/// call_fun2_tag ::= atom_value | lambda_number:compact(0) ; the atom safe or unsafe
/// noop ::= 16 test_heap_operands | 172 init_yregs_operands
/// int_code_end ::= 3
/// imports_chunk ::= "ImpT" chunk_size import_table padding
/// import_table ::= import_count import[import_count]
/// import_count ::= u32be
/// import ::= module_atom_index:u32be function_atom_index:u32be arity:u32be
/// lambdas_chunk ::= "FunT" chunk_size lambda_table padding
/// lambda_table ::= lambda_count:i32be lambda[lambda_count] lambda*
/// lambda ::= function_atom_index:i32be total_arity:i32be label:i32be index:i32be
///            free_count:i32be old_unique:i32be
/// value       ::= integer_value | atom_value
/// integer_value ::= compact(1) ; a large(1) payload is two's complement
/// atom_value  ::= compact(2)
/// nil_value ::= compact(2) ; atom index zero
/// literals_chunk ::= "LitT" chunk_size 0:u32be literal_count literal_entry*
/// literal_count ::= u32be
/// literal_entry ::= term_size:u32be 131 external_term ; term_size bytes in total
/// external_term ::= binary_term | string_term | nil_term | atom_term | tuple_term | list_term
///                 | map_term | small_integer_term | integer_term | export_term
/// small_integer_term ::= 97 value:u8
/// integer_term ::= 98 value:i32be
/// binary_term ::= 109 binary_size:u32be byte[binary_size]
/// string_term ::= 107 string_size:u16be byte[string_size]
/// nil_term    ::= 106
/// atom_term   ::= 119 name_size:u8 utf8_byte[name_size]
/// tuple_term  ::= 104 arity:u8 external_term[arity]
/// list_term   ::= 108 length:u32be external_term[length] tail:external_term
/// map_term    ::= 116 pair_count:u32be (key:external_term value:external_term)[pair_count]
/// export_term ::= 113 module:atom_term function:atom_term arity:small_integer_term
///
/// compact(tag) ::= short(tag) | extended(tag) | large(tag)
/// short(tag)   ::= value:4 0 tag:3
/// extended(tag) ::= high:3 1 0 tag:3 low:8
/// large(tag)   ::= size_code:3 1 1 tag:3 byte[size_code + 2] ; size_code in 0..=6
/// huge(tag)    ::= 7:3 1 1 tag:3 extra:compact(0) byte[extra + 9]
/// ```
///
/// `container_size` counts bytes after its field, including `BEAM` and chunks.
/// `chunk_size` counts payload bytes and excludes padding. The parser requires
/// the `AtU8`, `ExpT`, and `Code` chunks. `ImpT` is required for external
/// calls. `u32be` is an unsigned big-endian
/// integer. `i32be` is a signed big-endian integer. `byte[n]` means exactly
/// `n` bytes. `AtU8` stores a non-positive count; its absolute value gives the
/// number of atoms. Atom names use UTF-8. The module name is the first atom.
/// Atom indices are one-based. The parser puts each `AtU8` atom, in chunk
/// order, into the atom table of the program, and then each `LitT` atom, in
/// byte order, as the ERTS loader does. `ExpT` contains exactly the declared number of
/// 12-byte entries. Export and code labels must be in `1..=label_count`.
/// `function_count` must match the number of `function_info` instructions.
/// The `FunT` chunk is optional, and an empty `FunT` chunk is the same as no
/// chunk, as `parse_lambda_chunk` (`beam_file.c:394-441`) reads it. Its
/// bytes after the count are 24-byte entries, and `lambda_count` must not
/// be more than their number. A `function_atom_index` is in
/// `0..=atom_count`, and a lambda `label` is in `1..label_count`. A
/// lambda has `total_arity - free_count` arguments, so `free_count` must
/// be in `0..=total_arity`. `make_fun3` and a numeric `call_fun2_tag` refer to
/// a `FunT` entry by its zero-based position. The `environment` of
/// `make_fun3` lists the free variables: a `lambda_number` outside the
/// table or a `count` other than the `free_count` of the entry is an error.
/// For example, `[0xAB, 0x20, 0x03, 0x17, 0x10, 0x03]` makes a fun of
/// entry 2 with the free variable `x0` and writes it to `x0`.
/// `opcode_max` is not checked. The parser rejects duplicate chunk IDs. It
/// supports only the listed instructions and compact operand forms. `LitT`
/// supports uncompressed tables. A literal loads as a value only when every
/// nested tag is listed, such as a bignum (`SMALL_BIG_EXT`, 110, as
/// `1 bsl 64` is `[110, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]`, and
/// `LARGE_BIG_EXT`, 111); another tag, or a map key
/// that `Map` does not support yet, makes the literal unsupported, and
/// execution fails only if an instruction uses it. `[a|[b]]` and `[a,b]`
/// give the same list, a `list_term` whose tail is not a list gives an
/// improper list such as `[a|b]`, and a `list_term` of length 0 gives its
/// tail. `[70, 63, 248, 0, 0, 0, 0, 0, 0]` is the float 1.5, `[97, 1]` is the integer 1, and
/// `[98, 255, 255, 255, 255]` is `-1`. On a 64-bit ERTS each `INTEGER_EXT`
/// value is a small integer (`dec_term`, `external.c:4435-4447`). A
/// map literal is a flatmap up to 32 pairs and a hashmap above 32 pairs, in
/// any input order, as in ERTS. A map literal with a duplicate key is an
/// error, as ERTS rejects it with `badarg`. The
/// parser retains generated instructions that it does not execute. Execution
/// fails if it reaches one. Compact
/// tags occupy the low three bits. A `large` payload is big-endian. ERTS
/// sign-extends the payload of `large(1)` only, so `{integer,-1}` is
/// `[0x19, 0xFF, 0xFF]` and `{integer,-101714049538469044}` is `[0xD9]`
/// followed by its eight two's-complement bytes. The `short` and `extended`
/// forms are never negative. A payload of 9 or more bytes is a bignum
/// constant (see [`read_huge_integer`]). Bit fields are shown from high bit to low bit. Other
/// chunk payloads and padding byte values are ignored. The `ImpT` chunk maps
/// external call indexes to module/function/arity. The executor supports only
/// the listed instructions.
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let module = parse(&beam_bytes, &atoms)?;
/// assert_eq!(module.name, "Elixir.Hello");
/// assert!(atoms.get("Elixir.Hello").is_some());
/// ```
///
/// # Errors
///
/// A malformed or unsupported module is an error. A new atom when the atom
/// table is full is an error, as in ERTS.
pub(crate) fn parse(bytes: &[u8], atom_table: &AtomTable) -> Result<BeamModule, BeamError> {
    let chunks = read_chunks(bytes)?;
    let atoms = parse_atoms(required_chunk(&chunks, b"AtU8")?, atom_table)?;
    decode(&chunks, atoms, atom_table)
}

/// Why [`prepare`] gives no module.
#[derive(Debug)]
pub(crate) enum PrepareError {
    /// ERTS gives `{error, badfile}` for these bytes.
    BadFile,
    /// The PBeam loader cannot decode the bytes. ERTS can load such a module
    /// or give `{error, badfile}`, so the caller stops the run.
    Unsupported(BeamError),
}

/// Parse the BEAM bytes of the module `module` for
/// `erts_internal:prepare_loading/2`, as [`parse`] does. Bytes that are not
/// an IFF form give [`PrepareError::BadFile`], as `iff_init`
/// (`beam_file.c:2040-2058`) fails for them: fewer than 8 bytes, a first
/// word other than `FOR1`, or a form size that is negative or larger than
/// the bytes after it. A first atom other than `module` also gives
/// [`PrepareError::BadFile`], as `erts_prepare_loading` gives `badfile` when
/// the module name in the object code is not the given name
/// (`beam_load.c:185-187`). ERTS also gives `badfile` for the other chunk
/// errors, but the PBeam loader cannot tell a corrupt chunk from a form that
/// it does not decode yet, so each other error is
/// [`PrepareError::Unsupported`]. The atoms of the `AtU8` chunk go into
/// `atom_table` also when a later step fails, as `parse_atom_chunk` puts
/// them into the ERTS atom table before the name check.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// assert_eq!(prepare(LOAD_TARGET, "load_target", &atoms)?.name, "load_target");
/// assert!(matches!(prepare(LOAD_TARGET, "other", &atoms), Err(PrepareError::BadFile)));
/// assert!(matches!(prepare(b"FOR1bad", "load_target", &atoms), Err(PrepareError::BadFile)));
/// assert!(matches!(prepare(LOAD_ON_LOAD, "load_on_load", &atoms), Err(PrepareError::Unsupported(_))));
/// ```
pub(crate) fn prepare(
    bytes: &[u8],
    module: &str,
    atom_table: &AtomTable,
) -> Result<BeamModule, PrepareError> {
    let form_size = bytes
        .get(WORD_SIZE..WORD_SIZE * 2)
        .map(|size| i32::from_be_bytes(size.try_into().expect("a 4-byte slice")));
    let is_form = bytes.starts_with(b"FOR1")
        && form_size.is_some_and(|size| {
            usize::try_from(size).is_ok_and(|size| size <= bytes.len() - WORD_SIZE * 2)
        });
    if !is_form {
        return Err(PrepareError::BadFile);
    }
    let chunks = read_chunks(bytes).map_err(PrepareError::Unsupported)?;
    let atoms = required_chunk(&chunks, b"AtU8")
        .and_then(|data| parse_atoms(data, atom_table))
        .map_err(PrepareError::Unsupported)?;
    if atoms.first().map(|atom| atom_table.name_of(atom)) != Some(module) {
        return Err(PrepareError::BadFile);
    }
    decode(&chunks, atoms, atom_table).map_err(PrepareError::Unsupported)
}

/// Read the chunks of a BEAM container by chunk ID, as the grammar of
/// [`parse`] gives them.
///
/// # Examples
///
/// ```rust,ignore
/// let chunks = read_chunks(&beam_bytes)?;
/// assert!(chunks.contains_key(b"Code".as_slice()));
/// assert!(read_chunks(b"FOR1bad").is_err());
/// ```
///
/// # Errors
///
/// A container header that is not valid, a chunk outside the container,
/// and a duplicate chunk ID are errors.
fn read_chunks(bytes: &[u8]) -> Result<HashMap<Vec<u8>, &[u8]>, BeamError> {
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
    Ok(chunks)
}

/// Decode the module of the chunks of a BEAM container, with the atoms of
/// its `AtU8` chunk, as [`parse`] describes.
///
/// # Examples
///
/// ```rust,ignore
/// let chunks = read_chunks(&beam_bytes)?;
/// let atoms = parse_atoms(required_chunk(&chunks, b"AtU8")?, &mut table)?;
/// assert_eq!(decode(&chunks, atoms, &mut table)?.name, "Elixir.Hello");
/// ```
fn decode(
    chunks: &HashMap<Vec<u8>, &[u8]>,
    atoms: Vec<Atom>,
    atom_table: &AtomTable,
) -> Result<BeamModule, BeamError> {
    let exports = parse_exports(required_chunk(chunks, b"ExpT")?, &atoms, atom_table)?;
    let code = required_chunk(chunks, b"Code")?;
    let name = atom_table
        .name_of(
            atoms
                .first()
                .ok_or_else(|| error("empty BEAM atom table"))?,
        )
        .to_owned();
    let imports = chunks
        .get(b"ImpT".as_slice())
        .map(|data| parse_imports(data, &atoms, atom_table))
        .transpose()?
        .unwrap_or_default();
    let literals = chunks
        .get(b"LitT".as_slice())
        .map(|data| parse_literals(data, atom_table))
        .transpose()?
        .unwrap_or_default();
    let lambdas = chunks
        .get(b"FunT".as_slice())
        .filter(|data| !data.is_empty())
        .map(|data| parse_lambdas(data, &atoms, module_md5(chunks)))
        .transpose()?
        .unwrap_or_default();
    let strings = chunks.get(b"StrT".as_slice()).copied().unwrap_or_default();
    // ERTS reads a `Line` chunk that is not empty (`beam_file.c:1706-1711`).
    let mut lines = chunks
        .get(b"Line".as_slice())
        .filter(|data| !data.is_empty())
        .map(|data| LineTable::parse(data, &name))
        .transpose()?
        .unwrap_or_default();
    let (instructions, number_of_labels, on_load) = instruction::parse_code(
        code, atom_table, &atoms, &imports, &literals, &lambdas, strings, &mut lines,
    )?;
    if let Some(label) = exports
        .values()
        .find(|&&label| label < FIRST_LABEL_NUMBER || label > number_of_labels)
    {
        return Err(error(format!(
            "BEAM export label {label} is outside declared range 1..={number_of_labels}"
        )));
    }
    if let Some((lambda, _)) = lambdas.iter().find(|(lambda, _)| {
        lambda.label() < FIRST_LABEL_NUMBER || lambda.label() >= number_of_labels
    }) {
        return Err(error(format!(
            "BEAM lambda label {} is outside declared range 1..{number_of_labels}",
            lambda.label()
        )));
    }
    let imports = imports
        .into_iter()
        .map(|(module, name, arity)| Import {
            kind: import_kind(&module, &name, arity),
            module,
            name,
            arity,
        })
        .collect();
    let mut module = BeamModule::new(name, exports, imports, instructions)?;
    module.on_load = on_load;
    module.lines = lines;
    module.lambdas = lambdas.into_iter().map(|(lambda, _)| lambda).collect();
    module.native_declarations = chunks
        .get(b"Attr".as_slice())
        .map(|data| attributes::native_declarations(data))
        .transpose()?
        .unwrap_or_default();
    Ok(module)
}

/// Read the atoms of an `AtU8` chunk and put each one, in chunk order, into
/// the atom table (`parse_atom_chunk` in ERTS).
///
/// # Examples
///
/// ```rust,ignore
/// let mut table = AtomTable::new();
/// let atoms = parse_atoms(&[0xff, 0xff, 0xff, 0xff, 0x10, b'a'], &mut table)?;
/// assert_eq!(table.name_of(&atoms[0]), "a");
/// assert_eq!(table.get("a"), Some(atoms[0].clone()));
/// ```
fn parse_atoms(data: &[u8], atom_table: &AtomTable) -> Result<Vec<Atom>, BeamError> {
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
        let atom = atom_chunk_name(
            data.get(cursor..atom_name_end)
                .ok_or_else(|| error("truncated atom name"))?,
        )?;
        atoms.push(atom_table.put(atom)?);
        cursor = atom_name_end;
    }
    if cursor != data.len() {
        return Err(error("unexpected bytes after atom table"));
    }
    Ok(atoms)
}

/// Return the atom name of an `AtU8` entry. As `parse_atom_chunk` calls
/// `erts_atom_put` with `trunc` set, a name longer than 255 characters
/// becomes its first 255 characters, and ERTS does not check the bytes after
/// them. Thus two long names with the same first 255 characters are one atom.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(atom_chunk_name(b"ok")?, "ok");
/// let long = [b"a".repeat(255), vec![b'b', 0xff]].concat();
/// assert_eq!(atom_chunk_name(&long)?, "a".repeat(255));
/// assert!(atom_chunk_name(&[b'a', 0xff]).is_err());
/// ```
///
/// # Errors
///
/// A name that is not valid UTF-8 in its first 255 characters is the error
/// `invalid UTF-8 atom name`.
fn atom_chunk_name(bytes: &[u8]) -> Result<&str, BeamError> {
    let valid = std::str::from_utf8(bytes).unwrap_or_else(|invalid| {
        std::str::from_utf8(&bytes[..invalid.valid_up_to()]).expect("a valid UTF-8 prefix")
    });
    match valid.char_indices().nth(MAX_ATOM_CHARACTERS) {
        Some((end, _)) => Ok(&valid[..end]),
        None if valid.len() == bytes.len() || valid.chars().count() == MAX_ATOM_CHARACTERS => {
            Ok(valid)
        }
        None => Err(error("invalid UTF-8 atom name")),
    }
}

/// Read exported function names, arities, and labels from an `ExpT` chunk.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let names = vec![atoms.put("main")?];
/// let table = &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1];
/// let exports = parse_exports(table, &names, &atoms)?;
/// assert_eq!(exports.get(&("main".to_owned(), 0)), Some(&1));
/// ```
fn parse_exports(
    data: &[u8],
    atoms: &[Atom],
    atom_table: &AtomTable,
) -> Result<HashMap<(String, u32), u32>, BeamError> {
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
        let name = atom_table.name_of(atom_at(atoms, atom_index)?).to_owned();
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
/// let mut table = AtomTable::new();
/// let atoms = vec![table.put("IO")?, table.put("puts")?];
/// let entries = &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1];
/// assert_eq!(parse_imports(entries, &atoms, &table)?, [("IO".into(), "puts".into(), 1)]);
/// ```
fn parse_imports(
    data: &[u8],
    atoms: &[Atom],
    atom_table: &AtomTable,
) -> Result<Vec<(String, String, u32)>, BeamError> {
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
            let module = read_u32(&entry[..WORD_SIZE])? as usize;
            let function = read_u32(&entry[WORD_SIZE..WORD_SIZE * 2])? as usize;
            Ok((
                atom_table.name_of(atom_at(atoms, module)?).to_owned(),
                atom_table.name_of(atom_at(atoms, function)?).to_owned(),
                read_u32(&entry[WORD_SIZE * 2..])?,
            ))
        })
        .collect()
}

/// Read the entries of a `FunT` chunk, as `parse_lambda_chunk`
/// (`beam_file.c:394-441`) does, and return the fun entry and the number of
/// free variables of each one. The caller checks the labels against the
/// label count of the `Code` chunk.
///
/// # Examples
///
/// ```rust,ignore
/// // fun(X) -> {Tag, X} end: function atom 2, total arity 2, label 53,
/// // index 0, one free variable, old unique value 7.
/// let entry = [0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 53, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 7];
/// let lambdas = parse_lambdas(&entry, &atoms)?;
/// assert_eq!((lambdas[0].0.arity(), lambdas[0].1), (1, 1));
/// ```
///
/// # Errors
///
/// A negative count, a count of more entries than the chunk holds, bytes
/// that are not whole entries, a function atom index outside
/// `0..=atom_count`, and a number of free variables outside
/// `0..=total_arity` are errors.
fn parse_lambdas(
    data: &[u8],
    atoms: &[Atom],
    uniq: [u8; 16],
) -> Result<Vec<(Arc<Lambda>, usize)>, BeamError> {
    let field = |entry: &[u8], position: usize| {
        let bytes = &entry[position * WORD_SIZE..(position + 1) * WORD_SIZE];
        i32::from_be_bytes(bytes.try_into().expect("a FunT entry has six fields"))
    };
    let count =
        usize::try_from(read_u32(data)? as i32).map_err(|_| error("negative BEAM lambda count"))?;
    let entries = &data[WORD_SIZE..];
    // ERTS also hashes the entries into the module MD5, and it rejects
    // bytes that are not whole entries (`beam_file.c:1773-1802`).
    if !entries.len().is_multiple_of(LAMBDA_ENTRY_SIZE) || count > entries.len() / LAMBDA_ENTRY_SIZE
    {
        return Err(error("invalid BEAM lambda table size"));
    }
    let module = atom_at(atoms, FIRST_ATOM_INDEX)?;
    entries
        .chunks(LAMBDA_ENTRY_SIZE)
        .take(count)
        .enumerate()
        .map(|(position, entry)| {
            let [
                atom_index,
                total_arity,
                label,
                index,
                free_count,
                old_unique,
            ] = [0, 1, 2, 3, 4, 5].map(|position| field(entry, position));
            // ERTS keeps atom index 0 as the non-value slot of `[]`.
            if !usize::try_from(atom_index).is_ok_and(|index| index <= atoms.len()) {
                return Err(error("BEAM lambda function atom index out of range"));
            }
            let arity = total_arity
                .checked_sub(free_count)
                .filter(|&arity| free_count >= 0 && arity >= 0)
                .ok_or_else(|| error("BEAM lambda has more free variables than arguments"))?;
            let lambda = Lambda::new(
                module.clone(),
                index,
                position as i32,
                old_unique,
                label as u32,
                arity as u32,
            )
            .with_uniq(uniq);
            Ok((Arc::new(lambda), free_count as usize))
        })
        .collect()
}

/// Return the MD5 of the module, as `beamfile_read` computes it
/// (`beam_file.c:1751-1830`): the MD5 of the `AtU8`, `Code`, `StrT`, `ImpT`,
/// `ExpT`, `FunT`, `LitT`, `Meta`, `Recs`, and `DbgB` chunks in this order,
/// where each `OldUniq` field of the `FunT` entries counts as zero. A chunk
/// that the module does not have adds no bytes. The caller reads the `FunT`
/// chunk only when it is not empty and has whole entries, as
/// [`parse_lambdas`] checks.
///
/// # Examples
///
/// ```rust,ignore
/// let md5 = module_md5(&read_chunks(&beam_bytes)?)?;
/// assert_eq!(md5.len(), 16);
/// ```
///
/// # Panics
///
/// The vendored OpenSSL always has MD5, so the hash does not fail.
fn module_md5(chunks: &HashMap<Vec<u8>, &[u8]>) -> [u8; 16] {
    let chunk = |id: &[u8]| chunks.get(id).copied().unwrap_or_default();
    let mut bytes = Vec::new();
    for id in [b"AtU8", b"Code", b"StrT", b"ImpT", b"ExpT"] {
        bytes.extend_from_slice(chunk(id));
    }
    let lambdas = chunk(b"FunT");
    // A `FunT` chunk of fewer than 4 bytes, or with a partial entry, is
    // corrupt (`beam_file.c:1799-1802`), and `parse_lambdas` rejects it, so
    // its digest is never used.
    let (count, entries) = lambdas.split_at(WORD_SIZE.min(lambdas.len()));
    bytes.extend_from_slice(count);
    for entry in entries.as_chunks::<LAMBDA_ENTRY_SIZE>().0 {
        bytes.extend_from_slice(&entry[..LAMBDA_ENTRY_SIZE - WORD_SIZE]);
        bytes.extend_from_slice(&[0; WORD_SIZE]);
    }
    for id in [b"LitT", b"Meta", b"Recs", b"DbgB"] {
        bytes.extend_from_slice(chunk(id));
    }
    let digest = openssl::hash::hash(openssl::hash::MessageDigest::md5(), &bytes)
        .expect("the vendored OpenSSL has MD5");
    digest
        .as_ref()
        .try_into()
        .expect("an MD5 digest has 16 bytes")
}

/// Read supported terms from an uncompressed `LitT` chunk. Put the atoms of
/// each literal, in byte order, into the atom table, as `erts_decode_ext`
/// does.
///
/// # Examples
///
/// ```rust,ignore
/// let literals = parse_literals(&[0; 8], &AtomTable::new())?;
/// assert!(literals.is_empty());
/// ```
fn parse_literals(data: &[u8], atom_table: &AtomTable) -> Result<Vec<Option<Term>>, BeamError> {
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
        literals.push(read_literal(&data[cursor..literal_end], atom_table)?);
        cursor = literal_end;
    }
    if cursor != data.len() {
        return Err(error("unexpected bytes after BEAM literal table"));
    }
    Ok(literals)
}

/// Read one `LitT` entry. Return `None` when the term contains a tag that
/// `literal_entry` does not list, at any depth. The atoms before that tag
/// are in the atom table.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let term = read_literal(&[131, 109, 0, 0, 0, 2, b'o', b'k'], &atoms)?;
/// assert_eq!(term, Some(Term::binary(b"ok".to_vec())));
/// // [flush]
/// let bytes = [131, 108, 0, 0, 0, 1, 119, 5, b'f', b'l', b'u', b's', b'h', 106];
/// let list = read_literal(&bytes, &atoms)?;
/// assert_eq!(list, Some(Term::list_from(vec![Term::static_atom("flush")])));
/// ```
fn read_literal(data: &[u8], atom_table: &AtomTable) -> Result<Option<Term>, BeamError> {
    if data.first() != Some(&ETF_VERSION) {
        return Ok(None);
    }
    let mut cursor = 1;
    let Some(term) = read_external_term(data, &mut cursor, atom_table)? else {
        return Ok(None);
    };
    if cursor != data.len() {
        return Err(error("unexpected bytes after BEAM literal"));
    }
    Ok(Some(term))
}

/// A list, tuple, or map that `read_external_term` has started but not
/// finished.
enum PartialTerm {
    /// A tuple that still needs `remaining` elements.
    Tuple { remaining: u32, elements: Vec<Term> },
    /// A list that still needs `remaining` elements before its tail.
    List { remaining: u32, elements: Vec<Term> },
    /// A list whose elements are read and whose tail is next.
    ListTail { elements: Vec<Term> },
    /// A map that still needs `remaining` pairs. `key` holds the key of the
    /// pair whose value is next.
    Map {
        remaining: u32,
        pairs: Vec<(Term, Term)>,
        key: Option<Term>,
    },
}

/// Read one external term at the cursor and advance past it. Put each atom
/// into the atom table when it is read. Return `None` for a tag that
/// `literal_entry` does not list, or for a map key that `Map` does not
/// support yet.
/// Nested lists, tuples, and maps use a work stack, so nesting has no fixed
/// depth limit, as in the ERTS decoder.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// let mut atoms = AtomTable::new();
/// // {}
/// let term = read_external_term(&[104, 0], &mut cursor, &atoms)?;
/// assert_eq!(term, Some(Term::tuple(vec![])));
/// assert_eq!(cursor, 2);
/// ```
///
/// # Errors
///
/// A truncated term, an atom that is not UTF-8, a new atom when the atom
/// table is full, a map with a duplicate key, or an external fun with an
/// arity outside `0..=255` is an error.
fn read_external_term(
    data: &[u8],
    cursor: &mut usize,
    atom_table: &AtomTable,
) -> Result<Option<Term>, BeamError> {
    let mut partial = Vec::new();
    loop {
        let tag = take(data, cursor, 1)?[0];
        let mut term = match tag {
            ETF_BINARY_EXT => {
                let size = read_u32(take(data, cursor, WORD_SIZE)?)? as usize;
                Term::binary(take(data, cursor, size)?.to_vec())
            }
            ETF_BIT_BINARY_EXT => {
                let size = read_u32(take(data, cursor, WORD_SIZE)?)? as usize;
                let trailing = usize::from(take(data, cursor, 1)?[0]);
                // ERTS rejects a trailing bit count outside 1 to 8 and an
                // empty byte list (`external.c:4913-4917`).
                if !(1..=8).contains(&trailing) || size == 0 {
                    return Err(error("invalid BIT_BINARY_EXT trailing bit count"));
                }
                let bytes = Binary::from(take(data, cursor, size)?.to_vec());
                Term::Binary(bytes.bits(0, size * 8 - (8 - trailing)))
            }
            ETF_STRING_EXT => {
                let size_bytes = take(data, cursor, 2)?;
                let size = u16::from_be_bytes([size_bytes[0], size_bytes[1]]) as usize;
                let characters = take(data, cursor, size)?
                    .iter()
                    .map(|&byte| Term::Integer(i64::from(byte)));
                Term::list_from(characters)
            }
            ETF_NIL_EXT => Term::nil(),
            ETF_EXPORT_EXT => {
                // `dec_term` (`external.c:5006-5041`) reads two atoms and a
                // non-negative integer, else the decode fails. The arity can
                // be `INTEGER_EXT`, as `[98, 0, 0, 0, 1]` loads as arity 1.
                // A fun has at most 255 arguments. ERTS has no defined result
                // above 255: arity 256 makes a fun of arity 0, and arity 257
                // stops the VM.
                let mut part = || read_external_term(data, cursor, atom_table);
                let (Some(module), Some(function), Some(arity)) = (part()?, part()?, part()?)
                else {
                    return Ok(None);
                };
                let (Term::Atom(module), Term::Atom(function), &Term::Integer(arity)) =
                    (&module, &function, &arity)
                else {
                    return Err(error("invalid BEAM external fun literal"));
                };
                let Ok(arity) = u8::try_from(arity) else {
                    return Err(error("invalid BEAM external fun literal"));
                };
                Term::Fun(Fun::external(
                    module.clone(),
                    function.clone(),
                    u32::from(arity),
                ))
            }
            ETF_NEW_FLOAT_EXT => {
                let bytes: [u8; 8] = take(data, cursor, 8)?
                    .try_into()
                    .expect("take returns 8 bytes");
                // `dec_term` rejects a float that is not finite
                // (`external.c:4666-4668`).
                let value = Some(f64::from_be_bytes(bytes))
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| error("invalid BEAM float literal"))?;
                Term::Float(value)
            }
            ETF_SMALL_INTEGER_EXT => Term::Integer(i64::from(take(data, cursor, 1)?[0])),
            ETF_SMALL_BIG_EXT | ETF_LARGE_BIG_EXT => {
                let count = if tag == ETF_SMALL_BIG_EXT {
                    usize::from(take(data, cursor, 1)?[0])
                } else {
                    read_u32(take(data, cursor, WORD_SIZE)?)? as usize
                };
                // Any sign byte but 0 is negative, and leading zero bytes
                // are stripped (`external.c:4471-4486`).
                let negative = take(data, cursor, 1)?[0] != 0;
                let magnitude = BigInt::from_bytes_le(Sign::Plus, take(data, cursor, count)?);
                let value = if negative { -magnitude } else { magnitude };
                // `bytes_to_big` gives no term for more digits than a bignum
                // holds, and `dec_term` fails.
                if !fits_big_arity(&value) {
                    return Err(error("BEAM bignum literal exceeds BIG_ARITY_MAX digits"));
                }
                Term::from_bigint(value)
            }
            ETF_INTEGER_EXT => {
                let bytes = take(data, cursor, WORD_SIZE)?;
                Term::Integer(i64::from(i32::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3],
                ])))
            }
            ETF_SMALL_ATOM_UTF8_EXT => {
                let size = take(data, cursor, 1)?[0] as usize;
                let name = std::str::from_utf8(take(data, cursor, size)?)
                    .map_err(|_| error("invalid UTF-8 in BEAM atom literal"))?;
                Term::Atom(atom_table.put(name)?)
            }
            ETF_SMALL_TUPLE_EXT => match take(data, cursor, 1)?[0] {
                0 => Term::tuple(Vec::new()),
                remaining => {
                    let elements = Vec::new();
                    partial.push(PartialTerm::Tuple {
                        remaining: u32::from(remaining),
                        elements,
                    });
                    continue;
                }
            },
            ETF_LARGE_TUPLE_EXT => {
                let count = read_u32(take(data, cursor, WORD_SIZE)?)?;
                // ERTS rejects a tuple literal above `ERTS_MAX_TUPLE_SIZE`
                // (`external.c:6284-6289`, `erl_term.h:318-323`).
                if count as usize > MAX_TUPLE_SIZE {
                    return Err(error("BEAM tuple literal exceeds the maximum tuple size"));
                }
                match count {
                    0 => Term::tuple(Vec::new()),
                    remaining => {
                        let elements = Vec::new();
                        partial.push(PartialTerm::Tuple {
                            remaining,
                            elements,
                        });
                        continue;
                    }
                }
            }
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
            ETF_MAP_EXT => match read_u32(take(data, cursor, WORD_SIZE)?)? {
                0 => Term::Map(Map::new()),
                remaining => {
                    partial.push(PartialTerm::Map {
                        remaining,
                        pairs: Vec::new(),
                        key: None,
                    });
                    continue;
                }
            },
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
                        term = Term::tuple(elements);
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
                // A tail that is not a list gives an improper list, and a
                // LIST_EXT of length 0 gives its tail (`dec_term`, external.c:4581-4588).
                Some(PartialTerm::ListTail { elements }) => {
                    term = Term::list_with_tail(elements, term);
                    continue;
                }
                Some(PartialTerm::Map {
                    remaining,
                    pairs,
                    key: None,
                }) => partial.push(PartialTerm::Map {
                    remaining,
                    pairs,
                    key: Some(term),
                }),
                Some(PartialTerm::Map {
                    remaining,
                    mut pairs,
                    key: Some(key),
                }) => {
                    pairs.push((key, term));
                    if remaining == 1 {
                        let count = pairs.len();
                        let Ok(map) = Map::from_pairs(pairs, atom_table) else {
                            return Ok(None);
                        };
                        if map.len() < count {
                            return Err(error("duplicate key in BEAM map literal"));
                        }
                        term = Term::Map(map);
                        continue;
                    }
                    partial.push(PartialTerm::Map {
                        remaining: remaining - 1,
                        pairs,
                        key: None,
                    });
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
fn read_value(data: &[u8], cursor: &mut usize, atoms: &[Atom]) -> Result<Term, BeamError> {
    let first_byte = *data
        .get(*cursor)
        .ok_or_else(|| error("truncated BEAM operand"))?;
    match first_byte & COMPACT_TAG_MASK {
        TAG_INTEGER if first_byte == HUGE_INTEGER_FIRST_BYTE => read_huge_integer(data, cursor),
        TAG_INTEGER => {
            let payload = read_operand(data, cursor, TAG_INTEGER)?;
            Ok(Term::Integer(sign_extend(first_byte, payload)))
        }
        TAG_ATOM => {
            let index = read_operand(data, cursor, TAG_ATOM)? as usize;
            if index == 0 {
                return Ok(Term::nil());
            }
            Ok(Term::Atom(atom_at(atoms, index)?.clone()))
        }
        unsupported_tag => Err(error(format!(
            "unsupported constant operand tag {unsupported_tag}"
        ))),
    }
}

/// Read an integer operand with 9 or more payload bytes: the first byte
/// [`HUGE_INTEGER_FIRST_BYTE`], the byte count minus 9 as an unsigned
/// operand, and the bytes, big-endian in two's complement. ERTS loads the
/// value as a bignum literal (`marshal_integer`, `beam_file.c:2134-2215`):
/// `{integer,18446744073709551616}` is
/// `[0xF9, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0]`. A value whose magnitude
/// keeps two leading zero bytes is not normalized, and ERTS rejects the
/// file (`beam_file.c:2186-2202`). ERTS also rejects a value with more than
/// `BIG_ARITY_MAX` digits.
///
/// # Examples
///
/// ```rust,ignore
/// let mut cursor = 0;
/// let term = read_huge_integer(&[0xF9, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0], &mut cursor)?;
/// assert_eq!(term, Term::from_bigint(BigInt::from(1) << 64));
/// assert_eq!(cursor, 11);
/// ```
fn read_huge_integer(data: &[u8], cursor: &mut usize) -> Result<Term, BeamError> {
    *cursor += 1;
    let count = usize::try_from(read_operand(data, cursor, TAG_UNSIGNED)?)
        .ok()
        .and_then(|count| count.checked_add(9))
        .ok_or_else(|| error("BEAM integer operand size overflow"))?;
    let end = cursor
        .checked_add(count)
        .filter(|&end| end <= data.len())
        .ok_or_else(|| error("truncated BEAM integer operand"))?;
    let value = BigInt::from_signed_bytes_be(&data[*cursor..end]);
    *cursor = end;
    if value.magnitude().bits() <= (count as u64 - 2) * 8 {
        return Err(error("BEAM integer operand is not normalized"));
    }
    // `bytes_to_big` gives no term for more digits than a bignum holds, and
    // `marshal_integer` fails (`beam_file.c:2210-2211`).
    if !fits_big_arity(&value) {
        return Err(error("BEAM integer operand exceeds BIG_ARITY_MAX digits"));
    }
    Ok(Term::from_bigint(value))
}

/// Return the signed value of a `TAG_INTEGER` payload, as ERTS
/// `beamreader_read_tagged` does. The short and two-byte forms hold a value
/// in `0..2048`. The extended form holds 2 to 8 big-endian bytes in two's
/// complement, so ERTS sign-extends them from the highest payload bit.
///
/// # Examples
///
/// ```rust,ignore
/// // {integer,-1} is [0x19, 0xFF, 0xFF]: two payload bytes.
/// assert_eq!(sign_extend(0x19, 0xFFFF), -1);
/// // {integer,65535} is [0x39, 0x00, 0xFF, 0xFF]: three payload bytes.
/// assert_eq!(sign_extend(0x39, 0xFFFF), 65535);
/// // {integer,2047} is [0xE9, 0xFF]: the two-byte form has no sign.
/// assert_eq!(sign_extend(0xE9, 0x7FF), 2047);
/// ```
fn sign_extend(first_byte: u8, payload: u64) -> i64 {
    let extended = COMPACT_EXTENDED_FLAG | COMPACT_SIGN_FLAG;
    if first_byte & extended != extended {
        return payload as i64;
    }
    // A payload of 9 or more bytes fails in `read_integer_after_first`, so the
    // size code here is 0..=6 and the payload has 2..=8 bytes.
    let payload_bits = u32::from(first_byte >> 5) * 8 + 16;
    let shift = u64::BITS - payload_bits;
    ((payload << shift) as i64) >> shift
}

/// Return the atom at a one-based BEAM atom index.
///
/// # Examples
///
/// ```rust,ignore
/// let mut table = AtomTable::new();
/// let atoms = vec![table.put("hello")?];
/// assert_eq!(table.name_of(atom_at(&atoms, 1)?), "hello");
/// ```
fn atom_at(atoms: &[Atom], index: usize) -> Result<&Atom, BeamError> {
    if index < FIRST_ATOM_INDEX {
        return Err(error("BEAM atom index zero is invalid"));
    }
    atoms
        .get(index - FIRST_ATOM_INDEX)
        .ok_or_else(|| error(format!("BEAM atom index {index} out of range")))
}

/// Return a required chunk payload by its four-byte ID.
///
/// # Examples
///
/// ```rust,ignore
/// let payload = [0x03];
/// let chunks = HashMap::from([(b"Code".to_vec(), &payload[..])]);
/// assert_eq!(required_chunk(chunks, b"Code")?, &payload);
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
#[path = "../../tests/beam/code.rs"]
mod tests;
