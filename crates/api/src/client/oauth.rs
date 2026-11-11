mod backend;
mod manager;

use thiserror::Error;

pub(crate) use manager::OAuthHandle;

#[derive(Debug, Error)]
pub enum AuthError {
	#[error("OAuth token exchange failed")]
	Request(#[from] wreq::Error),
	#[error("OAuth response is missing `{0}`")]
	MissingField(&'static str),
	#[error("OAuth response contains an invalid `{0}` header")]
	InvalidHeader(&'static str),
	#[error("OAuth endpoint rejected the token exchange with HTTP {0}")]
	Rejected(u16),
	#[error("OAuth response contains an invalid expiry")]
	InvalidExpiry,
	#[error("OAuth request timed out")]
	Timeout,
	#[error("OAuth credentials are unavailable")]
	Unavailable,
	#[error("OAuth credentials expired")]
	Expired,
	#[error("OAuth rate limit exhausted")]
	RateLimited,
	#[error("OAuth manager stopped")]
	ManagerStopped,
}
