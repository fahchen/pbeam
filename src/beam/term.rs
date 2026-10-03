#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Atom(String),
    Binary(Vec<u8>),
    Integer(i64),
}
