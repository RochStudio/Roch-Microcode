use std::fmt;

/// Error categories mirror the .NET exception types the original relied on:
/// `InvalidDataException` (unsafe or unsupported input), `IOException`
/// (environment / device problems) and `OperationCanceledException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The input is unsupported, inconsistent or unsafe to modify.
    InvalidData(String),
    /// File-system, process or device failure.
    Io(String),
    /// The user cancelled the operation.
    Cancelled,
}

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Error::InvalidData(message.into())
    }
    pub fn io(message: impl Into<String>) -> Self {
        Error::Io(message.into())
    }
    pub fn is_invalid_data(&self) -> bool {
        matches!(self, Error::InvalidData(_))
    }
    pub fn is_io(&self) -> bool {
        matches!(self, Error::Io(_))
    }
    pub fn message(&self) -> &str {
        match self {
            Error::InvalidData(m) | Error::Io(m) => m,
            Error::Cancelled => "The operation was cancelled.",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Equivalent of the original `Require(condition, message)` guard.
#[inline]
pub fn require(ok: bool, message: impl Into<String>) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidData(message.into()))
    }
}
