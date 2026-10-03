#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Atom(String),
    Binary(Vec<u8>),
    Integer(i64),
    /// A proper list of BEAM terms.
    List(Vec<Term>),
    /// A BEAM tuple.
    Tuple(Vec<Term>),
    /// A process identifier owned by this runtime.
    Pid(ProcessId),
    /// A reference owned by this runtime.
    Reference(Reference),
}

/// An identifier for a process in this runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessId(pub(crate) u64);

/// An identifier for a reference in this runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reference(pub(crate) u64);
