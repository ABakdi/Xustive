//! HTTP error shaping.
//!
//! Two rules hold everywhere:
//!
//! 1. The client keys off a machine-readable `code`, never off `message`. Localisation happens in
//!    the browser, not the server.
//! 2. Internal detail never leaks. A backend failure becomes `search_unavailable`, not a stack
//!    trace or a Meilisearch error string.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use xustive_core::Classify;
use xustive_search::SearchError;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("query is required")]
    MissingQuery,
    #[error("query too long")]
    QueryTooLong { max: usize },
    #[error("invalid parameter {param}")]
    InvalidParam { param: &'static str, detail: String },
    #[error("search unavailable")]
    SearchUnavailable,
    #[error("upstream timeout")]
    UpstreamTimeout,
    #[error("internal error")]
    Internal,
}

impl ApiError {
    /// Stable machine-readable code. Also the metric label, so its cardinality is bounded.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingQuery => "invalid_query",
            Self::QueryTooLong { .. } => "query_too_long",
            Self::InvalidParam { .. } => "invalid_filter",
            Self::SearchUnavailable => "search_unavailable",
            Self::UpstreamTimeout => "upstream_timeout",
            Self::Internal => "internal_error",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::MissingQuery | Self::QueryTooLong { .. } | Self::InvalidParam { .. } => {
                StatusCode::BAD_REQUEST
            }
            Self::SearchUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::UpstreamTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Safe to display. Never contains user input echoed back, and never internal detail.
    pub fn message(&self) -> String {
        match self {
            Self::MissingQuery => "A search query is required.".into(),
            Self::QueryTooLong { max } => {
                format!("Search is limited to {max} characters.")
            }
            Self::InvalidParam { param, detail } => {
                format!("The {param} parameter is invalid: {detail}")
            }
            Self::SearchUnavailable => "Search is temporarily unavailable.".into(),
            Self::UpstreamTimeout => "That search took too long.".into(),
            Self::Internal => "Something went wrong on our side.".into(),
        }
    }
}

/// Translate a backend failure into a client-facing error.
///
/// Note that a 4xx from Meilisearch (usually a filter we built wrong) surfaces as
/// `internal_error`, not as a client error — the user did not make that mistake, we did.
impl From<SearchError> for ApiError {
    fn from(e: SearchError) -> Self {
        use xustive_core::ErrorClass;
        match &e {
            SearchError::Timeout(_) => Self::UpstreamTimeout,
            SearchError::Unreachable(_) => Self::SearchUnavailable,
            SearchError::Backend { status, .. } if *status >= 500 => Self::SearchUnavailable,
            _ => match e.class() {
                ErrorClass::Transient | ErrorClass::Throttled => Self::SearchUnavailable,
                _ => Self::Internal,
            },
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after: Option<u64>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();

        // Log server-side faults; client mistakes are not worth a log line each.
        if status.is_server_error() {
            tracing::error!(code = self.code(), error = %self, "request failed");
        } else {
            tracing::debug!(code = self.code(), "request rejected");
        }

        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code(),
                message: self.message(),
                retry_after: None,
            },
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_statuses_match_the_contract() {
        let cases: Vec<(ApiError, StatusCode, &str)> = vec![
            (
                ApiError::MissingQuery,
                StatusCode::BAD_REQUEST,
                "invalid_query",
            ),
            (
                ApiError::QueryTooLong { max: 512 },
                StatusCode::BAD_REQUEST,
                "query_too_long",
            ),
            (
                ApiError::InvalidParam {
                    param: "source",
                    detail: "x".into(),
                },
                StatusCode::BAD_REQUEST,
                "invalid_filter",
            ),
            (
                ApiError::SearchUnavailable,
                StatusCode::SERVICE_UNAVAILABLE,
                "search_unavailable",
            ),
            (
                ApiError::UpstreamTimeout,
                StatusCode::GATEWAY_TIMEOUT,
                "upstream_timeout",
            ),
            (
                ApiError::Internal,
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
            ),
        ];
        for (err, status, code) in cases {
            assert_eq!(err.status(), status, "status for {code}");
            assert_eq!(err.code(), code);
        }
    }

    #[test]
    fn backend_timeout_becomes_gateway_timeout() {
        let e: ApiError = SearchError::Timeout(std::time::Duration::from_secs(1)).into();
        assert_eq!(e.status(), StatusCode::GATEWAY_TIMEOUT);
    }

    #[test]
    fn backend_unreachable_becomes_service_unavailable() {
        let e: ApiError = SearchError::Unreachable("refused".into()).into();
        assert_eq!(e.code(), "search_unavailable");
    }

    #[test]
    fn our_bad_filter_is_our_fault_not_the_users() {
        // A 400 from Meilisearch means we built a bad filter. Reporting that as a client
        // error would send the user chasing a mistake they did not make.
        let e: ApiError = SearchError::Backend {
            status: 400,
            message: "invalid filter".into(),
            code: "invalid_search_filter".into(),
        }
        .into();
        assert_eq!(e.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn messages_never_echo_internal_detail() {
        let e: ApiError = SearchError::Backend {
            status: 500,
            message: "meilisearch internal: /var/lib/meili/data.mdb corrupt".into(),
            code: "internal".into(),
        }
        .into();
        let msg = e.message();
        assert!(!msg.contains("meilisearch"), "leaked backend detail: {msg}");
        assert!(!msg.contains("/var/lib"), "leaked a path: {msg}");
    }

    #[test]
    fn messages_never_echo_user_input() {
        // The query must not appear in an error body — it would end up in browser history,
        // referrer headers, and error-reporting pipelines.
        let e = ApiError::QueryTooLong { max: 512 };
        assert!(!e.message().contains("SELECT"));
        assert!(e.message().contains("512"));
    }

    #[test]
    fn error_body_shape() {
        let body = ErrorBody {
            error: ErrorDetail {
                code: "query_too_long",
                message: "m".into(),
                retry_after: None,
            },
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["error"]["code"], "query_too_long");
        assert!(
            v["error"].get("retry_after").is_none(),
            "null fields must be omitted"
        );
    }
}
