use crate::models::{WikiPage, WikiPageListing};
use crate::parsing::{parse_wiki_page, parse_wiki_page_listing};
use crate::service::reddit::{invalid_parameter, validate_subreddit};
use crate::service::{RedditService, ServiceError};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

impl RedditService {
	pub async fn wiki_pages(&self, subreddit: &str) -> Result<WikiPageListing, ServiceError> {
		validate_subreddit(subreddit)?;
		self.require_safe_subreddit(subreddit).await?;
		let json = self.client.json(format!("/r/{subreddit}/wiki/pages")).await?;
		Ok(parse_wiki_page_listing(&json)?)
	}

	pub async fn wiki_page(&self, subreddit: &str, page: &str, query: &WikiPageQuery) -> Result<WikiPage, ServiceError> {
		validate_subreddit(subreddit)?;
		self.require_safe_subreddit(subreddit).await?;
		if query.v.is_some() || query.v2.is_some() {
			return Err(ServiceError::BadRequest);
		}
		let page = encode_wiki_page(page)?;
		let path = format!("/r/{subreddit}/wiki/{page}");
		let json = self.client.json(path).await?;
		Ok(parse_wiki_page(&json)?)
	}
}

fn encode_wiki_page(page: &str) -> Result<String, ServiceError> {
	let valid = !page.is_empty() && page.split('/').all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."));
	if !valid {
		return Err(invalid_parameter("page", page));
	}
	Ok(
		page
			.split('/')
			.map(|segment| utf8_percent_encode(segment, NON_ALPHANUMERIC).to_string())
			.collect::<Vec<_>>()
			.join("/"),
	)
}

mod wiki_page_query {

	#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
	pub struct WikiPageQuery {
		pub v: Option<String>,
		pub v2: Option<String>,
	}
}

pub use wiki_page_query::WikiPageQuery;
