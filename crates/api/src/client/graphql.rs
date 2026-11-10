use super::{error::ClientError, RedditClient};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::DateTime;
use serde_json::{json, Value};
use url::form_urlencoded;

const FEED: &str = "97046e2a48050f10a507db354a8da60955ffccb586dcf63163bdfc6610d5cf7a";
const POPULAR: &str = "728c0f9a4d12c17b8cffe07669bb1a8770b0f2ec0c84e040485f8fad2d9c417f";
const ALL: &str = "0e9c0045dd244e934efd0ea4fefc00cb0b4d91523e2f09c8af075856cb59b937";
const BY_IDS: &str = "0e471495bcba33554b6c540b3cf1cae23dd5432a8b741dddc7fd6717dc2520a7";
const SEARCH: &str = "8610e33521c90caa8d12576800c18b708ce0d19df3dfac8e2aad55d9860e4bf9";
const COMMUNITY_SEARCH: &str = "a3333069c1ead3ceec6d9bf4da4185ff584f69d0ba8b586e545b32ea62170932";
const COMMUNITY: &str = "6b9c1679e69097e1c6df364adc11183afe6e2b6545dd1c0c1cc8f7490448c3e5";

// Only these known routes use GraphQL. Other Reddit endpoints keep their own handlers.
pub(super) async fn route(client: &RedditClient, path: &str) -> Option<Result<Value, ClientError>> {
	let (route, query) = path.split_once('?').unwrap_or((path, ""));
	let params: Vec<_> = form_urlencoded::parse(query.as_bytes()).collect();
	let get = |name: &str| params.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_ref());
	if let Some(ids) = route.strip_prefix("/by_id/") {
		return Some(by_ids(client, ids.split(',').map(str::to_owned).collect()).await);
	}
	if route == "/search" && matches!(get("type"), None | Some("link")) {
		return Some(
			search(
				client,
				get("q").unwrap_or(""),
				get("sort").unwrap_or("relevance"),
				get("t").unwrap_or("all"),
				get("after"),
				get("limit"),
				get("include_over_18"),
			)
			.await,
		);
	}
	if let ["r", name, "search"] = route.trim_matches('/').split('/').collect::<Vec<_>>().as_slice() {
		if matches!(get("type"), None | Some("link")) {
			let q = get("q").unwrap_or("");
			let scoped = if get("restrict_sr").is_some_and(|value| value != "false") {
				format!("subreddit:{name} {q}")
			} else {
				q.to_owned()
			};
			return Some(
				search(
					client,
					&scoped,
					get("sort").unwrap_or("relevance"),
					get("t").unwrap_or("all"),
					get("after"),
					get("limit"),
					get("include_over_18"),
				)
				.await,
			);
		}
	}
	if route == "/subreddits/search" {
		return Some(community_search(client, get("q").unwrap_or(""), get("limit")).await);
	}
	let pieces: Vec<_> = route.trim_matches('/').split('/').collect();
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

async fn search(client: &RedditClient, text: &str, sort: &str, range: &str, after: Option<&str>, limit: Option<&str>, nsfw: Option<&str>) -> Result<Value, ClientError> {
	let mut vars = json!({"query":text,"sort":sort.to_ascii_uppercase(),"filters":[{"key":"time_range","value":range},{"key":"nsfw","value":if nsfw == Some("true") || nsfw == Some("on") { "1" } else { "0" }}],"pageSize":limit.and_then(|n| n.parse::<u8>().ok()).unwrap_or(25),"productSurface":"android"});
	if let Some(after) = after {
		vars["afterCursor"] = after.into();
	}
	let response = client.graphql("SearchPosts", SEARCH, vars).await?;
	let (posts, after) = edges(response.pointer("/data/search/general/posts").ok_or_else(missing)?)?;
	Ok(listing(posts, after))
}

async fn community_search(client: &RedditClient, text: &str, limit: Option<&str>) -> Result<Value, ClientError> {
	let response = client
		.graphql(
			"SearchCommunities",
			COMMUNITY_SEARCH,
			json!({"query":text,"pageSize":limit.and_then(|n| n.parse::<u8>().ok()).unwrap_or(25),"productSurface":"android"}),
		)
		.await?;
	let nodes = response.pointer("/data/search/general/communities/edges").and_then(Value::as_array).ok_or_else(missing)?;
	let items = nodes.iter().filter_map(|edge| edge.get("node")).map(subreddit).collect::<Result<_, _>>()?;
	Ok(listing(items, None))
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

fn subreddit(node: &Value) -> Result<Value, ClientError> {
	let name = node["name"].as_str().ok_or_else(missing)?;
	let id = node["id"].as_str().ok_or_else(missing)?;
	Ok(
		json!({"kind":"t5","data":{"id":id.strip_prefix("t5_").unwrap_or(id),"name":id,"display_name":name,"display_name_prefixed":format!("r/{name}"),"title":node["title"].as_str().unwrap_or(name),"public_description":node["publicDescriptionText"].as_str().unwrap_or(""),"description":node["description"]["markdown"],"description_html":node["description"]["html"],"community_icon":node["styles"]["icon"].as_str().unwrap_or(""),"icon_img":node["styles"]["legacyIcon"]["url"],"subscribers":node["subscribersCount"],"accounts_active":node["communityStats"]["weeklyActiveUsersCount"],"wiki_enabled":node["wiki"]["indexPage"]["status"] == "VALID","over18":node["isNsfw"] == true,"subreddit_type":node["type"].as_str().unwrap_or("public").to_ascii_lowercase(),"url":format!("/r/{name}/"),"created_utc":timestamp(&node["createdAt"])}}),
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
	let subreddit = sub["name"].as_str().ok_or_else(missing)?;
	let url = node["url"].as_str().unwrap_or("");
	let is_self = node["isSelfPost"] == true;
	let mut data = json!({"id":name,"name":node["id"],"title":node["title"].as_str().or(node["postTitle"].as_str()).unwrap_or(""),"subreddit":subreddit,"subreddit_id":sub["id"],"author":node["authorInfo"]["name"].as_str().unwrap_or("[deleted]"),"author_fullname":node["authorInfo"]["id"],"permalink":node["permalink"],"url":url,"selftext":node["content"]["markdown"].as_str().unwrap_or(""),"selftext_html":node["content"]["html"],"created_utc":timestamp(&node["createdAt"]),"score":node["score"].as_i64().unwrap_or_default(),"hide_score":node["isScoreHidden"] == true,"upvote_ratio":node["upvoteRatio"].as_f64().unwrap_or(0.0),"num_comments":node["commentCount"].as_u64().unwrap_or_default(),"is_self":is_self,"over_18":node["isNsfw"] == true,"spoiler":node["isSpoiler"] == true,"stickied":node["isStickied"] == true,"pinned":false,"locked":node["isLocked"] == true,"archived":node["isArchived"] == true,"domain":node["domain"],"link_flair_text":node["flair"]["text"],"thumbnail":node["thumbnail"]["url"],"suggested_sort":node["suggestedCommentSort"].as_str().map(str::to_ascii_lowercase)});
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
	fn decodes_feed_cursor_without_mangling_search_cursor() {
		assert_eq!(cursor(&json!(STANDARD.encode("t3_abc123"))).as_deref(), Some("t3_abc123"));
		assert_eq!(cursor(&json!("24")).as_deref(), Some("24"));
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
