use super::{age, author_path, compact, display_author, signed_compact, CommentTreeEvent, CommentView};
use crate::markdown;
use ggreddit_api::models::{Comment, CommentChild, CommentReplies, More};
use url::form_urlencoded;

pub fn comment_tree(children: &[CommentChild], link_id: &str, sort: &str, renderer: markdown::Renderer<'_>) -> Vec<CommentTreeEvent> {
	let mut tree = Vec::new();
	append_comments(children, link_id, sort, renderer, &mut tree);
	tree
}

fn append_comments(children: &[CommentChild], link_id: &str, sort: &str, renderer: markdown::Renderer<'_>, tree: &mut Vec<CommentTreeEvent>) {
	for child in children {
		match child {
			CommentChild::Comment(comment) => {
				let data = &comment.data;
				tree.push(CommentTreeEvent::Open(Box::new(comment_view(data, renderer))));
				if let CommentReplies::Listing(replies) = &data.replies {
					if !replies.data.children.is_empty() {
						tree.push(CommentTreeEvent::OpenReplies);
						append_comments(&replies.data.children, link_id, sort, renderer, tree);
						tree.push(CommentTreeEvent::CloseReplies);
					}
				}
				tree.push(CommentTreeEvent::Close);
			}
			CommentChild::More(more) => {
				tree.push(CommentTreeEvent::Open(Box::new(more_view(&more.data, link_id, sort))));
				tree.push(CommentTreeEvent::Close);
			}
		}
	}
}

fn comment_view(comment: &Comment, renderer: markdown::Renderer<'_>) -> CommentView {
	CommentView {
		id: comment.id.clone(),
		author: display_author(&comment.author),
		author_url: author_path(&comment.author),
		body: String::new(),
		body_html: renderer.render(&comment.body, comment.extra.get("media_metadata")),
		score: signed_compact(comment.score),
		age: age(comment.created_utc),
		permalink: format!("#comment-{}", comment.id),
		parent_url: comment_parent_url(comment),
		more_url: String::new(),
		more: false,
	}
}

fn more_view(more: &More, link_id: &str, sort: &str) -> CommentView {
	let cursor = more.extra.get("gql_cursor").and_then(serde_json::Value::as_str);
	CommentView {
		id: more.id.clone(),
		author: String::new(),
		author_url: String::new(),
		body: if more.count == 0 {
			"Continue this thread".into()
		} else {
			format!("{} more replies", compact(more.count))
		},
		body_html: String::new(),
		score: String::new(),
		age: String::new(),
		permalink: if cursor.is_none() {
			continue_thread_url(link_id, &more.parent_id, sort)
		} else {
			String::new()
		},
		parent_url: String::new(),
		more_url: cursor.map_or_else(String::new, |cursor| graphql_comments_url(cursor, link_id, sort)),
		more: true,
	}
}

fn graphql_comments_url(cursor: &str, link_id: &str, sort: &str) -> String {
	let mut query = form_urlencoded::Serializer::new(String::new());
	query.append_pair("cursor", cursor).append_pair("link_id", link_id);
	if sort != "best" {
		query.append_pair("sort", sort);
	}
	format!("/more-comments?{}", query.finish())
}

fn continue_thread_url(link_id: &str, parent_id: &str, sort: &str) -> String {
	let (Some(article), Some(parent)) = (link_id.strip_prefix("t3_"), parent_id.strip_prefix("t1_")) else {
		return String::new();
	};
	let mut url = format!("/comments/{article}/_/{parent}");
	if sort != "best" {
		let mut query = form_urlencoded::Serializer::new(String::new());
		query.append_pair("sort", sort);
		url.push('?');
		url.push_str(&query.finish());
	}
	url.push_str("#comment-");
	url.push_str(parent);
	url
}

pub(super) fn comment_parent_url(comment: &Comment) -> String {
	let article = comment.link_id.trim_start_matches("t3_");
	comment
		.parent_id
		.strip_prefix("t1_")
		.map_or_else(|| format!("/comments/{article}#post"), |parent| format!("/comments/{article}/_/{parent}#comment-{parent}"))
}

pub(super) fn comment_url(comment: &Comment) -> String {
	let article = comment.link_id.trim_start_matches("t3_");
	format!("/comments/{article}/_/{}#comment-{}", comment.id, comment.id)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn graphql_cursor_links_to_next_page() {
		let more = More {
			id: String::new(),
			name: String::new(),
			parent_id: "t3_post".into(),
			count: 30,
			children: Vec::new(),
			depth: None,
			extra: serde_json::Map::from_iter([("gql_cursor".into(), serde_json::json!("a/b+="))]),
		};
		let view = more_view(&more, "t3_post", "new");
		assert!(view.permalink.is_empty());
		assert_eq!(view.more_url, "/more-comments?cursor=a%2Fb%2B%3D&link_id=t3_post&sort=new");
	}
}
