use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("retired: automatic control tasks are no longer supported")]
    Gone,
    #[error("io: {0}")]
    Io(String),
    #[error("yaml: {0}")]
    Yaml(String),
    #[error("frontmatter: {0}")]
    Frontmatter(String),
    #[error("json: {0}")]
    Json(String),
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
impl From<serde_norway::Error> for Error {
    fn from(e: serde_norway::Error) -> Self {
        Self::Yaml(e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e.to_string())
    }
}
impl Error {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Invalid(_) => "invalid",
            Self::Conflict(_) => "conflict",
            Self::Gone => "retired",
            Self::Io(_) => "io",
            Self::Yaml(_) => "yaml",
            Self::Frontmatter(_) => "frontmatter",
            Self::Json(_) => "json",
        }
    }
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Gone => StatusCode::GONE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            self.status(),
            Json(json!({"error":self.to_string(),"code":self.code()})),
        )
            .into_response()
    }
}
