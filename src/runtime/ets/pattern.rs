//! Match patterns of the ETS match BIFs. `ets:match/2,3` makes the match
//! specification `[{Pattern, [], ['$$']}]` (`erl_db.c:3771-3818`), and
//! `db_match_compile` (`erl_db_util.c:1700-2100`, `4060-4180`) compiles its
//! head. PBeam compiles only such a head: a pattern with no guard and the
//! body `['$$']`.

use crate::term::{AtomTable, Term};
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};

/// A compiled match pattern: the pattern, its flat code, and the number of
/// its variables. As `db_match_compile` (`erl_db_util.c:1700-2100`) does,
/// the compile and the match use a work stack and no recursion, so a pattern
/// of any depth compiles, matches, and drops.
#[derive(Debug, Clone)]
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
    /// The number of each variable, in ascending order. The place of a
    /// number in this list is the index of its value in a match.
    numbers: Vec<i32>,
}

/// One step of a compiled pattern, as one `dmc_one_term` instruction
/// (`erl_db_util.c:4060-4180`) matches it. Each step matches the term at the
/// top of the work stack, and a step of a tuple, a cons cell, or a map puts
/// the parts of the term on the stack for the next steps.
#[derive(Debug, Clone)]
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
            numbers,
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
    #[cfg(test)]
    pub(in crate::runtime) fn run(&self, object: &[Term]) -> Option<Term> {
        Some(Term::list_from(self.bind(object)?))
    }

    /// Match the object with the elements `object` and return the value of
    /// each variable, in ascending order of the variable numbers, or `None`
    /// when the object does not match. The index of a value in the result
    /// is the place of its number in [`Pattern::numbers`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let pattern = Pattern::compile(&atoms, &Term::tuple(vec![atom("$2"), atom("$1")])).unwrap();
    /// assert_eq!(pattern.bind(&[Term::Integer(1), Term::Integer(2)]), Some(vec![Term::Integer(2), Term::Integer(1)]));
    /// ```
    pub(in crate::runtime) fn bind(&self, object: &[Term]) -> Option<Vec<Term>> {
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
        Some(
            values
                .into_iter()
                .map(|value| value.expect("the pattern bound each variable"))
                .collect(),
        )
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

/// A compiled match specification, as `db_match_compile`
/// (`erl_db_util.c:1717-2100`) compiles a list of clauses. A clause whose
/// head matches and whose guards are all `true` gives the value of its
/// body. The first such clause decides the result.
#[derive(Debug, Clone)]
pub(in crate::runtime) struct MatchSpec {
    /// The specification term.
    specification: Term,
    /// The clauses, in the order of the list.
    clauses: Vec<Clause>,
}

/// One clause of a match specification: `{Head, Guards, Body}`.
#[derive(Debug, Clone)]
struct Clause {
    /// The head pattern.
    head: Pattern,
    /// The guard expressions. Each one must give `true`.
    guards: Vec<Expr>,
    /// The body expressions. The last one gives the result.
    body: Vec<Expr>,
}

/// One guard or body expression, as `dmc_expr` (`erl_db_util.c:5592-5660`)
/// compiles it.
#[derive(Debug, Clone)]
enum Expr {
    /// A term that does not depend on the object (`do_emit_constant`).
    Constant(Term),
    /// The value of a variable of the head, by the place of its number in
    /// the pattern.
    Variable(usize),
    /// `'$_'`: the whole object.
    Whole,
    /// `'$$'`: the values of the variables, in ascending number order.
    AllBindings,
    /// `{{...}}`: a tuple of the values of these expressions.
    Tuple(Vec<Expr>),
    /// A list of the values of these expressions and this tail.
    List(Vec<Expr>, Box<Expr>),
    /// A call of a guard function (`dmc_fun`, `erl_db_util.c:5460-5590`).
    Call(Guard, Vec<Expr>),
    /// `andalso`: `false` or the right value.
    AndAlso(Box<Expr>, Box<Expr>),
    /// `orelse`: `true` or the right value.
    OrElse(Box<Expr>, Box<Expr>),
    /// A one-operand `and` or `or` (checked) or `andalso` or `orelse`
    /// (unchecked): the operand value.
    BooleanOne(Box<Expr>, bool),
}

/// A guard function of the match specification dialect, as `guard_tab`
/// (`erl_db_util.c:572-976`) lists it.
#[derive(Debug, Clone, Copy)]
enum Guard {
    IsAtom,
    IsFloat,
    IsInteger,
    IsList,
    IsNumber,
    IsPid,
    IsPort,
    IsReference,
    IsTuple,
    IsMap,
    IsBinary,
    IsBitstring,
    IsBoolean,
    IsFunction1,
    IsFunction2,
    Abs,
    Element,
    Hd,
    Tl,
    Length,
    MapSize,
    MapGet,
    IsMapKey,
    Size,
    BitSize,
    ByteSize,
    TupleSize,
    Node0,
    Node1,
    Plus1,
    Minus1,
    Plus2,
    Minus2,
    Times,
    Div,
    IntDiv,
    Rem,
    Band,
    Bor,
    Bxor,
    Bnot,
    Bsl,
    Bsr,
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    EqExact,
    Neq,
    NeqExact,
    Not,
    Xor,
    And,
    Or,
    Min,
    Max,
}

/// Why a match specification did not compile.
#[derive(Debug)]
pub(in crate::runtime) enum SpecError {
    /// ERTS raises `badarg` for the specification.
    Badarg,
    /// ERTS has the function, and PBeam does not run it yet. The text names
    /// the function.
    Unsupported(String),
}

impl MatchSpec {
    /// Compile `spec`, as `db_match_compile` does. Return
    /// [`SpecError::Badarg`] for each shape that ERTS rejects: a
    /// specification that is not a list of 3-tuples, a head that is not a
    /// pattern, a guard or body that is not a proper list, a variable of a
    /// guard or body that the head does not bind, an unknown guard
    /// function, and a tuple of a guard or body that is neither a call nor
    /// the `{{...}}` tuple form. Return [`SpecError::Unsupported`] for a
    /// guard function that PBeam does not run yet.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let spec = MatchSpec::compile(&atoms, &Term::list_from(vec![Term::tuple(vec![
    ///     Term::tuple(vec![atom("$1"), atom("_")]),
    ///     Term::nil(),
    ///     Term::list_from(vec![atom("$1")]),
    /// ])])).unwrap();
    /// assert_eq!(spec.run(&atoms, &[Term::Integer(1), Term::Integer(2)]), Some(Term::Integer(1)));
    /// ```
    pub(in crate::runtime) fn compile(
        atoms: &AtomTable,
        spec: &Term,
    ) -> Result<MatchSpec, SpecError> {
        let Some(clauses) = spec.proper_list() else {
            return Err(SpecError::Badarg);
        };
        let mut compiled = Vec::new();
        for clause in clauses {
            let Term::Tuple(clause) = clause else {
                return Err(SpecError::Badarg);
            };
            let [head, guards, body] = clause.as_ref() else {
                return Err(SpecError::Badarg);
            };
            let head = Pattern::compile(atoms, head).ok_or(SpecError::Badarg)?;
            let Some(guards) = guards.proper_list() else {
                return Err(SpecError::Badarg);
            };
            let Some(body) = body.proper_list() else {
                return Err(SpecError::Badarg);
            };
            let numbers = &head.numbers;
            let guards = guards
                .map(|item| expression(atoms, item, numbers))
                .collect::<Result<Vec<_>, _>>()?;
            let body = body
                .map(|item| expression(atoms, item, numbers))
                .collect::<Result<Vec<_>, _>>()?;
            // ERTS raises `badarg` for a clause with no body expression
            // (`compile_guard_expr`, `erl_db_util.c:5663-5706`).
            if body.is_empty() {
                return Err(SpecError::Badarg);
            }
            compiled.push(Clause { head, guards, body });
        }
        Ok(MatchSpec {
            specification: spec.clone(),
            clauses: compiled,
        })
    }

    /// Return the specification term that made this program.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(spec.specification(), term);
    /// ```
    pub(in crate::runtime) fn specification(&self) -> Term {
        self.specification.clone()
    }

    /// Return the head pattern of each clause, in the order of the list.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(spec.heads().count(), 1);
    /// ```
    pub(in crate::runtime) fn heads(&self) -> impl Iterator<Item = &Term> {
        self.clauses.iter().map(|clause| &clause.head.term)
    }

    /// Run the clauses against the object with the elements `object` and
    /// return the value of the first clause whose head matches and whose
    /// guards are all `true`, else `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let spec = MatchSpec::compile(&atoms, &Term::list_from(vec![Term::tuple(vec![
    ///     atom("_"), Term::nil(), Term::list_from(vec![atom("true")]),
    /// ])])).unwrap();
    /// assert_eq!(spec.run(&atoms, &[Term::Integer(1)]), Some(atom("true")));
    /// ```
    pub(in crate::runtime) fn run(&self, atoms: &AtomTable, object: &[Term]) -> Option<Term> {
        for clause in &self.clauses {
            let Some(bindings) = clause.head.bind(object) else {
                continue;
            };
            let values = &bindings;
            let whole = Term::tuple(object.to_vec());
            if !clause.guards.iter().all(|guard| {
                evaluate(atoms, guard, values, &whole, true) == Some(Term::static_atom("true"))
            }) {
                continue;
            }
            let mut result = Term::static_atom("true");
            for body in &clause.body {
                // A body BIF that fails gives `'EXIT'`, as `matchCatch`
                // catches the failure of a BIF (`erl_db_util.c:3098-3105`).
                result = evaluate(atoms, body, values, &whole, false)
                    .unwrap_or_else(|| Term::static_atom("EXIT"));
            }
            return Some(result);
        }
        None
    }

    /// Return the key of the head when the specification has one clause
    /// whose head is fully bound at `keypos` (`analyze_pattern`,
    /// `erl_db_hash.c:3247-3290`), as [`Pattern::bound_key`] gives it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(spec.bound_key(1), Some(&atom("k")));
    /// ```
    pub(in crate::runtime) fn bound_key(&self, keypos: usize) -> Option<&Term> {
        match self.clauses.as_slice() {
            [clause] => clause.head.bound_key(keypos),
            _ => None,
        }
    }

    /// Return `true` when no object of a table with the key position
    /// `keypos` can match the specification.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(spec.matches_nothing(1));
    /// ```
    pub(in crate::runtime) fn matches_nothing(&self, keypos: usize) -> bool {
        match self.clauses.as_slice() {
            [clause] => clause.head.matches_nothing(keypos),
            _ => false,
        }
    }
}

/// Compile one guard or body expression of a clause whose head binds
/// `numbers`.
fn expression(atoms: &AtomTable, term: &Term, numbers: &[i32]) -> Result<Expr, SpecError> {
    if let Term::Atom(atom) = term {
        if atom.is_static("$_") {
            return Ok(Expr::Whole);
        }
        if atom.is_static("$$") {
            return Ok(Expr::AllBindings);
        }
        if let Some(number) = variable(atoms, term) {
            return numbers
                .iter()
                .position(|bound| *bound == number)
                .map(Expr::Variable)
                .ok_or(SpecError::Badarg);
        }
        return Ok(Expr::Constant(term.clone()));
    }
    match term {
        Term::Cons(_) => {
            let mut elements = Vec::new();
            let mut tail = term;
            while let Term::Cons(_) = tail {
                let (head, rest) = tail.cons_cell().expect("a cons term is a cell");
                elements.push(expression(atoms, head, numbers)?);
                tail = rest;
            }
            Ok(Expr::List(
                elements,
                Box::new(expression(atoms, tail, numbers)?),
            ))
        }
        Term::Tuple(elements) if elements.is_empty() => Ok(Expr::Constant(term.clone())),
        Term::Tuple(elements) => {
            if let [Term::Atom(name), value] = elements.as_ref()
                && name.is_static("const")
            {
                return Ok(Expr::Constant(value.clone()));
            }
            if let [Term::Tuple(inner)] = elements.as_ref() {
                return Ok(Expr::Tuple(
                    inner
                        .iter()
                        .map(|element| expression(atoms, element, numbers))
                        .collect::<Result<Vec<_>, _>>()?,
                ));
            }
            let Term::Atom(name) = &elements[0] else {
                return Err(SpecError::Badarg);
            };
            let name = atoms.name_of(name);
            let operands = elements.len() - 1;
            if operands == 1 && matches!(name, "and" | "or" | "andalso" | "orelse") {
                let checked = name == "and" || name == "or";
                let operand = expression(atoms, &elements[1], numbers)?;
                return Ok(Expr::BooleanOne(Box::new(operand), checked));
            }
            if operands == 0 && matches!(name, "and" | "or" | "andalso" | "orelse") {
                return Err(SpecError::Badarg);
            }
            if name == "andalso" || name == "orelse" {
                // `dmc_andalso` and `dmc_orelse` accept two or more
                // operands (`erl_db_util.c:4760-4827`).
                let mut operands = elements[1..]
                    .iter()
                    .map(|operand| expression(atoms, operand, numbers))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut operands = operands.drain(..);
                let first = operands.next().expect("the form has an operand");
                return Ok(operands.fold(first, |left, right| {
                    let (left, right) = (Box::new(left), Box::new(right));
                    if name == "andalso" {
                        Expr::AndAlso(left, right)
                    } else {
                        Expr::OrElse(left, right)
                    }
                }));
            }
            if (name == "and" || name == "or") && elements.len() > 3 {
                // `dmc_and` and `dmc_or` accept two or more operands
                // (`erl_db_util.c:4701-4757`). A left fold keeps the
                // left-to-right evaluation.
                let function = if name == "and" { Guard::And } else { Guard::Or };
                let mut operands = elements[1..]
                    .iter()
                    .map(|operand| expression(atoms, operand, numbers))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut operands = operands.drain(..);
                let first = operands.next().expect("the form has an operand");
                return Ok(
                    operands.fold(first, |left, right| Expr::Call(function, vec![left, right]))
                );
            }
            let Some(function) = guard(name, elements.len() - 1) else {
                return Err(if known_guard(name, elements.len() - 1) {
                    SpecError::Unsupported(format!("{name}/{}", elements.len() - 1))
                } else {
                    SpecError::Badarg
                });
            };
            let arguments = elements[1..]
                .iter()
                .map(|argument| expression(atoms, argument, numbers))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::Call(function, arguments))
        }
        Term::Map(_) => Err(SpecError::Unsupported("map construction".to_owned())),
        _ => Ok(Expr::Constant(term.clone())),
    }
}

/// Return the guard function `name/arity`, or `None` when PBeam does not
/// run it yet. A name and arity that ERTS does not know is
/// [`SpecError::Badarg`]; the caller tells the two apart.
fn guard(name: &str, arity: usize) -> Option<Guard> {
    let function = match (name, arity) {
        ("is_atom", 1) => Guard::IsAtom,
        ("is_float", 1) => Guard::IsFloat,
        ("is_integer", 1) => Guard::IsInteger,
        ("is_list", 1) => Guard::IsList,
        ("is_number", 1) => Guard::IsNumber,
        ("is_pid", 1) => Guard::IsPid,
        ("is_port", 1) => Guard::IsPort,
        ("is_reference", 1) => Guard::IsReference,
        ("is_tuple", 1) => Guard::IsTuple,
        ("is_map", 1) => Guard::IsMap,
        ("is_binary", 1) => Guard::IsBinary,
        ("is_bitstring", 1) => Guard::IsBitstring,
        ("is_boolean", 1) => Guard::IsBoolean,
        ("is_function", 1) => Guard::IsFunction1,
        ("is_function", 2) => Guard::IsFunction2,
        ("abs", 1) => Guard::Abs,
        ("element", 2) => Guard::Element,
        ("hd", 1) => Guard::Hd,
        ("tl", 1) => Guard::Tl,
        ("length", 1) => Guard::Length,
        ("map_size", 1) => Guard::MapSize,
        ("map_get", 2) => Guard::MapGet,
        ("is_map_key", 2) => Guard::IsMapKey,
        ("size", 1) => Guard::Size,
        ("bit_size", 1) => Guard::BitSize,
        ("byte_size", 1) => Guard::ByteSize,
        ("tuple_size", 1) => Guard::TupleSize,
        ("node", 0) => Guard::Node0,
        ("node", 1) => Guard::Node1,
        ("+", 1) => Guard::Plus1,
        ("-", 1) => Guard::Minus1,
        ("+", 2) => Guard::Plus2,
        ("-", 2) => Guard::Minus2,
        ("*", 2) => Guard::Times,
        ("/", 2) => Guard::Div,
        ("div", 2) => Guard::IntDiv,
        ("rem", 2) => Guard::Rem,
        ("band", 2) => Guard::Band,
        ("bor", 2) => Guard::Bor,
        ("bxor", 2) => Guard::Bxor,
        ("bnot", 1) => Guard::Bnot,
        ("bsl", 2) => Guard::Bsl,
        ("bsr", 2) => Guard::Bsr,
        (">", 2) => Guard::Gt,
        (">=", 2) => Guard::Ge,
        ("<", 2) => Guard::Lt,
        ("=<", 2) => Guard::Le,
        ("==", 2) => Guard::Eq,
        ("=:=", 2) => Guard::EqExact,
        ("/=", 2) => Guard::Neq,
        ("=/=", 2) => Guard::NeqExact,
        ("not", 1) => Guard::Not,
        ("xor", 2) => Guard::Xor,
        ("and", 2) => Guard::And,
        ("or", 2) => Guard::Or,
        ("min", 2) => Guard::Min,
        ("max", 2) => Guard::Max,
        _ => return None,
    };
    Some(function)
}

/// Return whether ERTS knows the guard function `name/arity`. ERTS raises
/// `badarg` for each other function.
fn known_guard(name: &str, arity: usize) -> bool {
    matches!(
        (name, arity),
        ("is_record" | "is_integer" | "binary_part", 3)
            | ("binary_part", 2)
            | ("round" | "trunc" | "float" | "ceil" | "floor", 1)
    )
}

/// Evaluate one guard or body expression, as `db_prog_match`
/// (`erl_db_util.c:2247-3110`) runs the compiled program. Return `None`
/// when a BIF of the expression fails; a guard then fails and a body gives
/// `'EXIT'`.
fn evaluate(
    atoms: &AtomTable,
    expression: &Expr,
    bindings: &[Term],
    whole: &Term,
    guard: bool,
) -> Option<Term> {
    match expression {
        Expr::Constant(term) => Some(term.clone()),
        Expr::Variable(index) => bindings.get(*index).cloned(),
        Expr::Whole => Some(whole.clone()),
        Expr::AllBindings => Some(Term::list_from(bindings.to_vec())),
        Expr::Tuple(elements) => Some(Term::tuple(
            elements
                .iter()
                .map(|element| part(atoms, element, bindings, whole, guard))
                .collect::<Option<Vec<_>>>()?,
        )),
        Expr::List(elements, tail) => {
            let elements: Vec<Term> = elements
                .iter()
                .map(|element| part(atoms, element, bindings, whole, guard))
                .collect::<Option<Vec<_>>>()?;
            let mut tail = part(atoms, tail, bindings, whole, guard)?;
            for element in elements.into_iter().rev() {
                tail = Term::cons(element, tail);
            }
            Some(tail)
        }
        Expr::Call(function, arguments) => {
            let arguments: Vec<Term> = arguments
                .iter()
                .map(|argument| part(atoms, argument, bindings, whole, guard))
                .collect::<Option<Vec<_>>>()?;
            call(atoms, *function, &arguments)
        }
        Expr::AndAlso(left, right) => match &evaluate(atoms, left, bindings, whole, guard)? {
            Term::Atom(atom) if atom.is_static("false") => Some(Term::static_atom("false")),
            Term::Atom(atom) if atom.is_static("true") => {
                evaluate(atoms, right, bindings, whole, guard)
            }
            _ => None,
        },
        Expr::OrElse(left, right) => match &evaluate(atoms, left, bindings, whole, guard)? {
            Term::Atom(atom) if atom.is_static("true") => Some(Term::static_atom("true")),
            Term::Atom(atom) if atom.is_static("false") => {
                evaluate(atoms, right, bindings, whole, guard)
            }
            _ => None,
        },
        Expr::BooleanOne(expression, checked) => {
            let value = evaluate(atoms, expression, bindings, whole, guard)?;
            if *checked {
                match &value {
                    Term::Atom(atom) if atom.is_one_of(&["true", "false"]) => Some(value),
                    _ => None,
                }
            } else {
                Some(value)
            }
        }
    }
}

/// Evaluate one part of a tuple, a list, or a call. A guard propagates a
/// failure; a body gives `EXIT` for that part (`erl_db_util.c:2580-2618`).
fn part(
    atoms: &AtomTable,
    expression: &Expr,
    bindings: &[Term],
    whole: &Term,
    guard: bool,
) -> Option<Term> {
    let value = evaluate(atoms, expression, bindings, whole, guard);
    if guard {
        value
    } else {
        Some(value.unwrap_or_else(|| Term::static_atom("EXIT")))
    }
}

/// Run the guard function `function` over `arguments`, as the BIF of the
/// match program does. Return `None` for a failure.
fn call(atoms: &AtomTable, function: Guard, arguments: &[Term]) -> Option<Term> {
    let truth = |value: bool| Some(Term::static_atom(if value { "true" } else { "false" }));
    match function {
        Guard::IsAtom => truth(matches!(arguments[0], Term::Atom(_))),
        Guard::IsFloat => truth(matches!(arguments[0], Term::Float(_))),
        Guard::IsInteger => truth(arguments[0].is_integer()),
        Guard::IsList => truth(arguments[0].cons_cell().is_some() || arguments[0].is_nil()),
        Guard::IsNumber => truth(matches!(
            arguments[0],
            Term::Integer(_) | Term::BigInt(_) | Term::Float(_)
        )),
        Guard::IsPid => truth(matches!(arguments[0], Term::Pid(_))),
        Guard::IsPort => truth(matches!(arguments[0], Term::Port(_))),
        Guard::IsReference => truth(matches!(
            arguments[0],
            Term::Reference(_) | Term::Resource(_)
        )),
        Guard::IsTuple => truth(matches!(arguments[0], Term::Tuple(_))),
        Guard::IsMap => truth(matches!(arguments[0], Term::Map(_))),
        // ERTS calls `is_binary_1` (`erl_db_util.c:641-642`).
        Guard::IsBinary => truth(arguments[0].is_binary()),
        Guard::IsBitstring => truth(matches!(arguments[0], Term::Binary(_))),
        Guard::IsBoolean => {
            truth(matches!(&arguments[0], Term::Atom(atom) if atom.is_one_of(&["true", "false"])))
        }
        Guard::IsFunction1 => truth(matches!(arguments[0], Term::Fun(_))),
        Guard::IsFunction2 => {
            let (Term::Fun(fun), Term::Integer(arity)) = (&arguments[0], &arguments[1]) else {
                return truth(false);
            };
            truth(*arity >= 0 && i64::from(fun.target().arity()) == *arity)
        }
        Guard::Abs => abs(&arguments[0]),
        Guard::Element => {
            let index = arguments[0].small_integer()?;
            let Term::Tuple(elements) = &arguments[1] else {
                return None;
            };
            if index < 1 || index as usize > elements.len() {
                return None;
            }
            Some(elements[index as usize - 1].clone())
        }
        Guard::Hd => arguments[0].cons_cell().map(|(head, _)| head.clone()),
        Guard::Tl => arguments[0].cons_cell().map(|(_, tail)| tail.clone()),
        Guard::Length => Some(Term::Integer(
            i64::try_from(arguments[0].proper_list()?.count()).ok()?,
        )),
        Guard::MapSize => match &arguments[0] {
            Term::Map(map) => Some(Term::Integer(map.iter().count() as i64)),
            _ => None,
        },
        Guard::MapGet => match &arguments[1] {
            Term::Map(map) => map.get(&arguments[0]).cloned(),
            _ => None,
        },
        Guard::IsMapKey => match &arguments[1] {
            Term::Map(map) => truth(map.get(&arguments[0]).is_some()),
            _ => None,
        },
        // ERTS calls `size_1` (`erl_db_util.c:731-732`), which floors the bit
        // size of a bitstring with `BYTE_SIZE` (`erl_bif_guard.c:323-324`).
        Guard::Size => match &arguments[0] {
            Term::Tuple(elements) => Some(Term::Integer(elements.len() as i64)),
            Term::Binary(binary) => Some(Term::Integer((binary.bit_size() / 8) as i64)),
            _ => None,
        },
        Guard::BitSize => match &arguments[0] {
            Term::Binary(binary) => Some(Term::Integer(binary.bit_size() as i64)),
            _ => None,
        },
        // ERTS calls `byte_size_1` (`erl_db_util.c:761-762`), which rounds the
        // bit size up with `NBYTES` (`erl_bif_guard.c:385`, `erl_bits.h:27`).
        Guard::ByteSize => match &arguments[0] {
            Term::Binary(binary) => Some(Term::Integer(binary.bit_size().div_ceil(8) as i64)),
            _ => None,
        },
        Guard::TupleSize => match &arguments[0] {
            Term::Tuple(elements) => Some(Term::Integer(elements.len() as i64)),
            _ => None,
        },
        Guard::Node0 => Some(Term::static_atom("nonode@nohost")),
        Guard::Node1 => matches!(
            arguments[0],
            Term::Pid(_) | Term::Port(_) | Term::Reference(_) | Term::Resource(_)
        )
        .then(|| Term::static_atom("nonode@nohost")),
        Guard::Plus1 => number(&arguments[0]),
        Guard::Minus1 => negate(&arguments[0]),
        Guard::Plus2 => arithmetic(Arithmetic::Add, &arguments[0], &arguments[1]),
        Guard::Minus2 => arithmetic(Arithmetic::Subtract, &arguments[0], &arguments[1]),
        Guard::Times => arithmetic(Arithmetic::Multiply, &arguments[0], &arguments[1]),
        Guard::Div => divide(&arguments[0], &arguments[1]),
        Guard::IntDiv => integer_division(&arguments[0], &arguments[1], false),
        Guard::Rem => integer_division(&arguments[0], &arguments[1], true),
        Guard::Band => bitwise(Bitwise::And, &arguments[0], &arguments[1]),
        Guard::Bor => bitwise(Bitwise::Or, &arguments[0], &arguments[1]),
        Guard::Bxor => bitwise(Bitwise::Xor, &arguments[0], &arguments[1]),
        Guard::Bnot => bitwise_not(&arguments[0]),
        Guard::Bsl => shift(&arguments[0], &arguments[1], false),
        Guard::Bsr => shift(&arguments[0], &arguments[1], true),
        Guard::Gt => compare(
            atoms,
            &arguments[0],
            &arguments[1],
            std::cmp::Ordering::is_gt,
        ),
        Guard::Ge => compare(
            atoms,
            &arguments[0],
            &arguments[1],
            std::cmp::Ordering::is_ge,
        ),
        Guard::Lt => compare(
            atoms,
            &arguments[0],
            &arguments[1],
            std::cmp::Ordering::is_lt,
        ),
        Guard::Le => compare(
            atoms,
            &arguments[0],
            &arguments[1],
            std::cmp::Ordering::is_le,
        ),
        Guard::Eq => compare(
            atoms,
            &arguments[0],
            &arguments[1],
            std::cmp::Ordering::is_eq,
        ),
        Guard::EqExact => compare_exact(
            atoms,
            &arguments[0],
            &arguments[1],
            std::cmp::Ordering::is_eq,
        ),
        Guard::Neq => compare(
            atoms,
            &arguments[0],
            &arguments[1],
            std::cmp::Ordering::is_ne,
        ),
        Guard::NeqExact => compare_exact(
            atoms,
            &arguments[0],
            &arguments[1],
            std::cmp::Ordering::is_ne,
        ),
        Guard::Not => match &arguments[0] {
            Term::Atom(atom) if atom.is_static("true") => truth(false),
            Term::Atom(atom) if atom.is_static("false") => truth(true),
            _ => None,
        },
        Guard::Xor => boolean(&arguments[0], &arguments[1], |left, right| left != right),
        Guard::And => boolean(&arguments[0], &arguments[1], |left, right| left && right),
        Guard::Or => boolean(&arguments[0], &arguments[1], |left, right| left || right),
        Guard::Min => Some(choose(atoms, &arguments[0], &arguments[1], false)?),
        Guard::Max => Some(choose(atoms, &arguments[0], &arguments[1], true)?),
    }
}

/// Return the arithmetic comparison of two terms as `true` or `false`, or
/// `None` when ERTS fails.
fn compare(
    atoms: &AtomTable,
    left: &Term,
    right: &Term,
    test: impl Fn(std::cmp::Ordering) -> bool,
) -> Option<Term> {
    Some(Term::static_atom(
        if test(left.compare(right, atoms).ok()?) {
            "true"
        } else {
            "false"
        },
    ))
}

/// Return the exact comparison of two terms as `true` or `false`.
fn compare_exact(
    atoms: &AtomTable,
    left: &Term,
    right: &Term,
    test: impl Fn(std::cmp::Ordering) -> bool,
) -> Option<Term> {
    Some(Term::static_atom(
        if test(left.compare_exact(right, atoms).ok()?) {
            "true"
        } else {
            "false"
        },
    ))
}

/// Return the smaller or the larger of two terms, as `min/2` and `max/2`
/// do.
fn choose(atoms: &AtomTable, left: &Term, right: &Term, larger: bool) -> Option<Term> {
    let order = left.compare(right, atoms).ok()?;
    Some(match order {
        std::cmp::Ordering::Less => {
            if larger {
                right.clone()
            } else {
                left.clone()
            }
        }
        std::cmp::Ordering::Greater => {
            if larger {
                left.clone()
            } else {
                right.clone()
            }
        }
        // ERTS keeps the first operand on a tie (`erl_bif_guard.c:475-484`).
        std::cmp::Ordering::Equal => left.clone(),
    })
}

/// Run a boolean operator that requires two booleans.
fn boolean(left: &Term, right: &Term, operation: impl Fn(bool, bool) -> bool) -> Option<Term> {
    let operand = |term: &Term| match term {
        Term::Atom(atom) if atom.is_static("true") => Some(true),
        Term::Atom(atom) if atom.is_static("false") => Some(false),
        _ => None,
    };
    Some(Term::static_atom(
        if operation(operand(left)?, operand(right)?) {
            "true"
        } else {
            "false"
        },
    ))
}

/// An arithmetic operation of two numbers.
#[derive(Clone, Copy)]
enum Arithmetic {
    Add,
    Subtract,
    Multiply,
}

/// Return the value of an arithmetic operation on two numbers.
fn arithmetic(operation: Arithmetic, left: &Term, right: &Term) -> Option<Term> {
    if let (Some(left), Some(right)) = (integer(left), integer(right)) {
        let value = match operation {
            Arithmetic::Add => left + right,
            Arithmetic::Subtract => left - right,
            Arithmetic::Multiply => left * right,
        };
        return Some(integer_term(value));
    }
    let (left, right) = (float(left)?, float(right)?);
    finite(match operation {
        Arithmetic::Add => left + right,
        Arithmetic::Subtract => left - right,
        Arithmetic::Multiply => left * right,
    })
}

/// Return the value of `/`, which always gives a float.
fn divide(left: &Term, right: &Term) -> Option<Term> {
    let (left, right) = (float(left)?, float(right)?);
    if right == 0.0 {
        return None;
    }
    finite(left / right)
}

/// Return the value of `div` or `rem` of two integers.
fn integer_division(left: &Term, right: &Term, remainder: bool) -> Option<Term> {
    let (left, right) = (integer(left)?, integer(right)?);
    if right == BigInt::from(0) {
        return None;
    }
    Some(integer_term(if remainder {
        left % right
    } else {
        left / right
    }))
}

/// A bitwise operation of two integers.
#[derive(Clone, Copy)]
enum Bitwise {
    And,
    Or,
    Xor,
}

/// Return the value of a bitwise operation on two integers.
fn bitwise(operation: Bitwise, left: &Term, right: &Term) -> Option<Term> {
    let (left, right) = (integer(left)?, integer(right)?);
    Some(integer_term(match operation {
        Bitwise::And => left & right,
        Bitwise::Or => left | right,
        Bitwise::Xor => left ^ right,
    }))
}

/// Return the value of `bnot` of an integer.
fn bitwise_not(term: &Term) -> Option<Term> {
    Some(integer_term(-integer(term)? - BigInt::from(1)))
}

/// Return the value of `bsl` or `bsr` of two integers.
fn shift(left: &Term, right: &Term, right_shift: bool) -> Option<Term> {
    /// The largest bignum arity on a 64-bit system (`BIG_ARITY_MAX`,
    /// `erl_term.h:465-466`).
    const BIG_ARITY_MAX: u128 = (1 << 16) - 1;
    let left = integer(left)?;
    let count = integer(right)?;
    let mut left_shift = !right_shift;
    let mut huge = false;
    let count = match count.to_i64() {
        // A negative count reverses the direction
        // (`erts_shift`, `erl_arith.c:198-308`).
        Some(count) if count < 0 => {
            left_shift = !left_shift;
            count.unsigned_abs()
        }
        Some(count) => count as u64,
        None => {
            // `N bsl NegativeBigNum` is `N bsr MAX_SMALL`, and
            // `N bsr NegativeBigNum` is `N bsl MAX_SMALL`
            // (`erl_arith.c:209-216`, `286-295`).
            if count.is_negative() {
                left_shift = !left_shift;
            }
            huge = true;
            u64::MAX
        }
    };
    if !huge && (count == 0 || left == BigInt::from(0)) {
        return Some(integer_term(left));
    }
    if !left_shift {
        if huge || count > u64::from(u32::MAX) {
            return Some(Term::Integer(if left.is_negative() { -1 } else { 0 }));
        }
        return Some(integer_term(left >> count as usize));
    }
    if left == BigInt::from(0) {
        return Some(Term::Integer(0));
    }
    if huge {
        return None;
    }
    // The pre-check of `erts_shift` gives `system_limit` when the result
    // would exceed the bignum arity (`erl_arith.c:246-278`).
    let bits = (left.magnitude().bits() as u128) + u128::from(count);
    if bits.div_ceil(64) > BIG_ARITY_MAX {
        return None;
    }
    Some(integer_term(left << count as usize))
}

/// Return the value of unary `+`, which requires a number.
fn number(term: &Term) -> Option<Term> {
    match term {
        Term::Integer(_) | Term::BigInt(_) | Term::Float(_) => Some(term.clone()),
        _ => None,
    }
}

/// Return the value of unary `-`.
fn negate(term: &Term) -> Option<Term> {
    match term {
        Term::Integer(_) | Term::BigInt(_) => Some(integer_term(-integer(term)?)),
        Term::Float(value) => finite(-value),
        _ => None,
    }
}

/// Return the absolute value of a number.
fn abs(term: &Term) -> Option<Term> {
    match term {
        Term::Integer(_) | Term::BigInt(_) => Some(integer_term(integer(term)?.abs())),
        Term::Float(value) => finite(value.abs()),
        _ => None,
    }
}

/// Return the integer of a numeric term.
fn integer(term: &Term) -> Option<BigInt> {
    match term {
        Term::Integer(value) => Some(BigInt::from(*value)),
        Term::BigInt(value) => Some((**value).clone()),
        _ => None,
    }
}

/// Return the float of a numeric term.
fn float(term: &Term) -> Option<f64> {
    match term {
        Term::Integer(value) => Some(*value as f64),
        Term::BigInt(value) => value.to_f64(),
        Term::Float(value) => Some(*value),
        _ => None,
    }
}

/// Return a finite float term, or `None` for an infinity or a NaN.
fn finite(value: f64) -> Option<Term> {
    value.is_finite().then_some(Term::Float(value))
}

/// Return the term of an integer in its canonical range.
fn integer_term(value: BigInt) -> Term {
    match value.to_i64() {
        Some(value) => Term::Integer(value),
        None => Term::from_bigint(value),
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
pub(super) fn variable(atoms: &AtomTable, term: &Term) -> Option<i32> {
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
pub(super) fn is_skip(atoms: &AtomTable, term: &Term) -> bool {
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
pub(super) fn fully_bound(atoms: &AtomTable, term: &Term) -> bool {
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
