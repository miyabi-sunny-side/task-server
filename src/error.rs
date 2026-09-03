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
    /// The database refused a write on one of its constraints (a unique index,
    /// a foreign key, a NOT NULL). Usually that is the stored state saying no
    /// to a request that raced another one, so it answers 409 with its own code
    /// and a client can decide whether the same request makes sense again once
    /// the rows have moved. It can also be a bug that built an impossible row;
    /// the message keeps sqlite's wording so a person can tell which.
    #[error("db constraint: {0}")]
    Constraint(String),
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
        if let rusqlite::Error::SqliteFailure(failure, _) = &value
            && failure.code == rusqlite::ErrorCode::ConstraintViolation
        {
            return Self::Constraint(value.to_string());
        }
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
            Self::Constraint(_) => "constraint",
            Self::Db(_) => "db",
        }
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ClaimMismatch
            | Self::Conflict(_)
            | Self::Precondition { .. }
            | Self::Constraint(_) => StatusCode::CONFLICT,
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
            (
                Error::Constraint("UNIQUE constraint failed".into()),
                StatusCode::CONFLICT,
                "constraint",
            ),
        ];
        for (error, status, code) in cases {
            assert_eq!(error.status(), status, "{error:?}");
            assert_eq!(error.code(), code, "{error:?}");
        }
    }

    /// A constraint the database refused is the state saying no, not the
    /// server breaking: it answers 409 with its own code so a client can decide
    /// whether to retry, while every other sqlite failure stays a 500.
    #[test]
    fn a_sqlite_constraint_violation_is_a_conflict_and_other_failures_stay_db() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id TEXT PRIMARY KEY); INSERT INTO t VALUES ('a');")
            .unwrap();
        let violation = conn
            .execute("INSERT INTO t VALUES ('a')", [])
            .expect_err("the primary key must refuse the duplicate");
        let mapped = Error::from(violation);
        assert!(matches!(mapped, Error::Constraint(_)), "{mapped:?}");
        assert_eq!(mapped.status(), StatusCode::CONFLICT);
        assert_eq!(mapped.code(), "constraint");

        let other = conn
            .execute("SELECT * FROM no_such_table", [])
            .expect_err("a missing table is a failure");
        let mapped = Error::from(other);
        assert!(matches!(mapped, Error::Db(_)), "{mapped:?}");
        assert_eq!(mapped.status(), StatusCode::INTERNAL_SERVER_ERROR);
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
