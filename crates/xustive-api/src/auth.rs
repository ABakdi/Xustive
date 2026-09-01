//! The admin surface's lock (M14-T01).
//!
//! `/api/v1/admin/*` can pause the crawler, rewrite ranking weights, replay dead letters, read
//! the search log and forget a visitor. Until 2026-08-30 it was open to anyone who could reach
//! the port, which is why [[Milestone 14 - One Server, Many Hands]] made this the blocker every
//! deployment task sits behind.
//!
//! Two rules, and the second is what makes the first safe:
//!
//! 1. **A key is required**, presented as `Authorization: Bearer <key>` or `X-Admin-Key: <key>`,
//!    compared in constant time.
//! 2. **No key configured means loopback only.** That keeps `make dev` frictionless without
//!    leaving a door open anywhere else — and a public bind with no key refuses to start at all
//!    ([`xustive_core::config`] validation), so this branch cannot be reached on a public
//!    address.

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::net::SocketAddr;

use crate::state::AppState;

/// Reject unless the caller proves it may steer the system.
pub async fn require_admin(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Read from the request rather than as extractors: `ConnectInfo` is only present when the
    // server was built with it, and an extractor that rejects would turn a missing peer address
    // into a confusing 500 instead of the refusal this is for.
    let headers = request.headers().clone();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    let configured = state.config.api.admin_key.trim();

    if configured.is_empty() {
        // Development: the console works over loopback, and nothing else does.
        let local = peer.is_some_and(|addr| addr.ip().is_loopback());
        return if local {
            Ok(next.run(request).await)
        } else {
            tracing::warn!("admin request refused: no admin key configured and caller is remote");
            Err(StatusCode::UNAUTHORIZED)
        };
    }

    if presented(&headers)
        .is_some_and(|given| constant_time_eq(given.as_bytes(), configured.as_bytes()))
    {
        return Ok(next.run(request).await);
    }

    // Deliberately no detail: a 401 that explains which half was wrong is a hint to whoever is
    // guessing. The operator's own failure shows up in the log line, not in the response.
    tracing::warn!("admin request refused: missing or wrong key");
    Err(StatusCode::UNAUTHORIZED)
}

/// The key the caller presented, from either accepted header.
pub(crate) fn presented(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-admin-key").and_then(|v| v.to_str().ok()) {
        return Some(v.trim().to_string());
    }
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, value) = auth.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| value.trim().to_string())
}

/// Compare without leaking, through timing, how much of the key was right.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[test]
    fn either_header_carries_the_key() {
        assert_eq!(
            presented(&headers(&[("x-admin-key", "abc")])).as_deref(),
            Some("abc")
        );
        assert_eq!(
            presented(&headers(&[("authorization", "Bearer abc")])).as_deref(),
            Some("abc")
        );
        assert_eq!(
            presented(&headers(&[("authorization", "bearer abc")])).as_deref(),
            Some("abc"),
            "the scheme is case-insensitive"
        );
        assert_eq!(presented(&headers(&[("authorization", "Basic abc")])), None);
        assert_eq!(presented(&HeaderMap::new()), None);
    }

    #[test]
    fn comparison_is_length_safe_and_correct() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secretlonger"));
        assert!(!constant_time_eq(b"", b"x"));
    }
}
