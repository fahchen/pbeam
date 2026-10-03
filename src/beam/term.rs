#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Atom(String),
    Binary(Vec<u8>),
    Integer(i64),
    /// A proper list of BEAM terms.
    List(Vec<Term>),
}
