use serde::Serialize;
use serde_json::{json, Value};
use std::{fmt, io};

pub type Result<T> = std::result::Result<T, Error>;

/// Stable codes are intended for programs. Messages are intended for people.
#[derive(Debug, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    pub details: Value,
    #[serde(skip)]
    pub exit: u8,
}

impl Error {
    pub fn new(code: &'static str, exit: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: json!({}),
            exit,
        }
    }

    pub fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("INVALID_ARGUMENT", 4, message)
    }

    pub fn io(action: &str, error: io::Error) -> Self {
        let code = match error.kind() {
            io::ErrorKind::NotFound => "FILE_NOT_FOUND",
            io::ErrorKind::PermissionDenied => "PERMISSION_DENIED",
            io::ErrorKind::AlreadyExists => "OUTPUT_EXISTS",
            _ => "IO_ERROR",
        };
        Self::new(code, 5, format!("{action}: {error}"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
