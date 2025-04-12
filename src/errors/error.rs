use std::fmt;
use std::io;
use std::path::StripPrefixError;
use crate::core::lockfile::LockError;

#[derive(Debug)]
pub enum Error {
    PathResolution(String),
    DirectoryCreation(String),
    InvalidPath(String),
    Generic(String),
    IO(io::Error),
    Pattern(String),  // For pattern matching errors
    Lock(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::PathResolution(msg) => write!(f, "Path resolution error: {}", msg),
            Error::DirectoryCreation(msg) => write!(f, "Directory creation error: {}", msg),
            Error::InvalidPath(msg) => write!(f, "Invalid path: {}", msg),
            Error::Generic(msg) => write!(f, "Error: {}", msg),
            Error::IO(err) => write!(f, "IO error: {}", err),
            Error::Pattern(msg) => write!(f, "Pattern error: {}", msg),
            Error::Lock(msg) => write!(f, "Lock error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::IO(error)
    }
}

impl From<StripPrefixError> for Error {
    fn from(error: StripPrefixError) -> Self {
        Error::Generic(format!("Failed to strip path prefix: {}", error))
    }
}

impl From<regex::Error> for Error {
    fn from(err: regex::Error) -> Self {
        Error::Pattern(format!("Invalid pattern: {}", err))
    }
}

impl From<LockError> for Error {
    fn from(error: LockError) -> Self {
        Error::Lock(format!("{:?}", error))
    }
}