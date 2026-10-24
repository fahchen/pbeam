mod atomics;
mod binary_pattern;
mod buffers;
mod capability;
mod dirty;
mod ets;
mod extension;
mod files;
mod heap_flags;
mod host;
mod interpreter;
mod mailbox;
mod nif;
mod pcre2;
mod persistent_term;
mod port;
mod process;
mod regex_code;
mod resources;
mod scheduler;
mod schedulers;
mod signal;
mod sockets;
mod time;
mod timer;
mod tty;
mod unicode;

pub use capability::{AllowAll, Capability, CapabilityKind, Constraint, Operation, Policy};
pub use extension::{
    Binding, Job, Library, LoadFn, NativeFn, NativeResult, NifContext, RegistrationError,
    register_library,
};
pub use host::environment::add_erlexec_path;
pub use host::signal::install_start_dispositions;
pub(crate) use interpreter::bif::{BifId, import_kind, resolve as resolve_bif};
pub use interpreter::{BeamProgram, NifGap, nif_manifest_gaps};
pub use schedulers::Schedulers;

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
