//! A local or external fun, as in ERTS OTP 29.1.1 (`erl_fun.h`).
//!
//! ERTS keeps a fun in two parts. The fun thing (`ErlFunThing`) on the
//! process heap points to an entry and holds the free variables. A local
//! fun points to its fun entry (`ErlFunEntry`), which identifies the
//! function of the fun: the defining module, the index of the lambda in the
//! module, its old unique value, and its arity. An external fun points to
//! the export entry (`Export`) of a module, a function, and an arity, and
//! has no free variables. Here `Lambda` is the fun entry, with the entry
//! label of its code, `Target` is the entry of either kind, and `Fun` is the
//! thing.

use std::cmp::Ordering;

use super::{Atom, Term};

/// The fun entry of a lambda in a loaded module: the `FunT` entry that the
/// loader resolves, as ERTS `ErlFunEntry` is.
///
/// # Examples
///
/// ```rust,ignore
/// // The `FunT` entry of `fun(X) -> {Tag, X} end` in `fun_probe:locals/0`.
/// let module = AtomTable::new().put("fun_probe")?;
/// let lambda = Lambda::new(module, 0, 31_263_486, 127, 1);
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
    fn compare(&self, other: &Lambda) -> Ordering {
        self.module
            .name()
            .cmp(other.module.name())
            .then(self.index.cmp(&other.index))
            .then(self.old_unique.cmp(&other.old_unique))
    }
}

/// The entry that a fun points to: the fun entry of a lambda for a local
/// fun, or the module, function, and arity for an external fun, as the
/// `entry` union of ERTS `ErlFunThing` is.
///
/// # Examples
///
/// ```rust,ignore
/// // fun fun_probe:id/1
/// let target = Target::External { module, function, arity: 1 };
/// assert_eq!(target.arity(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A local fun of a lambda, which `make_fun3` makes.
    Local(Lambda),
    /// An external fun of `module:function/arity`, which
    /// `erlang:make_fun/3` makes. ERTS does not check that the function
    /// exists (`erts_export_get_or_make_stub` in `bif.c:4467-4485`).
    External {
        module: Atom,
        function: Atom,
        arity: u32,
    },
}

impl Target {
    /// Return the number of arguments of a call of the fun.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Target::Local(Lambda::new(atom, 0, 1, 53, 1)).arity(), 1);
    /// ```
    pub(crate) fn arity(&self) -> u32 {
        match self {
            Target::Local(lambda) => lambda.arity(),
            Target::External { arity, .. } => *arity,
        }
    }

    /// Compare the entries of two funs, as `erts_cmp_compound`
    /// (`utils.c:2157-2207`) does. Two local funs compare by their fun
    /// entries. Two external funs compare by the module name, then by the
    /// function name, then by the arity. A local fun is less than an
    /// external fun.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use std::cmp::Ordering;
    ///
    /// let local = Target::Local(Lambda::new(atom.clone(), 0, 1, 53, 1));
    /// let external = Target::External { module: atom.clone(), function: atom, arity: 0 };
    /// assert_eq!(local.compare(&external), Ordering::Less);
    /// ```
    pub(super) fn compare(&self, other: &Target) -> Ordering {
        match (self, other) {
            (Target::Local(left), Target::Local(right)) => left.compare(right),
            (
                Target::External {
                    module,
                    function,
                    arity,
                },
                Target::External {
                    module: other_module,
                    function: other_function,
                    arity: other_arity,
                },
            ) => module
                .name()
                .cmp(other_module.name())
                .then(function.name().cmp(other_function.name()))
                .then(arity.cmp(other_arity)),
            (Target::Local(_), Target::External { .. }) => Ordering::Less,
            (Target::External { .. }, Target::Local(_)) => Ordering::Greater,
        }
    }
}

/// A fun: the entry that it points to and the values of its free
/// variables. A fun can hold a term of any depth, so `Term` copies,
/// compares, and drops the free variables on its work stacks. An external
/// fun has no free variables.
///
/// # Examples
///
/// ```rust,ignore
/// let fun = Fun::new(lambda, vec![Term::static_atom("tag")]);
/// assert_eq!(fun.free().len(), 1);
/// assert!(Fun::external(module, function, 1).free().is_empty());
/// ```
pub struct Fun {
    /// The entry of the fun.
    pub(super) target: Target,
    /// The values of the free variables, in `FunT` order.
    pub(super) free: Vec<Term>,
}

impl Fun {
    /// Return a local fun of a lambda with the values of its free
    /// variables, as `make_fun3` makes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let fun = Fun::new(lambda, vec![]);
    /// assert!(fun.free().is_empty());
    /// ```
    pub(crate) fn new(lambda: Lambda, free: Vec<Term>) -> Self {
        Self {
            target: Target::Local(lambda),
            free,
        }
    }

    /// Return the external fun of `module:function/arity`, as
    /// `erlang:make_fun/3` makes it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let fun = Fun::external(module, function, 1);
    /// assert_eq!(fun.target().arity(), 1);
    /// ```
    pub(crate) fn external(module: Atom, function: Atom, arity: u32) -> Self {
        Self {
            target: Target::External {
                module,
                function,
                arity,
            },
            free: Vec::new(),
        }
    }

    /// Return the entry of the fun.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Fun::new(lambda.clone(), vec![]).target(), &Target::Local(lambda));
    /// ```
    pub(crate) fn target(&self) -> &Target {
        &self.target
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
