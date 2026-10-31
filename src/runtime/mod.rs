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

pub use capability::{
    AllowAll, Capability, CapabilityKind, Constraint, Delegation, Operation, Policy,
};
pub use extension::{
    Binding, Job, Library, LoadFn, NativeFn, NativeResult, NifContext, RegistrationError,
    register_library,
};
pub use host::environment::{add_erlexec_path, erlexec_command};
pub use host::signal::install_start_dispositions;
pub(crate) use host::signal::{Disposition, set_disposition};
pub(crate) use interpreter::bif::{BifId, import_kind, resolve as resolve_bif};
pub use interpreter::{BeamProgram, NifGap, RootAuthority, nif_manifest_gaps};
pub(crate) use nif::abi::Resource;
pub use schedulers::Schedulers;

use crate::term::Term;

/// Return the name of the local node of the run that owns `atoms`: the
/// node of each local pid, port, reference, registered name, and port name
/// (`erts_this_dist_entry->sysname`). It is `nonode@nohost` until
/// `erlang:setnode/2` gives the node a name (`dist.c:4978-5071`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(local_node(&AtomTable::new()), Term::static_atom("nonode@nohost"));
/// ```
fn local_node(atoms: &crate::term::AtomTable) -> Term {
    Term::Atom(atoms.this_node().0)
}
