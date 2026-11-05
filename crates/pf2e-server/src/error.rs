use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pf2e_core::combat::CombatError;
use pf2e_store::StoreError;
use serde_json::json;

/// Uniform JSON error: `{ "error": "...", "details": ... }`.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError { status, message: message.into(), details: None }
    }
    pub fn not_found(what: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("{what} not found"))
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, msg)
    }
    pub fn with_details(mut self, d: serde_json::Value) -> Self {
        self.details = Some(d);
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": self.message, "details": self.details });
        (self.status, Json(body)).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound(w) => ApiError::not_found(w),
            StoreError::Conflict(m) => ApiError::conflict(m),
            other => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        }
    }
}

impl From<CombatError> for ApiError {
    fn from(e: CombatError) -> Self {
        let details = serde_json::to_value(&e).ok();
        ApiError { status: StatusCode::UNPROCESSABLE_ENTITY, message: e.to_string(), details }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::bad_request(format!("invalid JSON: {e}"))
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
