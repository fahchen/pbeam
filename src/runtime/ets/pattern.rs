//! Match patterns of the ETS match BIFs. `ets:match/2,3` makes the match
//! specification `[{Pattern, [], ['$$']}]` (`erl_db.c:3771-3818`), and
//! `db_match_compile` (`erl_db_util.c:1700-2100`, `4060-4180`) compiles its
//! head. PBeam compiles only such a head: a pattern with no guard and the
//! body `['$$']`.

use crate::term::{AtomTable, Term};

/// A compiled match pattern: the tree of the pattern, and the number of its
/// variables.
#[derive(Debug)]
pub(in crate::runtime) struct Pattern {
    /// The tree of the whole pattern.
    root: Node,
    /// The number of different variables of the pattern.
    variables: usize,
}

/// One part of a compiled pattern, as one `dmc_one_term` instruction
/// (`erl_db_util.c:4060-4180`) matches it.
#[derive(Debug)]
enum Node {
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
    /// A term with no variable (`matchEq`, `matchEqBin`, `matchEqRef`, and
    /// the other constant instructions): the term must be exactly equal to
    /// it.
    Constant(Term),
    /// A tuple (`matchPushT`): a tuple of the same size whose elements
    /// match.
    Tuple(Vec<Node>),
    /// The cons cells of a list (`matchPushL` for each cell): a list with
    /// the same number of cells whose heads match the nodes and whose last
    /// tail matches the last node. One node holds the whole spine, so a long
    /// list compiles and matches in a loop.
    List(Vec<Node>, Box<Node>),
    /// A map (`matchPushM` and `matchKey`): a map that has each key, with a
    /// value that matches. The map can have other keys.
    Map(Vec<(Term, Node)>),
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
    /// assert_eq!(pattern.run(&atoms, &[Term::Integer(1), Term::Integer(2)]), Some(Term::list_from([Term::Integer(2), Term::Integer(1)])));
    /// assert!(Pattern::compile(&atoms, &map_with_key(atom("_"))).is_none());
    /// ```
    pub(in crate::runtime) fn compile(atoms: &AtomTable, pattern: &Term) -> Option<Pattern> {
        let mut numbers = Vec::new();
        collect_variables(atoms, pattern, &mut numbers);
        numbers.sort_unstable();
        numbers.dedup();
        let mut bound = vec![false; numbers.len()];
        let root = match (compile_node(atoms, pattern, &numbers, &mut bound)?, pattern) {
            // The root stays a tuple node, so the key and the size of a
            // tuple pattern stay known.
            (Node::Constant(_), Term::Tuple(elements)) => Node::Tuple(
                elements
                    .iter()
                    .map(|element| Node::Constant(element.clone()))
                    .collect(),
            ),
            (root, _) => root,
        };
        Some(Pattern {
            root,
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
        let mut values = vec![None; self.variables];
        let matched = match &self.root {
            Node::Tuple(nodes) => {
                nodes.len() == object.len()
                    && nodes
                        .iter()
                        .zip(object)
                        .all(|(node, term)| node.matches(term, &mut values))
            }
            Node::Skip => true,
            node => node.matches(&Term::tuple(object.to_vec()), &mut values),
        };
        matched.then(|| Term::list_from(values.into_iter().flatten()))
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
        let Node::Tuple(nodes) = &self.root else {
            return None;
        };
        match nodes.get(keypos.checked_sub(1)?)? {
            Node::Constant(key) => Some(key),
            _ => None,
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
        match &self.root {
            Node::Skip | Node::Bind(_) => false,
            Node::Tuple(nodes) => nodes.len() < keypos,
            _ => true,
        }
    }
}

impl Node {
    /// Match `term` and set the value of each variable that the node binds
    /// in `values`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut values = vec![None];
    /// assert!(Node::Bind(0).matches(&Term::Integer(1), &mut values));
    /// assert!(!Node::Compare(0).matches(&Term::Integer(2), &mut values));
    /// ```
    fn matches(&self, term: &Term, values: &mut [Option<Term>]) -> bool {
        match self {
            Node::Skip => true,
            Node::Bind(index) => {
                values[*index] = Some(term.clone());
                true
            }
            Node::Compare(index) => values[*index].as_ref() == Some(term),
            Node::Constant(constant) => constant == term,
            Node::Tuple(nodes) => match term {
                Term::Tuple(elements) => {
                    nodes.len() == elements.len()
                        && nodes
                            .iter()
                            .zip(elements.iter())
                            .all(|(node, element)| node.matches(element, values))
                }
                _ => false,
            },
            Node::List(heads, tail) => {
                let mut term = term;
                for head in heads {
                    let Some((term_head, term_tail)) = term.cons_cell() else {
                        return false;
                    };
                    if !head.matches(term_head, values) {
                        return false;
                    }
                    term = term_tail;
                }
                tail.matches(term, values)
            }
            Node::Map(pairs) => match term {
                Term::Map(map) => pairs.iter().all(|(key, node)| {
                    map.get(key)
                        .is_some_and(|value| node.matches(value, values))
                }),
                _ => false,
            },
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

/// Compile `term` with the sorted variable numbers `numbers`. `bound` marks
/// each variable that an earlier place binds. Return `None` for a map key
/// that is a variable or `'_'`.
///
/// # Examples
///
/// ```rust,ignore
/// let node = compile_node(&atoms, &atom("$1"), &[1], &mut vec![false]);
/// assert!(matches!(node, Some(Node::Bind(0))));
/// ```
fn compile_node(
    atoms: &AtomTable,
    term: &Term,
    numbers: &[i32],
    bound: &mut [bool],
) -> Option<Node> {
    if is_skip(atoms, term) {
        return Some(Node::Skip);
    }
    if let Some(number) = variable(atoms, term) {
        let index = numbers
            .binary_search(&number)
            .expect("the walk collected each variable");
        let node = if bound[index] {
            Node::Compare(index)
        } else {
            Node::Bind(index)
        };
        bound[index] = true;
        return Some(node);
    }
    let node = match term {
        Term::Tuple(elements) => Node::Tuple(
            elements
                .iter()
                .map(|element| compile_node(atoms, element, numbers, bound))
                .collect::<Option<_>>()?,
        ),
        Term::Cons(_) => {
            let mut heads = Vec::new();
            let mut rest = term;
            while let Some((head, tail)) = rest.cons_cell() {
                heads.push(compile_node(atoms, head, numbers, bound)?);
                rest = tail;
            }
            Node::List(heads, Box::new(compile_node(atoms, rest, numbers, bound)?))
        }
        Term::Map(map) => Node::Map(
            map.iter()
                .map(|(key, value)| {
                    if is_skip(atoms, key) || variable(atoms, key).is_some() {
                        return None;
                    }
                    Some((key.clone(), compile_node(atoms, value, numbers, bound)?))
                })
                .collect::<Option<_>>()?,
        ),
        _ => return Some(Node::Constant(term.clone())),
    };
    Some(constant(node, term))
}

/// Return `node` as one constant when it has no variable, no `'_'`, and no
/// map, so the bound key of a pattern is one term. `term` is the term of the
/// node.
///
/// # Examples
///
/// ```rust,ignore
/// let node = constant(Node::Tuple(vec![Node::Constant(atom("a"))]), &Term::tuple(vec![atom("a")]));
/// assert!(matches!(node, Node::Constant(_)));
/// ```
fn constant(node: Node, term: &Term) -> Node {
    let ground = match &node {
        Node::Tuple(nodes) => nodes.iter().all(|node| matches!(node, Node::Constant(_))),
        Node::List(heads, tail) => {
            heads.iter().all(|node| matches!(node, Node::Constant(_)))
                && matches!(**tail, Node::Constant(_))
        }
        _ => false,
    };
    if ground {
        Node::Constant(term.clone())
    } else {
        node
    }
}

#[cfg(test)]
#[path = "../../../tests/runtime/ets/pattern.rs"]
mod tests;
