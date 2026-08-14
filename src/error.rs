use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("claim mismatch")]
    ClaimMismatch,
    #[error("unknown action")]
    UnknownAction,
    #[error("action not allowed")]
    ActionNotAllowed,
    #[error("dirty worktree")]
    DirtyWorktree,
    #[error("{0}")]
    Invalid(String),
    #[error("frontmatter: {0}")]
    Frontmatter(String),
    #[error("git: {0}")]
    Git(String),
    #[error("io: {0}")]
    Io(String),
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_norway::Error> for Error {
    fn from(value: serde_norway::Error) -> Self {
        Self::Frontmatter(value.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Invalid(value.to_string())
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ClaimMismatch | Self::DirtyWorktree => StatusCode::CONFLICT,
            Self::UnknownAction | Self::ActionNotAllowed | Self::Invalid(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::Frontmatter(_) | Self::Git(_) | Self::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}
