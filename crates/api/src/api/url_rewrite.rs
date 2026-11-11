use crate::media::MediaSigner;
use axum::{
	body::{to_bytes, Body},
	extract::{Request, State},
	http::{header, HeaderValue, StatusCode},
	middleware::Next,
	response::Response,
};
use tracing::error;

pub(super) async fn rewrite_reddit_urls(State(signer): State<MediaSigner>, request: Request, next: Next) -> Response {
	let response = next.run(request).await;
	if !is_json(&response) {
		return response;
	}

	let (mut parts, body) = response.into_parts();
	let body = match to_bytes(body, usize::MAX).await {
		Ok(body) => body,
		Err(error) => {
			error!(event = "response.rewrite_failed", stage = "read", error = %error, "failed to rewrite API response URLs");
			return rewrite_failure(parts);
		}
	};
	let mut value = match serde_json::from_slice(&body) {
		Ok(value) => value,
		Err(error) => {
			error!(event = "response.rewrite_failed", stage = "parse", error = %error, "failed to rewrite API response URLs");
			return rewrite_failure(parts);
		}
	};
	signer.rewrite_value(&mut value);
	let body = match serde_json::to_vec(&value) {
		Ok(body) => body,
		Err(error) => {
			error!(event = "response.rewrite_failed", stage = "serialize", error = %error, "failed to rewrite API response URLs");
			return rewrite_failure(parts);
		}
	};
	parts.headers.remove(header::CONTENT_LENGTH);
	Response::from_parts(parts, Body::from(body))
}

fn rewrite_failure(mut parts: axum::http::response::Parts) -> Response {
	parts.status = StatusCode::INTERNAL_SERVER_ERROR;
	parts.headers.remove(header::CONTENT_LENGTH);
	parts.headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
	Response::from_parts(parts, Body::from(r#"{"message":"Internal Server Error","error":500}"#))
}

fn is_json(response: &Response) -> bool {
	response
		.headers()
		.get(header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value.starts_with("application/json"))
}
