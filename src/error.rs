use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeamError(String);

impl fmt::Display for BeamError {
    /// Write the error message.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(error("bad module").to_string(), "bad module");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BeamError {}

/// Create a runtime error with a message.
///
/// # Examples
///
/// ```rust,ignore
/// let failure = error("missing BEAM module hello");
/// assert_eq!(failure.to_string(), "missing BEAM module hello");
/// ```
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
