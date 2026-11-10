use crate::models::{Listing, Post, Subreddit, Thing, WikiPage, WikiPageListing, WikiRevision};
use crate::parsing::content::parse_post_listing;
use crate::parsing::core::{parse_listing, parse_thing, validate_thing_kind};
use crate::parsing::error::ParseError;
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

pub(crate) fn parse_subreddit(json: &Value) -> Result<Thing<Subreddit>, ParseError> {
	parse_thing(json, "t5", "subreddit")
}

pub(crate) fn parse_subreddit_listing(json: &Value) -> Result<Listing<Thing<Subreddit>>, ParseError> {
	let listing = parse_listing(json)?;
	for child in &listing.data.children {
		validate_thing_kind(child, "t5", "subreddit listing child")?;
	}
	Ok(listing)
}

pub(crate) fn parse_wiki_page_listing(json: &Value) -> Result<WikiPageListing, ParseError> {
	let listing: WikiPageListing = parse_wiki_payload(json, "wiki page listing")?;
	validate_wiki_kind(&listing.kind, "wikipagelisting", "wiki page listing")?;
	if listing.data.iter().any(String::is_empty) {
		return Err(ParseError::InvalidField {
			entity: "wiki page listing",
			field: "data",
		});
	}
	Ok(listing)
}

pub(crate) fn parse_wiki_page(json: &Value) -> Result<WikiPage, ParseError> {
	let page: WikiPage = parse_wiki_payload(json, "wiki page")?;
	validate_wiki_kind(&page.kind, "wikipage", "wiki page")?;
	if let Some(author) = &page.data.revision_by {
		validate_thing_kind(author, "t2", "wiki page revision author")?;
	}
	Ok(page)
}

pub(crate) fn parse_wiki_revisions(json: &Value) -> Result<Listing<WikiRevision>, ParseError> {
	let listing: Listing<WikiRevision> = parse_listing(json)?;
	for revision in &listing.data.children {
		if revision.page.is_empty() {
			return Err(ParseError::InvalidField {
				entity: "wiki revision",
				field: "page",
			});
		}
		if Uuid::parse_str(&revision.id).is_err() {
			return Err(ParseError::InvalidField {
				entity: "wiki revision",
				field: "id",
			});
		}
		if let Some(author) = &revision.author {
			validate_thing_kind(author, "t2", "wiki revision author")?;
		}
	}
	Ok(listing)
}

pub(crate) fn parse_wiki_discussions(json: &Value) -> Result<Listing<Thing<Post>>, ParseError> {
	parse_post_listing(json)
}

fn parse_wiki_payload<T>(json: &Value, entity: &'static str) -> Result<T, ParseError>
where
	T: DeserializeOwned,
{
	T::deserialize(json).map_err(|source| ParseError::InvalidPayload { entity, source })
}

fn validate_wiki_kind(kind: &str, expected_kind: &str, entity: &'static str) -> Result<(), ParseError> {
	if kind == expected_kind {
		Ok(())
	} else {
		Err(ParseError::UnsupportedKind { entity, kind: kind.to_string() })
	}
}
