use super::{CommentChild, Listing, Thing};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Post {
	pub id: String,
	pub name: String,
	pub title: String,
	pub subreddit: String,
	pub subreddit_id: String,
	pub author: String,
	pub author_fullname: Option<String>,
	pub permalink: String,
	pub url: String,
	pub selftext: String,
	pub created_utc: f64,
	pub score: i64,
	pub hide_score: bool,
	pub upvote_ratio: f64,
	pub num_comments: u64,
	pub is_self: bool,
	pub over_18: bool,
	pub spoiler: bool,
	pub stickied: bool,
	pub pinned: bool,
	pub locked: bool,
	pub archived: bool,
	#[serde(flatten)]
	pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PostComments(pub Listing<Thing<Post>>, pub Listing<CommentChild>);
