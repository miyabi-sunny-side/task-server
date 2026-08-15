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
    #[error("{0}")]
    Invalid(String),
    /// The request is well formed but the current state refuses it: another
    /// merge already owns the target, or a release has nothing to stamp.
    #[error("{0}")]
    Conflict(String),
    /// A precondition of the request is not met yet, and `code` names which one
    /// so an MCP or agent client can branch on the reason instead of parsing
    /// prose. Answered as 409: the request may succeed once the world changes.
    #[error("{message}")]
    Precondition { code: &'static str, message: String },
    #[error("frontmatter: {0}")]
    Frontmatter(String),
    #[error("io: {0}")]
    Io(String),
    #[error("db: {0}")]
    Db(String),
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Self::Db(value.to_string())
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

impl Error {
    /// The stable slug an automated client branches on. Prose may be reworded;
    /// this may not.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::ClaimMismatch => "claim_mismatch",
            Self::Invalid(_) => "invalid",
            Self::Conflict(_) => "conflict",
            Self::Precondition { code, .. } => code,
            Self::Frontmatter(_) => "frontmatter",
            Self::Io(_) => "io",
            Self::Db(_) => "db",
        }
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ClaimMismatch | Self::Conflict(_) | Self::Precondition { .. } => {
                StatusCode::CONFLICT
            }
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Frontmatter(_) | Self::Io(_) | Self::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let body = json!({ "error": self.to_string(), "code": self.code() });
        (self.status(), Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::Error;

    /// Every variant answers with a status and a stable slug, so no refusal ever
    /// reaches a client as prose alone.
    #[test]
    fn every_variant_carries_a_status_and_a_stable_code() {
        let cases = [
            (
                Error::Unauthorized,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (Error::Forbidden, StatusCode::FORBIDDEN, "forbidden"),
            (Error::NotFound, StatusCode::NOT_FOUND, "not_found"),
            (Error::ClaimMismatch, StatusCode::CONFLICT, "claim_mismatch"),
            (
                Error::Invalid("nope".into()),
                StatusCode::BAD_REQUEST,
                "invalid",
            ),
            (
                Error::Conflict("taken".into()),
                StatusCode::CONFLICT,
                "conflict",
            ),
            (
                Error::Precondition {
                    code: "product_not_catalogued",
                    message: "product a/b is not in the catalogue".into(),
                },
                StatusCode::CONFLICT,
                "product_not_catalogued",
            ),
            (
                Error::Frontmatter("bad".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                "frontmatter",
            ),
            (
                Error::Io("gone".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                "io",
            ),
            (
                Error::Db("locked".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                "db",
            ),
        ];
        for (error, status, code) in cases {
            assert_eq!(error.status(), status, "{error:?}");
            assert_eq!(error.code(), code, "{error:?}");
        }
    }

    #[test]
    fn a_precondition_reports_its_own_message() {
        let error = Error::Precondition {
            code: "product_required",
            message: "task t-1 has no product".into(),
        };
        assert_eq!(error.to_string(), "task t-1 has no product");
    }
}
