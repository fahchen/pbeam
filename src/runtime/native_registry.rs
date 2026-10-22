//! A manual registry of Rust native functions, keyed by MFA.
//!
//! A launcher or a test registers a Rust function under an exact
//! `module:name/arity`. The interpreter checks this registry at the
//! `NifStart` instruction of a loaded module, before the static binding
//! table. A registered function runs only when the BEAM body is a NIF stub,
//! so an ordinary BEAM implementation keeps its behavior. A registered
//! function receives the arguments of the call and returns a term or the
//! `error:badarg` exception. Its [`Job`] tells where it runs: a `Normal`
//! function runs on the scheduler thread as a synchronous BIF, and a
//! `CpuBound` or `IoBound` function runs on a dirty thread of the node while
//! its caller switches out.
//!
//! The registry is global and process-safe. Duplicate registration of the
//! same MFA is an error.
//!
//! [`lookup_native`] runs at the `NifStart` instruction of every call, also
//! of the `prim_file`, `prim_tty`, `zlib`, and `crypto` bindings that PBeam
//! links without a registration. A `NON_EMPTY` flag skips the lock and the
//! two key allocations while the registry holds no entry.
//!
//! [`unregister_native`] removes a registration. A test removes its own
//! registration, so the next test sees no leftover entry of the global
//! registry. The operation stays public because an integration test compiles
//! without `cfg(test)`, and a test-only reset cannot serve it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::BeamError;
use crate::Term;

/// The Rust function of a registered native MFA. It receives the arguments
/// of the call, which have already passed the arity check of the call site.
pub type NativeFn = fn(&[Term]) -> Result<NativeResult, BeamError>;

/// The result of a registered native function.
///
/// The type is narrow, so the interpreter internals, such as the full
/// `BifResult`, stay crate-private. It has no trap or yield variant: a
/// registered function runs to completion.
#[derive(Debug)]
pub enum NativeResult {
    /// The native returns this value.
    Value(Term),
    /// The native raises `error:Reason` in the calling process, as the ERTS
    /// `enif_raise_exception` and `enif_make_badarg` do. ERTS lets a NIF
    /// raise only the class `error`.
    Raise {
        /// Reason of the exception, without a stack trace.
        reason: Term,
    },
}

impl NativeResult {
    /// Return the `error:badarg` exception, as `enif_make_badarg` does.
    pub fn badarg() -> NativeResult {
        NativeResult::Raise {
            reason: Term::static_atom("badarg"),
        }
    }
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

/// The error of a registration that cannot succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    /// The MFA already has a registered function.
    Duplicate,
}

/// The key of a registration: the module, the name, and the arity.
type Key = (String, String, u32);

/// The registry of native functions.
type Registry = Mutex<HashMap<Key, (Job, NativeFn)>>;

/// The registry. The first registration creates the map, so a lookup with no
/// registration finds no map and takes no lock.
static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Whether the registry holds at least one entry. [`lookup_native`] reads the
/// flag before the lock and the key allocations, so a lookup with an empty
/// registry stays cheap at the `NifStart` instruction of every call.
static NON_EMPTY: AtomicBool = AtomicBool::new(false);

/// Return the registry, creating it on the first use.
fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a Rust function under `module:name/arity`, which runs as `job`
/// gives.
///
/// # Errors
///
/// [`RegistrationError::Duplicate`] when the MFA already has a function.
pub fn register_native(
    module: &str,
    name: &str,
    arity: u32,
    job: Job,
    function: NativeFn,
) -> Result<(), RegistrationError> {
    let mut map = registry().lock().unwrap();
    let key = (module.to_owned(), name.to_owned(), arity);
    if map.contains_key(&key) {
        return Err(RegistrationError::Duplicate);
    }
    map.insert(key, (job, function));
    NON_EMPTY.store(true, Ordering::Release);
    Ok(())
}

/// Return the registered function of `module:name/arity`, or `None`.
pub fn lookup_native(module: &str, name: &str, arity: u32) -> Option<NativeFn> {
    lookup(module, name, arity).map(|(_, function)| function)
}

/// Return the job and the registered function of `module:name/arity`, or
/// `None`.
pub(crate) fn lookup(module: &str, name: &str, arity: u32) -> Option<(Job, NativeFn)> {
    // The fast path: with no registration the registry holds no entry, so
    // skip the lock and the two key allocations of a `NifStart` that runs a
    // static binding.
    if !NON_EMPTY.load(Ordering::Acquire) {
        return None;
    }
    registry()
        .lock()
        .unwrap()
        .get(&(module.to_owned(), name.to_owned(), arity))
        .copied()
}

/// Remove a registration. Return `true` when it existed.
pub fn unregister_native(module: &str, name: &str, arity: u32) -> bool {
    let mut map = registry().lock().unwrap();
    let removed = map
        .remove(&(module.to_owned(), name.to_owned(), arity))
        .is_some();
    if map.is_empty() {
        NON_EMPTY.store(false, Ordering::Release);
    }
    removed
}
