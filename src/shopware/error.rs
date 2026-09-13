//! Exit-code-bearing errors for Shopware commands.

use std::fmt;
use std::process::ExitCode;

#[derive(Debug)]
pub enum Error {
    /// Operator / environment failure (exit 1), matching recipe `die`.
    Fail(String),
    /// Verb or option not implemented yet (exit 2).
    NotImplemented(String),
}

impl Error {
    pub fn fail(msg: impl Into<String>) -> Self {
        Self::Fail(msg.into())
    }

    pub fn stub(msg: impl Into<String>) -> Self {
        Self::NotImplemented(msg.into())
    }

    pub fn print(&self) {
        match self {
            Self::Fail(m) => eprintln!("ERROR: {m}"),
            Self::NotImplemented(m) => eprintln!("not implemented: {m}"),
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Fail(_) => ExitCode::from(1),
            Self::NotImplemented(_) => ExitCode::from(2),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fail(m) => write!(f, "{m}"),
            Self::NotImplemented(m) => write!(f, "not implemented: {m}"),
        }
    }
}

impl std::error::Error for Error {}
