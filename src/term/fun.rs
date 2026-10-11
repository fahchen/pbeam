//! A local or external fun, as in ERTS OTP 29.1.1 (`erl_fun.h`).
//!
//! ERTS keeps a fun in two parts. The fun thing (`ErlFunThing`) on the
//! process heap points to an entry and holds the free variables. A local
//! fun points to its fun entry (`ErlFunEntry`), which identifies the
//! function of the fun: the defining module, the index of the lambda in the
//! module, its old unique value, and its arity. An external fun points to
//! the export entry (`Export`) of a module, a function, and an arity, and
//! has no free variables. Here `Lambda` is the fun entry, with the entry
//! label of its code, `Target` is the entry of either kind, and `FunThing`
//! is the thing. All copies of a fun share one thing, as an ERTS register
//! copy of a fun term is the same boxed pointer to the `ErlFunThing`.
//!
//! `Fun` owns that shared reference. It is in an `Option`, because `Term`
//! has a `Drop`: the drop cannot move the `Arc` out of the term, so it
//! takes the `Arc` out of the `Option` and calls `Arc::into_inner` (see
//! [`Fun::take_last`]). Each other reader finds the thing.

use std::{cmp::Ordering, sync::Arc};

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
#[derive(Debug, Clone)]
pub(crate) struct Lambda {
    /// The module that defines the lambda.
    module: Atom,
    /// The index of the lambda in the `FunT` chunk (`ErlFunEntry.index`).
    pub(super) index: i32,
    /// The old unique value of the `FunT` entry (`ErlFunEntry.old_uniq`).
    pub(super) old_unique: i32,
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

impl PartialEq for Lambda {
    /// Compare two fun entries by the fields that identify an ERTS fun
    /// entry: the module, the index, the old unique value, and the arity
    /// (`fun_cmp` in `erl_fun.c:79-85`). The entry label is only the
    /// address of the code, so two entries with other labels can be equal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Lambda::new(atom.clone(), 0, 1, 53, 1), Lambda::new(atom.clone(), 0, 1, 61, 1));
    /// assert_ne!(Lambda::new(atom.clone(), 0, 1, 53, 1), Lambda::new(atom, 0, 1, 53, 2));
    /// ```
    fn eq(&self, other: &Self) -> bool {
        self.module == other.module
            && self.index == other.index
            && self.old_unique == other.old_unique
            && self.arity == other.arity
    }
}

impl Eq for Lambda {}

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

/// The thing of a fun, as the `ErlFunThing` of ERTS is: the entry that the
/// fun points to and the values of its free variables. An external fun has
/// no free variables.
struct FunThing {
    /// The entry of the fun.
    target: Target,
    /// The values of the free variables, in `FunT` order.
    free: Vec<Term>,
}

/// A fun: a shared reference to its thing. All copies of the fun own one
/// reference to the thing, so a copy takes O(1) time and shares the free
/// variables. The last copy that drops releases the thing.
///
/// # Examples
///
/// ```rust,ignore
/// let fun = Fun::new(lambda, vec![Term::static_atom("tag")]);
/// assert_eq!(fun.free().len(), 1);
/// assert!(Fun::external(module, function, 1).free().is_empty());
/// ```
#[derive(Clone)]
#[repr(transparent)]
pub struct Fun(Option<Arc<FunThing>>);

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
        Self::of(Target::Local(lambda), free)
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
        Self::of(
            Target::External {
                module,
                function,
                arity,
            },
            Vec::new(),
        )
    }

    /// Make a fun of an entry and free variables.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let fun = Fun::of(Target::Local(lambda), vec![]);
    /// assert_eq!(fun.free().len(), 0);
    /// ```
    fn of(target: Target, free: Vec<Term>) -> Self {
        Fun(Some(Arc::new(FunThing { target, free })))
    }

    /// Return the entry of the fun.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Fun::new(lambda.clone(), vec![]).target(), &Target::Local(lambda));
    /// ```
    pub(crate) fn target(&self) -> &Target {
        &self.arc().target
    }

    /// Return the values of the free variables.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Fun::new(lambda, vec![Term::Integer(1)]).free(), &[Term::Integer(1)]);
    /// ```
    pub(crate) fn free(&self) -> &[Term] {
        &self.arc().free
    }

    /// Return true when the two funs share one thing, as two ERTS fun terms
    /// with the same boxed pointer are the same fun.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let fun = Fun::new(lambda.clone(), vec![]);
    /// assert!(fun.same_thing(&fun.clone()));
    /// assert!(!fun.same_thing(&Fun::new(lambda, vec![])));
    /// ```
    pub(super) fn same_thing(&self, other: &Fun) -> bool {
        Arc::ptr_eq(self.arc(), other.arc())
    }

    /// Take the thing out of the fun, and return its free variables when
    /// this fun was the last holder of the thing. The function uses
    /// `Arc::into_inner`, which gives the thing to exactly one of the
    /// holders that drop at the same time.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut fun = Fun::new(lambda, vec![Term::Integer(1)]);
    /// let mut copy = fun.clone();
    /// assert!(fun.take_last().is_none());
    /// assert_eq!(copy.take_last(), Some(vec![Term::Integer(1)]));
    /// ```
    pub(super) fn take_last(&mut self) -> Option<Vec<Term>> {
        self.0
            .take()
            .and_then(Arc::into_inner)
            .map(|thing| thing.free)
    }

    /// Return the shared thing of the fun.
    fn arc(&self) -> &Arc<FunThing> {
        self.0
            .as_ref()
            .expect("only the drop of a term takes its fun thing")
    }
}

#[cfg(test)]
#[path = "../../tests/term/fun.rs"]
mod tests;
