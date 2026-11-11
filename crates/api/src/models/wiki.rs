use crate::models::{Thing, User};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WikiPageListing {
	pub kind: String,
	pub data: Vec<String>,
	#[serde(flatten)]
	pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WikiPage {
	pub kind: String,
	pub data: WikiPageData,
	#[serde(flatten)]
	pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WikiPageData {
	pub may_revise: bool,
	pub revision_date: Option<f64>,
	pub content_html: String,
	pub revision_by: Option<Thing<User>>,
	pub content_md: String,
	#[serde(flatten)]
	pub extra: Map<String, Value>,
}
