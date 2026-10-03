use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeamError(String);

impl fmt::Display for BeamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BeamError {}

pub(crate) fn error(message: impl Into<String>) -> BeamError {
    BeamError(message.into())
}

#[cfg(test)]
mod tests {
    use super::error;

    #[test]
    fn formats_runtime_errors() {
        assert_eq!(error("bad module").to_string(), "bad module");
    }
}
