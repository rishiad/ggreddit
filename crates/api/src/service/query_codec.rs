use serde::Serialize;

pub(super) fn encode(value: &impl Serialize) -> String {
	serde_urlencoded::to_string(value).expect("query models serialize to form data")
}
