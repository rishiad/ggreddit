use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct User {
	pub id: String,
	pub name: String,
	pub created: f64,
	pub created_utc: f64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub total_karma: Option<i64>,
	pub comment_karma: i64,
	pub link_karma: i64,
	pub is_gold: bool,
	pub is_mod: bool,
	pub has_verified_email: bool,
	pub subreddit: Option<Value>,
	#[serde(flatten)]
	pub extra: Map<String, Value>,
}
