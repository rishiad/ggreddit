use crate::models::{Listing, Post, PublicThing, Subreddit, Thing, TrophyList};

use crate::parsing::{parse_info_listing, parse_post_listing, parse_subreddit_listing, parse_trophy_list, parse_user_comment_listing, parse_user_overview_listing};
use crate::service::query_codec;
use crate::service::reddit::{invalid_parameter, validate_listing_query, validate_listing_query_with, validate_subreddit, validate_username, with_query};
use crate::service::sanitize::{clear_listing_modhash, clear_public_listing_modhash};
use crate::service::{RedditService, ServiceError};
use url::Url;

impl RedditService {
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
		if !self.content.allows_nsfw() {
			let names: std::collections::BTreeSet<_> = listing
				.data
				.children
				.iter()
				.filter_map(|item| match item {
					PublicThing::Comment(comment) => comment.data.extra.get("subreddit").and_then(serde_json::Value::as_str).map(str::to_owned),
					_ => None,
				})
				.collect();
			let mut safe = std::collections::BTreeSet::new();
			for name in names {
				match self.subreddit_about(&name).await {
					Ok(_) => {
						safe.insert(name);
					}
					Err(ServiceError::ContentBlocked) => {}
					Err(error) => return Err(error),
				}
			}
			listing.data.children.retain(|item| match item {
				PublicThing::Comment(comment) => comment
					.data
					.extra
					.get("subreddit")
					.and_then(serde_json::Value::as_str)
					.is_some_and(|name| safe.contains(name)),
				_ => false,
			});
		}
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
	validate_listing_query_with(&query.listing, |parameter, cursor| {
		let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, cursor).ok();
		let decoded = decoded.as_deref().and_then(|bytes| std::str::from_utf8(bytes).ok());
		let valid = cursor.len() <= 256
			&& [Some(cursor), decoded]
				.into_iter()
				.flatten()
				.any(|id| (id.starts_with("t1_") || id.starts_with("t3_")) && !id[3..].is_empty() && id[3..].bytes().all(|byte| byte.is_ascii_alphanumeric()));
		if valid {
			Ok(())
		} else {
			Err(ServiceError::InvalidCursor { parameter, cursor: cursor.into() })
		}
	})?;
	if query.listing.before.is_some() {
		return Err(invalid_parameter("before", "unsupported by GraphQL user history"));
	}
	if query.context.is_some_and(|context| !(2..=10).contains(&context)) {
		return Err(invalid_parameter("context", &query.context.unwrap_or_default().to_string()));
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

mod subreddit_metadata {
	use crate::models::Sidebar;
	use crate::parsing::ParseError;
	use crate::service::{RedditService, ServiceError};

	impl RedditService {
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
pub use user_directory_sort::UserDirectorySort;
pub use user_history_query::{UserHistoryQuery, UserHistoryShow, UserHistorySort, UserHistoryType};

#[cfg(test)]
mod graphql_user_history_tests {
	use super::*;
	#[test]
	fn accepts_bounded_comment_cursors() {
		let query = UserHistoryQuery {
			listing: crate::service::ListingQuery {
				after: Some("dDFfb3B0ZnlxbA==".into()),
				..Default::default()
			},
			..Default::default()
		};
		assert!(validate_user_history_query(&query).is_ok());
		let query = UserHistoryQuery {
			listing: crate::service::ListingQuery {
				after: Some("not a cursor".into()),
				..Default::default()
			},
			..Default::default()
		};
		assert!(validate_user_history_query(&query).is_err());
	}
}
