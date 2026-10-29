//! The match and select BIFs on an `ordered_set` table, as `erl_db_tree.c`
//! walks the tree in key order. A continuation holds the last key of the
//! walk, as in ERTS, so it does not depend on the layout of the table.

use super::{
    AtomTable, BeamError, BifContext, BifResult, MatchSpec, Reference, Table, Term, Value,
};
use crate::runtime::ets::range::{KeyRange, cmp_partly_bound, key_range};
use crate::term::Direction;
use std::cmp::Ordering;

/// The most objects that one turn of a walk reads (`sc.max = 1000`,
/// `erl_db_tree.c:1309`, `1471`, `1841`, `2132`).
const TREE_ITERATIONS: i64 = 1000;

/// The BIF family of a select on an `ordered_set` table: whether the call is a
/// `select_reverse` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime::interpreter::bif) enum SelectOrder {
    /// `ets:select/2,3`, `ets:match/2,3`, and `ets:match_object/2,3`.
    Select,
    /// `ets:select_reverse/2,3`.
    SelectReverse,
}

impl SelectOrder {
    /// Return the direction of a walk of a call with the limit `chunk`: a
    /// call without a limit walks down and puts each match before the earlier
    /// ones, a call with a limit walks up, and `select_reverse` swaps both.
    fn direction(self, chunk: i64) -> Direction {
        if (chunk == 0) == (self == Self::SelectReverse) {
            Direction::Forward
        } else {
            Direction::Backward
        }
    }
}

/// Whether a range turn counts the matches or replaces them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RangeKind {
    /// `ets:select_count/2`.
    Count,
    /// `ets:select_replace/2`.
    Replace,
}

/// The state of one turn of a select (`struct select_context`,
/// `erl_db_tree.c:247-262`).
struct Walk<'a> {
    /// The atom table of the run.
    atoms: &'a AtomTable,
    /// The compiled match specification.
    spec: &'a MatchSpec,
    /// The 0-based key position of the table.
    key: usize,
    /// The partially bound key where the walk ends, or `None`.
    end: Option<Term>,
    /// The limit of the call, or 0 for a call without a limit.
    chunk: i64,
    /// The number of matches of a call with a limit.
    got: i64,
    /// The matches, the last match first.
    accum: Term,
    /// The objects that the turn can still read.
    max: i64,
    /// The key of the last object that the walk read.
    last: Option<Term>,
    /// Whether the walk only counts the results `true` in `got`, as
    /// `doit_select_count` does, and keeps no match.
    counting: bool,
}

impl Walk<'_> {
    /// Read the object `object`, as `doit_select` and `doit_select_chunk`
    /// do (`erl_db_tree.c:4134-4161`, `4188-4217`): stop at a key past the
    /// end key, else match the object, and stop when the turn has read its
    /// objects or found the limit of matches. Return `false` to stop.
    fn visit(&mut self, object: &[Term], direction: Direction) -> Result<bool, BeamError> {
        let key = &object[self.key];
        self.last = Some(key.clone());
        if let Some(end) = &self.end {
            let order = cmp_partly_bound(self.atoms, end, key)?;
            if (direction == Direction::Forward && order.is_lt())
                || (direction == Direction::Backward && order.is_gt())
            {
                return Ok(false);
            }
        }
        if let Some(found) = self.spec.run(self.atoms, object) {
            if self.counting {
                self.got += i64::from(found == Term::static_atom("true"));
            } else {
                if self.chunk != 0 {
                    self.got += 1;
                }
                self.accum = Term::cons(found, std::mem::replace(&mut self.accum, Term::nil()));
            }
        }
        self.max -= 1;
        Ok(self.max > 0 && (self.chunk == 0 || self.got != self.chunk))
    }

    /// Read the objects of `table` from the key next to `after`, up for
    /// [`Direction::Forward`], until [`Walk::visit`] stops.
    fn run(
        &mut self,
        table: &Table,
        after: Option<&Term>,
        direction: Direction,
    ) -> Result<(), BeamError> {
        for object in table.ordered_walk(self.atoms, after, direction)? {
            if !self.visit(object, direction)? {
                break;
            }
        }
        Ok(())
    }
}

/// Return the 0-based key position of `table`.
fn key_index(table: &Table) -> usize {
    usize::try_from(table.keypos()).map_or(usize::MAX, |keypos| keypos.wrapping_sub(1))
}

/// Return the key where a walk with the key range `range` starts and the
/// key where it ends, as `db_select_tree_common` and
/// `db_select_chunk_tree_common` find them with `find_prev_from_pb_key`
/// for a walk up and `find_next_from_pb_key` for a walk down
/// (`erl_db_tree.c:1500-1531`, `1875-1906`).
fn bounds(
    atoms: &AtomTable,
    table: &Table,
    range: &KeyRange,
    direction: Direction,
) -> Result<(Option<Term>, Option<Term>), BeamError> {
    let KeyRange::Partial { least, most } = range else {
        return Ok((None, None));
    };
    if direction == Direction::Forward {
        let (before, _) =
            table.ordered_split(|key| Ok(cmp_partly_bound(atoms, least, key)?.is_gt()))?;
        Ok((before.cloned(), Some(most.clone())))
    } else {
        let (_, after) =
            table.ordered_split(|key| Ok(!cmp_partly_bound(atoms, most, key)?.is_lt()))?;
        Ok((after.cloned(), Some(least.clone())))
    }
}

/// The end of a turn, after the release of the table lock.
pub(super) enum Turn {
    /// Return this value.
    Value(Term),
    /// Return `{Matches, '$end_of_table'}` through `ets:reverse/3`, or
    /// `'$end_of_table'` with no match.
    Last(Term, i64),
    /// Return the matches and a continuation through `ets:reverse/3`:
    /// the last key, the end key, and the program reference or `None` for a
    /// new one.
    Chunk(Term, Term, Term, Option<Reference>),
    /// Trap to `ets:select/1` with the continuation of the last key, the
    /// end key, the matches, and their number.
    Trap(Term, Term, Term, i64, Option<Reference>),
}

/// Run the first turn of `ets:select/2,3`, `ets:select_reverse/2,3`,
/// `ets:match/2,3`, or `ets:match_object/2` on the `ordered_set` table
/// `table`, as `db_select_tree_common` and `db_select_chunk_tree_common` do
/// (`erl_db_tree.c:1439-1566`, `1810-1974`). A call without a limit walks
/// down and puts each match before the earlier ones, so the list is in key
/// order; `reverse` walks up. A call with a limit walks up, and `reverse`
/// walks down. A bound key reads only its object and charges no reduction.
///
/// # Errors
///
/// Return a runtime error when a comparison meets two alias references.
pub(super) fn first_turn(
    atoms: &AtomTable,
    table: &Table,
    spec: &MatchSpec,
    chunk: i64,
    order: SelectOrder,
) -> Result<(Turn, i64), BeamError> {
    let keypos = usize::try_from(table.keypos())
        .ok()
        .filter(|&keypos| keypos > 0)
        .unwrap_or(usize::MAX);
    let range = key_range(atoms, spec, keypos)?;
    let mut walk = Walk {
        atoms,
        spec,
        key: key_index(table),
        end: None,
        chunk,
        got: 0,
        accum: Term::nil(),
        max: TREE_ITERATIONS,
        last: None,
        counting: false,
    };
    match &range {
        KeyRange::Impossible => {
            let value = if chunk == 0 {
                Term::nil()
            } else {
                Term::static_atom("$end_of_table")
            };
            return Ok((Turn::Value(value), 0));
        }
        KeyRange::Bound(key) => {
            if let Some(object) = table.ordered_get(atoms, key)? {
                walk.visit(object, Direction::Backward)?;
            }
            let value = match (chunk, walk.accum) {
                (0, accum) => accum,
                (_, Term::Nil) => Term::static_atom("$end_of_table"),
                (_, accum) => Term::tuple(vec![accum, Term::static_atom("$end_of_table")]),
            };
            return Ok((Turn::Value(value), 0));
        }
        KeyRange::Partial { .. } | KeyRange::Unbound => {}
    }
    let direction = order.direction(chunk);
    let (after, end) = bounds(atoms, table, &range, direction)?;
    walk.end = end;
    walk.run(table, after.as_ref(), direction)?;
    let end = walk.end.unwrap_or(Term::Nil);
    let reductions = TREE_ITERATIONS - walk.max;
    let turn = chunk_end(chunk, walk.max, walk.got, walk.last, walk.accum, end, None);
    Ok((turn, reductions))
}

/// Return the end of a turn of a call with the limit `chunk`, or with no
/// limit for 0, as the tails of `db_select_chunk_tree_common` and
/// `db_select_tree_common` decide it (`erl_db_tree.c:1538-1561`,
/// `1910-1969`).
fn chunk_end(
    chunk: i64,
    max: i64,
    got: i64,
    last: Option<Term>,
    accum: Term,
    end: Term,
    program: Option<Reference>,
) -> Turn {
    if max > 0 || (chunk != 0 && got == chunk) {
        if chunk == 0 {
            return Turn::Value(accum);
        }
        return match last {
            Some(last) if got >= chunk => Turn::Chunk(accum, last, end, program),
            _ => Turn::Last(accum, got),
        };
    }
    let last = last.expect("a turn that read its objects has a last key");
    Turn::Trap(last, end, accum, got, program)
}

/// Return the result of a turn: the value, a trap to `ets:reverse/3`, or a
/// trap to `ets:select/1` with the continuation
/// `{Table, LastKey, EndKey, Limit, Program, Matches, Reverse, Got}`
/// (`erl_db_tree.c:1548-1561`, `1935-1969`). ERTS puts a magic reference
/// to the compiled specification in `Program`, and PBeam puts a new
/// reference there and keeps the compiled specification under it.
///
/// # Errors
///
/// Return a runtime error when the references of the run are used up.
pub(super) fn finish(
    context: &mut BifContext<'_>,
    table: &Term,
    spec: &MatchSpec,
    chunk: i64,
    order: SelectOrder,
    (turn, reductions): (Turn, i64),
) -> Result<BifResult, BeamError> {
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(i32::try_from(reductions).unwrap_or(i32::MAX));
    let program = |context: &mut BifContext<'_>, program: Option<Reference>| {
        if let Some(program) = program {
            return Ok(program);
        }
        let program = context.scheduler.make_magic_ref()?;
        (context.scheduler.ets.exclusive()).remember_program(program, spec.clone());
        Ok::<_, BeamError>(program)
    };
    let continuation = |last, end, program, accum, got| {
        Term::tuple(vec![
            table.clone(),
            last,
            end,
            Term::Integer(chunk),
            Term::Reference(program),
            accum,
            Term::Integer(i64::from(order == SelectOrder::SelectReverse)),
            Term::Integer(got),
        ])
    };
    Ok(match turn {
        Turn::Value(value) => Value(value),
        Turn::Last(_, 0) => Value(Term::static_atom("$end_of_table")),
        Turn::Last(accum, _) => reverse_trap(accum, Term::static_atom("$end_of_table")),
        Turn::Chunk(accum, last, end, reference) => {
            let reference = program(context, reference)?;
            reverse_trap(accum, continuation(last, end, reference, Term::nil(), 0))
        }
        Turn::Trap(last, end, accum, got, reference) => {
            let reference = program(context, reference)?;
            BifResult::TrapBif {
                module: "ets",
                function: "select",
                arguments: vec![continuation(last, end, reference, accum, got)],
            }
        }
    })
}

/// Return the trap to `ets:reverse/3` that reverses the matches `accum`
/// and returns them with `rest` (`erl_db_tree.c:1199-1245`).
fn reverse_trap(accum: Term, rest: Term) -> BifResult {
    BifResult::TrapBif {
        module: "ets",
        function: "reverse",
        arguments: vec![accum, Term::nil(), rest],
    }
}

/// The most list cells that one turn of `ets:reverse/3` reverses
/// (`CONTEXT_REDS * 10`, `erl_db_tree.c:1210`).
const REVERSE_ITERATIONS: i64 = 40_000;

/// Run the trap export `ets:reverse/3`: put the cells of `list` before
/// `result` in reverse order and return `{Result, Rest}`, as
/// `ets_select_reverse` does (`erl_db_tree.c:1199-1245`). A turn reverses
/// up to 39999 cells, charges one reduction for each 10 cells, and traps
/// again with the rest after the last cell. Only the select BIFs make
/// this trap, so `list` is a proper list.
///
/// # Examples
///
/// ```rust,ignore
/// let result = reverse(&mut context, &list, Term::nil(), rest);
/// ```
pub(super) fn reverse(
    context: &mut BifContext<'_>,
    list: &Term,
    result: &Term,
    rest: &Term,
) -> BifResult {
    let mut list = list.clone();
    let mut result = result.clone();
    let mut left = REVERSE_ITERATIONS;
    while let Some((head, tail)) = list.cons_cell() {
        left -= 1;
        if left == 0 {
            context
                .scheduler
                .current_process_mut()
                .bump_all_reductions();
            return BifResult::TrapBif {
                module: "ets",
                function: "reverse",
                arguments: vec![list, result, rest.clone()],
            };
        }
        result = Term::cons(head.clone(), result);
        list = tail.clone();
    }
    let reductions = (REVERSE_ITERATIONS / 10 - left / 10) as i32;
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(reductions);
    Value(Term::tuple(vec![result, rest.clone()]))
}

/// The fields of a continuation of a walk of an `ordered_set` table.
pub(super) struct Continuation {
    /// The key of the last object that the walk read.
    pub(super) last: Term,
    /// The end key, or `[]`.
    pub(super) end: Term,
    /// The limit, or 0.
    pub(super) chunk: i64,
    /// The program reference.
    pub(super) program: Reference,
    /// The matches so far.
    pub(super) accum: Term,
    /// Whether the walk is reversed.
    pub(super) order: SelectOrder,
    /// The number of matches so far.
    pub(super) got: i64,
}

impl Continuation {
    /// Read the continuation `{Table, LastKey, EndKey, Limit, Program,
    /// Matches, Reverse, Got}`, as `db_select_continue_tree_common` checks
    /// it (`erl_db_tree.c:1287-1302`): `None` for another term. The caller
    /// checks the program reference.
    pub(super) fn read(elements: &[Term]) -> Option<Continuation> {
        let [_, last, end, chunk, program, accum, reverse, got] = elements else {
            return None;
        };
        let (Some(chunk), Some(reverse), Some(got), Term::Reference(program)) = (
            chunk.small_integer(),
            reverse.small_integer(),
            got.small_integer(),
            program,
        ) else {
            return None;
        };
        if !(accum.is_nil() || accum.cons_cell().is_some()) {
            return None;
        }
        Some(Continuation {
            last: last.clone(),
            end: end.clone(),
            chunk,
            program: *program,
            accum: accum.clone(),
            order: if reverse == 0 {
                SelectOrder::Select
            } else {
                SelectOrder::SelectReverse
            },
            got,
        })
    }
}

/// Run a turn of `ets:select/1` or `ets:select_reverse/1` on an
/// `ordered_set` table, as `db_select_continue_tree_common` does
/// (`erl_db_tree.c:1263-1420`): walk from the key next to the last key,
/// with no end key, and then end at the end key.
///
/// # Errors
///
/// Return a runtime error when a comparison meets two alias references.
pub(super) fn next_turn(
    atoms: &AtomTable,
    table: &Table,
    spec: &MatchSpec,
    state: Continuation,
) -> Result<(Turn, i64), BeamError> {
    let mut walk = Walk {
        atoms,
        spec,
        key: key_index(table),
        end: None,
        chunk: state.chunk,
        got: state.got,
        accum: state.accum,
        max: TREE_ITERATIONS,
        last: None,
        counting: false,
    };
    let direction = state.order.direction(state.chunk);
    walk.run(table, Some(&state.last), direction)?;
    let reductions = TREE_ITERATIONS - walk.max;
    let (max, got, last, accum) = (walk.max, walk.got, walk.last, walk.accum);
    let turn = chunk_end(
        state.chunk,
        max,
        got,
        last,
        accum,
        state.end.clone(),
        Some(state.program),
    );
    let Turn::Trap(last, end, accum, got, program) = turn else {
        return Ok((turn, reductions));
    };
    // The walk down of a call without a limit ends below the end key, and
    // each other walk ends above it.
    let above = (state.chunk != 0) != (state.order == SelectOrder::SelectReverse);
    let past = !end.is_nil()
        && cmp_partly_bound(atoms, &end, &last)?
            == if above {
                Ordering::Less
            } else {
                Ordering::Greater
            };
    if past {
        return Ok((
            chunk_end(state.chunk, 1, got, None, accum, end, program),
            reductions,
        ));
    }
    Ok((Turn::Trap(last, end, accum, got, program), reductions))
}

/// The count, the reductions, and the trap position and end key of one
/// turn of a count.
pub(super) type CountTurn = (i64, i64, Option<(Term, Term)>);

/// Run one turn of `ets:select_count/2` on the `ordered_set` table
/// `table`, as `db_select_count_tree_common` and
/// `db_select_count_continue_tree` do (`erl_db_tree.c:1690-1790`): an
/// impossible key counts nothing, a bound key reads only its object and
/// charges no reduction, and another key walks down from the greatest key
/// of the range, or from the key below `resume.0` of a trapped count, until
/// the end key `resume.1`. `got` is the count of the earlier turns. Return
/// the count, the reductions of the turn, and the last key and the end key
/// when the turn read 1000 objects and the count traps.
///
/// # Errors
///
/// Return a runtime error when a comparison meets two alias references.
pub(super) fn count(
    atoms: &AtomTable,
    table: &Table,
    spec: &MatchSpec,
    resume: Option<(&Term, &Term)>,
    got: i64,
) -> Result<CountTurn, BeamError> {
    let (count, (_, reductions, trap)) =
        range_turn(atoms, table, spec, resume, got, RangeKind::Count)?;
    Ok((count, reductions.unwrap_or(0), trap))
}

/// The matches, the last match first, the reductions or `None` for an
/// impossible or bound key, and the trap position and end key of one turn
/// of `ets:select_replace/2` on an `ordered_set` table.
pub(super) type ReplaceTurn = (Term, Option<i64>, Option<(Term, Term)>);

/// Read one turn of `ets:select_replace/2` on the `ordered_set` table
/// `table` in the order of `db_select_replace_tree_common` and
/// `db_select_replace_continue_tree_common` (`erl_db_tree.c:2226-2450`):
/// an impossible key matches nothing, a bound key reads only its object,
/// and another key walks down from the greatest key of the range, or from
/// the key below `resume.0`, until the end key `resume.1` or 1000 objects.
/// The caller stores the matches.
///
/// # Errors
///
/// Return a runtime error when a comparison meets two alias references.
pub(super) fn replace(
    atoms: &AtomTable,
    table: &Table,
    spec: &MatchSpec,
    resume: Option<(&Term, &Term)>,
) -> Result<ReplaceTurn, BeamError> {
    let (_, turn) = range_turn(atoms, table, spec, resume, 0, RangeKind::Replace)?;
    Ok(turn)
}

/// Run one backward turn of a count (`counting`) or of a replace. Return
/// the count of a count and the matches of a replace.
fn range_turn(
    atoms: &AtomTable,
    table: &Table,
    spec: &MatchSpec,
    resume: Option<(&Term, &Term)>,
    got: i64,
    kind: RangeKind,
) -> Result<(i64, ReplaceTurn), BeamError> {
    let mut walk = Walk {
        atoms,
        spec,
        key: key_index(table),
        end: None,
        chunk: 0,
        got,
        accum: Term::nil(),
        max: TREE_ITERATIONS,
        last: None,
        counting: kind == RangeKind::Count,
    };
    let after = if let Some((last, end)) = resume {
        walk.end = (!end.is_nil()).then(|| end.clone());
        Some(last.clone())
    } else {
        let keypos = usize::try_from(table.keypos())
            .ok()
            .filter(|&keypos| keypos > 0)
            .unwrap_or(usize::MAX);
        let range = key_range(atoms, spec, keypos)?;
        match &range {
            KeyRange::Impossible => return Ok((0, (Term::nil(), None, None))),
            KeyRange::Bound(key) => {
                if let Some(object) = table.ordered_get(atoms, key)? {
                    walk.visit(object, Direction::Backward)?;
                }
                return Ok((walk.got, (walk.accum, None, None)));
            }
            KeyRange::Partial { .. } | KeyRange::Unbound => {
                let (after, end) = bounds(atoms, table, &range, Direction::Backward)?;
                walk.end = end;
                after
            }
        }
    };
    walk.run(table, after.as_ref(), Direction::Backward)?;
    let reductions = TREE_ITERATIONS - walk.max;
    let trap = (walk.max == 0).then(|| {
        (
            walk.last.clone().unwrap_or(Term::Nil),
            walk.end.clone().unwrap_or(Term::Nil),
        )
    });
    Ok((walk.got, (walk.accum, Some(reductions), trap)))
}

/// The end of a turn of a select delete.
pub(super) enum DeleteTurn {
    /// Return the number of deleted objects.
    Done(i64),
    /// Trap to `ets:select_delete_trap/1` with the last key, the least key
    /// or `[]`, and the number of deleted objects.
    Trap(Term, Term, i64),
}

/// Run the first turn of `ets:internal_select_delete/2` on the
/// `ordered_set` table `table`, as `db_select_delete_tree_common` does
/// (`erl_db_tree.c:2097-2215`, `4220-4248`): walk down from the most key of
/// the range, and delete each object whose result is `true`. A bound key
/// reads only its object and charges no reduction. Return the end of the
/// turn and its reductions.
///
/// # Errors
///
/// Return a runtime error when a comparison meets two alias references.
pub(super) fn select_delete(
    atoms: &AtomTable,
    table: &mut Table,
    spec: &MatchSpec,
) -> Result<(DeleteTurn, i64), BeamError> {
    let keypos = usize::try_from(table.keypos())
        .ok()
        .filter(|&keypos| keypos > 0)
        .unwrap_or(usize::MAX);
    let range = key_range(atoms, spec, keypos)?;
    match &range {
        KeyRange::Impossible => Ok((DeleteTurn::Done(0), 0)),
        KeyRange::Bound(bound) => {
            let key = key_index(table);
            let doomed = match table.ordered_get(atoms, bound)? {
                Some(object) if deletes(atoms, spec, object) => Some(object[key].clone()),
                _ => None,
            };
            let count = doomed.iter().count() as i64;
            if let Some(doomed) = doomed {
                table.delete(atoms, &doomed);
            }
            Ok((DeleteTurn::Done(count), 0))
        }
        KeyRange::Partial { .. } | KeyRange::Unbound => {
            let (after, end) = bounds(atoms, table, &range, Direction::Backward)?;
            delete_turn(
                atoms,
                table,
                spec,
                after.as_ref(),
                end.as_ref(),
                end.clone().unwrap_or(Term::Nil),
                0,
            )
        }
    }
}

/// Run a turn of `ets:select_delete_trap/1` from the key next to `last`,
/// with no end key, as `db_select_delete_continue_tree_common` does
/// (`erl_db_tree.c:1988-2083`).
///
/// # Errors
///
/// Return a runtime error when a comparison meets two alias references.
pub(super) fn select_delete_next(
    atoms: &AtomTable,
    table: &mut Table,
    spec: &MatchSpec,
    last: &Term,
    lower: Term,
    count: i64,
) -> Result<(DeleteTurn, i64), BeamError> {
    delete_turn(atoms, table, spec, Some(last), None, lower, count)
}

/// Return the result of a turn of a select delete: the number of deleted
/// objects, or a trap to `ets:select_delete_trap/1` with the fields of the
/// continuation `{Table, LastKey, LeastKey, Program, Count}`
/// (`erl_db_tree.c:2072-2079`, `2198-2210`). The first trap puts a new
/// reference in `Program` and keeps the compiled specification under it.
///
/// # Errors
///
/// Return a runtime error when the references of the run are used up.
pub(super) fn delete_finish(
    context: &mut BifContext<'_>,
    table: &Term,
    spec: &MatchSpec,
    program: Option<Reference>,
    (turn, reductions): (DeleteTurn, i64),
) -> Result<BifResult, BeamError> {
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(i32::try_from(reductions).unwrap_or(i32::MAX));
    let (last, lower, count) = match turn {
        DeleteTurn::Done(count) => return Ok(Value(Term::Integer(count))),
        DeleteTurn::Trap(last, lower, count) => (last, lower, count),
    };
    let program = if let Some(program) = program {
        program
    } else {
        let program = context.scheduler.make_magic_ref()?;
        (context.scheduler.ets.exclusive()).remember_program(program, spec.clone());
        program
    };
    Ok(BifResult::TrapBif {
        module: "ets",
        function: "select_delete_trap",
        arguments: vec![Term::tuple(vec![
            table.clone(),
            last,
            lower,
            Term::Reference(program),
            Term::Integer(count),
        ])],
    })
}

/// Return whether the specification `spec` gives `true` for `object`.
fn deletes(atoms: &AtomTable, spec: &MatchSpec, object: &[Term]) -> bool {
    spec.run(atoms, object) == Some(Term::static_atom("true"))
}

/// Walk down from the key next to `after` and delete each object whose
/// result is `true`, as `doit_select_delete` does
/// (`erl_db_tree.c:4220-4248`): stop below the end key `end`, or after 1000
/// objects. A turn that read 1000 objects traps, unless its last key is
/// below the lower key `lower`. Return the end of the turn and its
/// reductions.
fn delete_turn(
    atoms: &AtomTable,
    table: &mut Table,
    spec: &MatchSpec,
    after: Option<&Term>,
    end: Option<&Term>,
    lower: Term,
    count: i64,
) -> Result<(DeleteTurn, i64), BeamError> {
    let key = key_index(table);
    let mut max = TREE_ITERATIONS;
    let mut last = None;
    let mut doomed = Vec::new();
    for object in table.ordered_walk(atoms, after, Direction::Backward)? {
        last = Some(object[key].clone());
        if let Some(end) = end
            && cmp_partly_bound(atoms, end, &object[key])?.is_gt()
        {
            break;
        }
        if deletes(atoms, spec, object) {
            doomed.push(object[key].clone());
        }
        max -= 1;
        if max == 0 {
            break;
        }
    }
    for key in &doomed {
        table.delete(atoms, key);
    }
    let count = count + doomed.len() as i64;
    let reductions = TREE_ITERATIONS - max;
    if max > 0 {
        return Ok((DeleteTurn::Done(count), reductions));
    }
    let last = last.expect("a turn that read its objects has a last key");
    if !lower.is_nil() && cmp_partly_bound(atoms, &lower, &last)?.is_gt() {
        return Ok((DeleteTurn::Done(count), reductions));
    }
    Ok((DeleteTurn::Trap(last, lower, count), reductions))
}
