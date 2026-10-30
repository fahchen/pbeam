//! Bodies of the BIFs that give the local node its name and its creation
//! (`dist.c`).

use super::{BifContext, BifResult, Term};
use crate::runtime::scheduler::node::Named;

/// The lowest creation of an alive node. `setnode/2` raises a lower
/// creation to it (`dist.c:5029-5030`).
const MIN_CREATION: u32 = 4;

/// Run `erlang:setnode(Name, Creation)` as ERTS `setnode_2` does
/// (`dist.c:4978-5071`): give the local node the name `Name` and the
/// creation `Creation`, mark the process registered as `net_kernel` as the
/// owner of the distribution, and return `true`. Its exit gives the node
/// `nonode@nohost` and the creation 0 again (see `Node::net_exits`).
///
/// `Creation` is a small integer that is not negative, and only its low 32
/// bits count (`term_to_Uint32`, `big.c:3648-3663`); a creation below 4
/// becomes 4. `badarg` comes for another creation, for a name that is not a
/// node name atom or is `nonode@nohost`, for a node that is alive, and when
/// no live process has the name `net_kernel`.
///
/// ERTS also skips each creation that a live identifier of another
/// incarnation of the same name has (`dist.c:5027-5035`). PBeam keeps no
/// node table, so it does not skip a creation; the row "Distribution" of
/// `docs/erts-differences.md` records this.
///
/// # Examples
///
/// ```rust,ignore
/// // net_kernel is registered and the node is not alive.
/// assert!(matches!(setnode(context, &name, &Term::Integer(1)), BifResult::Value(_)));
/// assert_eq!(context.scheduler.atoms.this_node(), (name, 4));
/// ```
pub(super) fn setnode(context: &mut BifContext<'_>, name: &Term, creation: &Term) -> BifResult {
    let scheduler = &context.scheduler;
    let atoms = &scheduler.atoms;
    // A small integer that is not negative gives its low 32 bits, as the
    // C cast to `Uint32` does.
    let Some(creation) = creation
        .small_integer()
        .filter(|creation| *creation >= 0)
        .map(|creation| {
            u32::try_from(creation & i64::from(u32::MAX)).expect("the low 32 bits fit in u32")
        })
    else {
        return BifResult::badarg();
    };
    let Term::Atom(name) = name else {
        return BifResult::badarg();
    };
    if !is_node_name(atoms.name_of(name).as_bytes()) || name.is_static("nonode@nohost") {
        return BifResult::badarg();
    }
    let Some(Named::Process(net_kernel)) = scheduler.name_owner("net_kernel") else {
        return BifResult::badarg();
    };
    let creation = creation.max(MIN_CREATION);
    if scheduler
        .node()
        .set_this_node(atoms, net_kernel, name, creation)
    {
        BifResult::Value(Term::static_atom("true"))
    } else {
        BifResult::badarg()
    }
}

/// Run `erts_internal:get_creation/0` as ERTS does (`dist.c:5571-5586`):
/// return the creation of the local node, or `undefined` for the creation 0
/// of a node that is not alive.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(get_creation(0), Term::static_atom("undefined"));
/// assert_eq!(get_creation(4), Term::Integer(4));
/// ```
pub(super) fn get_creation(creation: u32) -> Term {
    if creation == 0 {
        Term::static_atom("undefined")
    } else {
        Term::Integer(i64::from(creation))
    }
}

/// Return whether `name` is a full node name, as `is_node_name` checks it
/// (`dist.c:815-841`): one or more of `[a-zA-Z0-9_-]`, then `@`, then one or
/// more bytes with no other `@`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_node_name(b"a@host"));
/// assert!(!is_node_name(b"a.b@host"));
/// assert!(!is_node_name(b"a@b@c"));
/// assert!(!is_node_name(b"a@"));
/// ```
fn is_node_name(name: &[u8]) -> bool {
    let Some(at) = name
        .iter()
        .position(|&byte| !(byte == b'-' || byte == b'_' || byte.is_ascii_alphanumeric()))
    else {
        return false;
    };
    name[at] == b'@' && at > 0 && at + 1 < name.len() && !name[at + 1..].contains(&b'@')
}
