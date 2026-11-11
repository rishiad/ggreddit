use crate::client::error::ClientError;
use crate::parsing::ParseError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceError {
	#[error(transparent)]
	Client(#[from] ClientError),
	#[error(transparent)]
	Parse(#[from] ParseError),
	#[error("invalid subreddit `{subreddit}`")]
	InvalidSubreddit { subreddit: String },
	#[error("combined community feeds are not supported")]
	UnsupportedCombinedFeed,
	#[error("wiki revisions and diffs are not supported")]
	UnsupportedWikiRevision,
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
	#[error("this server has NSFW disabled")]
	ContentBlocked,
}
