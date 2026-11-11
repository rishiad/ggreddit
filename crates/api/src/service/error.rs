use crate::client::{error::ClientError, AuthError};
use crate::parsing::ParseError;
use axum::http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceError {
	#[error(transparent)]
	Client(#[from] ClientError),
	#[error(transparent)]
	Parse(#[from] ParseError),
	#[error("invalid subreddit `{subreddit}`")]
	InvalidSubreddit { subreddit: String },
	#[error("bad request")]
	BadRequest,
	#[error("`after` and `before` cannot be used together")]
	ConflictingCursors,
	#[error("invalid `{parameter}` listing cursor `{cursor}`")]
	InvalidCursor { parameter: &'static str, cursor: String },
	#[error("listing limit must be between 1 and 100, received {limit}")]
	InvalidLimit { limit: u8 },
	#[error("invalid `{parameter}` value `{value}`")]
	InvalidParameter { parameter: &'static str, value: String },
	#[error("Reddit returned invalid GraphQL comments")]
	InvalidGraphqlComments,
	#[error("post not found")]
	PostNotFound,
	#[error("this server has NSFW disabled")]
	ContentBlocked,
}

impl ServiceError {
	pub fn status_code(&self) -> StatusCode {
		match self {
			Self::InvalidSubreddit { .. } | Self::BadRequest | Self::ConflictingCursors | Self::InvalidCursor { .. } | Self::InvalidLimit { .. } | Self::InvalidParameter { .. } => {
				StatusCode::BAD_REQUEST
			}
			Self::ContentBlocked => StatusCode::FORBIDDEN,
			Self::PostNotFound | Self::Client(ClientError::NotFound) => StatusCode::NOT_FOUND,
			Self::Client(ClientError::Timeout | ClientError::Auth(AuthError::Timeout) | ClientError::UnexpectedStatus { status: 504, .. }) => StatusCode::GATEWAY_TIMEOUT,
			Self::Client(ClientError::Unauthorized | ClientError::RateLimited { .. } | ClientError::Auth(_) | ClientError::UnexpectedStatus { status: 429 | 503, .. }) => {
				StatusCode::SERVICE_UNAVAILABLE
			}
			Self::Client(ClientError::UnsupportedPath { .. } | ClientError::BuildClient(_) | ClientError::ResponseBuild(_)) => StatusCode::INTERNAL_SERVER_ERROR,
			Self::Client(_) | Self::Parse(_) | Self::InvalidGraphqlComments => StatusCode::BAD_GATEWAY,
		}
	}
}
