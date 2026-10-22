//! The static Rust extension API: a [`Library`] of [`Binding`] entries that
//! an extension crate links into the executable and registers before a run.
//!
//! An extension library is a static NIF library of one module, as the
//! libraries that PBeam links itself, such as `prim_tty`. ERTS selects a
//! static NIF library by the module name (`is_static_nif_module` in
//! `beam/erl_nif.c:5324-5331`). `erlang:load_nif/2` runs the checks of
//! ERTS against the `functions` of the library and then its load callback.
//! A binding becomes active for the loaded instance of the module only when
//! that load succeeds (`beam/erl_nif.c:4990-5007`); then the `nif_start`
//! instruction at the entry of the function runs it. Before the load, and
//! after a failed load, the function runs its BEAM stub body. A function
//! that the module declares with `PBeam.Native` runs its binding with no
//! `load_nif/2`. A function of `functions` without a binding runs its BEAM
//! stub body, or gives a runtime error when the library sets
//! `unbound_fails`.
//!
//! A launcher or a test calls [`register_library`] before it runs the
//! program. The registry is global and process-safe, and it keeps each
//! library for the life of the process. A second library of a module, or a
//! library of a module that PBeam links itself, is an error.
//!
//! A binding runs with a [`NifContext`], which gives the atoms of the run
//! and the exceptions of a NIF, and with the arguments of the call. Its
//! [`Job`] tells where it runs: a `Normal` binding runs on the scheduler
//! thread, and a `CpuBound` or `IoBound` binding runs on a dirty thread of
//! the node while its caller switches out.

use std::sync::RwLock;

use crate::BeamError;
use crate::Term;
use crate::error::error;
use crate::term::{Atom, AtomTable, MAX_ATOM_CHARACTERS};

/// The Rust function of a binding. It receives the context of the call and
/// the arguments of the call, which have already passed the arity check of
/// the call site.
pub type NativeFn = fn(&mut NifContext<'_>, &[Term]) -> Result<NativeResult, BeamError>;

/// The load callback of a library. It receives the context of the call and
/// the `LoadInfo` argument of `erlang:load_nif/2`, and returns the veto of
/// the callback: 0 loads the library, as the `load` function of an ERTS
/// `ErlNifEntry` (`beam/erl_nif.c:4980-4986`).
pub type LoadFn = fn(&mut NifContext<'_>, &Term) -> i32;

/// The result of a binding.
///
/// The type is narrow, so the interpreter internals, such as the full
/// `BifResult`, stay crate-private. It has no trap or yield variant: a
/// binding runs to completion. [`NifContext::raise`] and
/// [`NifContext::badarg`] give the `Raise` variant.
#[derive(Debug)]
pub enum NativeResult {
    /// The binding returns this value.
    Value(Term),
    /// The binding raises `error:Reason` in the calling process, as the
    /// ERTS `enif_raise_exception` and `enif_make_badarg` do. ERTS lets a
    /// NIF raise only the class `error`.
    Raise {
        /// Reason of the exception, without a stack trace.
        reason: Term,
    },
}

/// The scheduler that runs a NIF, as the `flags` of an ERTS `ErlNifFunc`:
/// 0, `ERL_NIF_DIRTY_JOB_CPU_BOUND`, or `ERL_NIF_DIRTY_JOB_IO_BOUND`
/// (`erl_nif.h:239-243`).
///
/// # Examples
///
/// ```rust
/// use pbeam_runtime::Job;
///
/// assert_ne!(Job::Normal, Job::IoBound);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// The NIF runs on the scheduler thread of its caller and must be short.
    Normal,
    /// The NIF runs on a dirty CPU thread. The node has one dirty CPU
    /// thread for each scheduler (`erl_init.c:865`).
    CpuBound,
    /// The NIF runs on a dirty IO thread. The node has 10 dirty IO threads
    /// (`erl_init.c:867`).
    IoBound,
}

/// The binding of one NIF function of a [`Library`]. ERTS finds a NIF of a
/// library by its exact name and arity (`beam/erl_nif.c:4845-4866`).
///
/// # Examples
///
/// ```rust
/// use pbeam_runtime::{Binding, Job, NativeResult, NifContext, Term};
///
/// fn zero(_: &mut NifContext<'_>, _: &[Term]) -> Result<NativeResult, pbeam_runtime::BeamError> {
///     Ok(NativeResult::Value(Term::Integer(0)))
/// }
///
/// const ZERO: Binding = Binding { name: "zero", arity: 0, job: Job::Normal, run: zero };
/// assert_eq!(ZERO.arity, 0);
/// ```
#[derive(Debug)]
pub struct Binding {
    /// The name of the NIF function.
    pub name: &'static str,
    /// The arity of the NIF function.
    pub arity: u32,
    /// The scheduler of the NIF, as the `flags` of its ERTS `ErlNifFunc`.
    pub job: Job,
    /// Runs the NIF. The dispatch gives exactly `arity` arguments.
    pub run: NativeFn,
}

/// The static NIF library of one module.
///
/// # Examples
///
/// ```rust
/// use pbeam_runtime::Library;
///
/// static EMPTY: Library = Library {
///     module: "my_nif",
///     functions: &[("info", 0)],
///     bindings: &[],
///     load: None,
///     unbound_fails: true,
/// };
/// assert_eq!(EMPTY.functions, [("info", 0)]);
/// ```
#[derive(Debug)]
pub struct Library {
    /// The name of the module of the library.
    pub module: &'static str,
    /// Each function of the library, as `(name, arity)` in the order of the
    /// `funcs` of an ERTS `ErlNifEntry`. `erlang:load_nif/2` checks them in
    /// this order.
    pub functions: &'static [(&'static str, u32)],
    /// The bindings of the functions that the library implements. Each
    /// binding must name a function of `functions`, at most once.
    pub bindings: &'static [Binding],
    /// The load callback, which runs with the `LoadInfo` of the first
    /// successful `erlang:load_nif/2` of the run. `None` is a callback that
    /// returns 0 for each load info.
    pub load: Option<LoadFn>,
    /// `true` when a call of a function of `functions` without a binding
    /// gives the runtime error `M:F/A NIF is not supported`, `false` when
    /// it runs the BEAM stub body of the function.
    pub unbound_fails: bool,
}

/// The error of a registration that cannot succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    /// The module already has a registered library or a library that PBeam
    /// links itself.
    Duplicate,
    /// The binding of `name/arity` names no function of `functions`.
    UnlistedBinding {
        /// The name of the binding.
        name: &'static str,
        /// The arity of the binding.
        arity: u32,
    },
    /// The library has two bindings of `name/arity`.
    DuplicateBinding {
        /// The name of the binding.
        name: &'static str,
        /// The arity of the binding.
        arity: u32,
    },
}

/// The context of a binding or a load callback, as the `ErlNifEnv` of an
/// ERTS NIF. It gives the atoms of the run and the exceptions of a NIF, and
/// it holds no scheduler state.
#[derive(Debug)]
pub struct NifContext<'a> {
    /// The atom table of the scheduler of the call, or `None` on a dirty
    /// thread, which has no atom table.
    atoms: Option<&'a AtomTable>,
}

impl<'a> NifContext<'a> {
    /// Return the context of a call on the scheduler that owns `atoms`.
    pub(crate) fn new(atoms: &'a AtomTable) -> Self {
        Self { atoms: Some(atoms) }
    }

    /// Return the context of a call on a dirty thread.
    pub(crate) fn dirty() -> Self {
        Self { atoms: None }
    }

    /// Return the atom of the UTF-8 `name`, as `enif_make_new_atom` with
    /// `ERL_NIF_UTF8` does (`beam/erl_nif.c:2100-2114`). On the scheduler
    /// thread, a new name goes into the atom table of the run. On a dirty
    /// thread, only a static atom, such as `ok` or `error`, is available.
    /// As with the failure result of `enif_make_new_atom`, the binding
    /// decides what an error gives, for example [`NifContext::badarg`].
    ///
    /// # Errors
    ///
    /// A name of more than 255 characters, which `erts_atom_put` rejects
    /// (`MAX_ATOM_CHARACTERS`), is the error `the atom name has more than
    /// 255 characters`. A name that is not a static atom on a dirty thread
    /// is the unsupported error `a dirty NIF cannot make the atom Name`. A
    /// new atom in a full atom table gives the error of the table.
    pub fn atom(&mut self, name: &str) -> Result<Atom, BeamError> {
        if name.chars().count() > MAX_ATOM_CHARACTERS {
            return Err(error(format!(
                "the atom name has more than {MAX_ATOM_CHARACTERS} characters"
            )));
        }
        match &self.atoms {
            Some(atoms) => atoms.put(name),
            None => Atom::from_static(name).ok_or_else(|| {
                error(format!(
                    "unsupported: a dirty NIF cannot make the atom {name}, which is not a static atom"
                ))
            }),
        }
    }

    /// Return the `error:Reason` exception, as `enif_raise_exception` does.
    pub fn raise(&self, reason: Term) -> NativeResult {
        NativeResult::Raise { reason }
    }

    /// Return the `error:badarg` exception, as `enif_make_badarg` does.
    pub fn badarg(&self) -> NativeResult {
        self.raise(Term::static_atom("badarg"))
    }
}

/// The registered extension libraries. A dispatch takes the read lock, so
/// the schedulers of a run look up a library at the same time.
static REGISTRY: RwLock<Vec<&'static Library>> = RwLock::new(Vec::new());

/// Register `library` for the runs of this process. Call it before the
/// run that loads the module of the library.
///
/// # Errors
///
/// - [`RegistrationError::Duplicate`] when the module already has a
///   registered library or a library that PBeam links itself.
/// - [`RegistrationError::UnlistedBinding`] when a binding names no
///   function of `functions`.
/// - [`RegistrationError::DuplicateBinding`] when two bindings name the
///   same function.
pub fn register_library(library: &'static Library) -> Result<(), RegistrationError> {
    for (index, binding) in library.bindings.iter().enumerate() {
        let (name, arity) = (binding.name, binding.arity);
        if !library.functions.contains(&(name, arity)) {
            return Err(RegistrationError::UnlistedBinding { name, arity });
        }
        if library.bindings[..index]
            .iter()
            .any(|earlier| (earlier.name, earlier.arity) == (name, arity))
        {
            return Err(RegistrationError::DuplicateBinding { name, arity });
        }
    }
    let mut registry = REGISTRY.write().unwrap();
    let linked = crate::runtime::interpreter::has_static_library(library.module);
    if linked || registry.iter().any(|found| found.module == library.module) {
        return Err(RegistrationError::Duplicate);
    }
    registry.push(library);
    Ok(())
}

/// Return the registered library of `module`, or `None`.
pub(crate) fn library(module: &str) -> Option<&'static Library> {
    let registry = REGISTRY.read().unwrap();
    registry
        .iter()
        .find(|library| library.module == module)
        .copied()
}

/// Return the binding of exactly `module:name/arity` in a registered
/// library, or `None`.
pub(crate) fn binding(module: &str, name: &str, arity: u32) -> Option<&'static Binding> {
    library(module)?
        .bindings
        .iter()
        .find(|binding| binding.name == name && binding.arity == arity)
}
