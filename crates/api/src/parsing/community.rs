use crate::models::{Subreddit, Thing, WikiPage, WikiPageListing};
use crate::parsing::core::{parse_thing, validate_thing_kind};
use crate::parsing::error::ParseError;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub(crate) fn parse_subreddit(json: &Value) -> Result<Thing<Subreddit>, ParseError> {
	parse_thing(json, "t5", "subreddit")
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
