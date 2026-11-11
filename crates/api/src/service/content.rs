use crate::models::{Listing, Post, PostComments, PublicThing, Subreddit, Thing, User};

use super::ServiceError;

#[derive(Clone, Copy, Debug)]
pub struct ContentPolicy {
	allow_nsfw: bool,
}

impl ContentPolicy {
	pub const fn new(allow_nsfw: bool) -> Self {
		Self { allow_nsfw }
	}

	pub const fn allows_nsfw(self) -> bool {
		self.allow_nsfw
	}

	pub(super) fn filter_posts(self, listing: &mut Listing<Thing<Post>>) {
		if !self.allow_nsfw {
			listing.data.children.retain(|post| !post.data.over_18);
			update_dist(listing);
		}
	}

	pub(super) fn filter_subreddits(self, listing: &mut Listing<Thing<Subreddit>>) {
		if !self.allow_nsfw {
			listing.data.children.retain(|subreddit| subreddit.data.over18 != Some(true));
			update_dist(listing);
		}
	}

	pub(super) fn filter_public(self, listing: &mut Listing<PublicThing>) {
		if !self.allow_nsfw {
			listing.data.children.retain(public_thing_is_safe);
			update_dist(listing);
		}
	}

	pub(super) fn require_subreddit(self, subreddit: &Thing<Subreddit>) -> Result<(), ServiceError> {
		if self.allow_nsfw || subreddit.data.over18 != Some(true) {
			Ok(())
		} else {
			Err(ServiceError::ContentBlocked)
		}
	}

	pub(super) fn require_user(self, user: &Thing<User>) -> Result<(), ServiceError> {
		let nsfw = user.data.subreddit.as_ref().and_then(|profile| profile.get("over_18")).and_then(serde_json::Value::as_bool);
		if self.allow_nsfw || nsfw != Some(true) {
			Ok(())
		} else {
			Err(ServiceError::ContentBlocked)
		}
	}

	pub(super) fn require_post_comments(self, comments: &PostComments) -> Result<(), ServiceError> {
		if self.allow_nsfw || comments.0.data.children.first().is_none_or(|post| !post.data.over_18) {
			Ok(())
		} else {
			Err(ServiceError::ContentBlocked)
		}
	}

	pub(super) fn require_post_listing(self, listing: &Listing<Thing<Post>>) -> Result<(), ServiceError> {
		if self.allow_nsfw || listing.data.children.first().is_none_or(|post| !post.data.over_18) {
			Ok(())
		} else {
			Err(ServiceError::ContentBlocked)
		}
	}
}

impl Default for ContentPolicy {
	fn default() -> Self {
		Self::new(true)
	}
}

fn public_thing_is_safe(thing: &PublicThing) -> bool {
	match thing {
		PublicThing::Post(post) => !post.data.over_18,
		PublicThing::Subreddit(subreddit) => subreddit.data.over18 != Some(true),
		PublicThing::Comment(comment) => comment.data.extra.get("over_18").and_then(serde_json::Value::as_bool) != Some(true),
		PublicThing::User(user) => user.data.subreddit.as_ref().and_then(|profile| profile.get("over_18")).and_then(serde_json::Value::as_bool) != Some(true),
	}
}

fn update_dist<T>(listing: &mut Listing<T>) {
	if listing.data.dist.is_some() {
		listing.data.dist = Some(listing.data.children.len() as u64);
	}
}
