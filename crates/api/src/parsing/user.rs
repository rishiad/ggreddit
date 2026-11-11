use crate::models::{Thing, User};
use crate::parsing::core::parse_thing;
use crate::parsing::error::ParseError;
use serde_json::Value;

pub(crate) fn parse_user(json: &Value) -> Result<Thing<User>, ParseError> {
	parse_thing(json, "t2", "user")
}
