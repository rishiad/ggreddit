use crate::api::response::ApiError;
use axum::{extract::FromRequestParts, http::request::Parts};
use serde::de::DeserializeOwned;
use std::convert::Infallible;

pub(super) struct ApiQuery<T>(pub(super) T);
pub(super) struct DefaultQuery<T>(pub(super) T);

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

impl<S, T> FromRequestParts<S> for DefaultQuery<T>
where
	S: Send + Sync,
	T: Default + DeserializeOwned,
{
	type Rejection = Infallible;

	async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
		Ok(Self(serde_urlencoded::from_str(parts.uri.query().unwrap_or_default()).unwrap_or_default()))
	}
}
