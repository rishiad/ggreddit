use crate::models::{WikiPage, WikiPageListing};
use crate::parsing::{parse_wiki_page, parse_wiki_page_listing};
use crate::service::reddit::{invalid_parameter, validate_subreddit};
use crate::service::{RedditService, ServiceError};

impl RedditService {
	pub async fn wiki_pages(&self, subreddit: &str) -> Result<WikiPageListing, ServiceError> {
		validate_subreddit(subreddit)?;
		self.require_safe_subreddit(subreddit).await?;
		let json = self.client.read_wiki(subreddit, None).await?;
		Ok(parse_wiki_page_listing(&json)?)
	}

	pub async fn wiki_page(&self, subreddit: &str, page: &str, query: &WikiPageQuery) -> Result<WikiPage, ServiceError> {
		validate_subreddit(subreddit)?;
		self.require_safe_subreddit(subreddit).await?;
		if query.v.is_some() || query.v2.is_some() {
			return Err(ServiceError::BadRequest);
		}
		validate_wiki_page(page)?;
		let json = self.client.read_wiki(subreddit, Some(page)).await?;
		Ok(parse_wiki_page(&json)?)
	}
}

fn validate_wiki_page(page: &str) -> Result<(), ServiceError> {
	let valid = !page.is_empty() && page.split('/').all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."));
	if !valid {
		return Err(invalid_parameter("page", page));
	}
	Ok(())
}

mod wiki_page_query {

	#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
	pub struct WikiPageQuery {
		pub v: Option<String>,
		pub v2: Option<String>,
	}
}

pub use wiki_page_query::WikiPageQuery;
