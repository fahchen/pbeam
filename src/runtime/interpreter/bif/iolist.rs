//! The iodata BIFs `erlang:iolist_size/1`, `erlang:iolist_to_iovec/1`, and
//! `erlang:list_to_binary/1`, and the iodata walk of ERTS
//! `generic_iolist_size` and `generic_iolist_copy` (`erl_iolist.c:113-649`)
//! that they share.

use super::BifResult::Value;
use super::*;

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

/// The smallest budget with which `erlang:list_to_binary/1` walks a list:
/// `IOL2B_MIN_EXEC_REDS` (`erl_iolist.c:34`), `CONTEXT_REDS / 4`. A caller
/// with fewer reductions walks with this budget.
const L2B_MIN_REDS: i64 = 1000;

/// The budget at or below which `erlang:list_to_binary/1` switches out the
/// caller before it reads its argument: `IOL2B_RESCHED_REDS`
/// (`erl_iolist.c:35`), `CONTEXT_REDS / 40`.
const L2B_RESCHED_REDS: i64 = 100;

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
    /// The cells of the current list that the walk did not read yet. A term
    /// that is not a list has no cells.
    cells: &'a [Term],
    /// The tail after `cells`: `None` for `[]`, or the term after the last
    /// cell. A term that is not a list is its own tail.
    tail: Option<&'a Term>,
    /// The bytes of the binary `tail` that a copy took before a yield, as
    /// `from_offset` (`erl_iolist.c:365-373`, `402-410`).
    offset: usize,
    /// The cells and the tails of the lists above the current list, as the
    /// `ESTACK` of the walk.
    stack: Vec<(&'a [Term], Option<&'a Term>)>,
}

impl<'a> Walk<'a> {
    /// Start a walk at `term`, with the tails of `stack` above it. A trap
    /// gives the walk state as these terms ([`Walk::term`]).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let walk = Walk::new(&Term::Binary(vec![1]), &[]);
    /// assert!(walk.cells.is_empty() && walk.stack.is_empty());
    /// ```
    fn new(term: &'a Term, stack: &'a [Term]) -> Self {
        let (cells, tail) = parts(term);
        Walk {
            cells,
            tail,
            offset: 0,
            stack: stack.iter().map(parts).collect(),
        }
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
    /// let list = Term::List(vec![Term::Integer(1), Term::Binary(vec![2])]);
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
            while let Some((head, rest)) = self.cells.split_first() {
                *steps -= 1;
                if *steps <= 0 {
                    return Step::Yield;
                }
                match head {
                    Term::List(inner) if inner.is_empty() => {}
                    Term::List(_) | Term::ImproperList(..) => {
                        self.stack.push((rest, self.tail));
                        (self.cells, self.tail) = parts(head);
                        continue;
                    }
                    Term::Integer(0..=255) | Term::Binary(_) => {
                        if let Some(taken) = leaf(head, 0, steps) {
                            // The rest of the binary comes first when the walk
                            // continues (`erl_iolist.c:528-537`).
                            self.stack.push((rest, self.tail));
                            (self.cells, self.tail, self.offset) = (&[], Some(head), taken);
                            return Step::Yield;
                        }
                    }
                    _ => return Step::BadArg,
                }
                self.cells = rest;
            }
            *steps -= 1;
            if *steps <= 0 {
                return Step::Yield;
            }
            match self.tail {
                None => {}
                Some(binary @ Term::Binary(_)) => {
                    if let Some(taken) = leaf(binary, self.offset, steps) {
                        self.offset += taken;
                        return Step::Yield;
                    }
                }
                Some(_) => return Step::BadArg,
            }
            self.offset = 0;
            let Some((cells, tail)) = self.stack.pop() else {
                return Step::Done;
            };
            (self.cells, self.tail) = (cells, tail);
        }
    }

    /// Return the current term and the stack of the walk as terms for the
    /// arguments of a trap. A binary with an offset gives the bytes after
    /// the offset.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let list = Term::List(vec![Term::Integer(1)]);
    /// let walk = Walk::new(&list, &[]);
    /// assert_eq!(walk.term(), (list.clone(), Term::List(vec![])));
    /// ```
    fn term(&self) -> (Term, Term) {
        let current = match self.tail {
            Some(Term::Binary(bytes)) if self.offset > 0 => {
                Term::Binary(bytes[self.offset..].to_vec())
            }
            tail => list_rest(self.cells, tail),
        };
        let stack = self
            .stack
            .iter()
            .map(|(cells, tail)| list_rest(cells, *tail));
        (current, Term::List(stack.collect()))
    }
}

/// Return the cells and the tail of a term: the elements of a list and its
/// tail, or no cells and the term itself for a term that is not a list.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(parts(&Term::List(vec![])), ([], None)));
/// assert!(matches!(parts(&Term::Binary(vec![])), ([], Some(Term::Binary(_)))));
/// ```
fn parts(term: &Term) -> (&[Term], Option<&Term>) {
    list_cells(term).unwrap_or((&[], Some(term)))
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
/// assert!(matches!(iolist_size(&mut process, &Term::Binary(b"abc".to_vec()))?, BifResult::Value(Term::Integer(3))));
/// assert_eq!(process.fcalls, 9);
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn iolist_size(process: &mut ProcessState, term: &Term) -> Result<BifResult, BeamError> {
    iolist_size_continue(process, term, &[], 0)
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
/// let ones = Term::List(vec![Term::Integer(1); 8]);
/// let trap = iolist_size_continue(&mut process, &ones, &[], 0)?;
/// assert!(matches!(trap, BifResult::TrapBif { function: "iolist_size_continue", .. }));
/// assert_eq!(process.fcalls, 0);
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn iolist_size_continue(
    process: &mut ProcessState,
    term: &Term,
    stack: &[Term],
    mut size: usize,
) -> Result<BifResult, BeamError> {
    let init = STEPS_PER_RED * process.bif_reductions_left()? as i64;
    let mut steps = init;
    let mut walk = Walk::new(term, stack);
    let step = walk.run(&mut steps, |leaf, _, _| {
        size += match leaf {
            Term::Binary(bytes) => bytes.len(),
            _ => 1,
        };
        None
    });
    if step == Step::Yield {
        process.bump_all_reductions();
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
/// let hello = Term::Binary(b"Hello".to_vec());
/// assert!(matches!(iolist_to_iovec(&mut process, &hello)?, BifResult::Value(Term::List(iovec)) if iovec == [hello]));
/// assert!(matches!(iolist_to_iovec(&mut process, &Term::static_atom("foo"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn iolist_to_iovec(
    process: &mut ProcessState,
    term: &Term,
) -> Result<BifResult, BeamError> {
    match term {
        Term::Binary(bytes) if bytes.is_empty() => Ok(Value(Term::List(Vec::new()))),
        Term::Binary(_) => Ok(Value(Term::List(vec![term.clone()]))),
        Term::List(elements) => match elements.as_slice() {
            [] => Ok(Value(Term::List(Vec::new()))),
            [Term::Binary(bytes)] if bytes.is_empty() => Ok(Value(Term::List(Vec::new()))),
            [Term::Binary(_)] => Ok(Value(term.clone())),
            _ => iovec_size(process, term, term, &[]),
        },
        Term::ImproperList(..) => iovec_size(process, term, term, &[]),
        _ => Ok(BifResult::badarg()),
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
/// let list = Term::List(vec![Term::Integer(1)]);
/// let iovec = iovec_size(&mut process, &list, &list, &[])?;
/// assert!(matches!(iovec, BifResult::Value(Term::List(binaries)) if binaries == [Term::Binary(vec![1])]));
/// assert_eq!(process.fcalls, 8);
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn iovec_size(
    process: &mut ProcessState,
    arg: &Term,
    term: &Term,
    stack: &[Term],
) -> Result<BifResult, BeamError> {
    let reds_left = process.bif_reductions_left()? as i64;
    let init = STEPS_PER_RED * reds_left;
    let mut steps = init;
    let mut walk = Walk::new(term, stack);
    let step = walk.run(&mut steps, |_, _, _| None);
    if step == Step::Yield {
        process.bump_all_reductions();
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
        let start = copy_state(arg, Term::List(Vec::new()), Vec::new(), Vec::new());
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
/// let list = Term::List(vec![Term::Integer(1), Term::Binary(vec![0; 513])]);
/// let iovec = iovec_copy(&mut process, 10, &list, &[], Vec::new(), Vec::new());
/// assert!(matches!(iovec, BifResult::Value(Term::List(binaries)) if binaries.len() == 2));
/// ```
pub(super) fn iovec_copy(
    process: &mut ProcessState,
    reds_left: i64,
    term: &Term,
    stack: &[Term],
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
        Term::Binary(bytes) if bytes.len() <= IOVEC_COPY_LIMIT => {
            copy_bytes(&mut merged, &bytes[offset..], steps)
        }
        large => {
            if !merged.is_empty() {
                iovec.push(Term::Binary(std::mem::take(&mut merged)));
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
        iovec.push(Term::Binary(merged));
    }
    Value(Term::List(iovec))
}

/// Return the trap arguments of the copy walk: the phase `copy`, the current
/// term, the stack, the binaries of the result so far, and the merged bytes.
///
/// # Examples
///
/// ```rust,ignore
/// let state = copy_state(&Term::List(vec![]), Term::List(vec![]), Vec::new(), vec![1]);
/// assert_eq!(state[4], Term::Binary(vec![1]));
/// ```
fn copy_state(term: &Term, stack: Term, iovec: Vec<Term>, merged: Vec<u8>) -> Vec<Term> {
    vec![
        Term::static_atom("copy"),
        term.clone(),
        stack,
        Term::List(iovec),
        Term::Binary(merged),
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

/// Copy the bytes `rest` of a binary after `bytes`, as
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
/// assert_eq!((bytes.len(), steps), (64, 0));
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

/// Run `erlang:list_to_binary/1` as ERTS `list_to_binary_1` and
/// `list_to_bitstring_bif` do (`erl_iolist.c:857-914`). A caller with at
/// most 100 reductions switches out first: the call uses the rest of its
/// budget as virtual reductions and runs again from the start in the next
/// turn (`ERTS_BIF_YIELD1`, `bif.h:501-505`). A caller with fewer than 1000
/// reductions walks with a budget of 1000, so that its budget can become 0.
/// Then `[]` gives `<<>>`, a list of one binary gives the binary, and a term
/// that is not a list raises `badarg`, all with no cost. Each other list
/// goes to [`l2b_size`].
///
/// # Examples
///
/// ```rust,ignore
/// scheduler.current_process_mut().fcalls = 4000;
/// let list = Term::List(vec![Term::Integer(1), Term::Binary(vec![2])]);
/// assert!(matches!(list_to_binary(&mut scheduler, &list)?, BifResult::Value(Term::Binary(bytes)) if bytes == [1, 2]));
/// assert!(matches!(list_to_binary(&mut scheduler, &Term::Binary(vec![1]))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn list_to_binary(
    scheduler: &mut Scheduler,
    term: &Term,
) -> Result<BifResult, BeamError> {
    let process = scheduler.current_process_mut();
    let reds_left = process.bif_reductions_left()? as i64;
    if reds_left <= L2B_RESCHED_REDS {
        let fcalls = process.fcalls;
        scheduler.vbump_reductions(fcalls);
        return Ok(BifResult::TrapBif {
            module: "erlang",
            function: "list_to_binary",
            arguments: vec![term.clone()],
        });
    }
    let reds_left = reds_left.max(L2B_MIN_REDS);
    let process = scheduler.current_process_mut();
    Ok(match term {
        Term::List(elements) => match elements.as_slice() {
            [] => Value(Term::Binary(Vec::new())),
            [binary @ Term::Binary(_)] => Value(binary.clone()),
            _ => l2b_size(process, reds_left, term, term, &[], 0),
        },
        Term::ImproperList(..) => l2b_size(process, reds_left, term, term, &[], 0),
        _ => BifResult::badarg(),
    })
}

/// Continue the size walk of `erlang:list_to_binary/1` for the argument
/// `arg` at `term`, with the tails of `stack` above it, the byte count
/// `size` so far, and the budget `reds_left`, as `list_to_bitstring_engine`
/// does before it copies (`erl_iolist.c:710-753`, `837-840`). The walk of
/// [`Walk::run`] has 8 steps for each reduction of the budget, and the call
/// raises `badarg` for a term that is not iodata (`erl_iolist.c:802-818`).
/// An empty result gives `<<>>` with no copy. When the walk ends with
/// reductions left, the copy walk of [`l2b_copy`] starts in the same call
/// with the reductions that remain. When no reduction remains, the call
/// uses all reductions and traps, and the copy starts in the next turn.
/// When the steps of the size walk end first, the call uses all reductions
/// and traps to the export `erts_internal:list_to_bitstring_continue`,
/// which continues with the budget of the next turn
/// (`erl_iolist.c:764-835`). ERTS keeps the walk state in a magic
/// reference, and PBeam keeps the phase, the argument, the walk state, and
/// the count as terms. ERTS raises `system_limit` only for a size above
/// `2^64 - 1` bits, which a PBeam term cannot have.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 4000;
/// let list = Term::List(vec![Term::Integer(1)]);
/// let binary = l2b_size(&mut process, 4000, &list, &list, &[], 0);
/// assert!(matches!(binary, BifResult::Value(Term::Binary(bytes)) if bytes == [1]));
/// assert_eq!(process.fcalls, 3998);
/// ```
pub(super) fn l2b_size(
    process: &mut ProcessState,
    reds_left: i64,
    arg: &Term,
    term: &Term,
    stack: &[Term],
    mut size: usize,
) -> BifResult {
    let init = STEPS_PER_RED * reds_left;
    let mut steps = init;
    let mut walk = Walk::new(term, stack);
    let step = walk.run(&mut steps, |leaf, _, _| {
        size += match leaf {
            Term::Binary(bytes) => bytes.len(),
            _ => 1,
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
        ]);
    }
    let cost = walk_cost(init, steps);
    process.bump_reductions(cost);
    if step == Step::BadArg {
        return BifResult::badarg();
    }
    if size == 0 {
        return Value(Term::Binary(Vec::new()));
    }
    let reds_left = reds_left - i64::from(cost);
    if reds_left <= 0 {
        process.bump_all_reductions();
        return l2b_trap(vec![
            Term::static_atom("copy"),
            arg.clone(),
            Term::List(Vec::new()),
            Term::Integer(size as i64),
            Term::Binary(Vec::new()),
        ]);
    }
    l2b_copy(process, reds_left, arg, &[], size, Vec::new())
}

/// Continue the copy walk of `erlang:list_to_binary/1` at `term`, with the
/// tails of `stack` above it, `reds_left` reductions, the result size
/// `size`, and the bytes `bytes` that the walk copied, as
/// `generic_iolist_copy` does for a binary result (`erl_iolist.c:424-649`,
/// `842-845`). Each byte and each binary goes into the result, and a copy
/// uses one more step for each 32 bytes ([`copy_bytes`]). When the steps
/// end after the last byte, the walk ends: the size walk checked the rest
/// (`erl_iolist.c:627-641`). When the walk ends, the call bumps its
/// reductions and returns the binary. Else it uses all reductions and
/// traps.
///
/// # Examples
///
/// ```rust,ignore
/// let list = Term::List(vec![Term::Integer(1), Term::Binary(vec![0; 600])]);
/// let binary = l2b_copy(&mut process, 10, &list, &[], 601, Vec::new());
/// assert!(matches!(binary, BifResult::Value(Term::Binary(bytes)) if bytes.len() == 601));
/// ```
pub(super) fn l2b_copy(
    process: &mut ProcessState,
    reds_left: i64,
    term: &Term,
    stack: &[Term],
    size: usize,
    mut bytes: Vec<u8>,
) -> BifResult {
    let init = STEPS_PER_RED * reds_left;
    let mut steps = init;
    let mut walk = Walk::new(term, stack);
    let step = walk.run(&mut steps, |leaf, offset, steps| match leaf {
        Term::Binary(binary) => copy_bytes(&mut bytes, &binary[offset..], steps),
        byte => {
            if let Term::Integer(byte) = byte {
                bytes.push(*byte as u8);
            }
            None
        }
    });
    if step == Step::Yield && bytes.len() < size {
        process.bump_all_reductions();
        let (term, stack) = walk.term();
        return l2b_trap(vec![
            Term::static_atom("copy"),
            term,
            stack,
            Term::Integer(size as i64),
            Term::Binary(bytes),
        ]);
    }
    process.bump_reductions(walk_cost(init, steps));
    Value(Term::Binary(bytes))
}

/// Return the trap of `erlang:list_to_binary/1` to the export
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
/// assert_eq!(iodata_bytes(&Term::Binary(b"ab".to_vec())), Some(b"ab".to_vec()));
/// // [$a|<<"b">>] gives <<"ab">>, and [$a|x] is not iodata.
/// let tail = |tail| Term::ImproperList(vec![Term::Integer(97)], Box::new(tail));
/// assert_eq!(iodata_bytes(&tail(Term::Binary(b"b".to_vec()))), Some(b"ab".to_vec()));
/// assert_eq!(iodata_bytes(&tail(Term::static_atom("x"))), None);
/// assert_eq!(iodata_bytes(&Term::static_atom("foo")), None);
/// ```
pub(in crate::runtime::interpreter) fn iodata_bytes(term: &Term) -> Option<Vec<u8>> {
    let (mut bytes, mut unlimited) = (Vec::new(), i64::MAX);
    let step = Walk::new(term, &[]).run(&mut unlimited, |leaf, _, _| {
        if let Term::Binary(binary) = leaf {
            bytes.extend_from_slice(binary);
        } else if let Term::Integer(byte) = leaf {
            bytes.push(*byte as u8);
        }
        None
    });
    (step == Step::Done).then_some(bytes)
}
