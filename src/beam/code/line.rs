//! The line table of a module: the `Line` chunk and the `line` operands of
//! the code, as ERTS reads them (`parse_line_chunk`, `beam_file.c:443-600`;
//! `add_line_entry`, `jit/asm_load.c:318-355`).
//!
//! The chunk holds five big-endian 32-bit fields, `version` (0), `flags`,
//! the count of line instructions, the count of items, and the count of file
//! names. Then come the items as compact terms: an atom tag `a` sets the
//! file index of the next items, and an integer tag `i` is one item with
//! that file and a line number. The file names follow, each a big-endian
//! 16-bit length and UTF-8 bytes. Item 0 is the implicit undefined location,
//! and file 0 is the implicit `Module.erl`. For example, the chunk bytes
//! `0 0 0 0, 0 0 0 0, 0 0 0 1, 0 0 0 1, 0 0 0 0, 0x51` give one line
//! instruction and the item 1 at line 5 of `Module.erl`.

use super::{
    COMPACT_EXTENDED_FLAG, COMPACT_SIGN_FLAG, COMPACT_TAG_MASK, TAG_ATOM, TAG_INTEGER, WORD_SIZE,
    read_integer_after_first, read_u32,
};
use crate::{
    error::{BeamError, error},
    term::Term,
};

/// The version of the `Line` chunk that ERTS reads. ERTS loads a module
/// with another version with no line table (`beam_file.c:459-463`).
const LINE_CHUNK_VERSION: u32 = 0;
/// The size of the five header fields of the `Line` chunk.
const LINE_HEADER_SIZE: usize = WORD_SIZE * 5;
/// The size of the length field of a file name.
const NAME_LENGTH_SIZE: usize = 2;
/// The file indexes and line numbers that `MAKE_LOCATION` accepts are below
/// these limits (`IS_VALID_LOCATION`, `beam_code.h:33-34`).
const FILE_LIMIT: usize = 255;
const LINE_LIMIT: u64 = (1 << 24) - 1;
/// The largest small integer of a 64-bit ERTS (`MAX_SMALL`). A larger line
/// does not fit a word, which `parse_line_chunk` rejects
/// (`beam_file.c:217-220`, `522`).
const MAX_SMALL: u64 = (1 << 59) - 1;

/// The line table of one module. The loader builds it once, and a stack
/// trace reads the location of an instruction from it.
///
/// Each entry is keyed by the index of its first instruction in the
/// instruction stream of the module, and the entries are in instruction
/// order. A pass that changes the instruction indexes, such as a later
/// instruction fusion, must remap the entries with the same index map as
/// the labels.
#[derive(Debug, Default)]
pub(crate) struct LineTable {
    /// The file name of each file index, as an Erlang string.
    files: Vec<Term>,
    /// The file index and the line of each item. Item 0 and each item that
    /// `MAKE_LOCATION` rejects (`beam_code.h:31-39`) are `None`.
    items: Vec<Option<(usize, u32)>>,
    /// The count of line entries that the chunk declares.
    declared_entries: usize,
    /// The item of the last entry that ERTS records, to drop a duplicate
    /// `line` as `add_line_entry` does.
    last_item: Option<usize>,
    /// The count of entries that ERTS records (`current_li`).
    recorded: usize,
    /// The first instruction index and the item of each entry.
    entries: Vec<(usize, usize)>,
}

impl LineTable {
    /// Read a `Line` chunk of the module `module`, as `parse_line_chunk`
    /// does (`beam_file.c:443-600`). A chunk of another version gives an
    /// empty line table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let chunk = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0x51];
    /// let table = LineTable::parse(&chunk, "probe")?;
    /// ```
    ///
    /// # Errors
    ///
    /// A truncated chunk, an item that is not an atom or an integer, a file
    /// index after the last file name, a line that does not fit a word, or a
    /// file name that is not UTF-8 is an error.
    pub(super) fn parse(data: &[u8], module: &str) -> Result<Self, BeamError> {
        let corrupt = || error("corrupt BEAM line table");
        let field = |index: usize| {
            data.get(index * WORD_SIZE..(index + 1) * WORD_SIZE)
                .ok_or_else(corrupt)
                .and_then(read_u32)
        };
        if field(0)? != LINE_CHUNK_VERSION {
            return Ok(Self::default());
        }
        let declared_entries = field(2)? as usize;
        let item_count = field(3)? as usize + 1;
        let name_count = field(4)? as usize + 1;
        let mut cursor = LINE_HEADER_SIZE;
        let mut items = Vec::with_capacity(item_count.min(data.len()));
        items.push(None);
        let mut file = 0;
        while items.len() < item_count {
            let first = *data.get(cursor).ok_or_else(corrupt)?;
            cursor += 1;
            let value = read_integer_after_first(data, &mut cursor, first)?;
            // ERTS sign-extends the bytes of a long integer, so a set top
            // bit is a negative line (`beam_file.c:206-220`).
            let long = COMPACT_EXTENDED_FLAG | COMPACT_SIGN_FLAG;
            let negative =
                first & long == long && data[cursor - (usize::from(first >> 5) + 2)] & 0x80 != 0;
            match first & COMPACT_TAG_MASK {
                TAG_ATOM if (value as usize) < name_count => file = value as usize,
                TAG_INTEGER if !negative && value <= MAX_SMALL => items.push(
                    (file < FILE_LIMIT && value < LINE_LIMIT && (file, value) != (0, 0))
                        .then_some((file, value as u32)),
                ),
                _ => return Err(corrupt()),
            }
        }
        let mut files = vec![charlist(&format!("{module}.erl"))];
        while files.len() < name_count {
            let length = data
                .get(cursor..cursor + NAME_LENGTH_SIZE)
                .ok_or_else(corrupt)?;
            let length = i16::from_be_bytes([length[0], length[1]]);
            cursor += NAME_LENGTH_SIZE;
            let name = usize::try_from(length)
                .ok()
                .and_then(|length| data.get(cursor..cursor + length))
                .ok_or_else(corrupt)?;
            cursor += name.len();
            let name = std::str::from_utf8(name).map_err(|_| corrupt())?;
            // ERTS gives an empty file name the name of file 0
            // (`beam_file.c:592-597`).
            files.push(if name.is_empty() {
                files[0].clone()
            } else {
                charlist(name)
            });
        }
        Ok(LineTable {
            files,
            items,
            declared_entries,
            ..Self::default()
        })
    }

    /// Record the `line` operand `item` before the instruction at index
    /// `instruction`. ERTS drops a `line` with the item of the entry before
    /// it (`jit/asm_load.c:678-684`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.line(12, 3)?;
    /// ```
    ///
    /// # Errors
    ///
    /// See [`LineTable::add`].
    pub(super) fn line(&mut self, instruction: usize, item: u64) -> Result<(), BeamError> {
        self.add(instruction, item, false)
    }

    /// Start the entries of the function whose `func_info` is at index
    /// `instruction`, with the item of the `line` directly before it, or
    /// with no location. ERTS keeps the `line` before `func_info` as the
    /// `func_line` of the function, which it records also when it is a
    /// duplicate, and an `empty_func_line` records no entry
    /// (`jit/x86/ops.tab:85-89`, `831-860`; `jit/asm_load.c:642-677`). The
    /// entry of no location ends the range of the entry before it, as the
    /// range of each function ends its line lookup (`lookup_loc`,
    /// `beam_ranges.c:330-368`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.function(10, Some(2))?;
    /// ```
    ///
    /// # Errors
    ///
    /// See [`LineTable::add`].
    pub(super) fn function(
        &mut self,
        instruction: usize,
        item: Option<u64>,
    ) -> Result<(), BeamError> {
        match item {
            Some(item) => self.add(instruction, item, true),
            None if self.items.is_empty() => Ok(()),
            None => {
                self.entries.push((instruction, 0));
                Ok(())
            }
        }
    }

    /// Record an entry as `add_line_entry` does (`jit/asm_load.c:318-355`).
    /// A module with no line table records no entry.
    ///
    /// # Errors
    ///
    /// An item after the last item of the chunk, or more entries than the
    /// chunk declares, is an error.
    fn add(&mut self, instruction: usize, item: u64, duplicates: bool) -> Result<(), BeamError> {
        if self.items.is_empty() {
            return Ok(());
        }
        let item = usize::try_from(item)
            .ok()
            .filter(|&item| item < self.items.len())
            .ok_or_else(|| error(format!("line instruction index overflow ({item})")))?;
        if self.recorded >= self.declared_entries {
            return Err(error(format!(
                "line instruction table overflow ({}/{})",
                self.recorded, self.declared_entries
            )));
        }
        if duplicates || self.last_item != Some(item) {
            self.entries.push((instruction, item));
            self.last_item = Some(item);
            self.recorded += 1;
        }
        Ok(())
    }

    /// Return `[{file, File}, {line, Line} | Rest]` for the instruction at
    /// index `instruction`, from the last entry at or before it, or `Rest`
    /// when it has no location, as `erts_build_mfa_item` builds the
    /// location of a frame (`utils.c:3430-3446`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // [{file, "probe.erl"}, {line, 5}]
    /// let location = module.lines.location(12, Term::nil());
    /// ```
    pub(crate) fn location(&self, instruction: usize, rest: Term) -> Term {
        let end = self
            .entries
            .partition_point(|&(start, _)| start <= instruction);
        let Some((file, line)) = end
            .checked_sub(1)
            .and_then(|last| self.items[self.entries[last].1])
        else {
            return rest;
        };
        Term::list_with_tail(
            vec![
                Term::tuple(vec![Term::static_atom("file"), self.files[file].clone()]),
                Term::tuple(vec![Term::static_atom("line"), Term::Integer(line.into())]),
            ],
            rest,
        )
    }
}

/// Return the Erlang string of the code points of `text`.
fn charlist(text: &str) -> Term {
    Term::list_from(
        text.chars()
            .map(|code| Term::Integer(u32::from(code).into())),
    )
}

#[cfg(test)]
#[path = "../../../tests/beam/code/line.rs"]
mod tests;
