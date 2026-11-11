use crate::client::error::ClientError;
use crate::service::{ServiceError, ServiceError::Client};
use axum::{
	body::Body,
	http::{header, HeaderValue, StatusCode},
	response::{IntoResponse, Response},
};
use serde::Serialize;
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub(super) enum ApiError {
	#[error(transparent)]
	Service(#[from] ServiceError),
	#[error("invalid query")]
	InvalidQuery,
}

impl ApiError {
	fn status(&self) -> StatusCode {
		match self {
			Self::InvalidQuery => StatusCode::BAD_REQUEST,
			Self::Service(error) => service_status(error),
		}
	}

	fn message(&self) -> &'static str {
		if matches!(self, Self::Service(ServiceError::ContentBlocked)) {
			return "This server has NSFW disabled";
		}
		match self.status() {
			StatusCode::BAD_REQUEST => "Bad Request",
			StatusCode::UNAUTHORIZED => "Unauthorized",
			StatusCode::FORBIDDEN => "Forbidden",
			StatusCode::NOT_FOUND => "Not Found",
			StatusCode::TOO_MANY_REQUESTS => "Too Many Requests",
			_ => "Bad Gateway",
		}
	}
}

fn service_status(error: &ServiceError) -> StatusCode {
	match error {
		ServiceError::InvalidSubreddit { .. }
		| ServiceError::ConflictingCursors
		| ServiceError::InvalidCursor { .. }
		| ServiceError::InvalidLimit { .. }
		| ServiceError::InvalidParameter { .. } => StatusCode::BAD_REQUEST,
		ServiceError::ContentBlocked => StatusCode::FORBIDDEN,
		ServiceError::Parse(_) | ServiceError::InvalidGraphqlComments => StatusCode::BAD_GATEWAY,
		Client(ClientError::Unauthorized) => StatusCode::UNAUTHORIZED,
		Client(ClientError::Quarantined | ClientError::Gated | ClientError::Private | ClientError::Banned | ClientError::Suspended) => StatusCode::FORBIDDEN,
		Client(ClientError::RateLimited { .. }) => StatusCode::TOO_MANY_REQUESTS,
		Client(ClientError::Reddit { code, .. }) => u16::try_from(*code)
			.ok()
			.and_then(|code| StatusCode::from_u16(code).ok())
			.filter(StatusCode::is_client_error)
			.unwrap_or(StatusCode::BAD_GATEWAY),
		Client(_) => StatusCode::BAD_GATEWAY,
	}
}

#[derive(Serialize)]
struct ErrorBody {
	message: &'static str,
	error: u16,
}

impl IntoResponse for ApiError {
	fn into_response(self) -> Response {
		let status = self.status();
		json(
			status,
			&ErrorBody {
				message: self.message(),
				error: status.as_u16(),
			},
		)
	}
}

pub(super) fn respond<T, E>(result: Result<T, E>) -> Response
where
	T: Serialize,
	E: Into<ApiError>,
{
	match result {
		Ok(value) => json(StatusCode::OK, &value),
		Err(error) => error.into().into_response(),
	}
}

fn json<T>(status: StatusCode, value: &T) -> Response
where
	T: Serialize,
{
	let (status, body) = match serde_json::to_vec(value) {
		Ok(body) => (status, body),
		Err(error) => {
			error!(event = "response.serialization_failed", error = %error, "failed to serialize API response");
			(StatusCode::INTERNAL_SERVER_ERROR, br#"{"message":"Internal Server Error","error":500}"#.to_vec())
		}
	};
	let mut response = Response::new(Body::from(body));
	*response.status_mut() = status;
	response
		.headers_mut()
		.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
	response
}
