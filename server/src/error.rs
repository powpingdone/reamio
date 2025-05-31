use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

/// Catch-all type for internal web errors on the webserver, which is returned
#[derive(Debug)]
pub enum ReamioWebError {
    SQLError(sqlx::Error, StatusCode),
    AxumError(axum::Error, StatusCode),
}

impl From<sqlx::Error> for ReamioWebError {
    fn from(value: sqlx::Error) -> Self {
        Self::SQLError(value, StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl From<(StatusCode, sqlx::Error)> for ReamioWebError {
    fn from((sc, err): (StatusCode, sqlx::Error)) -> Self {
        Self::SQLError(err, sc)
    }
}

impl From<axum::Error> for ReamioWebError {
    fn from(value: axum::Error) -> Self {
        Self::AxumError(value, StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl From<(StatusCode, axum::Error)> for ReamioWebError {
    fn from((sc, err): (StatusCode, axum::Error)) -> Self {
        Self::AxumError(err, sc)
    }
}

impl IntoResponse for ReamioWebError {
    fn into_response(self) -> Response {
        let code = match &self {
            ReamioWebError::SQLError(_, status_code)
            | ReamioWebError::AxumError(_, status_code) => status_code,
        };
        let msg = match &self {
            ReamioWebError::SQLError(error, _) => error.to_string(),
            ReamioWebError::AxumError(error, _) => error.to_string(),
        };

        (*code, msg).into_response()
    }
}

/// Catch-all type for picking up errors across multiple libs, only on the offline
/// processing side. This is specifically for _not_ user facing web points, but
/// more so for exposing raw errors in the console and (eventually) to the user as
/// well.
#[derive(Debug)]
pub enum ReamioProcessingErrorInternal {
    SQLError(sqlx::Error),
    IOError(std::io::Error),
    PathError(ReamioPathError),
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

impl From<ReamioPathError> for ReamioProcessingErrorInternal {
    fn from(value: ReamioPathError) -> Self {
        Self::PathError(value)
    }
}

#[derive(Debug)]
pub struct ReamioPathError {
    pub msg: String,
}
