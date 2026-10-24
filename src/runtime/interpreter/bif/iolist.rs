//! The iodata BIFs `erlang:iolist_size/1`, `erlang:iolist_to_iovec/1`,
//! `erlang:iolist_to_binary/1`, and `erlang:list_to_binary/1`, and the
//! iodata walk of ERTS `generic_iolist_size` and `generic_iolist_copy`
//! (`erl_iolist.c:113-649`) that they share.

use super::BifResult::Value;
use super::*;
use crate::runtime::interpreter::bits::BitWriter;
use crate::runtime::interpreter::bitstring;

/// The largest binary, in bytes, that `erlang:iolist_to_iovec/1` copies into
/// the merged binary of its result. ERTS `IOL2V_COPY_THRESHOLD`
/// (`erl_iolist.c:37`) is `ERL_ONHEAP_BITS_LIMIT * 8` bits, and
/// `ERL_ONHEAP_BITS_LIMIT` (`erl_bits.h:167-168`) is `64 * 8` bits, so the
/// limit is 4096 bits, or 512 bytes. A larger binary is an element of the
/// result.
const IOVEC_COPY_LIMIT: usize = 512;

/// The steps of the walk for each reduction of the budget
/// (`ERTS_IOLIST_SIZE_YIELDS_COUNT_PER_RED`, `erl_iolist.c:76`).
const STEPS_PER_RED: i64 = 8;

/// The bytes of a copy for each step of the walk
/// (`ERTS_IOLIST_COPY_BITS_PER_YIELD_COUNT`, `erl_iolist.c:305`, in bits).
const COPY_BYTES_PER_STEP: i64 = 32;

/// The bits of a bitstring copy for each step of the walk
/// (`ERTS_IOLIST_COPY_BITS_PER_YIELD_COUNT`, `erl_iolist.c:305`).
const COPY_BITS_PER_STEP: i64 = COPY_BYTES_PER_STEP * 8;

/// The smallest budget with which [`list_to_bitstring`] walks a list:
/// `IOL2B_MIN_EXEC_REDS` (`erl_iolist.c:34`), `CONTEXT_REDS / 4`. A caller
/// with fewer reductions walks with this budget.
const L2B_MIN_REDS: i64 = 1000;

/// The budget at or below which [`list_to_bitstring`] switches out the
/// caller before it reads its argument: `IOL2B_RESCHED_REDS`
/// (`erl_iolist.c:35`), `CONTEXT_REDS / 40`.
const L2B_RESCHED_REDS: i64 = 100;

/// The unit of the bitstrings that are iodata, as the `Unit` of
/// `generic_iolist_size` (`erl_iolist.c:182-184`, `217-219`): bytes for the
/// binary BIFs, and bits for `erlang:list_to_bitstring/1`. ERTS has only
/// these two units (`erl_iolist.c:864`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Unit {
    /// The unit 1: a bitstring of each size.
    Bit,
    /// The unit 8: a bitstring of a whole number of bytes.
    Byte,
}

impl Unit {
    /// Return the unit of `bits` bits, or `None` for a number that is not 1
    /// or 8.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Unit::from_bits(8), Some(Unit::Byte));
    /// assert_eq!(Unit::from_bits(2), None);
    /// ```
    pub(super) fn from_bits(bits: i64) -> Option<Unit> {
        match bits {
            1 => Some(Unit::Bit),
            8 => Some(Unit::Byte),
            _ => None,
        }
    }

    /// Return the number of bits of this unit.
    fn bits(self) -> i64 {
        match self {
            Unit::Bit => 1,
            Unit::Byte => 8,
        }
    }

    /// Return whether the size of `binary` is a multiple of this unit.
    fn holds(self, binary: &Binary) -> bool {
        self == Unit::Bit || binary.bit_size().is_multiple_of(8)
    }
}

/// How a walk of [`Walk::run`] ends.
#[derive(Debug, PartialEq)]
enum Step {
    /// The walk read all iodata.
    Done,
    /// The steps of the budget ended. The walk continues from its state in
    /// the next turn.
    Yield,
    /// An element or a tail is not iodata (`L_type_error`).
    BadArg,
}

/// The state of an iodata walk, as ERTS keeps it in `ErtsIOListState` and
/// `ErtsIOListCopyState`: the current term `obj` and a stack of the tails
/// of the lists above it.
struct Walk<'a> {
    /// The current term: the cons cell that the walk reads next, or the
    /// term after the last cell of the current list. A term that is not a
    /// list is its own tail.
    current: &'a Term,
    /// The bytes of the binary `current` that a copy took before a yield,
    /// as `from_offset` (`erl_iolist.c:365-373`, `402-410`).
    offset: usize,
    /// The rests of the lists above the current list, as the `ESTACK` of
    /// the walk.
    stack: Vec<&'a Term>,
    /// The unit of the bitstrings that are iodata. A walk of [`Unit::Bit`]
    /// counts the offset of a binary in bits, and a walk of [`Unit::Byte`]
    /// counts it in bytes.
    unit: Unit,
}

impl<'a> Walk<'a> {
    /// Start a walk at `term`, with the tails of `stack` above it. A trap
    /// gives the walk state as these terms ([`Walk::term`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let walk = Walk::new(&Term::binary(vec![1]), &[]);
    /// assert!(walk.stack.is_empty());
    /// ```
    fn new(term: &'a Term, stack: &[&'a Term]) -> Self {
        Walk {
            current: term,
            offset: 0,
            stack: stack.to_vec(),
            unit: Unit::Byte,
        }
    }

    /// Return this walk with the unit [`Unit::Bit`], so that a bitstring of each size is
    /// iodata, as for `erlang:list_to_bitstring/1`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Walk::new(&Term::nil(), &[]).bits().unit, Unit::Bit);
    /// ```
    fn bits(mut self) -> Self {
        self.unit = Unit::Bit;
        self
    }

    /// Return whether `term` is a bitstring that is iodata for the unit of
    /// this walk: each bitstring for [`Unit::Bit`], and a bitstring of a
    /// whole number of bytes for [`Unit::Byte`].
    fn is_leaf_binary(&self, term: &Term) -> bool {
        matches!(term, Term::Binary(binary) if self.unit.holds(binary))
    }

    /// Walk the iodata with `steps` steps, as `generic_iolist_size` and
    /// `generic_iolist_copy` do with `YieldSupport` (`erl_iolist.c:113-290`,
    /// `424-649`). Each cell of a list and each end of a list uses one step
    /// before the walk reads it. When the steps become 0 or less there, the
    /// walk yields, and it reads the same cell or end again when it
    /// continues. A byte integer of a cell, and a binary of a cell or of a
    /// tail, go to `leaf` with the offset of the binary. `leaf` can use more
    /// steps. It returns the bytes that it took when it stops in a binary,
    /// and the walk then yields. A list in a cell goes on the stack, and
    /// `[]` is empty. Each other element and tail gives [`Step::BadArg`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let list = Term::list_from(vec![Term::Integer(1), Term::binary(vec![2])]);
    /// let mut walk = Walk::new(&list, &[]);
    /// let mut steps = 8;
    /// assert_eq!(walk.run(&mut steps, |_, _, _| None), Step::Done);
    /// assert_eq!(steps, 5);
    /// ```
    fn run(
        &mut self,
        steps: &mut i64,
        mut leaf: impl FnMut(&'a Term, usize, &mut i64) -> Option<usize>,
    ) -> Step {
        loop {
            while let Some((head, rest)) = self.current.cons_cell() {
                *steps -= 1;
                if *steps <= 0 {
                    return Step::Yield;
                }
                match head {
                    _ if head.is_nil() => {}
                    _ if head.is_list() => {
                        self.stack.push(rest);
                        self.current = head;
                        continue;
                    }
                    // A bitstring of a size that is not a multiple of the unit
                    // is not iodata, as ERTS raises `badarg` for
                    // `iolist_size([<<1:4>>])`.
                    _ if matches!(head, Term::Integer(0..=255)) || self.is_leaf_binary(head) => {
                        if let Some(taken) = leaf(head, 0, steps) {
                            // The rest of the binary comes first when the walk
                            // continues (`erl_iolist.c:528-537`).
                            self.stack.push(rest);
                            (self.current, self.offset) = (head, taken);
                            return Step::Yield;
                        }
                    }
                    _ => return Step::BadArg,
                }
                self.current = rest;
            }
            *steps -= 1;
            if *steps <= 0 {
                return Step::Yield;
            }
            match self.current {
                tail if tail.is_nil() => {}
                binary if self.is_leaf_binary(binary) => {
                    if let Some(taken) = leaf(binary, self.offset, steps) {
                        self.offset += taken;
                        return Step::Yield;
                    }
                }
                _ => return Step::BadArg,
            }
            self.offset = 0;
            let Some(rest) = self.stack.pop() else {
                return Step::Done;
            };
            self.current = rest;
        }
    }

    /// Return the current term and the stack of the walk as terms for the
    /// arguments of a trap. A binary with an offset gives the bytes after
    /// the offset.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let list = Term::list_from(vec![Term::Integer(1)]);
    /// let walk = Walk::new(&list, &[]);
    /// assert_eq!(walk.term(), (list.clone(), Term::nil()));
    /// ```
    fn term(&self) -> (Term, Term) {
        let current = match self.current {
            Term::Binary(bits) if self.offset > 0 && self.unit == Unit::Bit => {
                Term::Binary(bits.bits(self.offset, bits.bit_size()))
            }
            Term::Binary(bytes) if self.offset > 0 => {
                Term::binary(leaf_bytes(bytes)[self.offset..].to_vec())
            }
            current => current.clone(),
        };
        let stack = self.stack.iter().map(|&rest| rest.clone());
        (current, Term::list_from(stack))
    }
}

/// Return the reductions of a walk that used `init - steps` steps and did
/// not yield: one for each 8 steps, rounded up, and at least 1
/// (`erl_iolist.c:246-255`, `600-608`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(walk_cost(80, 79), 1);
/// assert_eq!(walk_cost(80, 71), 2);
/// ```
fn walk_cost(init: i64, steps: i64) -> i32 {
    ((init - steps - 1) / STEPS_PER_RED + 1) as i32
}

/// Run `erlang:iolist_size/1` as ERTS `iolist_size_1` does
/// (`erl_iolist.c:1084-1094`): walk the term from the start. See
/// [`iolist_size_continue`].
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// assert!(matches!(iolist_size(&mut process, &Term::binary(b"abc".to_vec()))?, BifResult::Value(Term::Integer(3))));
/// assert_eq!(process.fcalls, 9);
/// ```
pub(super) fn iolist_size(process: &mut ProcessState, term: &Term) -> Result<BifResult, BeamError> {
    iolist_size_continue(process, term, &[], 0)
}

/// Use all reductions before a size walk yields, as `L_yield` of
/// `generic_iolist_size` does (`erl_iolist.c:280-289`), unless the walk
/// started with `init` steps of 0 or less. Such a walk yields at once with
/// no bump (`erl_iolist.c:133-135`), so a negative budget stays negative.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = -3;
/// bump_all_after_walk(&mut process, 0);
/// assert_eq!(process.fcalls, -3);
/// ```
fn bump_all_after_walk(process: &mut ProcessState, init: i64) {
    if init > 0 {
        process.bump_all_reductions();
    }
}

/// Continue `erlang:iolist_size/1` at `term`, with the tails of `stack`
/// above it and the byte count `size` so far, as `iolist_size_dispatch`
/// does (`erl_iolist.c:1004-1068`). The walk of [`Walk::run`] has 8 steps
/// for each reduction of the budget. When it ends, the call bumps its
/// reductions and returns the count of the bytes of each byte integer and
/// each binary, or raises `badarg` for a term that is not iodata. When the
/// steps end first, the call uses all reductions and traps to the export
/// `erts_internal:iolist_size_continue`, which continues the walk with no
/// cost in the next turn (`erl_iolist.c:61-63`, `280-289`, `1014-1031`,
/// `1070-1082`). ERTS keeps the walk state in a magic reference, and PBeam
/// keeps the current term, the stack, and the count as terms. ERTS raises
/// `system_limit` only for a size above 64 bignum words, which a PBeam term
/// cannot have.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 1;
/// let ones = Term::list_from(vec![Term::Integer(1); 8]);
/// let trap = iolist_size_continue(&mut process, &ones, &[], 0)?;
/// assert!(matches!(trap, BifResult::TrapBif { function: "iolist_size_continue", .. }));
/// assert_eq!(process.fcalls, 0);
/// ```
pub(super) fn iolist_size_continue(
    process: &mut ProcessState,
    term: &Term,
    stack: &[&Term],
    mut size: usize,
) -> Result<BifResult, BeamError> {
    let init = STEPS_PER_RED * process.bif_reductions_or_zero() as i64;
    let mut steps = init;
    let mut walk = Walk::new(term, stack);
    let step = walk.run(&mut steps, |leaf, _, _| {
        size += match leaf {
            Term::Binary(bytes) => leaf_bytes(bytes).len(),
            _ => 1,
        };
        None
    });
    if step == Step::Yield {
        bump_all_after_walk(process, init);
        let (term, stack) = walk.term();
        return Ok(BifResult::TrapBif {
            module: "erts_internal",
            function: "iolist_size_continue",
            arguments: vec![term, stack, Term::Integer(size as i64)],
        });
    }
    process.bump_reductions(walk_cost(init, steps));
    Ok(match step {
        Step::Done => Value(Term::Integer(size as i64)),
        _ => BifResult::badarg(),
    })
}

/// Run `erlang:iolist_to_iovec/1` as ERTS `iolist_to_iovec_1` does
/// (`erl_iolist.c:1265-1310`). `[]` and `<<>>` give `[]`, a binary `Bin`
/// that is not empty gives `[Bin]`, and a list of one binary gives the list
/// or `[]`, all with no cost. Another term that is not a list raises
/// `badarg`. Each other list goes to [`iovec_size`].
///
/// # Examples
///
/// ```rust,ignore
/// let hello = Term::binary(b"Hello".to_vec());
/// assert!(matches!(iolist_to_iovec(&mut process, &hello)?, BifResult::Value(iovec) if iovec == Term::list_from([hello])));
/// assert!(matches!(iolist_to_iovec(&mut process, &Term::static_atom("foo"))?, BifResult::Raise { .. }));
/// ```
pub(super) fn iolist_to_iovec(
    process: &mut ProcessState,
    term: &Term,
) -> Result<BifResult, BeamError> {
    match term {
        Term::Binary(bytes) if bytes.bit_size() == 0 => Ok(Value(Term::nil())),
        _ if term.is_binary() => Ok(Value(Term::list_from(vec![term.clone()]))),
        _ if term.is_nil() => Ok(Value(Term::nil())),
        _ => match term.cons_cell() {
            Some((Term::Binary(bytes), tail)) if tail.is_nil() && bytes.bit_size() == 0 => {
                Ok(Value(Term::nil()))
            }
            Some((head, tail)) if tail.is_nil() && head.is_binary() => Ok(Value(term.clone())),
            Some(_) => iovec_size(process, term, term, &[]),
            None => Ok(BifResult::badarg()),
        },
    }
}

/// Continue the size walk of `erlang:iolist_to_iovec/1` for the argument
/// `arg` at `term`, with the tails of `stack` above it, as
/// `iolist_to_iovec_engine` does before it copies (`erl_iolist.c:1121-1176`).
/// The walk raises `badarg` for a term that is not iodata. When the walk
/// ends with reductions left, the copy walk of [`iovec_copy`] starts in the
/// same call with the reductions that remain. When no reduction remains,
/// the call uses all reductions and traps, and the copy starts in the next
/// turn. When the steps of the size walk end first, the call uses all
/// reductions and traps to the export `erts_internal:iolist_to_iovec_continue`
/// (`erl_iolist.c:57-59`, `1183-1263`). PBeam keeps the phase, the argument,
/// and the walk state as terms.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// let list = Term::list_from(vec![Term::Integer(1)]);
/// let iovec = iovec_size(&mut process, &list, &list, &[])?;
/// assert!(matches!(iovec, BifResult::Value(binaries) if binaries == Term::list_from([Term::binary(vec![1])])));
/// assert_eq!(process.fcalls, 8);
/// ```
pub(super) fn iovec_size(
    process: &mut ProcessState,
    arg: &Term,
    term: &Term,
    stack: &[&Term],
) -> Result<BifResult, BeamError> {
    let reds_left = process.bif_reductions_or_zero() as i64;
    let init = STEPS_PER_RED * reds_left;
    let mut steps = init;
    let mut walk = Walk::new(term, stack);
    let step = walk.run(&mut steps, |_, _, _| None);
    if step == Step::Yield {
        bump_all_after_walk(process, init);
        let (term, stack) = walk.term();
        return Ok(iovec_trap(vec![
            Term::static_atom("size"),
            arg.clone(),
            term,
            stack,
        ]));
    }
    let cost = walk_cost(init, steps);
    process.bump_reductions(cost);
    if step == Step::BadArg {
        return Ok(BifResult::badarg());
    }
    let reds_left = reds_left - i64::from(cost);
    if reds_left <= 0 {
        process.bump_all_reductions();
        let start = copy_state(arg, Term::nil(), Vec::new(), Vec::new());
        return Ok(iovec_trap(start));
    }
    Ok(iovec_copy(
        process,
        reds_left,
        arg,
        &[],
        Vec::new(),
        Vec::new(),
    ))
}

/// Continue the copy walk of `erlang:iolist_to_iovec/1` at `term`, with the
/// tails of `stack` above it, `reds_left` reductions, the binaries `iovec`
/// of the result so far, and the bytes `merged` that the walk copied after
/// them, as `generic_iolist_copy` does (`erl_iolist.c:424-649`). Each byte
/// and each binary of at most [`IOVEC_COPY_LIMIT`] bytes goes into the
/// merged binary. A copy uses one more step for each 32 bytes, and it stops
/// in a binary when the steps end, so that the rest of the binary comes
/// first in the next turn (`erl_iolist.c:354-417`). Each larger binary
/// closes the merged binary, if it is not empty, and is an element of the
/// result itself (`generic_iolist_copy_vec_enqueue`, `erl_iolist.c:322-352`).
/// When the walk ends, the call bumps its reductions and returns the
/// binaries. Else it uses all reductions and traps.
///
/// # Examples
///
/// ```rust,ignore
/// let list = Term::list_from(vec![Term::Integer(1), Term::binary(vec![0; 513])]);
/// let iovec = iovec_copy(&mut process, 10, &list, &[], Vec::new(), Vec::new());
/// assert!(matches!(iovec, BifResult::Value(binaries) if binaries.proper_list().is_some_and(|binaries| binaries.len() == 2)));
/// ```
pub(super) fn iovec_copy(
    process: &mut ProcessState,
    reds_left: i64,
    term: &Term,
    stack: &[&Term],
    mut iovec: Vec<Term>,
    mut merged: Vec<u8>,
) -> BifResult {
    let init = STEPS_PER_RED * reds_left;
    let mut steps = init;
    let mut walk = Walk::new(term, stack);
    let step = walk.run(&mut steps, |leaf, offset, steps| match leaf {
        Term::Integer(byte) => {
            merged.push(*byte as u8);
            None
        }
        Term::Binary(bytes) if leaf_bytes(bytes).len() <= IOVEC_COPY_LIMIT => {
            copy_bytes(&mut merged, &leaf_bytes(bytes)[offset..], steps)
        }
        large => {
            if !merged.is_empty() {
                iovec.push(Term::binary(std::mem::take(&mut merged)));
            }
            iovec.push(large.clone());
            None
        }
    });
    if step == Step::Yield {
        process.bump_all_reductions();
        let (term, stack) = walk.term();
        return iovec_trap(copy_state(&term, stack, iovec, merged));
    }
    assert_eq!(step, Step::Done, "the size walk checked the iodata");
    process.bump_reductions(walk_cost(init, steps));
    if !merged.is_empty() {
        iovec.push(Term::binary(merged));
    }
    Value(Term::list_from(iovec))
}

/// Return the trap arguments of the copy walk: the phase `copy`, the current
/// term, the stack, the binaries of the result so far, and the merged bytes.
///
/// # Examples
///
/// ```rust,ignore
/// let state = copy_state(&Term::nil(), Term::nil(), Vec::new(), vec![1]);
/// assert_eq!(state[4], Term::binary(vec![1]));
/// ```
fn copy_state(term: &Term, stack: Term, iovec: Vec<Term>, merged: Vec<u8>) -> Vec<Term> {
    vec![
        Term::static_atom("copy"),
        term.clone(),
        stack,
        Term::list_from(iovec),
        Term::binary(merged),
    ]
}

/// Return the trap of `erlang:iolist_to_iovec/1` to the export
/// `erts_internal:iolist_to_iovec_continue` with `arguments`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(iovec_trap(vec![]), BifResult::TrapBif { function: "iolist_to_iovec_continue", .. }));
/// ```
fn iovec_trap(arguments: Vec<Term>) -> BifResult {
    BifResult::TrapBif {
        module: "erts_internal",
        function: "iolist_to_iovec_continue",
        arguments,
    }
}

/// Return the bytes of a binary leaf of a [`Walk`]. The walk gives only a
/// binary whose size is a whole number of bytes, because a bitstring is not
/// iodata.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(leaf_bytes(&Binary::from(vec![1])), &[1]);
/// ```
fn leaf_bytes(binary: &Binary) -> &[u8] {
    binary
        .bytes()
        .expect("the walk gives only a binary of whole bytes")
}

/// Copy the bytes `rest` of a binary into `bytes`, as
/// `generic_iolist_copy_bitstring` does with `YieldSupport`
/// (`erl_iolist.c:354-417`): a copy takes at most 32 bytes for each step
/// that remains, and one more step. When the copy takes all bytes, it uses
/// one step for each 32 bytes and returns `None`. Else it uses all steps
/// and returns the bytes that it took, so that the rest comes first in the
/// next turn.
///
/// # Examples
///
/// ```rust,ignore
/// let (mut bytes, mut steps) = (Vec::new(), 1);
/// assert_eq!(copy_bytes(&mut bytes, &[0; 100], &mut steps), Some(64));
/// assert_eq!(steps, 0);
/// ```
fn copy_bytes(bytes: &mut Vec<u8>, rest: &[u8], steps: &mut i64) -> Option<usize> {
    let taken = rest
        .len()
        .min(((*steps + 1) * COPY_BYTES_PER_STEP) as usize);
    bytes.extend_from_slice(&rest[..taken]);
    if taken < rest.len() {
        *steps = 0;
        return Some(taken);
    }
    *steps -= taken as i64 / COPY_BYTES_PER_STEP;
    None
}

/// Run `erlang:iolist_to_binary/1` as ERTS `iolist_to_binary_1` does
/// (`erl_iolist.c:916-928`). A binary gives itself with no cost. Each other
/// term goes to [`list_to_bitstring`].
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 4000;
/// let list = Term::list_from(vec![Term::Integer(1), Term::binary(vec![2])]);
/// assert!(matches!(iolist_to_binary(&mut scheduler, &list)?, BifResult::Value(Term::Binary(bytes)) if bytes == [1, 2]));
/// assert!(matches!(iolist_to_binary(&mut scheduler, &Term::binary(vec![1]))?, BifResult::Value(Term::Binary(bytes)) if bytes == [1]));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`list_to_bitstring`].
pub(super) fn iolist_to_binary(
    scheduler: &mut Scheduler,
    term: &Term,
) -> Result<BifResult, BeamError> {
    if term.is_binary() {
        return Ok(Value(term.clone()));
    }
    list_to_bitstring(scheduler, term, "iolist_to_binary", Unit::Byte)
}

/// Run `erlang:list_to_binary/1` as ERTS `list_to_binary_1` does
/// (`erl_iolist.c:910-914`): each term goes to [`list_to_bitstring`], so a
/// binary raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 4000;
/// let list = Term::list_from(vec![Term::Integer(1), Term::binary(vec![2])]);
/// assert!(matches!(list_to_binary(&mut scheduler, &list)?, BifResult::Value(Term::Binary(bytes)) if bytes == [1, 2]));
/// assert!(matches!(list_to_binary(&mut scheduler, &Term::binary(vec![1]))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`list_to_bitstring`].
pub(super) fn list_to_binary(
    scheduler: &mut Scheduler,
    term: &Term,
) -> Result<BifResult, BeamError> {
    list_to_bitstring(scheduler, term, "list_to_binary", Unit::Byte)
}

/// Run `erlang:list_to_bitstring/1` as ERTS `list_to_bitstring_1` does
/// (`erl_iolist.c:930-934`): [`list_to_bitstring`] with the unit 1, so that
/// a bitstring of each size can be an element or a tail of the list.
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 4000;
/// let list = Term::list_from(vec![Term::Integer(1)]);
/// assert!(matches!(list_to_bitstring_1(&mut scheduler, &list)?, BifResult::Value(Term::Binary(bytes)) if bytes == [1]));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`list_to_bitstring`].
pub(super) fn list_to_bitstring_1(
    scheduler: &mut Scheduler,
    term: &Term,
) -> Result<BifResult, BeamError> {
    list_to_bitstring(scheduler, term, "list_to_bitstring", Unit::Bit)
}

/// Run the shared part of `erlang:iolist_to_binary/1`,
/// `erlang:list_to_binary/1`, and `erlang:list_to_bitstring/1` as ERTS
/// `list_to_bitstring_bif` does (`erl_iolist.c:857-902`), for the BIF
/// `erlang:<bif>/1` with the bitstring unit `unit`. A caller with at
/// most 100 reductions switches out first: the call uses the rest of its
/// budget as virtual reductions and runs `erlang:<bif>/1` again from the
/// start in the next turn (`ERTS_BIF_YIELD1`, `bif.h:501-505`). A caller
/// with fewer than 1000 reductions walks with a budget of 1000, so that its
/// budget can become 0. Then `[]` gives `<<>>`, a list of one bitstring
/// whose size is a multiple of `unit` gives the bitstring, and a term that is not a list raises `badarg`, all with no
/// cost. Each other list goes to [`l2b_size`].
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 4000;
/// let list = Term::list_from(vec![Term::Integer(1), Term::binary(vec![2])]);
/// assert!(matches!(list_to_bitstring(&mut scheduler, &list, "list_to_binary", Unit::Byte)?, BifResult::Value(Term::Binary(bytes)) if bytes == [1, 2]));
/// assert!(matches!(list_to_bitstring(&mut scheduler, &Term::static_atom("a"), "iolist_to_binary", Unit::Byte)?, BifResult::Raise { .. }));
/// ```
fn list_to_bitstring(
    scheduler: &mut Scheduler,
    term: &Term,
    bif: &'static str,
    unit: Unit,
) -> Result<BifResult, BeamError> {
    let process = scheduler.current_process_mut();
    let mut reds_left = process.bif_reductions_or_zero() as i64;
    if reds_left <= L2B_RESCHED_REDS {
        scheduler.vbump_all_reductions();
        return Ok(BifResult::TrapBif {
            module: "erlang",
            function: bif,
            arguments: vec![term.clone()],
        });
    }
    reds_left = reds_left.max(L2B_MIN_REDS);
    let process = scheduler.current_process_mut();
    if term.is_nil() {
        return Ok(Value(Term::binary(Vec::new())));
    }
    match term.cons_cell() {
        Some((binary @ Term::Binary(bits), tail)) if tail.is_nil() && unit.holds(bits) => {
            Ok(Value(binary.clone()))
        }
        Some(_) => Ok(l2b_size(process, reds_left, term, term, &[], 0, unit)),
        None => Ok(BifResult::badarg()),
    }
}

/// Continue the size walk of [`list_to_bitstring`] for the argument
/// `arg` at `term`, with the tails of `stack` above it, the bit count
/// `size` so far, the bitstring unit `unit`, and the budget `reds_left`, as `list_to_bitstring_engine`
/// does before it copies (`erl_iolist.c:710-753`, `837-840`). The walk of
/// [`Walk::run`] has 8 steps for each reduction of the budget, and it
/// raises `badarg` for a term that is not iodata (`erl_iolist.c:802-818`).
/// An empty result gives `<<>>` with no copy. When the walk ends with
/// reductions left, the copy walk of [`l2b_copy`] starts in the same call
/// with the reductions that remain. When no reduction remains, the call
/// uses all reductions and traps, and the copy starts in the next turn.
/// When the steps of the size walk end first, the call uses all reductions
/// and traps to the export `erts_internal:list_to_bitstring_continue`,
/// which continues with the budget of the next turn
/// (`erl_iolist.c:764-835`). ERTS keeps the walk state in a magic
/// reference, and PBeam keeps the phase, the argument, the walk state, the
/// count, and the unit as terms. ERTS raises `system_limit` only for a size above
/// `2^64 - 1` bits, which a PBeam term cannot have.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 4000;
/// let list = Term::list_from(vec![Term::Integer(1)]);
/// let binary = l2b_size(&mut process, 4000, &list, &list, &[], 0, Unit::Byte);
/// assert!(matches!(binary, BifResult::Value(Term::Binary(bytes)) if bytes == [1]));
/// assert_eq!(process.fcalls, 3998);
/// ```
pub(super) fn l2b_size(
    process: &mut ProcessState,
    reds_left: i64,
    arg: &Term,
    term: &Term,
    stack: &[&Term],
    mut size: usize,
    unit: Unit,
) -> BifResult {
    let init = STEPS_PER_RED * reds_left;
    let mut steps = init;
    let mut walk = Walk::new(term, stack);
    walk.unit = unit;
    let step = walk.run(&mut steps, |leaf, _, _| {
        size += match leaf {
            Term::Binary(bits) => bits.bit_size(),
            _ => 8,
        };
        None
    });
    if step == Step::Yield {
        process.bump_all_reductions();
        let (term, stack) = walk.term();
        return l2b_trap(vec![
            Term::static_atom("size"),
            arg.clone(),
            term,
            stack,
            Term::Integer(size as i64),
            Term::Integer(unit.bits()),
        ]);
    }
    let cost = walk_cost(init, steps);
    process.bump_reductions(cost);
    if step == Step::BadArg {
        return BifResult::badarg();
    }
    if size == 0 {
        return Value(Term::binary(Vec::new()));
    }
    let reds_left = reds_left - i64::from(cost);
    if reds_left <= 0 {
        process.bump_all_reductions();
        return l2b_trap(vec![
            Term::static_atom("copy"),
            arg.clone(),
            Term::nil(),
            Term::Integer(size as i64),
            Term::binary(Vec::new()),
        ]);
    }
    l2b_copy(process, reds_left, arg, &[], size, BitWriter::default())
}

/// Continue the copy walk of [`list_to_bitstring`] at `term`, with
/// the tails of `stack` above it, `reds_left` reductions, the result size
/// `size` in bits, and the bits `bits` that the walk copied, as
/// `generic_iolist_copy` does (`erl_iolist.c:424-649`, `842-845`). Each
/// byte and each bitstring goes into the result at the current bit offset
/// (`erl_iolist.c:481-505`), and a copy uses one more step for each 256
/// bits ([`copy_bits`]). The size walk checked the unit, so the copy walk
/// takes each bitstring. When the steps end after the last bit, the walk
/// ends: the size walk checked the rest (`erl_iolist.c:627-641`). When the
/// walk ends, the call bumps its reductions and returns the bitstring.
/// Else it uses all reductions and traps.
///
/// # Examples
///
/// ```rust,ignore
/// let list = Term::list_from(vec![Term::Integer(1), Term::binary(vec![0; 600])]);
/// let binary = l2b_copy(&mut process, 10, &list, &[], 601 * 8, BitWriter::default());
/// assert!(matches!(binary, BifResult::Value(Term::Binary(bytes)) if bytes.len() == 601));
/// ```
pub(super) fn l2b_copy(
    process: &mut ProcessState,
    reds_left: i64,
    term: &Term,
    stack: &[&Term],
    size: usize,
    mut bits: BitWriter,
) -> BifResult {
    let init = STEPS_PER_RED * reds_left;
    let mut steps = init;
    let mut walk = Walk::new(term, stack).bits();
    let step = walk.run(&mut steps, |leaf, offset, steps| match leaf {
        Term::Binary(binary) => copy_bits(&mut bits, binary, offset, steps),
        byte => {
            if let Term::Integer(byte) = byte {
                bits.push_bits(*byte as u8, 8);
            }
            None
        }
    });
    if step == Step::Yield && (bits.bit_size() as usize) < size {
        process.bump_all_reductions();
        let (term, stack) = walk.term();
        let (bytes, copied) = bits.into_bytes();
        return l2b_trap(vec![
            Term::static_atom("copy"),
            term,
            stack,
            Term::Integer(size as i64),
            Term::Binary(bitstring(bytes, copied)),
        ]);
    }
    process.bump_reductions(walk_cost(init, steps));
    let (bytes, copied) = bits.into_bytes();
    Value(Term::Binary(bitstring(bytes, copied)))
}

/// Copy the bits of `binary` from the bit `offset` into `bits`, as
/// `generic_iolist_copy_bitstring` does with `YieldSupport`
/// (`erl_iolist.c:354-417`): a copy takes at most 256 bits for each step
/// that remains, and one more step. When the copy takes all bits, it uses
/// one step for each 256 bits and returns `None`. Else it uses all steps
/// and returns the bits that it took, so that the rest comes first in the
/// next turn. An offset is a sum of such parts, so it is a whole number of
/// bytes.
///
/// # Examples
///
/// ```rust,ignore
/// let (mut bits, mut steps) = (BitWriter::default(), 0);
/// assert_eq!(copy_bits(&mut bits, &Binary::from(vec![0; 40]), 0, &mut steps), Some(256));
/// ```
fn copy_bits(
    bits: &mut BitWriter,
    binary: &Binary,
    offset: usize,
    steps: &mut i64,
) -> Option<usize> {
    let rest = binary.bit_size() - offset;
    let taken = rest.min(((*steps + 1) * COPY_BITS_PER_STEP) as usize);
    bits.push_bit_bytes(&binary.bit_bytes()[offset / 8..], taken);
    if taken < rest {
        *steps = 0;
        return Some(taken);
    }
    *steps -= taken as i64 / COPY_BITS_PER_STEP;
    None
}

/// Return the trap of [`list_to_bitstring`] to the export
/// `erts_internal:list_to_bitstring_continue` with `arguments`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(l2b_trap(vec![]), BifResult::TrapBif { function: "list_to_bitstring_continue", .. }));
/// ```
fn l2b_trap(arguments: Vec<Term>) -> BifResult {
    BifResult::TrapBif {
        module: "erts_internal",
        function: "list_to_bitstring_continue",
        arguments,
    }
}

/// Return the bytes of iodata: the bytes of a binary, or the bytes of each
/// element and each binary tail of an iolist in order, as
/// `erts_iolist_to_buf` writes them. Return `None` for any other term and
/// for an iolist with an element or a tail that is not iodata. This walk
/// has no budget.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(iodata_bytes(&Term::binary(b"ab".to_vec())), Some(b"ab".to_vec()));
/// // [$a|<<"b">>] gives <<"ab">>, and [$a|x] is not iodata.
/// let tail = |tail| Term::list_with_tail(vec![Term::Integer(97)], tail);
/// assert_eq!(iodata_bytes(&tail(Term::binary(b"b".to_vec()))), Some(b"ab".to_vec()));
/// assert_eq!(iodata_bytes(&tail(Term::static_atom("x"))), None);
/// assert_eq!(iodata_bytes(&Term::static_atom("foo")), None);
/// ```
pub(crate) fn iodata_bytes(term: &Term) -> Option<Vec<u8>> {
    let (mut bytes, mut unlimited) = (Vec::new(), i64::MAX);
    let step = Walk::new(term, &[]).run(&mut unlimited, |leaf, _, _| {
        if let Term::Binary(binary) = leaf {
            bytes.extend_from_slice(leaf_bytes(binary));
        } else if let Term::Integer(byte) = leaf {
            bytes.push(*byte as u8);
        }
        None
    });
    (step == Step::Done).then_some(bytes)
}

#[cfg(test)]
#[path = "../../../../tests/runtime/interpreter/bif/iolist.rs"]
mod tests;
