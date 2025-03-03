#[derive(Debug)]
pub enum ReamioWebError {
    SQLError(sqlx::Error),
}

impl From<sqlx::Error> for ReamioWebError {
    fn from(value: sqlx::Error) -> Self {
        Self::SQLError(value)
    }
}

/// Polyglot type for picking up errors across multiple libs, only on the offline
/// processing side. This is specifically for _not_ user facing web points, but
/// more so for exposing raw errors in the console and (eventually) to the user as
/// well.
#[derive(Debug)]
pub enum ReamioProcessingErrorInternal {
    SQLError(sqlx::Error),
    IOError(std::io::Error),
}

impl From<sqlx::Error> for ReamioProcessingErrorInternal {
    fn from(value: sqlx::Error) -> Self {
        Self::SQLError(value)
    }
}

impl From<std::io::Error> for ReamioProcessingErrorInternal {
    fn from(value: std::io::Error) -> Self {
        Self::IOError(value)
    }
}
