mod atomics;
mod binary_pattern;
mod buffers;
mod capability;
mod dirty_io;
mod ets;
mod files;
mod host;
mod interpreter;
mod mailbox;
mod native_registry;
mod pcre2;
mod persistent_term;
mod port;
mod process;
mod regex_code;
mod resources;
mod scheduler;
mod signal;
mod time;
mod timer;
mod tty;
mod unicode;

pub use capability::{AllowAll, Capability, CapabilityKind, Constraint, Operation, Policy};
pub use host::signal::install_start_dispositions;
pub use interpreter::BeamProgram;
pub use interpreter::ExceptionClass;
pub use interpreter::bif::BifResult;
pub(crate) use interpreter::bif::{BifId, import_kind, resolve as resolve_bif};
pub use native_registry::{
    NativeFn, RegistrationError, lookup_native, register_native, unregister_native,
};
pub use scheduler::Scheduler;

use crate::term::Term;

/// Return the name of the local node.
///
/// PBeam is a local node that is not distributed, so every pid, registered
/// name, and port name is local and its node is `nonode@nohost`, as on ERTS
/// (`dist.c`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(local_node(), Term::static_atom("nonode@nohost"));
/// ```
fn local_node() -> Term {
    Term::static_atom("nonode@nohost")
}
