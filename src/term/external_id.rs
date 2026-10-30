//! The pids, ports, and references of another node.
//!
//! ERTS keeps such an identifier as an `ExternalThing`: a pointer to the
//! node entry of the node name and its creation, and the identifier data
//! (`erl_node_tables.h:213-227`, `erl_term.h:1150-1230`). `binary_to_term/1`
//! makes one for a pid, a port, or a reference whose node name and creation
//! are not those of the local node (`dec_pid`, `external.c:3133-3200`,
//! `4685-4740`, `4786-4890`). PBeam is never alive, so its local node is
//! `nonode@nohost` with the creation 0.

use super::{Atom, PortId, ProcessId, Reference, Term};

/// The number of the bits of the process number in the data of a local
/// pid (`_PID_NUM_SIZE`, `erl_term.h:609-627`).
pub(crate) const PID_NUMBER_BITS: u32 = 28;
/// The mask of the process number in the data of a local pid.
pub(crate) const PID_NUMBER_MASK: u64 = (1 << PID_NUMBER_BITS) - 1;

/// The name of the local node of a node that is not alive.
pub(crate) const LOCAL_NODE: &str = "nonode@nohost";

/// A pid, a port, or a reference of another node: the node name, the
/// creation of the node, and the identifier data.
///
/// # Examples
///
/// ```rust,ignore
/// let id = ExternalId { node, creation: 5, data: ExternalData::Port(7) };
/// // printed as #Port<N.7>, with N the atom index of the node name
/// ```
#[derive(Debug, Clone)]
pub struct ExternalId {
    /// The node name, an atom of the atom table of the program.
    pub(crate) node: Atom,
    /// The creation of the node incarnation.
    pub(crate) creation: u32,
    /// The pid, port, or reference data.
    pub(crate) data: ExternalData,
}

/// The data of an [`ExternalId`].
#[derive(Debug, Clone)]
pub(crate) enum ExternalData {
    /// A pid: the process number and the serial (`data.pid`).
    Pid { number: u32, serial: u32 },
    /// A port: the 64-bit port number (`data.port.id`).
    Port(u64),
    /// A reference: its 0 to 5 reference numbers, the low number first
    /// (`external_thing_ref_numbers`).
    Reference(Box<[u32]>),
}

impl PartialEq for ExternalId {
    /// Compare as `eq` (`utils.c:1279-1380`) does: the node, the creation,
    /// and the data. Two references are equal when their numbers are equal
    /// after the high numbers that are 0.
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
            && self.creation == other.creation
            && match (&self.data, &other.data) {
                (
                    ExternalData::Pid { number, serial },
                    ExternalData::Pid {
                        number: other_number,
                        serial: other_serial,
                    },
                ) => (number, serial) == (other_number, other_serial),
                (ExternalData::Port(left), ExternalData::Port(right)) => left == right,
                (ExternalData::Reference(left), ExternalData::Reference(right)) => {
                    significant(left) == significant(right)
                }
                _ => false,
            }
    }
}

/// Return the reference numbers without the high numbers that are 0.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(significant(&[9, 0, 0]), &[9]);
/// ```
pub(crate) fn significant(numbers: &[u32]) -> &[u32] {
    let length = numbers
        .iter()
        .rposition(|&number| number != 0)
        .map_or(0, |last| last + 1);
    &numbers[..length]
}

/// The node of a pid, a port, or a reference term: its node name and
/// creation, `None` for the local node.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(node_of(&Term::Pid(pid)).is_none());
/// ```
pub(crate) fn node_of(term: &Term) -> Option<(&Atom, u32)> {
    match term {
        Term::External(id) => Some((&id.node, id.creation)),
        _ => None,
    }
}

/// The pid number and serial of a local or a remote pid term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(pid_parts(&Term::Pid(ProcessId(5))), Some((5, 0)));
/// ```
pub(crate) fn pid_parts(term: &Term) -> Option<(u64, u64)> {
    match term {
        Term::Pid(ProcessId(data)) => Some((data & PID_NUMBER_MASK, data >> PID_NUMBER_BITS)),
        Term::External(id) => match id.data {
            ExternalData::Pid { number, serial } => Some((u64::from(number), u64::from(serial))),
            _ => None,
        },
        _ => None,
    }
}

/// The port number of a local or a remote port term.
pub(crate) fn port_number(term: &Term) -> Option<u64> {
    match term {
        Term::Port(PortId(number)) => Some(*number),
        Term::External(id) => match id.data {
            ExternalData::Port(number) => Some(number),
            _ => None,
        },
        _ => None,
    }
}

/// Return true for a local or a remote reference term, also a resource
/// term, which is a magic reference.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_reference(&Term::Reference(Reference(1))));
/// ```
pub(crate) fn is_reference(term: &Term) -> bool {
    match term {
        Term::Reference(_) | Term::Resource(_) => true,
        Term::External(id) => matches!(id.data, ExternalData::Reference(_)),
        _ => false,
    }
}

/// The reference numbers, the low number first, of a local or a remote
/// reference term.
pub(crate) fn reference_numbers(term: &Term) -> Option<Vec<u32>> {
    if let Term::External(id) = term
        && let ExternalData::Reference(numbers) = &id.data
    {
        return Some(numbers.to_vec());
    }
    term.reference()
        .map(|reference: Reference| reference.numbers().to_vec())
}
