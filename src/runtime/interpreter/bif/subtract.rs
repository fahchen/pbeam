//! The body of `erlang:'--'/2`, as ERTS `subtract` runs it in stages that
//! can each trap (`erl_bif_lists.c:340-1083`).

use super::BifResult::Value;
use super::*;
use std::cmp::Ordering;

/// The most elements of a list that a naive scan keeps in its array
/// (`SUBTRACT_LHS_THRESHOLD`, `SUBTRACT_RHS_THRESHOLD`,
/// `erl_bif_lists.c:367-368`).
const NAIVE_THRESHOLD: usize = 16;

/// The stage of a subtraction (`ErtsSubtractCtxStage`,
/// `erl_bif_lists.c:370-388`). The trap state keeps it as an integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Count the elements of the left list.
    LengthLeft = 0,
    /// Remove each element of the right list from the at most 16 elements
    /// of the left list.
    NaiveLeft = 1,
    /// Count the elements of the right list.
    LengthRight = 2,
    /// Keep each element of the left list that the at most 16 elements of
    /// the right list do not remove.
    NaiveRight = 3,
    /// Put each element of the right list into a multiset.
    SetBuild = 4,
    /// Keep each element of the left list that the multiset does not
    /// remove.
    SetFinish = 5,
}

impl Stage {
    /// Return the stage of its integer in the trap state.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Stage::from_index(5), Stage::SetFinish);
    /// ```
    ///
    /// # Panics
    ///
    /// Panic for an integer that is not a stage: only
    /// [`Subtract::into_terms`] makes the trap state.
    fn from_index(index: &Term) -> Self {
        const STAGES: [Stage; 6] = [
            Stage::LengthLeft,
            Stage::NaiveLeft,
            Stage::LengthRight,
            Stage::NaiveRight,
            Stage::SetBuild,
            Stage::SetFinish,
        ];
        STAGES[count(index)]
    }
}

/// Return the count of a non-negative small integer of the trap state.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(count(&Term::Integer(3)), 3);
/// ```
///
/// # Panics
///
/// Panic for another term: only [`Subtract::into_terms`] makes the trap
/// state.
fn count(term: &Term) -> usize {
    term.small_integer()
        .and_then(|value| usize::try_from(value).ok())
        .expect("the trap state of erlang:'--'/2 holds a count")
}

/// Return the elements of a proper list of the trap state.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(elements(&Term::nil()), Vec::new());
/// ```
///
/// # Panics
///
/// Panic for another term: only [`Subtract::into_terms`] makes the trap
/// state.
fn elements(term: &Term) -> Vec<Term> {
    term.proper_list()
        .expect("the trap state of erlang:'--'/2 holds a proper list")
        .cloned()
        .collect()
}

/// The state of one subtraction across its traps, as `ErtsSubtractContext`
/// (`erl_bif_lists.c:400-427`).
#[derive(Debug)]
struct Subtract {
    /// The stage that runs next.
    stage: Stage,
    /// The left argument.
    left: Term,
    /// The right argument.
    right: Term,
    /// The next cons cell of the list that the stage walks, or the tail
    /// after its last cell.
    iterator: Term,
    /// The number of elements of the left list that the length stage
    /// counted. ERTS also counts it down in the later stages to size its
    /// heap allocations, which PBeam does not need.
    left_remaining: usize,
    /// The elements of the right list that the stages did not count or
    /// use yet.
    right_remaining: usize,
    /// The array of a naive stage: the left elements that remain in
    /// [`Stage::NaiveLeft`], or the right elements that remain in
    /// [`Stage::NaiveRight`], in the order of the ERTS array.
    elements: Vec<Term>,
    /// The multiset of the set stages: each distinct right element and its
    /// count, sorted in the term order, as the red-black tree of
    /// `rhs_set` (`erl_bif_lists.c:418-425`) keeps it.
    set: Vec<(Term, usize)>,
    /// The kept elements of the left list so far, in order.
    result: Vec<Term>,
}

/// The end of one stage call.
enum Step {
    /// The stage ended, and the subtraction continues with the next stage.
    Next,
    /// The budget ended before the stage: the BIF traps.
    Trap,
    /// A list is improper: the BIF raises `badarg`.
    Badarg,
    /// The subtraction ended with this result.
    Done(Term),
}

/// Run `erlang:'--'(Left, Right)`: return the elements of `Left` without
/// the first occurrence of each element of `Right`, as ERTS `subtract`
/// (`erl_bif_lists.c:1002-1083`) does. A term that is not a list raises
/// `badarg`. The stages walk the lists in the order of `subtract_continue`
/// (`erl_bif_lists.c:931-1000`): they count the left list, and then use a
/// naive scan when it has at most 16 elements and `Right` is not `[]`; else
/// they count the right list, and use a naive scan when it has at most 16
/// elements, or else a multiset of its elements. An improper list raises
/// `badarg` when a stage reaches its tail. Each stage reads the budget of
/// the process when it starts and bumps the reductions of its work, as
/// [`Subtract::length`], [`Subtract::naive_left`],
/// [`Subtract::naive_right`], [`Subtract::set_build`], and
/// [`Subtract::set_finish`] state. When the budget ends before the work,
/// the BIF traps to its export with the state (see [`continue_trap`]). PBeam
/// terms have no float, so the term order of the set stages and the exact
/// equality of the naive stages agree.
///
/// # Examples
///
/// ```rust,ignore
/// let list = |values: &[i64]| Term::list_from(values.iter().map(|value| Term::Integer(*value)));
/// let result = subtract(&mut process, &list(&[1, 2, 1, 3]), &list(&[1, 3]))?;
/// assert!(matches!(result, BifResult::Value(value) if value == list(&[2, 1])));
/// assert!(matches!(subtract(&mut process, &Term::Integer(1), &Term::nil())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// The runtime error of [`Term::compare`] for two different references in
/// a set stage, and the runtime error of
/// [`ProcessState::bif_reductions_left`] for a negative budget.
pub(super) fn subtract(
    process: &mut ProcessState,
    atoms: &AtomTable,
    left: &Term,
    right: &Term,
) -> Result<BifResult, BeamError> {
    if !left.is_list() || !right.is_list() {
        return Ok(BifResult::badarg());
    }
    let state = Subtract {
        stage: Stage::LengthLeft,
        left: left.clone(),
        right: right.clone(),
        iterator: left.clone(),
        left_remaining: 0,
        right_remaining: 0,
        elements: Vec::new(),
        set: Vec::new(),
        result: Vec::new(),
    };
    state.run(process, atoms)
}

/// Continue a subtraction from the trap state `arguments` that
/// [`subtract`] gave its export. ERTS traps to `erlang:'--'/2` with a magic
/// reference to the state (`erl_bif_lists.c:1015-1024`, `1040-1058`).
///
/// # Examples
///
/// ```rust,ignore
/// let BifResult::TrapBif { arguments, .. } = subtract(&mut process, &long, &other)? else { unreachable!() };
/// process.fcalls = 4000;
/// let result = continue_trap(&mut process, &arguments)?;
/// ```
///
/// # Errors
///
/// The errors of [`subtract`].
///
/// # Panics
///
/// Panic for a trap state that [`subtract`] did not make.
pub(super) fn continue_trap(
    process: &mut ProcessState,
    atoms: &AtomTable,
    arguments: &[Term; 10],
) -> Result<BifResult, BeamError> {
    Subtract::from_terms(arguments).run(process, atoms)
}

impl Subtract {
    /// Run the stages until the subtraction ends, raises `badarg`, or
    /// traps, as `subtract_continue` (`erl_bif_lists.c:931-1000`) does.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let result = state.run(&mut process)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The errors of [`subtract`].
    fn run(
        mut self,
        process: &mut ProcessState,
        atoms: &AtomTable,
    ) -> Result<BifResult, BeamError> {
        loop {
            let step = match self.stage {
                Stage::LengthLeft => self.length(process, true)?,
                Stage::NaiveLeft => self.naive_left(process)?,
                Stage::LengthRight => self.length(process, false)?,
                Stage::NaiveRight => self.naive_right(process)?,
                Stage::SetBuild => self.set_build(process, atoms)?,
                Stage::SetFinish => self.set_finish(process, atoms)?,
            };
            match step {
                Step::Next => {}
                Step::Trap => {
                    return Ok(BifResult::TrapBif {
                        module: "erlang",
                        function: "--",
                        arguments: self.into_terms(),
                    });
                }
                Step::Badarg => return Ok(BifResult::badarg()),
                Step::Done(result) => return Ok(Value(result)),
            }
        }
    }

    /// Count the next elements of the left list (`left` is `true`) or of the
    /// right list, as `subtract_get_length` (`erl_bif_lists.c:574-605`)
    /// does: up to 32 elements for each reduction left. A tail that is not
    /// `[]` raises `badarg` with no bump. Else the stage bumps one reduction
    /// for each 32 elements, and traps when the list continues. At the end
    /// of the left list, the next stage is [`Stage::NaiveLeft`] for 1 to 16
    /// elements and a right list that is not `[]`, else
    /// [`Stage::LengthRight`]. At the end of the right list, an empty list
    /// gives the result at once, and the next stage is
    /// [`Stage::NaiveRight`] for at most 16 right elements, else
    /// [`Stage::SetBuild`] (`erl_bif_lists.c:937-991`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let step = state.length(&mut process, true)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The runtime error of [`ProcessState::bif_reductions_left`].
    fn length(&mut self, process: &mut ProcessState, left: bool) -> Result<Step, BeamError> {
        const ELEMENTS_PER_RED: usize = 32;
        let budget = ELEMENTS_PER_RED * process.bif_reductions_left()?;
        let mut elements = self.iterator.list_iter().expect("a stage walks a list");
        let count = elements.by_ref().take(budget).count();
        let rest = elements.rest().clone();
        if !rest.is_list() {
            return Ok(Step::Badarg);
        }
        process.bump_reductions((count / ELEMENTS_PER_RED) as i32);
        self.iterator = rest;
        if left {
            self.left_remaining += count;
        } else {
            self.right_remaining += count;
        }
        if self.iterator.is_nonempty_list() {
            return Ok(Step::Trap);
        }
        if left {
            if (1..=NAIVE_THRESHOLD).contains(&self.left_remaining) && self.right.is_nonempty_list()
            {
                self.stage = Stage::NaiveLeft;
                self.elements = self.left.list_iter().expect("counted").cloned().collect();
                self.iterator = self.right.clone();
            } else {
                self.stage = Stage::LengthRight;
                self.iterator = self.right.clone();
            }
            return Ok(Step::Next);
        }
        if self.left_remaining == 0 {
            return Ok(Step::Done(Term::nil()));
        }
        if self.right_remaining == 0 {
            return Ok(Step::Done(self.left.clone()));
        }
        if self.right_remaining <= NAIVE_THRESHOLD {
            self.stage = Stage::NaiveRight;
            self.elements = self.right.list_iter().expect("counted").cloned().collect();
            self.iterator = self.left.clone();
        } else {
            self.stage = Stage::SetBuild;
            self.iterator = self.right.clone();
        }
        Ok(Step::Next)
    }

    /// Remove the next elements of the right list from the left elements,
    /// as `subtract_naive_lhs` (`erl_bif_lists.c:632-695`) does. Each right
    /// element removes the first equal left element, and costs the number of
    /// left elements that remain, at least 1, of a budget of 16 for each
    /// reduction left. The stage bumps the cost, at most the budget, divided
    /// by 16. It traps when the right list continues, and raises `badarg` at
    /// a tail that is not `[]`. Else the result is the left elements that
    /// remain.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let step = state.naive_left(&mut process)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The runtime error of [`ProcessState::bif_reductions_left`].
    fn naive_left(&mut self, process: &mut ProcessState) -> Result<Step, BeamError> {
        const CHECKS_PER_RED: usize = 16;
        let budget = CHECKS_PER_RED * process.bif_reductions_left()?;
        let mut checks = 0;
        while checks < budget
            && let Some((value, next)) = self.iterator.cons_cell()
        {
            if let Some(found) = self.elements.iter().position(|element| element == value) {
                self.elements.remove(found);
            }
            checks += self.elements.len().max(1);
            self.iterator = next.clone();
        }
        process.bump_reductions((checks.min(budget) / CHECKS_PER_RED) as i32);
        if self.iterator.is_nonempty_list() {
            return Ok(Step::Trap);
        }
        if !self.iterator.is_nil() {
            return Ok(Step::Badarg);
        }
        Ok(Step::Done(Term::list_from(std::mem::take(
            &mut self.elements,
        ))))
    }

    /// Keep the next elements of the left list that the right elements do
    /// not remove, as `subtract_naive_rhs` (`erl_bif_lists.c:723-796`) does.
    /// A left element that equals a right element uses that right element:
    /// the last right element takes its place in the array. When no right
    /// element remains, the rest of the left list is the tail of the result.
    /// Each left element costs the number of right elements that remain
    /// after it, of a budget of 16 for each reduction left, and the stage
    /// bumps the cost, at most the budget, divided by 16. It traps when the
    /// left list continues.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let step = state.naive_right(&mut process)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The runtime error of [`ProcessState::bif_reductions_left`].
    fn naive_right(&mut self, process: &mut ProcessState) -> Result<Step, BeamError> {
        const CHECKS_PER_RED: usize = 16;
        let budget = CHECKS_PER_RED * process.bif_reductions_left()?;
        let mut checks = 0;
        while checks < budget
            && let Some((value, next)) = self.iterator.cons_cell()
        {
            match self.elements.iter().rposition(|element| element == value) {
                None => self.result.push(value.clone()),
                Some(found) => {
                    if self.elements.len() == 1 {
                        process.bump_reductions((checks.min(budget) / CHECKS_PER_RED) as i32);
                        let kept = std::mem::take(&mut self.result);
                        return Ok(Step::Done(Term::list_with_tail(kept, next.clone())));
                    }
                    self.elements.swap_remove(found);
                }
            }
            checks += self.elements.len();
            self.iterator = next.clone();
        }
        process.bump_reductions((checks.min(budget) / CHECKS_PER_RED) as i32);
        if self.iterator.is_nonempty_list() {
            return Ok(Step::Trap);
        }
        Ok(Step::Done(Term::list_from(std::mem::take(
            &mut self.result,
        ))))
    }

    /// Put the next elements of the right list into the multiset, as
    /// `subtract_set_build` (`erl_bif_lists.c:813-856`) does: up to 16
    /// insertions for each reduction left, and one reduction bumped for each
    /// 16 insertions. An element that compares equal to an element of the
    /// set adds 1 to its count. The stage traps when the right list
    /// continues, else the next stage is [`Stage::SetFinish`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let step = state.set_build(&mut process)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The runtime error of [`Term::compare`] and of
    /// [`ProcessState::bif_reductions_left`].
    fn set_build(
        &mut self,
        process: &mut ProcessState,
        atoms: &AtomTable,
    ) -> Result<Step, BeamError> {
        const INSERTIONS_PER_RED: usize = 16;
        let budget = INSERTIONS_PER_RED * process.bif_reductions_left()?;
        let mut insertions = 0;
        while insertions < budget
            && let Some((value, next)) = self.iterator.cons_cell()
        {
            match self.find(value, atoms)? {
                Ok(found) => self.set[found].1 += 1,
                Err(position) => self.set.insert(position, (value.clone(), 1)),
            }
            self.iterator = next.clone();
            insertions += 1;
        }
        process.bump_reductions((insertions / INSERTIONS_PER_RED) as i32);
        if self.iterator.is_nonempty_list() {
            return Ok(Step::Trap);
        }
        self.stage = Stage::SetFinish;
        self.iterator = self.left.clone();
        Ok(Step::Next)
    }

    /// Keep the next elements of the left list that the multiset does not
    /// remove, as `subtract_set_finish` (`erl_bif_lists.c:869-929`) does. A
    /// left element that compares equal to an element of the set takes 1
    /// from its count. When no right element remains, the rest of the left
    /// list is the tail of the result. Each left element is one check of a
    /// budget of 8 for each reduction left, and the stage bumps one reduction
    /// for each 8 checks. It traps when the left list continues.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let step = state.set_finish(&mut process)?;
    /// ```
    ///
    /// # Errors
    ///
    /// The runtime error of [`Term::compare`] and of
    /// [`ProcessState::bif_reductions_left`].
    fn set_finish(
        &mut self,
        process: &mut ProcessState,
        atoms: &AtomTable,
    ) -> Result<Step, BeamError> {
        const CHECKS_PER_RED: usize = 8;
        let budget = CHECKS_PER_RED * process.bif_reductions_left()?;
        let mut checks = 0;
        while checks < budget
            && let Some((value, next)) = self.iterator.cons_cell()
        {
            match self.find(value, atoms)? {
                Err(_) => self.result.push(value.clone()),
                Ok(found) => {
                    if self.right_remaining == 1 {
                        process.bump_reductions((checks / CHECKS_PER_RED) as i32);
                        let kept = std::mem::take(&mut self.result);
                        return Ok(Step::Done(Term::list_with_tail(kept, next.clone())));
                    }
                    self.right_remaining -= 1;
                    if self.set[found].1 == 1 {
                        self.set.remove(found);
                    } else {
                        self.set[found].1 -= 1;
                    }
                }
            }
            self.iterator = next.clone();
            checks += 1;
        }
        process.bump_reductions((checks / CHECKS_PER_RED) as i32);
        if self.iterator.is_nonempty_list() {
            return Ok(Step::Trap);
        }
        Ok(Step::Done(Term::list_from(std::mem::take(
            &mut self.result,
        ))))
    }

    /// Find `value` in the sorted multiset: `Ok` with its index, or `Err`
    /// with the index where it goes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(state.find(&Term::Integer(1))?.is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// The runtime error of [`Term::compare`].
    fn find(&self, value: &Term, atoms: &AtomTable) -> Result<Result<usize, usize>, BeamError> {
        let (mut low, mut high) = (0, self.set.len());
        while low < high {
            let middle = (low + high) / 2;
            match self.set[middle].0.compare(value, atoms)? {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => return Ok(Ok(middle)),
            }
        }
        Ok(Err(low))
    }

    /// Return the trap state: the stage, the arguments, the iterator, the
    /// two counts, the naive array, the elements and the counts of the
    /// multiset, and the result so far.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let terms = state.into_terms();
    /// let state = Subtract::from_terms(&terms.try_into().unwrap());
    /// ```
    fn into_terms(self) -> Vec<Term> {
        let (set, counts): (Vec<_>, Vec<_>) = self
            .set
            .into_iter()
            .map(|(element, count)| (element, Term::Integer(count as i64)))
            .unzip();
        vec![
            Term::Integer(self.stage as i64),
            self.left,
            self.right,
            self.iterator,
            Term::Integer(self.left_remaining as i64),
            Term::Integer(self.right_remaining as i64),
            Term::list_from(self.elements),
            Term::list_from(set),
            Term::list_from(counts),
            Term::list_from(self.result),
        ]
    }

    /// Return the state of a trap state that [`Subtract::into_terms`] made.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let state = Subtract::from_terms(&terms);
    /// ```
    ///
    /// # Panics
    ///
    /// Panic for a trap state that [`Subtract::into_terms`] did not make.
    fn from_terms(terms: &[Term; 10]) -> Self {
        let [
            stage,
            left,
            right,
            iterator,
            left_remaining,
            right_remaining,
            naive,
            set,
            counts,
            result,
        ] = terms;
        Subtract {
            stage: Stage::from_index(stage),
            left: left.clone(),
            right: right.clone(),
            iterator: iterator.clone(),
            left_remaining: count(left_remaining),
            right_remaining: count(right_remaining),
            elements: elements(naive),
            set: elements(set)
                .into_iter()
                .zip(elements(counts).iter().map(count))
                .collect(),
            result: elements(result),
        }
    }
}
