use super::{error::ClientError, RedditClient};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::DateTime;
use serde_json::{json, Value};
use url::form_urlencoded;

const FEED: &str = "97046e2a48050f10a507db354a8da60955ffccb586dcf63163bdfc6610d5cf7a";
const POPULAR: &str = "728c0f9a4d12c17b8cffe07669bb1a8770b0f2ec0c84e040485f8fad2d9c417f";
const ALL: &str = "0e9c0045dd244e934efd0ea4fefc00cb0b4d91523e2f09c8af075856cb59b937";
const BY_IDS: &str = "0e471495bcba33554b6c540b3cf1cae23dd5432a8b741dddc7fd6717dc2520a7";
const COMMUNITY: &str = "6b9c1679e69097e1c6df364adc11183afe6e2b6545dd1c0c1cc8f7490448c3e5";
const USER_PROFILE: &str = "a05c628c5469c6b107c85988a8b1d0b79a2e7b6150ebd3a449c67e226e975ddc";
const USER_DATE: &str = "1412e2c935de284ee35ec458baabc6920b530d87309da5eecdaa8b83f87f36c4";
const USER_POSTS: &str = "78a213c02a340db4a707d4bcd375ce34f733b75893379603acb4ec20388fafe6";
const USER_COMMENTS: &str = "e1b37f521fb40af2120946b70e7fbf6a3d924db2abaf28ff2164d4cfacca28db";
const COMMENTS_BY_IDS: &str = "51ab9f2e0eb87b6dac9aac02012ba7046049e516ec55ec442abfed0f19c6f311";
const WIKI: &str = "32f9314683f033a76812f772bbba771958760286c5cd28c558071a3ff4eb231f";

// Only these known routes use GraphQL. Other Reddit endpoints keep their own handlers.
pub(super) async fn route(client: &RedditClient, path: &str) -> Option<Result<Value, ClientError>> {
	let (route, query) = path.split_once('?').unwrap_or((path, ""));
	let params: Vec<_> = form_urlencoded::parse(query.as_bytes()).collect();
	let get = |name: &str| params.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_ref());
	if let Some(ids) = route.strip_prefix("/by_id/") {
		return Some(by_ids(client, ids.split(',').map(str::to_owned).collect()).await);
	}
	let pieces: Vec<_> = route.trim_matches('/').split('/').collect();
	if let ["user", name, "about"] = pieces.as_slice() {
		return Some(user(client, name).await);
	}
	if let ["user", name, kind @ ("submitted" | "comments")] = pieces.as_slice() {
		return Some(user_activity(client, name, kind, get("sort"), get("t"), get("after"), get("limit")).await);
	}
	if let ["r", name, "wiki", "pages"] = pieces.as_slice() {
		return Some(wiki(client, name, None).await);
	}
	if let ["r", name, "wiki", page @ ..] = pieces.as_slice() {
		if !page.is_empty() {
			if get("v").is_some() || get("v2").is_some() {
				return Some(Err(missing()));
			}
			return Some(wiki(client, name, Some(&page.join("/"))).await);
		}
	}
	if let ["r", name, "about"] = pieces.as_slice() {
		return Some(community(client, name).await);
	}
	let (name, sort) = match pieces.as_slice() {
		["r", name, sort] => (*name, *sort),
		[sort] if matches!(*sort, "hot" | "best" | "new" | "rising" | "top" | "controversial") => ("all", *sort),
		_ => return None,
	};
	if !matches!(sort, "hot" | "best" | "new" | "rising" | "top" | "controversial") {
		return None;
	}
	Some(feed(client, name, sort, get("t"), get("after"), get("limit")).await)
}

fn missing() -> ClientError {
	ClientError::InvalidEnvelope
}

fn listing(items: Vec<Value>, cursor: Option<String>) -> Value {
	json!({"kind":"Listing","data":{"children":items,"after":cursor,"before":null}})
}

fn cursor(value: &Value) -> Option<String> {
	let value = value.as_str()?;
	let decoded = STANDARD.decode(value).ok().and_then(|bytes| String::from_utf8(bytes).ok());
	Some(decoded.filter(|id| id.starts_with("t3_")).unwrap_or_else(|| value.to_owned()))
}

fn edges(value: &Value) -> Result<(Vec<Value>, Option<String>), ClientError> {
	let nodes = value["edges"].as_array().ok_or_else(missing)?;
	let posts = nodes
		.iter()
		.filter_map(|edge| edge.get("node"))
		.filter(|node| node["id"].as_str().is_some_and(|id| id.starts_with("t3_")))
		.map(post)
		.collect::<Result<Vec<_>, _>>()?;
	Ok((posts, cursor(&value["pageInfo"]["endCursor"])))
}

async fn feed(client: &RedditClient, name: &str, sort: &str, range: Option<&str>, after: Option<&str>, limit: Option<&str>) -> Result<Value, ClientError> {
	let mut variables = json!({"sort":sort.to_ascii_uppercase(),"pageSize":limit.and_then(|n| n.parse::<u8>().ok()).unwrap_or(25)});
	if let Some(range) = range {
		variables["range"] = range.to_ascii_uppercase().into();
	}
	if let Some(after) = after {
		variables["after"] = if after.starts_with("t3_") { STANDARD.encode(after).into() } else { after.into() };
	}
	let name = name.replace("%2B", "+").replace("%2b", "+");
	if name.contains('+') {
		return Err(missing());
	}
	let (operation, hash, root) = match name.as_str() {
		"all" => ("AllFeedSdui", ALL, "/data/allV3/elements"),
		"popular" => ("PopularFeedElements", POPULAR, "/data/popular/elements"),
		_ => {
			variables["subredditName"] = name.into();
			variables["includeSubredditInPosts"] = true.into();
			("SubredditFeedElements", FEED, "/data/postFeed/elements")
		}
	};
	if operation == "AllFeedSdui" {
		variables.as_object_mut().unwrap().remove("pageSize");
		variables.as_object_mut().unwrap().remove("range");
		if let Some(range) = range {
			variables["time"] = range.to_ascii_uppercase().into();
		}
	}
	if operation == "AllFeedSdui" {
		let mut ids = Vec::new();
		let mut after = None;
		for _ in 0..3 {
			let response = client.graphql(operation, hash, variables.clone()).await?;
			let elements = response.pointer(root).ok_or_else(missing)?;
			for edge in elements["edges"].as_array().ok_or_else(missing)? {
				let node = &edge["node"];
				let ad = node["cells"]
					.as_array()
					.is_some_and(|cells| cells.iter().any(|cell| cell["__typename"] == "AdMetadataCell"));
				if !ad {
					if let Some(id) = node["groupId"].as_str().filter(|id| id.starts_with("t3_")) {
						if !ids.iter().any(|existing| existing == id) {
							ids.push(id.to_owned());
						}
					}
				}
			}
			after = cursor(&elements["pageInfo"]["endCursor"]);
			if ids.len() >= limit.and_then(|n| n.parse::<usize>().ok()).unwrap_or(25) || after.is_none() {
				break;
			}
			variables["after"] = if after.as_deref().is_some_and(|value| value.starts_with("t3_")) {
				STANDARD.encode(after.as_deref().unwrap()).into()
			} else {
				after.clone().unwrap().into()
			};
		}
		let result = by_ids(client, ids).await?;
		let mut posts = result["data"]["children"].as_array().cloned().ok_or_else(missing)?;
		let requested = limit.and_then(|n| n.parse::<usize>().ok()).unwrap_or(25);
		if posts.len() > requested {
			posts.truncate(requested);
			after = posts.last().and_then(|post| post["data"]["name"].as_str()).map(str::to_owned);
		}
		return Ok(listing(posts, after));
	}
	let response = client.graphql(operation, hash, variables).await?;
	let elements = response.pointer(root).ok_or_else(missing)?;
	let (posts, after) = if operation == "PopularFeedElements" {
		let nodes = elements["edges"].as_array().ok_or_else(missing)?;
		let posts = nodes
			.iter()
			.filter_map(|edge| edge["node"].get("postInfo"))
			.filter(|node| node["id"].is_string())
			.map(post)
			.collect::<Result<Vec<_>, _>>()?;
		(posts, cursor(&elements["pageInfo"]["endCursor"]))
	} else {
		edges(elements)?
	};
	Ok(listing(posts, after))
}

async fn by_ids(client: &RedditClient, ids: Vec<String>) -> Result<Value, ClientError> {
	if ids.is_empty() {
		return Ok(listing(vec![], None));
	}
	let response = client
		.graphql(
			"PostsByIds",
			BY_IDS,
			json!({"ids":ids,"includePostContentPostHint":true,"includePostContentThumbnailEnabled":true}),
		)
		.await?;
	let nodes = response.pointer("/data/postsInfoByIds").and_then(Value::as_array).ok_or_else(missing)?;
	let posts = nodes
		.iter()
		.filter(|node| node["id"].is_string() && node["subreddit"]["name"].is_string())
		.map(post)
		.collect::<Result<Vec<_>, _>>()?;
	Ok(listing(posts, None))
}

async fn community(client: &RedditClient, name: &str) -> Result<Value, ClientError> {
	let response = client
		.graphql(
			"SubredditInfoByName",
			COMMUNITY,
			json!({"subredditName":name,"includeDevvitData":false,"includeRecapFields":false}),
		)
		.await?;
	subreddit(response.pointer("/data/subredditInfoByName").ok_or_else(missing)?)
}

async fn user(client: &RedditClient, name: &str) -> Result<Value, ClientError> {
	let response = client
		.graphql("UserProfile", USER_PROFILE, json!({"name":name,"includePremiumAvatarTreatment":false}))
		.await?;
	let node = response.pointer("/data/redditorInfoByName").ok_or_else(missing)?;
	let id = node["id"].as_str().and_then(|id| id.strip_prefix("t2_")).ok_or_else(missing)?;
	let username = node["name"].as_str().ok_or_else(missing)?;
	let date = client.graphql("GetMatrixChatUsersByIds", USER_DATE, json!({"userKindWithIds":[node["id"]]})).await?;
	let created = date
		.pointer("/data/redditorsInfoByIds")
		.and_then(Value::as_array)
		.ok_or_else(missing)?
		.iter()
		.find(|user| user["id"] == node["id"])
		.and_then(|user| user["cakeDayOn"].as_str())
		.and_then(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
		.and_then(|date| date.and_hms_opt(0, 0, 0))
		.map(|date| date.and_utc().timestamp() as f64)
		.or_else(|| node["profile"]["createdAt"].as_str().map(|_| timestamp(&node["profile"]["createdAt"])))
		.filter(|date| *date > 0.0)
		.ok_or_else(missing)?;
	let profile = &node["profile"];
	// The app query omits these required REST account flags; do not infer them from unrelated fields.
	Ok(json!({"kind":"t2","data":{
		"id":id,"name":username,"created":created,"created_utc":created,
		"total_karma":whole_number(&node["karma"]["total"]),"link_karma":whole_number(&node["karma"]["fromPosts"]).unwrap_or_default(),
		"comment_karma":whole_number(&node["karma"]["fromComments"]).unwrap_or_default(),
		"is_gold":false,"is_mod":false,"has_verified_email":false,
		"subreddit":{"title":profile["title"],"public_description":profile["publicDescriptionText"],"icon_img":profile["styles"]["icon"],"over_18":profile["isNsfw"] == true}
	}}))
}

fn whole_number(value: &Value) -> Option<i64> {
	value
		.as_i64()
		.or_else(|| value.as_f64().filter(|number| number.abs() < 1e12 && number.fract() == 0.0).map(|number| number as i64))
}

fn count(value: &Value) -> Option<u64> {
	whole_number(value).and_then(|number| u64::try_from(number).ok())
}

async fn user_activity(
	client: &RedditClient,
	name: &str,
	kind: &str,
	sort: Option<&str>,
	range: Option<&str>,
	after: Option<&str>,
	limit: Option<&str>,
) -> Result<Value, ClientError> {
	let mut vars = json!({"sort":sort.unwrap_or("new").to_ascii_uppercase(),"pageSize":limit.and_then(|n|n.parse::<u8>().ok()).unwrap_or(25)});
	if let Some(range) = range {
		vars["range"] = range.to_ascii_uppercase().into();
	}
	if let Some(after) = after {
		vars["after"] = if after.starts_with("t3_") || after.starts_with("t1_") {
			STANDARD.encode(after).into()
		} else {
			after.into()
		};
	}
	if kind == "submitted" {
		vars["username"] = name.into();
		let response = client.graphql("UserSubmittedPosts", USER_POSTS, vars).await?;
		let nodes = response.pointer("/data/postFeed/posts").ok_or_else(missing)?;
		let edges = nodes["edges"].as_array().ok_or_else(missing)?;
		let posts = edges.iter().map(|edge| post(&edge["node"])).collect::<Result<Vec<_>, _>>()?;
		return Ok(listing(posts, cursor(&nodes["pageInfo"]["endCursor"])));
	}
	vars["name"] = name.into();
	let response = client.graphql("SubmittedComments", USER_COMMENTS, vars).await?;
	let comments = response.pointer("/data/redditorInfoByName/comments").ok_or_else(missing)?;
	let previews = comments["edges"].as_array().ok_or_else(missing)?;
	if previews.is_empty() {
		return Ok(listing(Vec::new(), None));
	}
	let ids: Vec<_> = previews.iter().map(|edge| edge["node"]["id"].as_str().ok_or_else(missing)).collect::<Result<_, _>>()?;
	let full = client
		.graphql("LiveCommentsByIds", COMMENTS_BY_IDS, json!({"ids":ids,"includePremiumAvatarTreatment":false}))
		.await?;
	let nodes = full.pointer("/data/commentsByIds").and_then(Value::as_array).ok_or_else(missing)?;
	let items = previews
		.iter()
		.filter_map(|edge| {
			let preview = &edge["node"];
			let node = nodes.iter().find(|node| node["id"] == preview["id"])?;
			let id = node["id"].as_str()?.strip_prefix("t1_")?;
			let link_id = preview["postInfo"]["id"].as_str()?;
			let subreddit = preview["postInfo"]["subreddit"]["prefixedName"].as_str().unwrap_or("").strip_prefix("r/").unwrap_or("");
			let body = if node["__typename"] == "DeletedComment" {
				"[deleted]"
			} else if node["isRemoved"] == true {
				"[removed]"
			} else {
				node["content"]["markdown"].as_str()?
			};
			Some(json!({"kind":"t1","data":{
				"id":id,"name":node["id"],"parent_id":node["parentId"].as_str().unwrap_or(link_id),"link_id":link_id,
				"author":node["authorInfo"]["name"].as_str().unwrap_or("[deleted]"),
				"body":body,"body_html":null,
				"score":node["score"].as_i64().unwrap_or_default(),"created_utc":timestamp(&node["createdAt"]),
				"edited":false,"replies":"","subreddit":subreddit,"link_title":preview["postInfo"]["title"]
			}}))
		})
		.collect();
	Ok(listing(items, cursor(&comments["pageInfo"]["endCursor"])))
}

async fn wiki(client: &RedditClient, subreddit: &str, page: Option<&str>) -> Result<Value, ClientError> {
	let name = page.unwrap_or("index");
	let decoded = percent_encoding::percent_decode_str(name).decode_utf8().map_err(|_| missing())?;
	if decoded.is_empty() || decoded.split('/').any(|part| part.is_empty() || matches!(part, "." | "..")) {
		return Err(missing());
	}
	let response = client
		.graphql(
			"SubredditWiki",
			WIKI,
			json!({
				"subredditName":subreddit,"pageName":decoded,"includePage":true,"includePageContent":true,
				"includeMediaFragment":true,"includeIsRevisable":false,"includeRevisionsCount":false,"includePageTitle":true
			}),
		)
		.await?;
	let root = response.pointer("/data/subredditInfoByName").ok_or_else(missing)?;
	if root["__typename"] != "Subreddit" {
		return Err(missing());
	}
	if page.is_none() {
		let tree = root.pointer("/wiki/index/pageTree").and_then(Value::as_array).ok_or_else(missing)?;
		let pages: Vec<_> = tree
			.iter()
			.filter(|node| node["isPagePresent"] == true)
			.filter_map(|node| node["path"].as_str())
			.filter(|path| path.split('/').all(|part| !part.is_empty() && !matches!(part, "." | "..") && !part.contains('\\')))
			.map(str::to_owned)
			.collect();
		return Ok(json!({"kind":"wikipagelisting","data":pages}));
	}
	let node = root.pointer("/wiki/page").ok_or_else(missing)?;
	if node["status"] != "VALID" || node["name"].as_str() != Some(decoded.as_ref()) {
		return Err(missing());
	}
	let content = node["content"]["richtext"].as_str().ok_or_else(missing)?;
	if content.len() > 2_000_000 {
		return Err(missing());
	}
	let richtext: Value = serde_json::from_str(content).map_err(|_| missing())?;
	let mut html = String::new();
	let mut plain = String::new();
	let blocks = richtext["document"].as_array().ok_or_else(missing)?;
	for block in blocks {
		render_wiki(block, &mut html, &mut plain, 0)?;
	}
	if html.len() > 2_000_000 {
		return Err(missing());
	}
	Ok(json!({"kind":"wikipage","data":{
		"may_revise":false,"revision_date":node["revision"]["createdAt"].as_str().map(|_| timestamp(&node["revision"]["createdAt"])),
		"content_html":html,"content_md":plain.trim_end(),"revision_by":null,"gql_rendered":true
	}}))
}

fn escape_html(text: &str, out: &mut String) {
	for char in text.chars() {
		match char {
			'&' => out.push_str("&amp;"),
			'<' => out.push_str("&lt;"),
			'>' => out.push_str("&gt;"),
			'"' => out.push_str("&quot;"),
			'\'' => out.push_str("&#39;"),
			_ => out.push(char),
		}
	}
}

fn render_wiki(node: &Value, html: &mut String, plain: &mut String, depth: usize) -> Result<(), ClientError> {
	if depth > 32 {
		return Err(missing());
	}
	let element = node["e"].as_str().ok_or_else(missing)?;
	let tag = match element {
		"par" => Some("p"),
		"li" => Some("li"),
		"list" => Some(if node["o"] == true { "ol" } else { "ul" }),
		"h" => Some("h2"),
		"blockquote" => Some("blockquote"),
		_ => None,
	};
	if let Some(tag) = tag {
		html.push('<');
		html.push_str(tag);
		html.push('>');
	}
	if let Some(text) = node["t"].as_str() {
		if element == "link" {
			let url = node["u"].as_str().unwrap_or("");
			if url::Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "https" | "http")) {
				html.push_str("<a href=\"");
				escape_html(url, html);
				html.push_str("\">");
				escape_html(text, html);
				html.push_str("</a>");
				plain.push_str(text);
				plain.push_str(" (");
				plain.push_str(url);
				plain.push(')');
			} else {
				escape_html(text, html);
				plain.push_str(text);
			}
		} else {
			escape_html(text, html);
			plain.push_str(text);
		}
	}
	if let Some(children) = node["c"].as_array() {
		for child in children {
			render_wiki(child, html, plain, depth + 1)?;
		}
	}
	if let Some(tag) = tag {
		html.push_str("</");
		html.push_str(tag);
		html.push('>');
		if matches!(element, "par" | "li" | "h" | "list") {
			plain.push('\n');
		}
	}
	if element == "br" {
		html.push_str("<br>");
		plain.push('\n');
	}
	Ok(())
}

fn subreddit(node: &Value) -> Result<Value, ClientError> {
	let name = node["name"].as_str().ok_or_else(missing)?;
	let id = node["id"].as_str().ok_or_else(missing)?;
	let description_html = node["description"]["markdown"].as_str().map(|markdown| {
		let mut html = String::new();
		pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new(markdown));
		ammonia::clean(&html)
	});
	Ok(
		json!({"kind":"t5","data":{"id":id.strip_prefix("t5_").unwrap_or(id),"name":id,"display_name":name,"display_name_prefixed":format!("r/{name}"),"title":node["title"].as_str().unwrap_or(name),"public_description":node["publicDescriptionText"].as_str().unwrap_or(""),"description":node["description"]["markdown"],"description_html":description_html,"community_icon":node["styles"]["icon"].as_str().unwrap_or(""),"icon_img":node["styles"]["legacyIcon"]["url"],"subscribers":count(&node["subscribersCount"]),"accounts_active":count(&node["communityStats"]["weeklyActiveUsersCount"]),"wiki_enabled":node["wiki"]["indexPage"].is_object(),"over18":node["isNsfw"] == true,"subreddit_type":node["type"].as_str().unwrap_or("public").to_ascii_lowercase(),"url":format!("/r/{name}/"),"created_utc":timestamp(&node["createdAt"])}}),
	)
}

fn timestamp(value: &Value) -> f64 {
	value
		.as_str()
		.and_then(|s| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z").ok())
		.map(|date| date.timestamp_millis() as f64 / 1000.0)
		.unwrap_or_default()
}

fn post(node: &Value) -> Result<Value, ClientError> {
	let name = node["id"].as_str().and_then(|v| v.strip_prefix("t3_")).ok_or_else(missing)?;
	let sub = &node["subreddit"];
	let subreddit = sub["name"]
		.as_str()
		.map(str::to_owned)
		.or_else(|| {
			(node["__typename"] == "ProfilePost")
				.then(|| node["authorInfo"]["name"].as_str().map(|name| format!("u_{name}")))
				.flatten()
		})
		.ok_or_else(missing)?;
	let url = node["url"].as_str().unwrap_or("");
	let is_self = node["isSelfPost"] == true;
	let mut data = json!({"id":name,"name":node["id"],"title":node["title"].as_str().or(node["postTitle"].as_str()).unwrap_or(""),"subreddit":subreddit,"subreddit_id":sub["id"].as_str().or(node["profile"]["id"].as_str()).unwrap_or(""),"author":node["authorInfo"]["name"].as_str().unwrap_or("[deleted]"),"author_fullname":node["authorInfo"]["id"],"permalink":node["permalink"],"url":url,"selftext":node["content"]["markdown"].as_str().unwrap_or(""),"selftext_html":node["content"]["html"],"created_utc":timestamp(&node["createdAt"]),"score":node["score"].as_i64().unwrap_or_default(),"hide_score":node["isScoreHidden"] == true,"upvote_ratio":node["upvoteRatio"].as_f64().unwrap_or(0.0),"num_comments":node["commentCount"].as_u64().unwrap_or_default(),"is_self":is_self,"over_18":node["isNsfw"] == true,"spoiler":node["isSpoiler"] == true,"stickied":node["isStickied"] == true,"pinned":false,"locked":node["isLocked"] == true,"archived":node["isArchived"] == true,"domain":node["domain"],"link_flair_text":node["flair"]["text"],"thumbnail":node["thumbnail"]["url"],"suggested_sort":node["suggestedCommentSort"].as_str().map(str::to_ascii_lowercase)});
	if let Some(hint) = node["postHint"].as_str() {
		data["post_hint"] = hint.to_ascii_lowercase().replace('_', ":").into();
	}
	let media = &node["media"];
	let source = &media["still"]["source"];
	if source["url"].is_string() {
		data["preview"] = json!({"images":[{"source":{"url":source["url"],"width":source["dimensions"]["width"],"height":source["dimensions"]["height"]}}]});
	}
	if media["streaming"]["hlsUrl"].is_string() || media["download"]["url"].is_string() {
		data["secure_media"] = json!({"reddit_video":{"hls_url":media["streaming"]["hlsUrl"],"fallback_url":media["packagedMedia"]["muxedMp4s"]["recommended"]["url"].as_str().or(media["download"]["url"].as_str()),"is_gif":media["streaming"]["isGif"] == true}});
		data["is_video"] = true.into();
	}
	if let Some(items) = node["gallery"]["items"].as_array() {
		let mut metadata = serde_json::Map::new();
		let mut gallery_items = Vec::new();
		for item in items {
			let image = &item["media"];
			let (Some(id), Some(url)) = (image["id"].as_str(), image["url"].as_str()) else {
				continue;
			};
			gallery_items.push(json!({"media_id":id,"caption":item["caption"]}));
			metadata.insert(id.to_owned(), json!({"m":image["mimetype"],"s":{"u":url,"x":image["width"],"y":image["height"]}}));
		}
		if !gallery_items.is_empty() {
			data["is_gallery"] = true.into();
			data["gallery_data"] = json!({"items":gallery_items});
			data["media_metadata"] = metadata.into();
		}
	}
	if !is_self {
		data["url_overridden_by_dest"] = url.into();
	}
	Ok(json!({"kind":"t3","data":data}))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::parsing::parse_post_listing;

	#[test]
	fn parses_integral_float_community_counts() {
		let node = json!({"name":"AskHistorians","id":"t5_2ssp3","subscribersCount":2930602.0,"communityStats":{"weeklyActiveUsersCount":41920.0}});
		let value = subreddit(&node).unwrap();
		let community = crate::parsing::parse_subreddit(&value).unwrap();
		assert_eq!(community.data.subscribers, Some(2930602));
		assert_eq!(community.data.accounts_active, Some(41920));
		assert_eq!(count(&json!(2.5)), None);
		assert_eq!(count(&json!(-1)), None);
	}

	#[test]
	fn reads_integer_karma_from_graphql_float() {
		assert_eq!(whole_number(&json!(2338471.0)), Some(2338471));
		assert_eq!(whole_number(&json!(1.5)), None);
	}

	#[test]
	fn maps_post_into_existing_listing() {
		let node = json!({"id":"t3_abc123","title":"Example","subreddit":{"id":"t5_rust","name":"rust"},"authorInfo":{"name":"alice"},"permalink":"/r/rust/comments/abc123/example/","url":"https://example.com/","createdAt":"2026-11-10T12:00:00.000+0000","score":4,"commentCount":2,"upvoteRatio":0.9,"content":{"markdown":"hello"},"media":{"still":{"source":{"url":"https://i.redd.it/example.jpg","dimensions":{"width":400,"height":300}}}}});
		let value = listing(vec![post(&node).unwrap()], Some("t3_next".into()));
		let parsed = parse_post_listing(&value).unwrap();
		assert_eq!(parsed.data.after.as_deref(), Some("t3_next"));
		assert_eq!(parsed.data.children[0].data.name, "t3_abc123");
		assert_eq!(parsed.data.children[0].data.extra["preview"]["images"][0]["source"]["width"], 400);
		assert!(parsed.data.children[0].data.created_utc > 1_700_000_000.0);
	}

	#[test]
	fn maps_profile_posts_without_a_community() {
		let node = json!({"__typename":"ProfilePost","id":"t3_abc123","title":"Example","authorInfo":{"name":"alice"},"permalink":"/user/alice/comments/abc123/example/","url":"https://reddit.com/user/alice/comments/abc123/example/"});
		let post = post(&node).unwrap();
		assert_eq!(post["data"]["subreddit"], "u_alice");
		assert!(parse_post_listing(&listing(vec![post], None)).is_ok());
	}

	#[test]
	fn wiki_html_escapes_text_and_rejects_unsafe_links() {
		let mut html = String::new();
		let mut plain = String::new();
		render_wiki(&json!({"e":"par","c":[{"e":"text","t":"<script>alert(1)</script>"},{"e":"link","u":"javascript:alert(1)","t":"bad"},{"e":"link","u":"https://example.com/?a=1&b=2","t":"good"}]}), &mut html, &mut plain, 0).unwrap();
		assert!(!html.contains("<script>"));
		assert!(!html.contains("javascript:"));
		assert!(html.contains("&lt;script&gt;"));
		assert!(html.contains("href=\"https://example.com/?a=1&amp;b=2\""));
	}

	#[test]
	fn sidebar_html_sanitizes_reddit_markdown() {
		let node = json!({"name":"rust","id":"t5_rust","description":{"markdown":"hello <script>alert(1)</script> **world**"}});
		let data = subreddit(&node).unwrap();
		let html = data["data"]["description_html"].as_str().unwrap();
		assert!(!html.contains("<script>"));
		assert!(html.contains("<strong>world</strong>"));
	}

	#[test]
	fn decodes_feed_cursor() {
		assert_eq!(cursor(&json!(STANDARD.encode("t3_abc123"))).as_deref(), Some("t3_abc123"));
	}

	#[test]
	fn refuses_incomplete_post() {
		assert!(post(&json!({"id":"t3_abc123"})).is_err());
	}

	#[test]
	fn maps_gallery_and_video_media() {
		let mut node = json!({"id":"t3_abc123","title":"Example","subreddit":{"id":"t5_rust","name":"rust"},"permalink":"/r/rust/comments/abc123/example/","url":"https://reddit.com/r/rust/comments/abc123/example/","createdAt":"2026-11-10T12:00:00.000+0000","gallery":{"items":[{"media":{"id":"one","url":"https://i.redd.it/a.jpg","mimetype":"image/jpeg","width":400,"height":300},"caption":"Image"}]},"postHint":"HOSTED_VIDEO","media":{"streaming":{"hlsUrl":"https://v.redd.it/a/HLSPlaylist.m3u8"},"packagedMedia":{"muxedMp4s":{"recommended":{"url":"https://v.redd.it/a/muxed.mp4"}}}}});
		let mapped = post(&node).unwrap();
		let data = &mapped["data"];
		assert_eq!(data["post_hint"], "hosted:video");
		assert_eq!(data["gallery_data"]["items"][0]["media_id"], "one");
		assert_eq!(data["secure_media"]["reddit_video"]["fallback_url"], "https://v.redd.it/a/muxed.mp4");
		node["subreddit"] = Value::Null;
		assert!(post(&node).is_err());
	}
}
