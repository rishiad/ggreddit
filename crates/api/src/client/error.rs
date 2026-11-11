use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
	#[error("Reddit request deadline exceeded: 20 seconds")]
	Timeout,
	#[error("upstream response exceeds the 8 MiB response limit")]
	BodyTooLarge,
	#[error("Reddit returned unexpected HTTP status {status} for `{path}`")]
	UnexpectedStatus { path: String, status: u16 },
	#[error("Reddit returned an unexpected HTTP status or JSON error envelope")]
	InvalidEnvelope,
	#[error("Reddit did not find the requested resource")]
	NotFound,
	#[error("failed to build Reddit HTTP client")]
	BuildClient(#[source] wreq::Error),

	#[error(transparent)]
	Auth(#[from] super::oauth::AuthError),

	#[error("request to `{url}` failed")]
	Request {
		url: String,
		#[source]
		source: wreq::Error,
	},

	#[error("invalid proxy URL `{url}`")]
	InvalidProxyUrl { url: String },

	#[error("failed to receive the response body for `{path}`")]
	Body {
		path: String,
		#[source]
		source: wreq::Error,
	},

	#[error("failed to convert the upstream response")]
	ResponseBuild(#[source] axum::http::Error),

	#[error("failed to decode Reddit JSON for `{path}`")]
	Decode {
		path: String,
		#[source]
		source: serde_json::Error,
	},

	#[error("Reddit rate limit exceeded{reset_message}", reset_message = reset.as_ref().map(|value| format!("; resets in {value}")) .unwrap_or_default())]
	RateLimited { reset: Option<String> },

	#[error("Reddit OAuth token expired")]
	Unauthorized,
}
