use crate::models::{Listing, PublicThing};
use crate::parsing::core::parse_listing;
use crate::parsing::error::ParseError;
use serde_json::Value;

fn parse_public_listing(json: &Value) -> Result<Listing<PublicThing>, ParseError> {
	let listing: Listing<PublicThing> = parse_listing(json)?;
	for child in &listing.data.children {
		if child.kind() != child.expected_kind() {
			return Err(ParseError::UnsupportedKind {
				entity: "public listing child",
				kind: child.kind().to_string(),
			});
		}
	}
	Ok(listing)
}

pub(crate) fn parse_user_comment_listing(json: &Value) -> Result<Listing<PublicThing>, ParseError> {
	let listing = parse_public_listing(json)?;
	ensure_children(&listing, "user comment child", |child| matches!(child, PublicThing::Comment(_)))?;
	Ok(listing)
}

fn ensure_children(listing: &Listing<PublicThing>, entity: &'static str, allowed: impl Fn(&PublicThing) -> bool) -> Result<(), ParseError> {
	for child in &listing.data.children {
		if !allowed(child) {
			return Err(ParseError::UnsupportedKind {
				entity,
				kind: child.kind().to_string(),
			});
		}
	}
	Ok(())
}

impl PublicThing {
	fn kind(&self) -> &str {
		match self {
			Self::Comment(thing) => &thing.kind,
			Self::User(thing) => &thing.kind,
			Self::Post(thing) => &thing.kind,
			Self::Subreddit(thing) => &thing.kind,
		}
	}

	fn expected_kind(&self) -> &'static str {
		match self {
			Self::Comment(_) => "t1",
			Self::User(_) => "t2",
			Self::Post(_) => "t3",
			Self::Subreddit(_) => "t5",
		}
	}
}
