//! A manual registry of Rust native functions, keyed by MFA.
//!
//! A launcher or a test registers a Rust function under an exact
//! `module:name/arity`. The interpreter checks this registry at the
//! `NifStart` instruction of a loaded module, before the static binding
//! table. A registered function runs only when the BEAM body is a NIF stub,
//! so an ordinary BEAM implementation keeps its behavior. A registered
//! function runs as a synchronous BIF: it receives the arguments of the call
//! and returns a term or a runtime error.
//!
//! The registry is global and process-safe. Duplicate registration of the
//! same MFA is an error.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::BeamError;
use crate::runtime::interpreter::bif::BifResult;
use crate::runtime::scheduler::Scheduler;

/// The Rust function of a registered native MFA. It receives the current
/// scheduler and the arguments of the call. The arguments have already passed
/// the arity check of the call site.
#[allow(private_interfaces)]
pub type NativeFn = fn(&mut Scheduler, &[crate::Term]) -> Result<BifResult, BeamError>;

/// The error of a registration that cannot succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    /// The MFA already has a registered function.
    Duplicate,
}

#[allow(clippy::type_complexity)]
static REGISTRY: Mutex<Option<HashMap<(String, String, u32), NativeFn>>> = Mutex::new(None);

#[allow(clippy::type_complexity)]
fn registry() -> std::sync::MutexGuard<'static, Option<HashMap<(String, String, u32), NativeFn>>> {
    let mut guard = REGISTRY.lock().unwrap();
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

/// Register a Rust function under `module:name/arity`.
///
/// # Errors
///
/// [`RegistrationError::Duplicate`] when the MFA already has a function.
pub fn register_native(
    module: &str,
    name: &str,
    arity: u32,
    function: NativeFn,
) -> Result<(), RegistrationError> {
    let mut guard = registry();
    let map = guard.as_mut().unwrap();
    let key = (module.to_owned(), name.to_owned(), arity);
    if map.contains_key(&key) {
        return Err(RegistrationError::Duplicate);
    }
    map.insert(key, function);
    Ok(())
}

/// Return the registered function of `module:name/arity`, or `None`.
pub fn lookup_native(module: &str, name: &str, arity: u32) -> Option<NativeFn> {
    let guard = registry();
    guard
        .as_ref()
        .unwrap()
        .get(&(module.to_owned(), name.to_owned(), arity))
        .copied()
}

/// Remove a registration. Returns `true` when it existed.
pub fn unregister_native(module: &str, name: &str, arity: u32) -> bool {
    let mut guard = registry();
    guard
        .as_mut()
        .unwrap()
        .remove(&(module.to_owned(), name.to_owned(), arity))
        .is_some()
}
