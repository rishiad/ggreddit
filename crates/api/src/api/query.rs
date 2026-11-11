use crate::api::response::ApiError;
use axum::{extract::FromRequestParts, http::request::Parts};
use serde::de::DeserializeOwned;

pub(super) struct ApiQuery<T>(pub(super) T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
	S: Send + Sync,
	T: DeserializeOwned,
{
	type Rejection = ApiError;

	async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
		serde_urlencoded::from_str(parts.uri.query().unwrap_or_default())
			.map(Self)
			.map_err(|_| ApiError::InvalidQuery)
	}
}
