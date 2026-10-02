#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Atom(String),
    Integer(i64),
}
