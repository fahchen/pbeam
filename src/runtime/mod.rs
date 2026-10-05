mod ets;
mod host;
mod interpreter;
mod mailbox;
mod pcre2;
mod persistent_term;
mod process;
mod regex_code;
mod resources;
mod scheduler;
mod time;
mod timer;
mod tty;
mod unicode;

pub use interpreter::BeamProgram;

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
