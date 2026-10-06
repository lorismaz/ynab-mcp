use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use subtle::ConstantTimeEq;

use crate::config::Secret;

pub async fn require_bearer(State(token): State<Secret>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if path == "/" || path == "/health" {
        return next.run(request).await;
    }
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if presented.is_some_and(|header| bearer_matches(header, token.expose())) {
        return next.run(request).await;
    }
    tracing::debug!(path, "rejected request without a valid bearer token");
    unauthorized()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer realm=\"ynab-mcp\"")],
        Json(json!({"error": "unauthorized"})),
    )
        .into_response()
}

fn bearer_matches(header: &str, expected: &str) -> bool {
    let Some(provided) = extract_bearer(header) else {
        let _ = expected.as_bytes().ct_eq(expected.as_bytes());
        return false;
    };
    let provided = provided.as_bytes();
    let expected = expected.as_bytes();
    if provided.len() != expected.len() {
        let _ = expected.ct_eq(expected);
        return false;
    }
    bool::from(provided.ct_eq(expected))
}

fn extract_bearer(header: &str) -> Option<&str> {
    let (scheme, rest) = header.split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = rest.trim();
    if token.is_empty() || token.split_whitespace().nth(1).is_some() {
        return None;
    }
    Some(token)
}

#[cfg(test)]
mod tests {
    use super::bearer_matches;

    #[test]
    fn bearer_match_is_exact() {
        assert!(bearer_matches(
            "Bearer secret-token-value",
            "secret-token-value"
        ));
        assert!(bearer_matches(
            "bearer secret-token-value",
            "secret-token-value"
        ));
        assert!(!bearer_matches(
            "Bearer other-token-value",
            "secret-token-value"
        ));
        assert!(!bearer_matches(
            "Basic secret-token-value",
            "secret-token-value"
        ));
        assert!(!bearer_matches("Bearer", "secret-token-value"));
    }
}
