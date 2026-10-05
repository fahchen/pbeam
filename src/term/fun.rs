//! A local fun, as in ERTS OTP 29.1.1 (`erl_fun.h`).
//!
//! ERTS keeps a fun in two parts. The fun entry (`ErlFunEntry`) identifies
//! the function of the fun: the defining module, the index of the lambda in
//! the module, its old unique value, and its arity. The fun thing
//! (`ErlFunThing`) on the process heap points to the entry and holds the
//! free variables. Here `Lambda` is the entry, with the entry label of its
//! code, and `Fun` is the thing.

use super::{Atom, Term};

/// The fun entry of a lambda in a loaded module: the `FunT` entry that the
/// loader resolves, as ERTS `ErlFunEntry` is.
///
/// # Examples
///
/// ```rust,ignore
/// // The `FunT` entry of `fun(X) -> {Tag, X} end` in `fun_probe:locals/0`.
/// let module = AtomTable::new().put("fun_probe")?;
/// let lambda = Lambda::new(module, 0, 92_581_221, 53, 1);
/// assert_eq!(lambda.arity(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lambda {
    /// The module that defines the lambda.
    module: Atom,
    /// The index of the lambda in the `FunT` chunk (`ErlFunEntry.index`).
    index: i32,
    /// The old unique value of the `FunT` entry (`ErlFunEntry.old_uniq`).
    old_unique: i32,
    /// The entry label of the code of the lambda.
    label: u32,
    /// The number of arguments of a call of the fun, without the free
    /// variables.
    arity: u32,
}

impl Lambda {
    /// Return the fun entry of a `FunT` entry of `module`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let lambda = Lambda::new(atom, 0, 1, 53, 1);
    /// assert_eq!(lambda.label(), 53);
    /// ```
    pub(crate) fn new(module: Atom, index: i32, old_unique: i32, label: u32, arity: u32) -> Self {
        Self {
            module,
            index,
            old_unique,
            label,
            arity,
        }
    }

    /// Return the module that defines the lambda.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(lambda.module().name(), "fun_probe");
    /// ```
    pub(crate) fn module(&self) -> &Atom {
        &self.module
    }

    /// Return the entry label of the code of the lambda.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Lambda::new(atom, 0, 1, 53, 1).label(), 53);
    /// ```
    pub(crate) fn label(&self) -> u32 {
        self.label
    }

    /// Return the number of arguments of a call of the fun.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Lambda::new(atom, 0, 1, 53, 1).arity(), 1);
    /// ```
    pub(crate) fn arity(&self) -> u32 {
        self.arity
    }

    /// Compare the fun entries of two local funs, as `erts_cmp_compound`
    /// (`utils.c:2157-2177`) does: by the module name, then by the index,
    /// then by the old unique value.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use std::cmp::Ordering;
    ///
    /// let first = Lambda::new(atom.clone(), 0, 1, 53, 1);
    /// let second = Lambda::new(atom, 1, 1, 51, 0);
    /// assert_eq!(first.compare(&second), Ordering::Less);
    /// ```
    pub(super) fn compare(&self, other: &Lambda) -> std::cmp::Ordering {
        self.module
            .name()
            .cmp(other.module.name())
            .then(self.index.cmp(&other.index))
            .then(self.old_unique.cmp(&other.old_unique))
    }
}

/// A local fun: the fun entry of its lambda and the values of its free
/// variables. A fun can hold a term of any depth, so `Term` copies,
/// compares, and drops the free variables on its work stacks.
///
/// # Examples
///
/// ```rust,ignore
/// let fun = Fun::new(lambda, vec![Term::static_atom("tag")]);
/// assert_eq!(fun.free().len(), 1);
/// ```
pub struct Fun {
    /// The fun entry of the lambda.
    pub(super) lambda: Lambda,
    /// The values of the free variables, in `FunT` order.
    pub(super) free: Vec<Term>,
}

impl Fun {
    /// Return a fun of a lambda with the values of its free variables, as
    /// `make_fun3` makes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let fun = Fun::new(lambda, vec![]);
    /// assert!(fun.free().is_empty());
    /// ```
    pub(crate) fn new(lambda: Lambda, free: Vec<Term>) -> Self {
        Self { lambda, free }
    }

    /// Return the fun entry of the lambda.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Fun::new(lambda.clone(), vec![]).lambda(), &lambda);
    /// ```
    pub(crate) fn lambda(&self) -> &Lambda {
        &self.lambda
    }

    /// Return the values of the free variables.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Fun::new(lambda, vec![Term::Integer(1)]).free(), &[Term::Integer(1)]);
    /// ```
    pub(crate) fn free(&self) -> &[Term] {
        &self.free
    }
}
