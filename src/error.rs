use std::fmt;

/// The end of a load or a run without a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeamError {
    /// A runtime error, with its message.
    Runtime(String),
    /// `erlang:halt/2` stopped the run with this status code, as ERTS
    /// `erts_exit` (`bif.c:4346-4362`) stops the OS process. The code is the
    /// `Status` argument masked to `INT_MAX`. The host exit status is its low
    /// 8 bits, as for the C `exit`.
    Halt(i32),
}

impl fmt::Display for BeamError {
    /// Write the error message, or the status code of a halt.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(error("bad module").to_string(), "bad module");
    /// assert_eq!(BeamError::Halt(3).to_string(), "halted with status 3");
    /// ```
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(message) => formatter.write_str(message),
            Self::Halt(status) => write!(formatter, "halted with status {status}"),
        }
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
    BeamError::Runtime(message.into())
}

#[cfg(test)]
#[path = "../tests/error.rs"]
mod tests;
