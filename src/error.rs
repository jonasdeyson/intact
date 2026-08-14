use std::fmt;

/// Error categories. Each maps to a distinct process exit code so that a calling
/// agent can branch on the failure without parsing prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Catch-all failure.
    Other,
    /// I/O failure (permissions, missing directory, ...).
    Io,
    /// Bad/contradictory command-line arguments.
    Usage,
    /// The requested target (string, pattern) was not found.
    NoMatch,
    /// The target matched more times than the command allows.
    Ambiguous,
    /// Encoding problem: undecodable file, or text that cannot be represented
    /// in the file's encoding.
    Encoding,
    /// A line number or range is outside the file.
    Range,
    /// Refusing to overwrite an existing file.
    Exists,
    /// The file does not exist.
    NotFound,
}

impl ErrorKind {
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::Other => 1,
            ErrorKind::Usage => 2,
            ErrorKind::NoMatch => 3,
            ErrorKind::Ambiguous => 4,
            ErrorKind::Encoding => 5,
            ErrorKind::Range => 6,
            ErrorKind::Exists => 7,
            ErrorKind::NotFound => 8,
            ErrorKind::Io => 9,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Other => "other",
            ErrorKind::Usage => "usage",
            ErrorKind::NoMatch => "no_match",
            ErrorKind::Ambiguous => "ambiguous",
            ErrorKind::Encoding => "encoding",
            ErrorKind::Range => "range",
            ErrorKind::Exists => "exists",
            ErrorKind::NotFound => "not_found",
            ErrorKind::Io => "io",
        }
    }
}

#[derive(Debug)]
pub struct AppError {
    pub kind: ErrorKind,
    pub message: String,
    /// Optional actionable hint printed after the message.
    pub hint: Option<String>,
}

impl AppError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        AppError {
            kind,
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, "\nhint: {hint}")?;
        }
        Ok(())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        let kind = match e.kind() {
            std::io::ErrorKind::NotFound => ErrorKind::NotFound,
            _ => ErrorKind::Io,
        };
        AppError::new(kind, e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[macro_export]
macro_rules! bail {
    ($kind:expr, $($arg:tt)*) => {
        return Err($crate::error::AppError::new($kind, format!($($arg)*)))
    };
}

#[macro_export]
macro_rules! bail_hint {
    ($kind:expr, $hint:expr, $($arg:tt)*) => {
        return Err($crate::error::AppError::new($kind, format!($($arg)*)).with_hint($hint))
    };
}
