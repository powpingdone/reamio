use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::prelude::*;

/// Catch-all type for internal web errors on the webserver, which is returned
#[derive(Debug)]
pub enum ReamioWebError {
    SQLError(sqlx::Error, StatusCode),
    AxumError(axum::Error, StatusCode),
}

impl From<sqlx::Error> for ReamioWebError {
    fn from(value: sqlx::Error) -> Self {
        ReamioWebError::from((StatusCode::INTERNAL_SERVER_ERROR, value))
    }
}

impl From<(StatusCode, sqlx::Error)> for ReamioWebError {
    #[tracing::instrument]
    fn from((sc, err): (StatusCode, sqlx::Error)) -> Self {
        error!(?sc, ?err, "reamioweberror generated");
        Self::SQLError(err, sc)
    }
}

impl From<axum::Error> for ReamioWebError {
    fn from(value: axum::Error) -> Self {
        ReamioWebError::from((StatusCode::INTERNAL_SERVER_ERROR, value))
    }
}

impl From<(StatusCode, axum::Error)> for ReamioWebError {
    #[tracing::instrument]
    fn from((sc, err): (StatusCode, axum::Error)) -> Self {
        error!(?sc, ?err, "reamioweberror generated");
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
#[allow(dead_code)]
#[derive(Debug)]
pub enum ReamioProcessingErrorInternal {
    SQL(sqlx::Error),
    IO(std::io::Error),
    PathError(ReamioPathError),
    ID3(id3::Error),
    MetaFlac(metaflac::Error),
}

impl From<sqlx::Error> for ReamioProcessingErrorInternal {
    #[tracing::instrument]
    fn from(value: sqlx::Error) -> Self {
        Self::SQL(value)
    }
}

impl From<std::io::Error> for ReamioProcessingErrorInternal {
    #[tracing::instrument]
    fn from(value: std::io::Error) -> Self {
        Self::IO(value)
    }
}

impl From<ReamioPathError> for ReamioProcessingErrorInternal {
    #[tracing::instrument]
    fn from(value: ReamioPathError) -> Self {
        Self::PathError(value)
    }
}

impl From<id3::Error> for ReamioProcessingErrorInternal {
    #[tracing::instrument]
    fn from(value: id3::Error) -> Self {
        Self::ID3(value)
    }
}

impl From<metaflac::Error> for ReamioProcessingErrorInternal {
    #[tracing::instrument]
    fn from(value: metaflac::Error) -> Self {
        Self::MetaFlac(value)
    }
}

#[derive(Debug)]
pub struct ReamioPathError {
    pub msg: String,
}
