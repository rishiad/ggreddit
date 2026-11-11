use crate::client::RedditClient;
use crate::models::{Listing, Post, PostComments, Subreddit, Thing, User};
use crate::parsing::{parse_post_listing, parse_subreddit, parse_user};
use crate::service::query_codec;
use crate::service::sanitize::{clear_listing_modhash, clear_post_comments_modhash};
use crate::service::{ContentPolicy, ServiceError};

#[derive(Clone)]
pub struct RedditService {
	pub(super) client: RedditClient,
	pub(crate) content: ContentPolicy,
}

impl RedditService {
	pub fn new(client: RedditClient) -> Self {
		Self::with_content_policy(client, ContentPolicy::default())
	}

	pub fn with_content_policy(client: RedditClient, content: ContentPolicy) -> Self {
		Self { client, content }
	}

	pub async fn all_posts(&self, sort: PostSort, query: &ListingQuery) -> Result<Listing<Thing<Post>>, ServiceError> {
		if sort == PostSort::Best {
			return Err(invalid_parameter("sort", sort.as_str()));
		}
		self.post_listing("all", sort, query).await
	}

	pub async fn subreddit_posts(&self, subreddit: &str, sort: PostSort, query: &ListingQuery) -> Result<Listing<Thing<Post>>, ServiceError> {
		validate_subreddit(subreddit)?;
		if subreddit.contains('+') {
			return Err(ServiceError::UnsupportedCombinedFeed);
		}
		self.require_safe_subreddit(subreddit).await?;
		if sort == PostSort::Best {
			return Err(invalid_parameter("sort", sort.as_str()));
		}
		self.post_listing(subreddit, sort, query).await
	}

	pub async fn posts_by_id(&self, names: &str) -> Result<Listing<Thing<Post>>, ServiceError> {
		let names = canonical_post_fullnames(names)?;
		let json = self.client.json(format!("/by_id/{names}")).await?;
		let mut listing = parse_post_listing(&json)?;
		clear_listing_modhash(&mut listing);
		self.content.filter_posts(&mut listing);
		Ok(listing)
	}

	pub async fn post_comments(&self, article: &str, query: &CommentQuery) -> Result<PostComments, ServiceError> {
		self.comments(article, query).await
	}

	pub async fn subreddit_post_comments(&self, subreddit: &str, article: &str, query: &CommentQuery) -> Result<PostComments, ServiceError> {
		validate_subreddit(subreddit)?;
		self.comments(article, query).await
	}

	pub async fn subreddit_about(&self, subreddit: &str) -> Result<Thing<Subreddit>, ServiceError> {
		validate_subreddit(subreddit)?;
		let json = self.client.json(format!("/r/{subreddit}/about")).await?;
		let subreddit = parse_subreddit(&json)?;
		self.content.require_subreddit(&subreddit)?;
		Ok(subreddit)
	}

	pub(super) async fn require_safe_subreddit(&self, subreddit: &str) -> Result<(), ServiceError> {
		if self.content.allows_nsfw() {
			return Ok(());
		}
		for name in subreddit.split('+') {
			self.subreddit_about(name).await?;
		}
		Ok(())
	}

	pub async fn user_about(&self, username: &str) -> Result<Thing<User>, ServiceError> {
		validate_username(username)?;
		let json = self.client.json(format!("/user/{username}/about")).await?;
		let user = parse_user(&json)?;
		self.content.require_user(&user)?;
		Ok(user)
	}

	async fn post_listing(&self, subreddit: &str, sort: PostSort, query: &ListingQuery) -> Result<Listing<Thing<Post>>, ServiceError> {
		validate_listing_query(query)?;
		let base = format!("/r/{}/{}", subreddit.replace('+', "%2B"), sort.as_str());
		let path = with_query(base, query_codec::encode(query));
		let json = self.client.json(path).await?;
		let mut listing = parse_post_listing(&json)?;
		clear_listing_modhash(&mut listing);
		self.content.filter_posts(&mut listing);
		Ok(listing)
	}

	async fn comments(&self, article: &str, query: &CommentQuery) -> Result<PostComments, ServiceError> {
		validate_id36("article", article)?;
		validate_comment_query(query)?;
		let mut comments = self.graphql_comments(article, query).await?;
		clear_post_comments_modhash(&mut comments);
		self.content.require_post_comments(&comments)?;
		Ok(comments)
	}
}

pub(super) fn with_query(base: String, query: String) -> String {
	if query.is_empty() {
		base
	} else {
		format!("{base}?{query}")
	}
}

pub(super) fn validate_subreddit(subreddit: &str) -> Result<(), ServiceError> {
	let valid = !subreddit.is_empty()
		&& subreddit
			.split('+')
			.all(|name| !name.is_empty() && name.len() <= 21 && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'));

	if valid {
		Ok(())
	} else {
		Err(ServiceError::InvalidSubreddit { subreddit: subreddit.to_string() })
	}
}

pub(super) fn validate_username(username: &str) -> Result<(), ServiceError> {
	let valid = !username.is_empty() && username.len() <= 20 && username.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
	if valid {
		Ok(())
	} else {
		Err(invalid_parameter("username", username))
	}
}

pub(super) fn validate_listing_query(query: &ListingQuery) -> Result<(), ServiceError> {
	validate_listing_query_with(query, validate_cursor)
}

pub(super) fn validate_listing_query_with(query: &ListingQuery, cursor_validator: fn(&'static str, &str) -> Result<(), ServiceError>) -> Result<(), ServiceError> {
	if query.after.is_some() && query.before.is_some() {
		return Err(ServiceError::ConflictingCursors);
	}
	if let Some(cursor) = &query.after {
		cursor_validator("after", cursor)?;
	}
	if let Some(cursor) = &query.before {
		cursor_validator("before", cursor)?;
	}
	if query.limit.is_some_and(|limit| !(1..=100).contains(&limit)) {
		return Err(ServiceError::InvalidLimit {
			limit: query.limit.unwrap_or_default(),
		});
	}
	if let Some(geo_filter) = &query.geo_filter {
		let valid = !geo_filter.is_empty() && geo_filter.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
		if !valid {
			return Err(invalid_parameter("g", geo_filter));
		}
	}
	Ok(())
}

fn validate_comment_query(query: &CommentQuery) -> Result<(), ServiceError> {
	if let Some(comment) = &query.comment {
		validate_id36("comment", comment)?;
	}
	if query.context.is_some_and(|context| context > 8) {
		return Err(invalid_parameter("context", &query.context.unwrap_or_default().to_string()));
	}
	if query.truncate.is_some_and(|truncate| truncate > 50) {
		return Err(invalid_parameter("truncate", &query.truncate.unwrap_or_default().to_string()));
	}
	Ok(())
}

fn validate_cursor(parameter: &'static str, cursor: &str) -> Result<(), ServiceError> {
	let valid = cursor.split_once('_').is_some_and(|(prefix, id)| {
		prefix
			.strip_prefix('t')
			.is_some_and(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
			&& !id.is_empty()
			&& id.bytes().all(|byte| byte.is_ascii_alphanumeric())
	});
	if valid {
		Ok(())
	} else {
		Err(ServiceError::InvalidCursor {
			parameter,
			cursor: cursor.to_string(),
		})
	}
}

pub(super) fn validate_id36(parameter: &'static str, value: &str) -> Result<(), ServiceError> {
	if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
		Ok(())
	} else {
		Err(invalid_parameter(parameter, value))
	}
}

fn canonical_post_fullnames(names: &str) -> Result<String, ServiceError> {
	let names = names
		.split(|character: char| character == ',' || character.is_ascii_whitespace())
		.filter(|name| !name.is_empty())
		.map(|name| {
			let valid = name
				.strip_prefix("t3_")
				.is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric()));
			if valid {
				Ok(name)
			} else {
				Err(invalid_parameter("names", name))
			}
		})
		.collect::<Result<Vec<_>, _>>()?;
	if names.is_empty() {
		return Err(invalid_parameter("names", ""));
	}
	Ok(names.join(","))
}

pub(super) fn invalid_parameter(parameter: &'static str, value: &str) -> ServiceError {
	ServiceError::InvalidParameter {
		parameter,
		value: value.to_string(),
	}
}

mod comment_query {

	#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
	#[serde(rename_all = "lowercase")]
	pub enum CommentSort {
		#[default]
		Confidence,
		Top,
		New,
		Controversial,
		Old,
		Random,
		Qa,
		Live,
	}

	impl CommentSort {
		pub const fn as_str(self) -> &'static str {
			match self {
				Self::Confidence => "confidence",
				Self::Top => "top",
				Self::New => "new",
				Self::Controversial => "controversial",
				Self::Old => "old",
				Self::Random => "random",
				Self::Qa => "qa",
				Self::Live => "live",
			}
		}
	}

	impl std::str::FromStr for CommentSort {
		type Err = ();

		fn from_str(value: &str) -> Result<Self, Self::Err> {
			match value {
				"best" | "confidence" => Ok(Self::Confidence),
				"top" => Ok(Self::Top),
				"new" => Ok(Self::New),
				"controversial" => Ok(Self::Controversial),
				"old" => Ok(Self::Old),
				"random" => Ok(Self::Random),
				"qa" => Ok(Self::Qa),
				"live" => Ok(Self::Live),
				_ => Err(()),
			}
		}
	}

	#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
	#[serde(rename_all = "lowercase")]
	pub enum CommentTheme {
		Default,
		Dark,
	}

	#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
	pub struct CommentQuery {
		pub comment: Option<String>,
		pub cursor: Option<String>,
		pub context: Option<u8>,
		pub depth: Option<u32>,
		pub limit: Option<u32>,
		pub showedits: Option<bool>,
		pub showmedia: Option<bool>,
		pub showmore: Option<bool>,
		pub showtitle: Option<bool>,
		pub sort: Option<CommentSort>,
		pub sr_detail: Option<bool>,
		pub theme: Option<CommentTheme>,
		pub threaded: Option<bool>,
		pub truncate: Option<u8>,
	}
}

mod listing_query {

	#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
	#[serde(rename_all = "lowercase")]
	pub enum ListingShow {
		All,
	}

	#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
	#[serde(rename_all = "lowercase")]
	pub enum ListingTime {
		Hour,
		Day,
		Week,
		Month,
		Year,
		All,
	}

	impl ListingTime {
		pub const fn as_str(self) -> &'static str {
			match self {
				Self::Hour => "hour",
				Self::Day => "day",
				Self::Week => "week",
				Self::Month => "month",
				Self::Year => "year",
				Self::All => "all",
			}
		}
	}

	impl std::str::FromStr for ListingTime {
		type Err = ();

		fn from_str(value: &str) -> Result<Self, Self::Err> {
			match value {
				"hour" => Ok(Self::Hour),
				"day" => Ok(Self::Day),
				"week" => Ok(Self::Week),
				"month" => Ok(Self::Month),
				"year" => Ok(Self::Year),
				"all" => Ok(Self::All),
				_ => Err(()),
			}
		}
	}

	#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
	pub struct ListingQuery {
		pub after: Option<String>,
		pub before: Option<String>,
		pub limit: Option<u8>,
		pub count: Option<u32>,
		pub show: Option<ListingShow>,
		#[serde(rename = "t")]
		pub time: Option<ListingTime>,
		pub sr_detail: Option<bool>,
		#[serde(rename = "g")]
		pub geo_filter: Option<String>,
	}
}

mod post_sort {
	#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
	pub enum PostSort {
		#[default]
		Hot,
		Best,
		New,
		Rising,
		Top,
		Controversial,
	}

	impl PostSort {
		pub const fn as_str(self) -> &'static str {
			match self {
				Self::Hot => "hot",
				Self::Best => "best",
				Self::New => "new",
				Self::Rising => "rising",
				Self::Top => "top",
				Self::Controversial => "controversial",
			}
		}
	}

	impl std::str::FromStr for PostSort {
		type Err = ();

		fn from_str(value: &str) -> Result<Self, Self::Err> {
			match value {
				"hot" => Ok(Self::Hot),
				"best" => Ok(Self::Best),
				"new" => Ok(Self::New),
				"rising" => Ok(Self::Rising),
				"top" => Ok(Self::Top),
				"controversial" => Ok(Self::Controversial),
				_ => Err(()),
			}
		}
	}
}

pub use comment_query::{CommentQuery, CommentSort, CommentTheme};
pub use listing_query::{ListingQuery, ListingShow, ListingTime};
pub use post_sort::PostSort;
