//! HTTP error mapping.
//!
//! Domain, identity, and storage failures are all flattened into
//! [`RunvaneError`] at service call sites and re-rendered here as a stable
//! `{ code, message, status }` body so clients can branch on `code` without
//! parsing prose.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::domain::error::DomainError;
use crate::error::{RunvaneError, StorageError};

/// Standard error body attached to non-2xx responses.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    /// Structured error detail.
    pub error: ErrorDetail,
}

/// Machine-readable failure detail.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorDetail {
    /// Stable category: `validation`, `not_found`, `conflict`, `storage`, ...
    pub code: String,
    /// Human-readable description for operators.
    pub message: String,
    /// HTTP status code echoed for convenience.
    pub status: u16,
}

/// An error the API layer is ready to render as an HTTP response.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    /// Builds an error with an explicit status/code.
    pub fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// A 400 validation failure.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "validation", message)
    }

    /// A 404 for a missing entity.
    pub fn not_found(entity: &str, id: impl std::fmt::Display) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{entity} {id} does not exist"),
        )
    }

    /// A 409 state conflict (illegal transition, duplicate).
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    /// A 500 internal failure.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }

    /// The HTTP status the error maps to.
    pub fn http_status(&self) -> u16 {
        self.status.as_u16()
    }

    /// The operator-facing message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Maps a flattened platform error onto status + category.
    pub fn from_runvane(err: RunvaneError) -> Self {
        let status =
            StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        Self {
            status,
            code: err.category(),
            message: err.to_string(),
        }
    }
}

impl From<RunvaneError> for ApiError {
    fn from(err: RunvaneError) -> Self {
        Self::from_runvane(err)
    }
}

impl From<StorageError> for ApiError {
    fn from(err: StorageError) -> Self {
        Self::from_runvane(RunvaneError::from(err))
    }
}

impl From<DomainError> for ApiError {
    fn from(err: DomainError) -> Self {
        Self::from_runvane(RunvaneError::from(err))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code.to_owned(),
                message: self.message,
                status: self.status.as_u16(),
            },
        };
        (self.status, Json(body)).into_response()
    }
}

/// Bridges any error that flattens into [`RunvaneError`] (domain, ids, ...).
pub fn api_err<E: Into<RunvaneError>>(err: E) -> ApiError {
    ApiError::from_runvane(err.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::IdError;

    #[test]
    fn domain_error_renders_validation_body() {
        let api = ApiError::from(RunvaneError::from(IdError::Malformed {
            kind: "run",
            id: "rn_1".into(),
            reason: "bad chars".into(),
        }));
        assert_eq!(api.status, StatusCode::BAD_REQUEST);
        assert_eq!(api.code, "validation");
        let body = serde_json::to_value(ErrorBody {
            error: ErrorDetail {
                code: api.code.to_owned(),
                message: api.message,
                status: api.status.as_u16(),
            },
        })
        .unwrap();
        assert_eq!(body["error"]["code"], "validation");
        assert_eq!(body["error"]["status"], 400);
    }

    #[test]
    fn storage_not_found_maps_to_404() {
        let api = ApiError::from(StorageError::NotFound("run rn_1".into()));
        assert_eq!(api.status, StatusCode::NOT_FOUND);
        assert_eq!(api.code, "not_found");
    }

    #[test]
    fn conflict_stays_conflict() {
        let api = ApiError::conflict("run already terminal");
        assert_eq!(api.status, StatusCode::CONFLICT);
        assert_eq!(api.code, "conflict");
    }
}