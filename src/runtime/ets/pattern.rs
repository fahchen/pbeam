//! Match patterns of the ETS match BIFs. `ets:match/2,3` makes the match
//! specification `[{Pattern, [], ['$$']}]` (`erl_db.c:3771-3818`), and
//! `db_match_compile` (`erl_db_util.c:1700-2100`, `4060-4180`) compiles its
//! head. PBeam compiles only such a head: a pattern with no guard and the
//! body `['$$']`.

use crate::term::{AtomTable, Term};

/// A compiled match pattern: the pattern, its flat code, and the number of
/// its variables. As `db_match_compile` (`erl_db_util.c:1700-2100`) does,
/// the compile and the match use a work stack and no recursion, so a pattern
/// of any depth compiles, matches, and drops.
#[derive(Debug)]
pub(in crate::runtime) struct Pattern {
    /// The whole pattern.
    term: Term,
    /// One step for each part of the pattern, in the order of a depth-first
    /// walk from the left.
    code: Vec<Step>,
    /// For a tuple pattern, whether each element has no variable, no `'_'`,
    /// and no map (`db_is_fully_bound`, `erl_db_util.c:3962-4000`).
    ground: Vec<bool>,
    /// The number of different variables of the pattern.
    variables: usize,
}

/// One step of a compiled pattern, as one `dmc_one_term` instruction
/// (`erl_db_util.c:4060-4180`) matches it. Each step matches the term at the
/// top of the work stack, and a step of a tuple, a cons cell, or a map puts
/// the parts of the term on the stack for the next steps.
#[derive(Debug)]
enum Step {
    /// `'_'` (`matchSkip`): each term matches.
    Skip,
    /// The first place of a variable (`matchBind`): each term matches, and
    /// the term becomes the value of the variable with this index. The
    /// index is the place of the variable number among the variable numbers
    /// of the pattern.
    Bind(usize),
    /// A later place of a variable (`matchCmp`): the term must be exactly
    /// equal to the value of the variable with this index.
    Compare(usize),
    /// A term that is not a tuple, a list cell, a map, or a variable
    /// (`matchEq` and the other constant instructions): the term must be
    /// exactly equal to it.
    Constant(Term),
    /// A tuple of this size (`matchPushT`). Its elements come next.
    Tuple(usize),
    /// A cons cell (`matchPushL`). Its head comes next, then its tail.
    Cons,
    /// A map that has each of these keys (`matchPushM` and `matchKey`). The
    /// map can have other keys. The values of the keys come next.
    Map(Vec<Term>),
}

impl Pattern {
    /// Compile `pattern`, as `db_match_compile` compiles a match head. An
    /// atom `'$N'`, where `N` is `0` or a decimal number with no leading
    /// zero, is a variable (`db_is_variable`, `erl_db_util.c:3896-3927`),
    /// and `'_'` matches each term. Each other atom, also `'$_'`, `'$$'`, and
    /// `'$01'`, is a constant. Return `None` for a map key that is a
    /// variable or `'_'` (`erl_db_util.c:1786-1866`), which makes the BIF
    /// raise `badarg`. `atoms` owns the name of each atom of the pattern.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let pattern = Pattern::compile(&atoms, &Term::tuple(vec![atom("$2"), atom("$1")])).unwrap();
    /// assert_eq!(pattern.run(&[Term::Integer(1), Term::Integer(2)]), Some(Term::list_from([Term::Integer(2), Term::Integer(1)])));
    /// assert!(Pattern::compile(&atoms, &map_with_key(atom("_"))).is_none());
    /// ```
    pub(in crate::runtime) fn compile(atoms: &AtomTable, pattern: &Term) -> Option<Pattern> {
        let mut numbers = Vec::new();
        collect_variables(atoms, pattern, &mut numbers);
        numbers.sort_unstable();
        numbers.dedup();
        let mut bound = vec![false; numbers.len()];
        let mut code = Vec::new();
        let mut stack = vec![pattern];
        while let Some(term) = stack.pop() {
            if is_skip(atoms, term) {
                code.push(Step::Skip);
                continue;
            }
            if let Some(number) = variable(atoms, term) {
                let index = numbers
                    .binary_search(&number)
                    .expect("the walk collected each variable");
                code.push(if bound[index] {
                    Step::Compare(index)
                } else {
                    Step::Bind(index)
                });
                bound[index] = true;
                continue;
            }
            match term {
                Term::Tuple(elements) => {
                    code.push(Step::Tuple(elements.len()));
                    stack.extend(elements.iter().rev());
                }
                Term::Cons(_) => {
                    let (head, tail) = term.cons_cell().expect("a cons term is a cell");
                    code.push(Step::Cons);
                    stack.push(tail);
                    stack.push(head);
                }
                Term::Map(map) => {
                    let mut keys = Vec::new();
                    let mut values = Vec::new();
                    for (key, value) in map.iter() {
                        if is_skip(atoms, key) || variable(atoms, key).is_some() {
                            return None;
                        }
                        keys.push(key.clone());
                        values.push(value);
                    }
                    code.push(Step::Map(keys));
                    stack.extend(values.into_iter().rev());
                }
                _ => code.push(Step::Constant(term.clone())),
            }
        }
        let ground = match pattern {
            Term::Tuple(elements) => elements
                .iter()
                .map(|element| fully_bound(atoms, element))
                .collect(),
            _ => Vec::new(),
        };
        Some(Pattern {
            term: pattern.clone(),
            code,
            ground,
            variables: numbers.len(),
        })
    }

    /// Match the object with the elements `object` and return the list of
    /// the values of the variables in the order of their numbers, as the
    /// body `['$$']` gives it (`dmc_all_bindings`,
    /// `erl_db_util.c:4664-4684`), or `None` when the object does not match.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let pattern = Pattern::compile(&atoms, &atom("_")).unwrap();
    /// assert_eq!(pattern.run(&[Term::Integer(1)]), Some(Term::nil()));
    /// ```
    pub(in crate::runtime) fn run(&self, object: &[Term]) -> Option<Term> {
        let whole;
        let mut values = vec![None; self.variables];
        let mut terms: Vec<&Term> = Vec::new();
        let mut code = self.code.iter();
        match self.code.first() {
            // The object is a tuple, so a tuple pattern reads its elements
            // with no new tuple.
            Some(Step::Tuple(size)) => {
                if *size != object.len() {
                    return None;
                }
                code.next();
                terms.extend(object.iter().rev());
            }
            _ => {
                whole = Term::tuple(object.to_vec());
                terms.push(&whole);
            }
        }
        for step in code {
            let term = terms.pop().expect("each step has a term");
            match step {
                Step::Skip => {}
                Step::Bind(index) => values[*index] = Some(term.clone()),
                Step::Compare(index) => {
                    if values[*index].as_ref() != Some(term) {
                        return None;
                    }
                }
                Step::Constant(constant) => {
                    if constant != term {
                        return None;
                    }
                }
                Step::Tuple(size) => match term {
                    Term::Tuple(elements) if elements.len() == *size => {
                        terms.extend(elements.iter().rev());
                    }
                    _ => return None,
                },
                Step::Cons => {
                    let (head, tail) = term.cons_cell()?;
                    terms.push(tail);
                    terms.push(head);
                }
                Step::Map(keys) => {
                    let Term::Map(map) = term else {
                        return None;
                    };
                    for key in keys.iter().rev() {
                        terms.push(map.get(key)?);
                    }
                }
            }
        }
        Some(Term::list_from(values.into_iter().flatten()))
    }

    /// Return the element at the 1-based key position `keypos` of the
    /// pattern when it has no variable, no `'_'`, and no map
    /// (`db_is_fully_bound`, `erl_db_util.c:3962-4000`). Then a match reads
    /// only the bucket of that key (`analyze_pattern`,
    /// `erl_db_hash.c:3247-3290`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let pattern = Pattern::compile(&atoms, &Term::tuple(vec![atom("k"), atom("$1")])).unwrap();
    /// assert_eq!(pattern.bound_key(1), Some(&atom("k")));
    /// assert_eq!(pattern.bound_key(2), None);
    /// ```
    pub(in crate::runtime) fn bound_key(&self, keypos: usize) -> Option<&Term> {
        let Term::Tuple(elements) = &self.term else {
            return None;
        };
        let index = keypos.checked_sub(1)?;
        if *self.ground.get(index)? {
            elements.get(index)
        } else {
            None
        }
    }

    /// Return `true` when no object of a table with the 1-based key position
    /// `keypos` can match: the pattern is not a tuple, `'_'`, or a variable,
    /// or it is a tuple with fewer elements than `keypos`
    /// (`db_getkey`, `erl_db_util.c:3940-3960`; `erl_db_hash.c:3241-3247`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(Pattern::compile(&atoms, &atom("foo")).unwrap().matches_nothing(1));
    /// assert!(!Pattern::compile(&atoms, &atom("$1")).unwrap().matches_nothing(1));
    /// ```
    pub(in crate::runtime) fn matches_nothing(&self, keypos: usize) -> bool {
        match self.code.first() {
            Some(Step::Skip | Step::Bind(_)) => false,
            Some(Step::Tuple(size)) => *size < keypos,
            _ => true,
        }
    }
}

/// Return the variable number of `term`, as `db_is_variable`
/// (`erl_db_util.c:3896-3927`) reads the name of an atom `'$N'`: `'$0'`, or
/// digits with no leading zero. The C `int` arithmetic wraps, and a negative
/// number is no variable.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(variable(&atoms, &atom("$12")), Some(12));
/// assert_eq!(variable(&atoms, &atom("$01")), None);
/// ```
fn variable(atoms: &AtomTable, term: &Term) -> Option<i32> {
    let Term::Atom(atom) = term else {
        return None;
    };
    let digits = atoms.name_of(atom).strip_prefix('$')?.as_bytes();
    match digits {
        [b'0'] => Some(0),
        [b'1'..=b'9', ..] if digits.iter().all(u8::is_ascii_digit) => {
            let number = digits.iter().fold(0_i32, |number, digit| {
                number
                    .wrapping_mul(10)
                    .wrapping_add(i32::from(digit - b'0'))
            });
            (number >= 0).then_some(number)
        }
        _ => None,
    }
}

/// Return `true` when `term` is the atom `'_'`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_skip(&atoms, &atom("_")));
/// ```
fn is_skip(atoms: &AtomTable, term: &Term) -> bool {
    matches!(term, Term::Atom(atom) if atoms.name_of(atom) == "_")
}

/// Add the number of each variable of `term` to `numbers`. A map key is a
/// constant, so the walk reads only map values.
///
/// # Examples
///
/// ```rust,ignore
/// let mut numbers = Vec::new();
/// collect_variables(&atoms, &Term::tuple(vec![atom("$3"), atom("$1")]), &mut numbers);
/// assert_eq!(numbers, [3, 1]);
/// ```
fn collect_variables(atoms: &AtomTable, term: &Term, numbers: &mut Vec<i32>) {
    let mut stack = vec![term];
    while let Some(term) = stack.pop() {
        match term {
            Term::Tuple(elements) => stack.extend(elements.iter().rev()),
            Term::Cons(_) => {
                let (head, tail) = term.cons_cell().expect("a cons term is a cell");
                stack.push(tail);
                stack.push(head);
            }
            Term::Map(map) => stack.extend(map.iter().map(|(_, value)| value)),
            _ => numbers.extend(variable(atoms, term)),
        }
    }
}

/// Return `true` when `term` has no variable, no `'_'`, and no map
/// (`db_is_fully_bound`, `erl_db_util.c:3962-4000`). The walk uses a work
/// stack, so a term of any depth gives a result.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(fully_bound(&atoms, &Term::tuple(vec![atom("a")])));
/// assert!(!fully_bound(&atoms, &Term::tuple(vec![atom("_")])));
/// ```
fn fully_bound(atoms: &AtomTable, term: &Term) -> bool {
    let mut stack = vec![term];
    while let Some(term) = stack.pop() {
        match term {
            Term::Tuple(elements) => stack.extend(elements.iter()),
            Term::Cons(_) => {
                let (head, tail) = term.cons_cell().expect("a cons term is a cell");
                stack.push(tail);
                stack.push(head);
            }
            Term::Map(_) => return false,
            _ if is_skip(atoms, term) || variable(atoms, term).is_some() => return false,
            _ => {}
        }
    }
    true
}

#[cfg(test)]
#[path = "../../../tests/runtime/ets/pattern.rs"]
mod tests;
