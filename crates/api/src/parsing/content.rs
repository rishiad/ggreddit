use crate::models::{CommentChild, CommentReplies, Listing, Post, PostComments, Thing};
use crate::parsing::core::{parse_listing, validate_listing_kind, validate_thing_kind};
use crate::parsing::error::ParseError;
use serde::Deserialize;
use serde_json::Value;

pub(crate) fn parse_comments(json: &Value) -> Result<PostComments, ParseError> {
	let comments = PostComments::deserialize(json).map_err(|source| ParseError::InvalidPayload { entity: "post comments", source })?;
	validate_post_listing(&comments.0)?;
	validate_listing_kind(&comments.1, "comment listing")?;
	validate_comment_children(&comments.1.data.children)?;
	Ok(comments)
}

pub(crate) fn parse_post_listing(json: &Value) -> Result<Listing<Thing<Post>>, ParseError> {
	let listing = parse_listing(json)?;
	validate_post_listing(&listing)?;
	Ok(listing)
}

fn validate_post_listing(listing: &Listing<Thing<Post>>) -> Result<(), ParseError> {
	validate_listing_kind(listing, "post listing")?;
	for child in &listing.data.children {
		validate_thing_kind(child, "t3", "post listing child")?;
	}
	Ok(())
}

fn validate_comment_children(children: &[CommentChild]) -> Result<(), ParseError> {
	for child in children {
		match child {
			CommentChild::Comment(comment) => {
				validate_thing_kind(comment, "t1", "comment")?;
				match &comment.data.replies {
					CommentReplies::Empty(value) if !value.is_empty() => {
						return Err(ParseError::InvalidField {
							entity: "comment",
							field: "replies",
						})
					}
					CommentReplies::Listing(listing) => {
						validate_listing_kind(listing, "comment replies")?;
						validate_comment_children(&listing.data.children)?;
					}
					CommentReplies::Empty(_) => {}
				}
			}
			CommentChild::More(more) => validate_thing_kind(more, "more", "more comments")?,
		}
	}
	Ok(())
}
