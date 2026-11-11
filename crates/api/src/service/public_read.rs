use crate::models::{Listing, Post, PublicThing, Thing};

use crate::parsing::{parse_post_listing, parse_user_comment_listing};
use crate::service::query_codec;
use crate::service::reddit::{invalid_parameter, validate_listing_query_with, validate_username, with_query};
use crate::service::sanitize::{clear_listing_modhash, clear_public_listing_modhash};
use crate::service::{RedditService, ServiceError};

impl RedditService {
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
	for (name, present) in [("context", query.context.is_some()), ("show", query.show.is_some()), ("type", query.content_type.is_some())] {
		if present {
			return Err(invalid_parameter(name, "unsupported by GraphQL user history"));
		}
	}
	Ok(())
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

pub use user_history_query::{UserHistoryQuery, UserHistoryShow, UserHistorySort, UserHistoryType};
