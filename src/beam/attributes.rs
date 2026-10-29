//! The persisted attributes of a BEAM module.
//!
//! A module attribute of Elixir that a compiler keeps stays in the `Attr`
//! chunk of the BEAM file. The declaration `use PBeam.Native` writes the
//! attribute `pbeam_native`: the list of the `{name, arity}` pairs that the
//! module declares as native. PBeam reads that list during the load of a
//! module, so that it can check the declarations against the registered
//! native functions before a run uses the module.
//!
//! The payload of the `Attr` chunk is one term of the external term format
//! (`term_to_binary/1`, `external.c:4270-4300`): a list of `{name, value}`
//! pairs. The ERTS loader decodes that one term at the load of a module and
//! refuses a module whose encoding is bad (`asm_load.c:1080-1094`), so PBeam
//! reads the chunk during a load too. Each other attribute has a value of
//! any shape, so [`native_declarations`] skips it with
//! [`Cursor::skip_term`], which accepts each tag and each count that the
//! ERTS walk `decoded_size` accepts (`external.c:6041-6540`), except the tag
//! `LOCAL_EXT` (`docs/erts-differences.md`).

use crate::error::{BeamError, error};
use crate::term::{MAX_ATOM_CHARACTERS, MAX_ATOM_SZ_LIMIT, MAX_BIG_DIGITS, MAX_TUPLE_SIZE};

/// The first byte of an external term: `VERSION_MAGIC` (`external.c:60`).
const VERSION_MAGIC: u8 = 131;

/// The module attribute that `use PBeam.Native` writes.
const DECLARATIONS_ATTRIBUTE: &str = "pbeam_native";

/// The largest count of numbers in a reference (`ERTS_MAX_REF_NUMBERS`,
/// `erl_term.h:780`).
const MAX_REF_NUMBERS: u64 = 5;

/// The largest count of arguments of a fun (`MAX_ARG`, `erl_vm.h:54`).
const MAX_ARG: u64 = 255;

/// The largest count of pairs in a map on a 64-bit host (`external.c:6311-6320`).
const MAX_MAP_PAIRS: u64 = (1 << 31) - 1;

/// The native functions that the `Attr` chunk of a module declares.
///
/// Return each `{name, arity}` pair of the attribute `pbeam_native`, in the
/// order of the chunk. A chunk with no such attribute gives an empty list.
///
/// # Examples
///
/// ```rust,ignore
/// let pairs = native_declarations(attr)?;
/// assert_eq!(pairs, [("sha256".to_owned(), 1)]);
/// ```
///
/// # Errors
///
/// The payload must hold one term of the external term format with a valid
/// encoding. Another payload gives an error, as the ERTS loader refuses the
/// module with `badfile` (`asm_load.c:1086-1091`). The tag `LOCAL_EXT` is
/// the one difference between the two accept sets: PBeam refuses it, and
/// ERTS accepts it only with a matching block hash
/// (`docs/erts-differences.md`). A payload of no bytes is not read at all,
/// as in ERTS (`asm_load.c:1080`).
pub(crate) fn native_declarations(attr: &[u8]) -> Result<Vec<(String, u32)>, BeamError> {
    if attr.is_empty() {
        return Ok(Vec::new());
    }
    let mut cursor = Cursor::new(attr);
    if cursor.byte()? != VERSION_MAGIC {
        return Err(error("the BEAM attribute chunk is not an external term"));
    }
    // ERTS asks nothing else of the term than a valid encoding: a term of
    // another shape loads, and bytes after the term are ignored. PBeam
    // follows that, and reads no declaration from a term that is not a list
    // of pairs.
    let Some(mut count) = cursor.list_count()? else {
        cursor.skip_term()?;
        return Ok(Vec::new());
    };
    let mut declarations = Vec::new();
    loop {
        for _ in 0..count {
            if let Some((name, value)) = cursor.pair()?
                && name == DECLARATIONS_ATTRIBUTE
            {
                declarations = Cursor::new(value).declarations()?;
            }
        }
        // ERTS decodes a list whose encoding continues through another
        // `LIST_EXT` as one list (`external.c:4581-4614`), so read each
        // segment up to the `NIL_EXT` or the improper tail.
        if !cursor.list_tail()? {
            break;
        }
        count = cursor.number(4)?;
    }
    Ok(declarations)
}

/// Check that the `CInf` chunk `compile_info` holds one term of the
/// external term format with a valid encoding, as the ERTS loader refuses
/// another chunk with `badfile` (`jit/asm_load.c:1096-1111`). A chunk of no
/// bytes is not read at all, as in ERTS (`jit/asm_load.c:1096`). Bytes after
/// the term are ignored, as for the `Attr` chunk.
///
/// # Examples
///
/// ```rust,ignore
/// check_compile_info(&[131, 106])?;
/// assert!(check_compile_info(&[0]).is_err());
/// ```
///
/// # Errors
///
/// A payload that is not one valid external term gives an error. The tag
/// `LOCAL_EXT` gives an error, as for [`native_declarations`].
pub(crate) fn check_compile_info(compile_info: &[u8]) -> Result<(), BeamError> {
    if compile_info.is_empty() {
        return Ok(());
    }
    let mut cursor = Cursor::new(compile_info);
    if cursor.byte()? != VERSION_MAGIC {
        return Err(error(
            "the BEAM compile information chunk is not an external term",
        ));
    }
    cursor.skip_term()
}

/// The read position of one external term.
struct Cursor<'a> {
    /// The input bytes.
    bytes: &'a [u8],
    /// The position of the next byte.
    pos: usize,
    /// The count of identity bytes of a pid, a port, or a reference, which
    /// the tag of the atom of its node consumes (`atom_extra_skip`,
    /// `external.c:6194-6248`). The count stays until that atom, also across
    /// the elements of a container, as in the ERTS walk.
    extra: u64,
}

impl<'a> Cursor<'a> {
    /// Return a cursor at the start of `bytes`.
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            extra: 0,
        }
    }

    /// Take `count` bytes and return them.
    fn take(&mut self, count: u64) -> Result<&'a [u8], BeamError> {
        let count = usize::try_from(count).map_err(|_| error("truncated external term"))?;
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| error("truncated external term"))?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| error("truncated external term"))?;
        self.pos = end;
        Ok(bytes)
    }

    /// Take one byte.
    fn byte(&mut self) -> Result<u8, BeamError> {
        Ok(self.take(1)?[0])
    }

    /// Return the next byte without consuming it.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Take a big-endian unsigned number of `count` bytes.
    fn number(&mut self, count: usize) -> Result<u64, BeamError> {
        let mut value = 0u64;
        for byte in self.take(count as u64)? {
            value = (value << 8) | u64::from(*byte);
        }
        Ok(value)
    }

    /// Read one atom tag and return its name.
    ///
    /// The tags `ATOM_EXT` and `SMALL_ATOM_EXT` hold a name of Latin-1
    /// bytes, and `ATOM_UTF8_EXT` and `SMALL_ATOM_UTF8_EXT` hold a name of
    /// UTF-8 bytes (`dec_atom`, `external.c:4485-4560`). A name above the
    /// ERTS limit is an error, as in the walk `decoded_size`
    /// (`external.c:6152-6189`).
    ///
    /// The name of the node of a pid, a port, or a reference holds the
    /// identity bytes of that term, which the walk counts in
    /// [`Cursor::extra`] and this read consumes with the tag of the atom
    /// (`dec_atom`, `external.c:4485-4516`).
    fn atom(&mut self) -> Result<String, BeamError> {
        let (length, latin1, limit) = match self.byte()? {
            100 => (self.number(2)?, true, MAX_ATOM_CHARACTERS as u64),
            118 => (self.number(2)?, false, MAX_ATOM_SZ_LIMIT as u64),
            115 => (u64::from(self.byte()?), true, MAX_ATOM_CHARACTERS as u64),
            119 => (u64::from(self.byte()?), false, MAX_ATOM_SZ_LIMIT as u64),
            tag => {
                return Err(error(format!("the external term tag {tag} is not an atom")));
            }
        };
        if length > limit {
            return Err(error("an external term atom name is too long"));
        }
        let bytes = self.take(length + self.extra)?;
        self.extra = 0;
        let name = &bytes[..usize::try_from(length).expect("an atom length that fits")];
        if latin1 {
            Ok(name.iter().map(|byte| char::from(*byte)).collect())
        } else {
            String::from_utf8(name.to_vec())
                .map_err(|_| error("an external term atom is not UTF-8"))
        }
    }

    /// Read a list tag and return the count of its elements.
    ///
    /// `NIL_EXT` is the empty list, and any other tag is not a list
    /// (`dec_term`, `external.c:4270-4300`). A tag that is not `LIST_EXT`
    /// gives `None`, and the cursor then stands before the term.
    fn list_count(&mut self) -> Result<Option<u64>, BeamError> {
        if self.peek() != Some(108) {
            return Ok(None);
        }
        self.pos += 1;
        Ok(Some(self.number(4)?))
    }

    /// Advance over the tail of one list segment.
    ///
    /// Return `true` when another `LIST_EXT` segment follows, whose count is
    /// next. Return `false` when the list ends with `NIL_EXT`, or when the
    /// tail is another term, which this method skips. ERTS decodes a list
    /// whose encoding continues through another `LIST_EXT` as one list
    /// (`external.c:4581-4614`).
    fn list_tail(&mut self) -> Result<bool, BeamError> {
        match self.peek() {
            Some(108) => {
                self.pos += 1;
                Ok(true)
            }
            Some(106) => {
                // `NIL_EXT` names the node of a local pid, a port, or a
                // reference only inside a local encoding
                // (`external.c:6249-6262`).
                if self.extra != 0 {
                    return Err(error(
                        "the node of a pid, a port, or a reference is not an atom",
                    ));
                }
                self.pos += 1;
                Ok(false)
            }
            Some(_) => {
                self.skip_term()?;
                Ok(false)
            }
            None => Err(error("truncated external term")),
        }
    }

    /// Read the arity of a tuple tag and return it.
    fn tuple_length(&mut self) -> Result<u64, BeamError> {
        match self.byte()? {
            104 => Ok(u64::from(self.byte()?)),
            105 => self.number(4),
            tag => Err(error(format!("the external term tag {tag} is not a tuple"))),
        }
    }

    /// Read one `{name, value}` pair and return the name and the value
    /// bytes.
    ///
    /// An element of another shape gives `None`, and the cursor then stands
    /// after the element. ERTS loads a module with such an element, so it is
    /// not an error.
    fn pair(&mut self) -> Result<Option<(String, &'a [u8])>, BeamError> {
        let start = self.pos;
        let extra = self.extra;
        if let Some(pair) = self.read_pair() {
            return Ok(Some(pair));
        }
        self.pos = start;
        self.extra = extra;
        self.skip_term()?;
        Ok(None)
    }

    /// Read one `{name, value}` pair when the element has that shape, and
    /// the bytes of the value.
    fn read_pair(&mut self) -> Option<(String, &'a [u8])> {
        if !matches!(self.tuple_length(), Ok(2)) {
            return None;
        }
        let name = self.atom().ok()?;
        let value = self.pos;
        self.skip_term().ok()?;
        Some((name, &self.bytes[value..self.pos]))
    }

    /// Read the value of `pbeam_native`: a list of `{name, arity}` pairs.
    fn declarations(&mut self) -> Result<Vec<(String, u32)>, BeamError> {
        // A value of another shape declares no native function. ERTS loads
        // such a module, so it is not an error.
        let Some(mut count) = self.list_count()? else {
            return Ok(Vec::new());
        };
        let mut declarations = Vec::new();
        loop {
            // The caller has walked the value, so the count fits the bytes of
            // the elements and the tail below.
            for _ in 0..count {
                let start = self.pos;
                let extra = self.extra;
                if let Some(declaration) = self.declaration() {
                    declarations.push(declaration);
                } else {
                    self.pos = start;
                    self.extra = extra;
                    self.skip_term()?;
                }
            }
            // The declaration list can also continue through more than one
            // `LIST_EXT` segment (`external.c:4581-4614`).
            if !self.list_tail()? {
                break;
            }
            count = self.number(4)?;
        }
        Ok(declarations)
    }

    /// Read one `{name, arity}` declaration, or `None` when the element has
    /// another shape.
    fn declaration(&mut self) -> Option<(String, u32)> {
        if !matches!(self.tuple_length(), Ok(2)) {
            return None;
        }
        let name = self.atom().ok()?;
        let arity = self.arity().ok()?;
        Some((name, arity))
    }

    /// Read one non-negative integer of an arity.
    fn arity(&mut self) -> Result<u32, BeamError> {
        let arity = match self.byte()? {
            97 => u32::from(self.byte()?),
            98 => {
                let value = i32::from_be_bytes(self.take(4)?.try_into().expect("four bytes"));
                u32::try_from(value)
                    .map_err(|_| error("a pbeam_native arity is not a non-negative integer"))?
            }
            tag => {
                return Err(error(format!(
                    "the external term tag {tag} is not a pbeam_native arity"
                )));
            }
        };
        Ok(arity)
    }

    /// Skip one complete term of any accepted tag, as the ERTS walk
    /// `decoded_size` walks it (`external.c:6041-6540`).
    ///
    /// The walk refuses each tag and each count that `decoded_size` refuses,
    /// so a module loads in PBeam when the ERTS loader accepts its attribute
    /// chunk (`asm_load.c:1080-1094`). The tag `LOCAL_EXT` is the one
    /// exception: ERTS accepts it only with a matching block hash, and PBeam
    /// refuses it (`docs/erts-differences.md`).
    ///
    /// `remaining` counts the terms that the walk still expects in the
    /// current container, and `stack` holds the count of each enclosing
    /// container, so a term of any depth needs no recursion. The identity
    /// bytes of a pid, a port, or a reference stay in [`Cursor::extra`] until
    /// the tag of the atom of its node consumes them.
    fn skip_term(&mut self) -> Result<(), BeamError> {
        let mut remaining: u64 = 1;
        let mut stack: Vec<u64> = Vec::new();
        loop {
            while remaining == 0 {
                match stack.pop() {
                    Some(outer) => remaining = outer,
                    None => return Ok(()),
                }
            }
            remaining -= 1;
            let tag = self.byte()?;
            match tag {
                // A small integer, an integer, a float, or a new float.
                97 => {
                    self.take(1)?;
                }
                98 => {
                    self.take(4)?;
                }
                99 => {
                    self.take(31)?;
                }
                70 => {
                    self.take(8)?;
                }
                // An atom of four tag and length encodings. The name of the
                // node of a pid, a port, or a reference carries the bytes of
                // the identity (`external.c:6152-6189`).
                100 | 118 => {
                    let limit = if tag == 100 {
                        MAX_ATOM_CHARACTERS as u64
                    } else {
                        MAX_ATOM_SZ_LIMIT as u64
                    };
                    let length = self.number(2)?;
                    if length > limit {
                        return Err(error("an external term atom name is too long"));
                    }
                    let extra = self.extra;
                    self.take(length + extra)?;
                    self.extra = 0;
                }
                // A small atom holds a one-byte length, which is below both
                // limits, so the walk does not compare it
                // (`external.c:6178-6189`).
                115 | 119 => {
                    let length = u64::from(self.byte()?);
                    let extra = self.extra;
                    self.take(length + extra)?;
                    self.extra = 0;
                }
                // The atom references of a distribution or internal encoding
                // carry the same bytes (`external.c:6190-6193`,
                // `external.c:6405-6412`).
                82 => {
                    let extra = self.extra;
                    self.take(1 + extra)?;
                    self.extra = 0;
                }
                73 => {
                    let extra = self.extra;
                    self.take(2 + extra)?;
                    self.extra = 0;
                }
                75 => {
                    let extra = self.extra;
                    self.take(3 + extra)?;
                    self.extra = 0;
                }
                // The empty list, a string, and a binary.
                106 => {
                    // `NIL_EXT` names the node of a local pid, a port, or a
                    // reference only inside a local encoding
                    // (`external.c:6249-6262`).
                    if self.extra != 0 {
                        return Err(error(
                            "the node of a pid, a port, or a reference is not an atom",
                        ));
                    }
                }
                107 => {
                    let length = self.number(2)?;
                    self.take(length)?;
                }
                109 => {
                    let length = self.number(4)?;
                    self.take(length)?;
                }
                77 => {
                    let length = self.number(4)?;
                    self.take(1 + length)?;
                }
                // A big number of one count byte or four count bytes, then one
                // sign byte. `decoded_size` bounds the digits of a large big
                // number by `BIG_ARITY_MAX` digits of an `ErtsDigit`
                // (`external.c:6143-6151`).
                110 => {
                    let length = u64::from(self.byte()?);
                    self.take(1 + length)?;
                }
                111 => {
                    let digits = self.number(4)?;
                    if digits > MAX_BIG_DIGITS * 8 {
                        return Err(error("an external term big number is too long"));
                    }
                    self.take(1 + digits)?;
                }
                // A pid, a port, or a reference: the walk reads the node as
                // one more term and adds the identity bytes to its atom tag
                // (`external.c:6194-6248`).
                88 => {
                    self.extra = 12;
                    remaining += 1;
                }
                103 => {
                    self.extra = 9;
                    remaining += 1;
                }
                120 => {
                    self.extra = 12;
                    remaining += 1;
                }
                89 => {
                    self.extra = 8;
                    remaining += 1;
                }
                102 => {
                    self.extra = 5;
                    remaining += 1;
                }
                101 => {
                    self.extra = 5;
                    remaining += 1;
                }
                90 | 114 => {
                    let count = self.number(2)?;
                    if count > MAX_REF_NUMBERS {
                        return Err(error("an external term reference has too many numbers"));
                    }
                    // `NEWER_REFERENCE_EXT` holds one creation byte and
                    // `NEW_REFERENCE_EXT` four, then four bytes for each
                    // number.
                    self.extra = if tag == 90 {
                        4 + 4 * count
                    } else {
                        1 + 4 * count
                    };
                    remaining += 1;
                }
                // The containers: a list of one count plus its tail, a tuple,
                // a map, an export, a new fun, and a record.
                108 => {
                    let count = self.number(4)?;
                    stack.push(remaining);
                    remaining = count + 1;
                }
                104 => {
                    let count = u64::from(self.byte()?);
                    stack.push(remaining);
                    remaining = count;
                }
                105 => {
                    let count = self.number(4)?;
                    if count > MAX_TUPLE_SIZE as u64 {
                        return Err(error("an external term tuple is too large"));
                    }
                    stack.push(remaining);
                    remaining = count;
                }
                116 => {
                    let count = self.number(4)?;
                    if count > MAX_MAP_PAIRS {
                        return Err(error("an external term map is too large"));
                    }
                    stack.push(remaining);
                    remaining = count * 2;
                }
                113 => {
                    stack.push(remaining);
                    remaining = 3;
                }
                112 => {
                    // The size, the arity, the unique value, and the index,
                    // then the count of free variables, the module, the old
                    // index, the old unique value, the creator pid, and the
                    // free variables (`external.c:6379-6398`).
                    self.take(4 + 1 + 16 + 4)?;
                    let free = self.number(4)?;
                    if free > MAX_ARG {
                        return Err(error("an external term fun has too many free variables"));
                    }
                    stack.push(remaining);
                    remaining = 4 + free;
                }
                67 => {
                    // The field count, the flags, the module and the name,
                    // the field names, and the values (`external.c:6447-6463`).
                    let count = self.number(4)?;
                    self.take(1)?;
                    stack.push(remaining);
                    remaining = 2 + 2 * count;
                }
                // `FUN_EXT` is the fun encoding that OTP 23 removed
                // (`external.c:6399-6404`), and `LOCAL_EXT` is a term that
                // the walk accepts only with a matching block hash
                // (`external.c:6437-6446`).
                _ => {
                    return Err(error(format!(
                        "the external term tag {tag} is not supported"
                    )));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/beam/attributes.rs"]
mod tests;
