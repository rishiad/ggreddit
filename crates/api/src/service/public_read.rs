use crate::models::{Listing, Post, PublicThing, Subreddit, Thing, TrophyList, User};

use crate::parsing::{
	parse_info_listing, parse_post_listing, parse_search_listing, parse_subreddit_listing, parse_trophy_list, parse_user_comment_listing, parse_user_listing,
	parse_user_overview_listing,
};
use crate::service::query_codec;
use crate::service::reddit::{invalid_parameter, validate_listing_query, validate_listing_query_with, validate_subreddit, validate_username, with_query};
use crate::service::sanitize::{clear_listing_modhash, clear_public_listing_modhash};
use crate::service::{RedditService, ServiceError};
use url::Url;
use uuid::Uuid;

impl RedditService {
	pub async fn search(&self, query: &SearchQuery) -> Result<Listing<PublicThing>, ServiceError> {
		self.search_listing(None, query).await
	}

	pub async fn search_subreddit(&self, subreddit: &str, query: &SearchQuery) -> Result<Listing<PublicThing>, ServiceError> {
		validate_subreddit(subreddit)?;
		self.require_safe_subreddit(subreddit).await?;
		self.search_listing(Some(subreddit), query).await
	}

	pub async fn info(&self, query: &InfoQuery) -> Result<Listing<PublicThing>, ServiceError> {
		self.info_listing(None, query).await
	}

	pub async fn subreddit_info(&self, subreddit: &str, query: &InfoQuery) -> Result<Listing<PublicThing>, ServiceError> {
		validate_subreddit(subreddit)?;
		self.require_safe_subreddit(subreddit).await?;
		self.info_listing(Some(subreddit), query).await
	}

	pub async fn user_overview(&self, username: &str, query: &UserHistoryQuery) -> Result<Listing<PublicThing>, ServiceError> {
		validate_username(username)?;
		validate_user_history_query(query)?;
		let path = with_query(format!("/user/{username}/overview"), query_codec::encode(query));
		let json = self.client.json(path).await?;
		let mut listing = parse_user_overview_listing(&json)?;
		clear_public_listing_modhash(&mut listing);
		self.content.filter_public(&mut listing);
		Ok(listing)
	}

	pub async fn user_submitted(&self, username: &str, query: &UserHistoryQuery) -> Result<Listing<Thing<Post>>, ServiceError> {
		validate_username(username)?;
		validate_user_history_query(query)?;
		let path = with_query(format!("/user/{username}/submitted"), query_codec::encode(query));
		let json = self.client.json(path).await?;
		let mut listing = parse_post_listing(&json)?;
		clear_listing_modhash(&mut listing);
		self.content.filter_posts(&mut listing);
		Ok(listing)
	}

	pub async fn user_comments(&self, username: &str, query: &UserHistoryQuery) -> Result<Listing<PublicThing>, ServiceError> {
		validate_username(username)?;
		validate_user_history_query(query)?;
		let path = with_query(format!("/user/{username}/comments"), query_codec::encode(query));
		let json = self.client.json(path).await?;
		let mut listing = parse_user_comment_listing(&json)?;
		clear_public_listing_modhash(&mut listing);
		self.content.filter_public(&mut listing);
		Ok(listing)
	}

	pub async fn user_trophies(&self, username: &str) -> Result<TrophyList, ServiceError> {
		validate_username(username)?;
		let json = self.client.json(format!("/api/v1/user/{username}/trophies")).await?;
		Ok(parse_trophy_list(&json)?)
	}

	pub async fn users(&self, sort: UserDirectorySort, query: &crate::service::ListingQuery) -> Result<Listing<Thing<Subreddit>>, ServiceError> {
		validate_listing_query(query)?;
		let path = with_query(format!("/users/{}", sort.as_str()), query_codec::encode(query));
		let json = self.client.json(path).await?;
		let mut listing = parse_subreddit_listing(&json)?;
		clear_listing_modhash(&mut listing);
		self.content.filter_subreddits(&mut listing);
		Ok(listing)
	}

	pub async fn search_users(&self, query: &UserSearchQuery) -> Result<Listing<Thing<User>>, ServiceError> {
		validate_user_search_query(query)?;
		let path = with_query("/users/search".to_string(), query_codec::encode(query));
		let json = self.client.json(path).await?;
		let mut listing = parse_user_listing(&json)?;
		clear_listing_modhash(&mut listing);
		self.content.filter_users(&mut listing);
		Ok(listing)
	}

	async fn search_listing(&self, subreddit: Option<&str>, query: &SearchQuery) -> Result<Listing<PublicThing>, ServiceError> {
		validate_search_query(query)?;
		if !self.content.allows_nsfw() && query.include_over_18 == Some(true) {
			return Err(ServiceError::ContentBlocked);
		}
		let mut query = query.clone();
		if !self.content.allows_nsfw() {
			query.include_over_18 = Some(false);
		}
		let base = match subreddit {
			Some(subreddit) => format!("/r/{subreddit}/search"),
			None => "/search".to_string(),
		};
		let json = self.client.json(with_query(base, query_codec::encode(&query))).await?;
		let mut listing = parse_search_listing(&json)?;
		clear_listing_modhash(&mut listing);
		self.content.filter_public(&mut listing);
		Ok(listing)
	}

	async fn info_listing(&self, subreddit: Option<&str>, query: &InfoQuery) -> Result<Listing<PublicThing>, ServiceError> {
		validate_info_query(query)?;
		let base = match subreddit {
			Some(subreddit) => format!("/r/{subreddit}/api/info"),
			None => "/api/info".to_string(),
		};
		let json = self.client.json(with_query(base, query_codec::encode(query))).await?;
		let mut listing = parse_info_listing(&json)?;
		clear_public_listing_modhash(&mut listing);
		self.content.filter_public(&mut listing);
		Ok(listing)
	}
}

fn validate_search_query(query: &SearchQuery) -> Result<(), ServiceError> {
	validate_listing_query_with(&query.listing, |parameter, cursor| {
		if !cursor.is_empty() && cursor.len() <= 256 && cursor.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'_')) {
			Ok(())
		} else {
			Err(ServiceError::InvalidCursor {
				parameter,
				cursor: cursor.to_owned(),
			})
		}
	})?;
	if query.query.is_empty() || query.query.chars().count() > 512 {
		return Err(invalid_parameter("q", &query.query));
	}
	if query.category.as_ref().is_some_and(|category| category.chars().count() > 5) {
		return Err(invalid_parameter("category", query.category.as_deref().unwrap_or_default()));
	}
	Ok(())
}

fn validate_info_query(query: &InfoQuery) -> Result<(), ServiceError> {
	for id in &query.ids {
		let valid = ["t1_", "t3_", "t5_"].iter().any(|prefix| {
			id.strip_prefix(prefix)
				.is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
		});
		if !valid {
			return Err(invalid_parameter("id", id));
		}
	}
	for subreddit in &query.subreddit_names {
		validate_subreddit(subreddit)?;
	}
	if let Some(value) = &query.url {
		let url = Url::parse(value).map_err(|_| invalid_parameter("url", value))?;
		if !matches!(url.scheme(), "http" | "https") {
			return Err(invalid_parameter("url", value));
		}
	}
	Ok(())
}

fn validate_user_history_query(query: &UserHistoryQuery) -> Result<(), ServiceError> {
	validate_listing_query(&query.listing)?;
	if query.context.is_some_and(|context| !(2..=10).contains(&context)) {
		return Err(invalid_parameter("context", &query.context.unwrap_or_default().to_string()));
	}
	Ok(())
}

fn validate_user_search_query(query: &UserSearchQuery) -> Result<(), ServiceError> {
	validate_listing_query(&query.listing)?;
	if query.query.is_empty() {
		return Err(invalid_parameter("q", ""));
	}
	if let Some(search_query_id) = &query.search_query_id {
		Uuid::parse_str(search_query_id).map_err(|_| invalid_parameter("search_query_id", search_query_id))?;
	}
	Ok(())
}

mod info_query {

	#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
	pub struct InfoQuery {
		#[serde(rename = "id", default, with = "crate::service::query_codec::comma", skip_serializing_if = "Vec::is_empty")]
		pub ids: Vec<String>,
		#[serde(rename = "sr_name", default, with = "crate::service::query_codec::comma", skip_serializing_if = "Vec::is_empty")]
		pub subreddit_names: Vec<String>,
		pub url: Option<String>,
	}
}

mod search_query {
	use crate::service::ListingQuery;
	use std::{fmt, str::FromStr};

	#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
	pub struct SearchQuery {
		#[serde(flatten)]
		pub listing: ListingQuery,
		pub category: Option<String>,
		pub include_facets: Option<bool>,
		#[serde(default, with = "crate::service::query_codec::option_on_off", skip_serializing_if = "Option::is_none")]
		pub include_over_18: Option<bool>,
		#[serde(rename = "q")]
		pub query: String,
		pub restrict_sr: Option<bool>,
		pub sort: Option<SearchSort>,
		#[serde(rename = "type", default, with = "crate::service::query_codec::comma", skip_serializing_if = "Vec::is_empty")]
		pub result_types: Vec<SearchResultType>,
	}

	#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
	#[serde(rename_all = "lowercase")]
	pub enum SearchSort {
		Relevance,
		Hot,
		Top,
		New,
		Comments,
	}

	#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
	pub enum SearchResultType {
		#[serde(rename = "sr")]
		Subreddit,
		#[serde(rename = "link")]
		Post,
		#[serde(rename = "user")]
		User,
	}

	impl SearchResultType {
		pub(super) const fn as_str(self) -> &'static str {
			match self {
				Self::Subreddit => "sr",
				Self::Post => "link",
				Self::User => "user",
			}
		}
	}

	impl fmt::Display for SearchResultType {
		fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
			formatter.write_str(self.as_str())
		}
	}

	impl FromStr for SearchResultType {
		type Err = &'static str;

		fn from_str(value: &str) -> Result<Self, Self::Err> {
			match value {
				"sr" => Ok(Self::Subreddit),
				"link" => Ok(Self::Post),
				"user" => Ok(Self::User),
				_ => Err("unknown search result type"),
			}
		}
	}
}

mod user_directory_sort {
	#[derive(Clone, Copy, Debug, Eq, PartialEq)]
	pub enum UserDirectorySort {
		New,
		Popular,
	}

	impl UserDirectorySort {
		pub(super) const fn as_str(self) -> &'static str {
			match self {
				Self::New => "new",
				Self::Popular => "popular",
			}
		}
	}
}

mod user_history_query {
	use crate::service::ListingQuery;

	#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
	pub struct UserHistoryQuery {
		#[serde(flatten)]
		pub listing: ListingQuery,
		pub context: Option<u8>,
		pub show: Option<UserHistoryShow>,
		pub sort: Option<UserHistorySort>,
		#[serde(rename = "type")]
		pub content_type: Option<UserHistoryType>,
	}

	#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
	#[serde(rename_all = "lowercase")]
	pub enum UserHistoryShow {
		Given,
	}

	#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
	#[serde(rename_all = "lowercase")]
	pub enum UserHistorySort {
		Hot,
		New,
		Top,
		Controversial,
	}

	impl UserHistorySort {
		pub const fn as_str(self) -> &'static str {
			match self {
				Self::Hot => "hot",
				Self::New => "new",
				Self::Top => "top",
				Self::Controversial => "controversial",
			}
		}
	}

	impl std::str::FromStr for UserHistorySort {
		type Err = ();

		fn from_str(value: &str) -> Result<Self, Self::Err> {
			match value {
				"hot" => Ok(Self::Hot),
				"new" => Ok(Self::New),
				"top" => Ok(Self::Top),
				"controversial" => Ok(Self::Controversial),
				_ => Err(()),
			}
		}
	}

	#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
	pub enum UserHistoryType {
		#[serde(rename = "links")]
		Posts,
		#[serde(rename = "comments")]
		Comments,
	}
}

mod user_search_query {
	use crate::service::{ListingQuery, SubredditSearchSort, Typeahead};

	#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
	pub struct UserSearchQuery {
		#[serde(flatten)]
		pub listing: ListingQuery,
		#[serde(rename = "q")]
		pub query: String,
		pub search_query_id: Option<String>,
		pub sort: Option<SubredditSearchSort>,
		pub typeahead_active: Option<Typeahead>,
	}
}

mod subreddit_metadata {
	use crate::models::{Sidebar, SubredditRules};
	use crate::parsing::{parse_subreddit_rules, ParseError};
	use crate::service::reddit::validate_subreddit;
	use crate::service::{RedditService, ServiceError};

	impl RedditService {
		pub async fn subreddit_rules(&self, subreddit: &str) -> Result<SubredditRules, ServiceError> {
			validate_subreddit(subreddit)?;
			self.require_safe_subreddit(subreddit).await?;
			let json = self.client.json(format!("/r/{subreddit}/about/rules")).await?;
			Ok(parse_subreddit_rules(&json)?)
		}

		pub async fn subreddit_sidebar(&self, subreddit: &str) -> Result<Sidebar, ServiceError> {
			let subreddit = self.subreddit_about(subreddit).await?;
			Ok(Sidebar {
				description: subreddit.data.description,
				description_html: subreddit.data.description_html.ok_or(ParseError::InvalidField {
					entity: "subreddit",
					field: "description_html",
				})?,
			})
		}
	}
}

pub use info_query::InfoQuery;
pub use search_query::{SearchQuery, SearchResultType, SearchSort};
pub use user_directory_sort::UserDirectorySort;
pub use user_history_query::{UserHistoryQuery, UserHistoryShow, UserHistorySort, UserHistoryType};
pub use user_search_query::UserSearchQuery;
