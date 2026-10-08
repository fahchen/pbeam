//! Bodies of the `lists` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;
use crate::runtime::scheduler::CONTEXT_REDS;

/// Run `lists:keyfind/3`: return the first tuple in `list` whose element at
/// the 1-based `position` equals `key`, or `false`, as ERTS `keyfind` does
/// (`erl_bif_lists.c:1294-1381`). Elements that are not tuples, and tuples
/// with fewer than `position` elements, do not match. ERTS compares numbers
/// with `==`; PBeam has only integers, so exact equality gives the same
/// result. An improper `list` gives a tuple that comes before its tail, and
/// raises `badarg` when no tuple matches. A `position` that is not a small
/// integer or is below 1, and a `list` that is not a list, raise `badarg`.
///
/// The BIF costs no reduction. After `10 * CONTEXT_REDS` elements with no
/// match, it uses all reductions and traps to its export with the rest of
/// the list (`erl_bif_lists.c:1304`, `1340-1344`).
///
/// # Examples
///
/// ```rust,ignore
/// let entry = Term::Tuple(vec![Term::static_atom("ok"), Term::Integer(1)]);
/// let found = keyfind(&mut process, &Term::static_atom("ok"), &Term::Integer(1), &Term::List(vec![entry.clone()]));
/// assert!(matches!(found, BifResult::Value(term) if term == entry));
/// let missing = keyfind(&mut process, &Term::Integer(1), &Term::Integer(1), &Term::List(vec![]));
/// assert!(matches!(missing, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// assert!(matches!(keyfind(&mut process, &Term::Integer(1), &Term::Integer(0), &Term::List(vec![])), BifResult::Raise { .. }));
/// let improper = Term::ImproperList(vec![Term::Integer(1)], Box::new(Term::static_atom("x")));
/// assert!(matches!(keyfind(&mut process, &Term::Integer(1), &Term::Integer(1), &improper), BifResult::Raise { .. }));
/// ```
pub(super) fn keyfind(
    process: &mut ProcessState,
    key: &Term,
    position: &Term,
    list: &Term,
) -> BifResult {
    key_search(process, "keyfind", key, position, list)
}

/// Run `lists:keymember/3`: return `true` when [`keyfind`] finds a tuple,
/// else the result of [`keyfind`], as ERTS `lists_keymember_3` does
/// (`erl_bif_lists.c:1265-1277`). Thus `false` comes when no tuple matches,
/// the arguments that [`keyfind`] does not accept raise `badarg`, and a
/// trap goes to the export of `lists:keymember/3`.
///
/// # Examples
///
/// ```rust,ignore
/// let entry = Term::Tuple(vec![Term::static_atom("ok"), Term::Integer(1)]);
/// let found = keymember(&mut process, &Term::static_atom("ok"), &Term::Integer(1), &Term::List(vec![entry]));
/// assert!(matches!(found, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let missing = keymember(&mut process, &Term::Integer(1), &Term::Integer(1), &Term::List(vec![]));
/// assert!(matches!(missing, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// assert!(matches!(keymember(&mut process, &Term::Integer(1), &Term::Integer(0), &Term::List(vec![])), BifResult::Raise { .. }));
/// ```
pub(super) fn keymember(
    process: &mut ProcessState,
    key: &Term,
    position: &Term,
    list: &Term,
) -> BifResult {
    match key_search(process, "keymember", key, position, list) {
        Value(Term::Tuple(_)) => Value(Term::static_atom("true")),
        result => result,
    }
}

/// Search `list` for a tuple with `key` at `position`, as ERTS `keyfind`
/// does for the export `lists:function/3` that it traps to
/// (`erl_bif_lists.c:1301-1381`).
///
/// # Examples
///
/// ```rust,ignore
/// let missing = key_search(&mut process, "keyfind", &Term::Integer(1), &Term::Integer(1), &Term::List(vec![]));
/// assert!(matches!(missing, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// ```
fn key_search(
    process: &mut ProcessState,
    function: &'static str,
    key: &Term,
    position: &Term,
    list: &Term,
) -> BifResult {
    let index = position
        .small_integer()
        .and_then(|position| usize::try_from(position).ok())
        .and_then(|position| position.checked_sub(1));
    let (Some(index), Some((elements, tail))) = (index, list_cells(list)) else {
        return BifResult::badarg();
    };
    let max_iter = 10 * CONTEXT_REDS as usize;
    let chunk = &elements[..elements.len().min(max_iter)];
    let found = chunk.iter().find(
        |element| matches!(element, Term::Tuple(elements) if elements.get(index) == Some(key)),
    );
    match (found, tail) {
        (Some(found), _) => Value(found.clone()),
        (None, _) if chunk.len() < elements.len() => {
            process.bump_all_reductions();
            BifResult::TrapBif {
                module: "lists",
                function,
                arguments: vec![
                    key.clone(),
                    position.clone(),
                    list_rest(&elements[max_iter..], tail),
                ],
            }
        }
        (None, None) => Value(Term::static_atom("false")),
        (None, Some(_)) => BifResult::badarg(),
    }
}

/// Run `lists:member/2`: return `true` when an element of `list` is
/// exactly equal to `element`, else `false`, as ERTS `lists_member_2`
/// (`erl_bif_lists.c:1108-1142`) does. An improper `list` gives `true` for
/// an element that comes before its tail, and raises `badarg` when no
/// element matches. A `list` that is not a list raises `badarg`.
///
/// A chunk takes at most 16 elements for each reduction of the budget. The
/// BIF bumps one reduction for each 16 elements that it compared, rounded
/// up. When the chunk ends with no match and elements remain, it uses all
/// reductions and traps to its export with the rest of the list.
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("x");
/// let improper = Term::ImproperList(vec![Term::Integer(1)], Box::new(x.clone()));
/// assert!(matches!(member(&mut process, &Term::Integer(1), &improper)?, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// assert!(matches!(member(&mut process, &Term::Integer(2), &Term::List(vec![]))?, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// assert!(matches!(member(&mut process, &Term::Integer(2), &improper)?, BifResult::Raise { .. }));
/// assert!(matches!(member(&mut process, &Term::Integer(2), &x)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn member(
    process: &mut ProcessState,
    element: &Term,
    list: &Term,
) -> Result<BifResult, BeamError> {
    let Some((elements, tail)) = list_cells(list) else {
        return Ok(BifResult::badarg());
    };
    let reds_left = process.bif_reductions_left()?;
    let max_iter = 16 * reds_left;
    let chunk = &elements[..elements.len().min(max_iter)];
    // `BIF_RET2` and `BUMP_REDS` with `reds_left - max_iter / 16`, where
    // `max_iter` counts down for each compared element.
    let cost = |compared: usize| (reds_left - (max_iter - compared) / 16) as i32;
    if let Some(index) = chunk.iter().position(|item| item == element) {
        process.bump_reductions(cost(index + 1));
        return Ok(Value(Term::static_atom("true")));
    }
    if chunk.len() < elements.len() {
        process.bump_all_reductions();
        return Ok(BifResult::TrapBif {
            module: "lists",
            function: "member",
            arguments: vec![element.clone(), list_rest(&elements[max_iter..], tail)],
        });
    }
    process.bump_reductions(cost(chunk.len()));
    Ok(match tail {
        Some(_) => BifResult::badarg(),
        None => Value(Term::static_atom("false")),
    })
}

/// Run `lists:reverse/2`: return the elements of `list` in reverse order,
/// followed by `tail`, as ERTS `lists_reverse_2`
/// (`erl_bif_lists.c:1246-1263`) does. `tail` can be any term, so
/// `lists:reverse([a], x)` gives the improper list `[a|x]`, and an empty
/// `list` gives `tail` itself with no cost. A `list` that is not a proper
/// list raises `badarg`.
///
/// As `lists_reverse_onheap` (`erl_bif_lists.c:1194-1244`), a chunk takes at
/// most 60 elements for each reduction of the budget and bumps one
/// reduction for each 60 elements that it took. When elements remain, it
/// uses all reductions and traps to its export with the rest of the list
/// and the reversed part as the new tail. ERTS reverses on the heap only
/// while the heap has free words, and else continues in
/// `lists_reverse_alloc` with 40 elements for each reduction
/// (`erl_bif_lists.c:1144-1192`, `1235-1237`). PBeam has no process heap
/// and always uses the cost of the heap copy (see "Garbage collection
/// reductions" in `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("x");
/// assert!(matches!(reverse(&mut process, &Term::List(vec![]), &x)?, BifResult::Value(term) if term == x));
/// let list = Term::List(vec![Term::Integer(1), Term::Integer(2)]);
/// let reversed = reverse(&mut process, &list, &Term::List(vec![Term::Integer(3)]))?;
/// assert!(matches!(reversed, BifResult::Value(Term::List(elements)) if elements == [2, 1, 3].map(Term::Integer)));
/// assert!(matches!(reverse(&mut process, &list, &x)?, BifResult::Value(Term::ImproperList(..))));
/// assert!(matches!(reverse(&mut process, &x, &Term::List(vec![]))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`ProcessState::bif_reductions_left`] for a
/// negative budget.
pub(super) fn reverse(
    process: &mut ProcessState,
    list: &Term,
    tail: &Term,
) -> Result<BifResult, BeamError> {
    let Some((elements, list_tail)) = list_cells(list) else {
        return Ok(BifResult::badarg());
    };
    let max_cells = 60 * process.bif_reductions_left()?;
    let walked = elements.len().min(max_cells);
    process.bump_reductions((walked / 60) as i32);
    let reversed = Term::list_with_tail(
        elements[..walked].iter().rev().cloned().collect(),
        tail.clone(),
    );
    if walked < elements.len() {
        process.bump_all_reductions();
        return Ok(BifResult::TrapBif {
            module: "lists",
            function: "reverse",
            arguments: vec![list_rest(&elements[walked..], list_tail), reversed],
        });
    }
    Ok(match list_tail {
        Some(_) => BifResult::badarg(),
        None => Value(reversed),
    })
}
