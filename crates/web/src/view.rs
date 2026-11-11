mod comments;
mod controls;
mod format;

pub use comments::comment_tree;
use comments::{comment_parent_url, comment_url};
pub use controls::{comment_sort_controls, feed_controls, user_controls};
use format::{age, compact, signed_compact};

use std::borrow::Cow;

use ammonia::{Builder, UrlRelative};
use askama::Template;
use askama_web::WebTemplate;

use neddit_api::media::MediaSigner;
use neddit_api::models::{Post, PublicThing, Subreddit, User, WikiPage, WikiPageListing};
use serde_json::Value;
use url::{form_urlencoded, Url};

use crate::{markdown, ImageDisplay};

#[derive(Clone, Debug)]
pub struct FeedItem {
	pub title: String,
	pub href: String,
	pub domain: String,
	pub show_domain: bool,
	pub flair: Option<PostFlair>,
	pub author: String,
	pub author_url: String,
	pub subreddit: String,
	pub show_subreddit: bool,
	pub permalink: String,
	pub age: String,
	pub score: String,
	pub comments: String,
	pub stickied: bool,
	pub badges: Vec<&'static str>,
	pub video: Option<VideoView>,
}

#[derive(Clone, Debug)]
pub struct PostFlair {
	pub text: String,
}

#[derive(Clone, Debug)]
pub struct VideoView {
	pub poster: Option<String>,
	pub source: String,
	pub hls: bool,
}

#[derive(Clone, Debug)]
pub struct SelectChoice {
	pub value: &'static str,
	pub label: &'static str,
	pub checked: bool,
}

#[derive(Clone, Debug)]
pub struct SortControls {
	pub action: String,
	pub label: &'static str,
	pub choice_label: &'static str,
	pub sorts: Vec<SelectChoice>,
	pub times: Vec<SelectChoice>,
}

#[derive(Template, WebTemplate)]
#[template(path = "feed.html")]
pub struct FeedTemplate {
	pub items: Vec<FeedItem>,
	pub controls: SortControls,
	pub pagination: Pagination,
	pub feed_label: String,
}

#[derive(Clone, Debug)]
pub struct Pagination {
	pub page: u32,
	pub previous_url: String,
	pub has_previous: bool,
	pub next_url: String,
	pub has_next: bool,
}

#[derive(Clone, Debug)]
pub struct ActivityItem {
	pub fullname: String,
	pub metric: String,
	pub metric_label: String,
	pub title: String,
	pub href: String,
	pub domain: Option<String>,
	pub metadata: Vec<ActivityMetadata>,
}

#[derive(Clone, Debug)]
pub struct ActivityMetadata {
	pub text: String,
	pub href: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CommunityView {
	pub display_name_prefixed: String,
	pub title: String,
	pub public_description: Option<String>,
	pub description_html: Option<String>,
	pub members: String,
	pub active: String,
	pub posts_url: String,
	pub wiki_url: String,
	pub has_wiki: bool,
	pub over_18: bool,
}

#[derive(Clone, Debug)]
pub struct UserProfileView {
	pub name: String,
	pub description: Option<String>,
	pub karma: String,
	pub joined: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "user.html")]
pub struct UserTemplate {
	pub profile: UserProfileView,
	pub results: Vec<ActivityItem>,
	pub controls: SortControls,
	pub posts_url: String,
	pub comments_url: String,
	pub posts_active: bool,
	pub comments_active: bool,
	pub pagination: Pagination,
}

#[derive(Template, WebTemplate)]
#[template(path = "subreddit.html")]
pub struct SubredditTemplate {
	pub community: CommunityView,
	pub items: Vec<FeedItem>,
	pub controls: SortControls,
	pub pagination: Pagination,
	pub feed_label: String,
}

#[derive(Clone, Debug)]
pub struct WikiLink {
	pub label: String,
	pub href: String,
	pub active: bool,
}

#[derive(Clone, Debug)]
pub struct WikiView {
	pub title: String,
	pub content_html: String,
	pub revision_age: String,
	pub show_revision: bool,
	pub links: Vec<WikiLink>,
	pub generated: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "wiki.html")]
pub struct WikiTemplate {
	pub community: CommunityView,
	pub wiki: WikiView,
}

#[derive(Clone, Debug)]
pub struct PostView {
	pub item: FeedItem,
	pub gallery: Option<GalleryView>,
	pub image_url: Option<String>,
	pub inline_image: bool,
	pub selftext_html: String,
	pub show_selftext: bool,
	pub hide_content: bool,
	pub content_url: String,
}

#[derive(Clone, Debug)]
pub struct GalleryView {
	pub image: GalleryImageView,
	pub position: usize,
	pub total: usize,
	pub has_previous: bool,
	pub previous_url: String,
	pub has_next: bool,
	pub next_url: String,
}

#[derive(Clone, Debug)]
pub struct GalleryImageView {
	pub url: String,
	pub alt: String,
	pub inline: bool,
	pub caption: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CommentView {
	pub id: String,
	pub author: String,
	pub author_url: String,
	pub body: String,
	pub body_html: String,
	pub score: String,
	pub age: String,
	pub permalink: String,
	pub parent_url: String,
	pub more_url: String,
	pub more: bool,
}

#[derive(Clone, Debug)]
pub enum CommentTreeEvent {
	Open(Box<CommentView>),
	OpenReplies,
	CloseReplies,
	Close,
}

#[derive(Template, WebTemplate)]
#[template(path = "post.html")]
pub struct PostTemplate {
	pub post: PostView,
	pub comment_tree: Vec<CommentTreeEvent>,
	pub controls: SortControls,
	pub comments_heading: String,
	pub empty_message: &'static str,
	pub has_comments: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "partials/gallery.html")]
pub struct GalleryTemplate {
	pub gallery: GalleryView,
}

#[derive(Template, WebTemplate)]
#[template(path = "partials/post_content.html")]
pub struct PostContentTemplate {
	pub post: PostView,
}

#[derive(Template, WebTemplate)]
#[template(path = "more_comments.html")]
pub struct MoreCommentsTemplate {
	pub comment_tree: Vec<CommentTreeEvent>,
}

pub fn feed_item(post: &Post, signer: &MediaSigner) -> FeedItem {
	let permalink = local_web_navigation(&post.permalink, signer).unwrap_or_else(|| post.permalink.clone());
	let href = if post.is_self || is_reddit_video(post) {
		permalink.clone()
	} else {
		signer
			.signed_media_url(&post.url)
			.filter(|_| !post.spoiler)
			.or_else(|| safe_outbound(&post.url, signer))
			.unwrap_or_else(|| permalink.clone())
	};
	let domain = outbound_domain(&href).unwrap_or_default();
	let mut badges = Vec::with_capacity(4);
	if post.stickied {
		badges.push("pinned");
	}
	if post.over_18 {
		badges.push("NSFW");
	}
	if post.spoiler {
		badges.push("spoiler");
	}
	if post.locked {
		badges.push("locked");
	}

	FeedItem {
		title: if post.title.is_empty() { "untitled".into() } else { post.title.clone() },
		href,
		show_domain: !domain.is_empty(),
		domain,
		flair: None,
		author: display_author(&post.author),
		author_url: author_path(&post.author),
		subreddit: post.subreddit.clone(),
		show_subreddit: true,
		permalink,
		age: age(post.created_utc),
		score: if post.hide_score { "—".into() } else { compact(post.score.max(0).unsigned_abs()) },
		comments: compact(post.num_comments),
		stickied: post.stickied,
		badges,
		video: None,
	}
}

pub fn subreddit_feed_item(post: &Post, signer: &MediaSigner, subreddit_is_nsfw: bool) -> FeedItem {
	let mut item = feed_item(post, signer);
	item.show_subreddit = false;
	item.flair = post_flair(post);
	if subreddit_is_nsfw {
		item.badges.retain(|badge| *badge != "NSFW");
	}
	item
}

fn feed_item_with_video(post: &Post, signer: &MediaSigner) -> FeedItem {
	let mut item = feed_item(post, signer);
	let mp4 = reddit_video_source(post).and_then(|source| signer.signed_media_url(source));
	let hls = mp4.is_none();
	let source = mp4.or_else(|| reddit_hls_source(post).and_then(|source| signer.signed_media_url(source)));
	if let Some(source) = source {
		item.video = Some(VideoView {
			poster: preview_url(post).and_then(|url| signer.signed_media_url(url)),
			source,
			hls,
		});
	}
	item
}

#[cfg(test)]
mod video_tests {
	use super::*;
	use neddit_api::media::DomainPolicy;
	use serde_json::json;

	#[test]
	fn reddit_video_plays_inline_but_external_video_links_out() {
		let signer = MediaSigner::from_secret(&[1; 32])
			.unwrap()
			.with_domains(DomainPolicy::new(&[], &[], &["v.redd.it".into()]).unwrap());
		let mut post: Post = serde_json::from_value(json!({
			"id": "abc", "name": "t3_abc", "title": "Video", "subreddit": "test", "subreddit_id": "t5_test",
			"author": "tester", "author_fullname": null, "permalink": "/r/test/comments/abc/video/",
			"url": "https://v.redd.it/abc", "selftext": "", "created_utc": 0.0, "score": 1,
			"hide_score": false, "upvote_ratio": 1.0, "num_comments": 0, "is_self": false,
			"over_18": false, "spoiler": false, "stickied": false, "pinned": false,
			"locked": false, "archived": false,
			"secure_media": {"reddit_video": {"fallback_url": "https://v.redd.it/abc/DASH_720.mp4"}}
		}))
		.unwrap();
		let native = post_view(&post, &signer, ImageDisplay::Inline);
		assert!(native.item.video.as_ref().unwrap().source.starts_with("/media/"));
		assert_eq!(native.item.href, post.permalink);

		post.url = "https://example.com/watch/abc".into();
		post.extra.remove("secure_media");
		post.extra.insert("post_hint".into(), json!("rich:video"));
		let external = post_view(&post, &signer, ImageDisplay::Inline);
		assert!(external.item.video.is_none());
		assert_eq!(external.item.href, post.url);
	}
}

fn reddit_video_source(post: &Post) -> Option<&str> {
	post
		.extra
		.get("secure_media")
		.and_then(|media| media.pointer("/reddit_video/fallback_url"))
		.or_else(|| post.extra.get("media").and_then(|media| media.pointer("/reddit_video/fallback_url")))
		.and_then(Value::as_str)
}

fn reddit_hls_source(post: &Post) -> Option<&str> {
	post
		.extra
		.get("secure_media")
		.and_then(|media| media.pointer("/reddit_video/hls_url"))
		.or_else(|| post.extra.get("media").and_then(|media| media.pointer("/reddit_video/hls_url")))
		.and_then(Value::as_str)
}

fn is_reddit_video(post: &Post) -> bool {
	reddit_video_source(post).is_some()
		|| post.extra.get("is_video").and_then(Value::as_bool) == Some(true)
		|| post.extra.get("post_hint").and_then(Value::as_str) == Some("hosted:video")
}

fn preview_url(post: &Post) -> Option<&str> {
	let image = post.extra.get("preview")?.get("images")?.as_array()?.first()?;
	let preview = if post.spoiler { image.pointer("/variants/obfuscated").unwrap_or(image) } else { image };
	let sources = preview
		.get("resolutions")
		.and_then(|value| value.as_array())
		.into_iter()
		.flatten()
		.chain(preview.get("source"));
	let best = sources
		.filter_map(|source| Some((source.get("width")?.as_u64()?, source.get("url")?.as_str()?)))
		.min_by_key(|(width, _)| width.abs_diff(640))
		.map(|(_, url)| url);
	best.or_else(|| post.extra.get("thumbnail")?.as_str().filter(|url| url.starts_with("https://") || url.starts_with("//")))
}

pub fn activity_result(item: &PublicThing, signer: &MediaSigner) -> Option<ActivityItem> {
	match item {
		PublicThing::Post(thing) => Some(post_result(&thing.data, signer)),
		PublicThing::Comment(thing) => {
			let c = &thing.data;
			let mut metadata = vec![
				optional_link_metadata(format!("by {}", display_author(&c.author)), author_path(&c.author)),
				plain_metadata(age(c.created_utc)),
			];
			if let Some(subreddit) = c.extra.get("subreddit").and_then(|value| value.as_str()) {
				metadata.push(linked_metadata(format!("in r/{subreddit}"), format!("/r/{subreddit}")));
			}
			metadata.push(linked_metadata("parent".into(), comment_parent_url(c)));
			Some(ActivityItem {
				fullname: c.name.clone(),
				metric: c.score.to_string(),
				metric_label: "points".into(),
				title: c.body.chars().take(160).collect(),
				href: comment_url(c),
				domain: None,
				metadata,
			})
		}
		PublicThing::User(_) | PublicThing::Subreddit(_) => None,
	}
}

pub fn post_result(post: &Post, signer: &MediaSigner) -> ActivityItem {
	let item = feed_item(post, signer);
	ActivityItem {
		fullname: post.name.clone(),
		metric: item.score,
		metric_label: "points".into(),
		title: item.title,
		href: item.href,
		domain: item.show_domain.then_some(item.domain),
		metadata: vec![
			optional_link_metadata(format!("by {}", item.author), item.author_url),
			plain_metadata(item.age),
			linked_metadata(format!("{} comments", item.comments), item.permalink),
			linked_metadata(format!("in r/{}", post.subreddit), format!("/r/{}", post.subreddit)),
		],
	}
}

fn linked_metadata(text: String, href: String) -> ActivityMetadata {
	ActivityMetadata { text, href: Some(href) }
}

fn plain_metadata(text: String) -> ActivityMetadata {
	ActivityMetadata { text, href: None }
}

fn optional_link_metadata(text: String, href: String) -> ActivityMetadata {
	if href.is_empty() {
		plain_metadata(text)
	} else {
		linked_metadata(text, href)
	}
}

fn author_path(author: &str) -> String {
	if author.is_empty() || author == "[deleted]" {
		return String::new();
	}
	let mut url = Url::parse("http://neddit.local").expect("static base URL is valid");
	url.path_segments_mut().expect("HTTP URLs support path segments").extend(["user", author]);
	url.path().to_owned()
}

pub fn user_profile(user: &User) -> UserProfileView {
	let karma = user.total_karma.unwrap_or_else(|| user.comment_karma.saturating_add(user.link_karma));
	UserProfileView {
		name: user.name.clone(),
		description: user
			.subreddit
			.as_ref()
			.and_then(|subreddit| subreddit.get("public_description"))
			.and_then(|description| description.as_str())
			.map(str::trim)
			.and_then(nonempty),
		karma: format!("{} karma", signed_compact(karma)),
		joined: format!("Joined {}", age(user.created_utc)),
	}
}

pub fn community_view(subreddit: &Subreddit, signer: &MediaSigner, has_wiki: bool) -> CommunityView {
	let title = if subreddit.title.trim().is_empty() {
		subreddit.display_name_prefixed.clone()
	} else {
		subreddit.title.trim().to_owned()
	};
	let public_description = nonempty(subreddit.public_description.trim());
	let description_html = subreddit
		.description_html
		.as_deref()
		.map(|html| sanitize_html(html, signer))
		.and_then(|html| nonempty(&html));
	let posts_url = format!("/r/{}", subreddit.display_name);
	CommunityView {
		display_name_prefixed: subreddit.display_name_prefixed.clone(),
		title,
		public_description,
		description_html,
		members: subreddit.subscribers.map_or_else(|| "—".into(), compact),
		active: subreddit.accounts_active.map_or_else(|| "—".into(), compact),
		wiki_url: if has_wiki { wiki_path(&subreddit.display_name, "index") } else { String::new() },
		posts_url,
		has_wiki,
		over_18: subreddit.over18 == Some(true),
	}
}

pub fn has_public_wiki_pages(pages: &WikiPageListing) -> bool {
	pages.data.iter().any(|page| is_public_wiki_page(page))
}

pub fn has_wiki_page(pages: &WikiPageListing, page: &str) -> bool {
	pages.data.iter().any(|candidate| candidate == page && is_public_wiki_page(candidate))
}

fn is_public_wiki_page(page: &str) -> bool {
	!page.starts_with("config/")
}

pub fn wiki_links(subreddit: &str, active_page: Option<&str>, pages: &WikiPageListing) -> Vec<WikiLink> {
	let mut links: Vec<_> = pages
		.data
		.iter()
		.filter(|page| is_public_wiki_page(page))
		.map(|page| WikiLink {
			label: if page == "index" { "Home".into() } else { page.replace('_', " ") },
			href: wiki_path(subreddit, page),
			active: active_page == Some(page),
		})
		.collect();
	links.sort_by_key(|link| (link.label != "Home", link.label.clone()));
	links
}

pub fn wiki_view(subreddit: &str, page: &str, wiki: &WikiPage, pages: &WikiPageListing, signer: &MediaSigner, image_display: ImageDisplay) -> WikiView {
	let title = if page == "index" { "Wiki".into() } else { page.replace(['_', '-'], " ") };
	let content_html = if wiki.data.extra.get("gql_rendered") == Some(&Value::Bool(true)) {
		sanitize_html(&wiki.data.content_html, signer)
	} else {
		markdown::render(&wiki.data.content_md, wiki.data.extra.get("media_metadata"), signer, image_display)
	};
	WikiView {
		title,
		content_html,
		revision_age: wiki.data.revision_date.map_or_else(String::new, age),
		show_revision: wiki.data.revision_date.is_some(),
		links: wiki_links(subreddit, Some(page), pages),
		generated: false,
	}
}

pub fn generated_wiki_index(subreddit: &str, pages: &WikiPageListing) -> WikiView {
	WikiView {
		title: "Wiki".into(),
		content_html: String::new(),
		revision_age: String::new(),
		show_revision: false,
		links: wiki_links(subreddit, None, pages),
		generated: true,
	}
}

pub(crate) fn sanitize_html(value: &str, signer: &MediaSigner) -> String {
	let signer = signer.clone();
	let mut builder = Builder::new();
	for heading in ["h1", "h2", "h3", "h4", "h5", "h6"] {
		builder.add_tag_attributes(heading, &["id"]);
	}
	builder
		.add_tag_attributes("span", &["tabindex"])
		.add_allowed_classes("span", &["md-spoiler-text"])
		.url_relative(UrlRelative::PassThrough)
		.attribute_filter(move |element, attribute, value| {
			if element == "a" && attribute == "href" {
				let rewritten = signer.rewrite_text(value);
				return Some(if rewritten == value { Cow::Borrowed(value) } else { Cow::Owned(rewritten) });
			}
			if attribute == "id" && !value.starts_with("wiki_") {
				return None;
			}
			Some(Cow::Borrowed(value))
		})
		.clean(value)
		.to_string()
}

fn nonempty(value: &str) -> Option<String> {
	(!value.is_empty()).then(|| value.to_owned())
}

pub fn post_view(post: &Post, signer: &MediaSigner, image_display: ImageDisplay) -> PostView {
	let mut item = feed_item_with_video(post, signer);
	item.flair = post_flair(post);
	let gallery = gallery_view(post, signer, image_display, 0);
	let image_url = if gallery.is_none() && item.video.is_none() && is_image_post(post) {
		signer.signed_media_url(&post.url)
	} else {
		None
	};
	let selftext = post.selftext.trim();
	PostView {
		show_selftext: !selftext.is_empty(),
		item,
		gallery,
		image_url,
		inline_image: image_display == ImageDisplay::Inline,
		selftext_html: markdown::render(selftext, post.extra.get("media_metadata"), signer, image_display),
		hide_content: post.spoiler,
		content_url: format!("/post-content/{}", post.id),
	}
}

fn post_flair(post: &Post) -> Option<PostFlair> {
	let text = post.extra.get("link_flair_text")?.as_str()?.trim();
	if text.is_empty() {
		return None;
	}
	Some(PostFlair { text: text.to_owned() })
}

pub fn gallery_view(post: &Post, signer: &MediaSigner, image_display: ImageDisplay, index: usize) -> Option<GalleryView> {
	let metadata = post.extra.get("media_metadata")?.as_object()?;
	let images: Vec<_> = post
		.extra
		.get("gallery_data")?
		.get("items")?
		.as_array()?
		.iter()
		.filter_map(|item| {
			let media_id = item.get("media_id")?.as_str()?;
			let media = metadata.get(media_id)?;
			let source = media.get("s")?;
			let upstream = source
				.get("gif")
				.or_else(|| source.get("u"))
				.and_then(|value| value.as_str())
				.or_else(|| media.get("p")?.as_array()?.last()?.get("u")?.as_str())?;
			let url = signer.signed_media_url(upstream)?;
			let caption = item
				.get("caption")
				.and_then(|value| value.as_str())
				.map(str::trim)
				.filter(|caption| !caption.is_empty())
				.map(str::to_owned);
			Some((url, caption))
		})
		.collect();
	let total = images.len();
	let (url, caption) = images.get(index)?.clone();
	let position = index + 1;
	Some(GalleryView {
		image: GalleryImageView {
			alt: caption.clone().unwrap_or_else(|| format!("Gallery image {position} of {total}")),
			url,
			inline: image_display == ImageDisplay::Inline,
			caption,
		},
		position,
		total,
		has_previous: index > 0,
		previous_url: format!("/gallery/{}/{}", post.id, index.saturating_sub(1)),
		has_next: position < total,
		next_url: format!("/gallery/{}/{position}", post.id),
	})
}

fn is_image_post(post: &Post) -> bool {
	if post.extra.get("post_hint").and_then(|value| value.as_str()) == Some("image") {
		return true;
	}
	let decoded = if post.url.starts_with("//") { format!("https:{}", post.url) } else { post.url.clone() };
	let Ok(url) = Url::parse(&decoded) else {
		return false;
	};
	matches!(
		url.path().rsplit('.').next().map(str::to_ascii_lowercase).as_deref(),
		Some("avif" | "gif" | "jpeg" | "jpg" | "png" | "webp")
	)
}

pub fn pagination(page: u32, previous_url: String, next_url: String) -> Pagination {
	Pagination {
		page,
		has_previous: !previous_url.is_empty(),
		previous_url,
		has_next: !next_url.is_empty(),
		next_url,
	}
}

pub(crate) const MAX_HISTORY_BYTES: usize = 2048;

pub fn feed_pagination(base: &str, sort: &str, time: &str, history: &[String], current_after: Option<&str>, next_after: Option<&str>) -> Pagination {
	let previous_url = history.last().map_or_else(String::new, |cursor| {
		listing_page_url(base, sort, time, (!cursor.is_empty()).then_some(cursor), &history[..history.len() - 1])
	});
	let mut next_history = history.to_vec();
	next_history.push(current_after.unwrap_or_default().to_owned());
	let next_url = next_after
		.and_then(|cursor| {
			let json = serde_json::to_string(&next_history).ok()?;
			(json.len() <= MAX_HISTORY_BYTES).then(|| listing_page_url(base, sort, time, Some(cursor), &next_history))
		})
		.unwrap_or_default();
	pagination(history.len() as u32 + 1, previous_url, next_url)
}

fn listing_page_url(base: &str, sort: &str, time: &str, cursor: Option<&str>, history: &[String]) -> String {
	let mut query = form_urlencoded::Serializer::new(String::new());
	if sort != "hot" {
		query.append_pair("sort", sort);
	}
	if sort == "top" {
		query.append_pair("t", time);
	}
	if let Some(cursor) = cursor {
		query.append_pair("after", cursor);
	}
	if !history.is_empty() {
		query.append_pair("history", &serde_json::to_string(history).expect("cursor history is serializable"));
	}
	let encoded = query.finish();
	if encoded.is_empty() {
		base.to_owned()
	} else {
		format!("{base}?{encoded}")
	}
}

pub(crate) fn wiki_path(subreddit: &str, page: &str) -> String {
	let mut url = Url::parse("http://neddit.local").expect("static base URL is valid");
	{
		let mut segments = url.path_segments_mut().expect("HTTP URLs support path segments");
		segments.extend(["r", subreddit, "wiki"]);
		segments.extend(page.split('/'));
	}
	url.path().to_owned()
}

fn safe_outbound(value: &str, signer: &MediaSigner) -> Option<String> {
	let url = Url::parse(value).ok()?;
	if !matches!(url.scheme(), "http" | "https") {
		return None;
	}
	Some(local_web_navigation(url.as_str(), signer).unwrap_or_else(|| url.to_string()))
}

fn local_web_navigation(value: &str, signer: &MediaSigner) -> Option<String> {
	let local = signer.rewrite_navigation(value)?;
	let base = Url::parse("http://neddit.local").expect("static base URL is valid");
	let url = base.join(&local).ok()?;
	let segments: Vec<_> = url.path_segments()?.filter(|segment| !segment.is_empty()).collect();

	let supported = matches!(segments.as_slice(), [] | ["comments", _, ..] | ["r" | "user", _] | ["r", _, "comments" | "wiki", ..]);
	if !supported {
		return None;
	}

	let path = url.path();
	let mut local = if matches!(segments.as_slice(), ["r", _, "wiki", _, ..]) {
		path.trim_end_matches('/').to_owned()
	} else {
		path.to_owned()
	};
	if let Some(query) = url.query() {
		local.push('?');
		local.push_str(query);
	}
	if let Some(fragment) = url.fragment() {
		local.push('#');
		local.push_str(fragment);
	}
	Some(local)
}

fn outbound_domain(value: &str) -> Option<String> {
	let url = Url::parse(value).ok()?;
	let host = url.host_str()?;
	Some(host.strip_prefix("www.").unwrap_or(host).to_owned())
}

fn display_author(author: &str) -> String {
	if author.is_empty() {
		"[deleted]".into()
	} else {
		author.into()
	}
}

#[cfg(test)]
mod sidebar_tests {
	use super::*;

	#[test]
	fn community_uses_sanitized_sidebar_html_without_rerendering_markdown() {
		let subreddit: Subreddit = serde_json::from_value(serde_json::json!({
			"id":"2s7lj","name":"t5_2s7lj","display_name":"rust","display_name_prefixed":"r/rust",
			"title":"Rust","public_description":"Example","description":"different **Markdown**",
			"description_html":"<p><strong>Sidebar</strong></p><script>alert(1)</script>",
			"community_icon":"","subreddit_type":"public","url":"/r/rust/","created_utc":1.0
		}))
		.unwrap();
		let signer = MediaSigner::random().unwrap();
		let view = community_view(&subreddit, &signer, false);
		let html = view.description_html.unwrap();
		assert!(html.contains("<strong>Sidebar</strong>"));
		assert!(!html.contains("<script>"));
		assert!(!html.contains("Markdown"));
	}
}
