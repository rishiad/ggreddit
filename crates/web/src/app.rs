use axum::{
	extract::{Path, Query, State},
	response::{IntoResponse, Redirect, Response},
};
use neddit_api::{
	media::MediaSigner,
	models::{Post, PostComments},
	service::{CommentQuery, CommentSort, ListingQuery, ListingTime, PostSort, RedditService, ServiceError, UserHistoryQuery, UserHistorySort, WikiPageQuery},
};
use serde::Deserialize;

use crate::{
	error::AppError,
	markdown,
	view::{
		activity_result, comment_sort_controls, comment_tree, community_view, feed_controls, feed_item, feed_pagination, gallery_view, generated_wiki_index,
		has_public_wiki_pages, has_wiki_page, post_result, post_view, subreddit_feed_item, user_controls, user_profile, wiki_path, wiki_view, ActivityItem, FeedTemplate,
		GalleryTemplate, MoreCommentsTemplate, PostContentTemplate, PostTemplate, SubredditTemplate, UserTemplate, WikiTemplate, MAX_HISTORY_BYTES, MAX_HISTORY_PAGES,
	},
	ImageDisplay,
};

const PAGE_SIZE: u8 = 25;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedQuery {
	sort: Option<String>,
	t: Option<String>,
	after: Option<String>,
	history: Option<String>,
}

impl FeedQuery {
	fn listing(&self, time: Option<ListingTime>) -> Result<(ListingQuery, Vec<String>), AppError> {
		let history = match &self.history {
			Some(raw) if raw.len() <= MAX_HISTORY_BYTES => serde_json::from_str::<Vec<String>>(raw).map_err(|_| AppError::InvalidPagination)?,
			Some(_) => return Err(AppError::InvalidPagination),
			None => Vec::new(),
		};
		if history.len() > MAX_HISTORY_PAGES
			|| history.first().is_some_and(|cursor| !cursor.is_empty())
			|| history.iter().skip(1).any(String::is_empty)
			|| self.after.as_deref().is_some_and(str::is_empty)
			|| (self.after.is_some() != !history.is_empty())
		{
			return Err(AppError::InvalidPagination);
		}
		Ok((
			ListingQuery {
				after: self.after.clone(),
				limit: Some(PAGE_SIZE),
				time,
				..ListingQuery::default()
			},
			history,
		))
	}
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PostQuery {
	sort: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PostPath {
	article: String,
	#[serde(default)]
	subreddit: Option<String>,
	#[serde(default)]
	comment: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MoreCommentsQuery {
	cursor: String,
	link_id: String,
	sort: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct WikiQuery {
	v: Option<String>,
	v2: Option<String>,
}

pub async fn front_page(State(service): State<RedditService>, State(signer): State<MediaSigner>, Query(query): Query<FeedQuery>) -> Result<FeedTemplate, AppError> {
	let (sort, sort_name) = parse_post_sort(query.sort.as_deref(), false)?;
	let (time, time_name) = parse_time(sort == PostSort::Top, query.t.as_deref())?;
	let (listing_query, history) = query.listing(time)?;
	let listing = service.all_posts(sort, &listing_query).await?;
	let pagination = feed_pagination("/", sort_name, time_name, &history, query.after.as_deref(), listing.data.after.as_deref());
	let items = listing.data.children.iter().map(|thing| feed_item(&thing.data, &signer)).collect();

	Ok(FeedTemplate {
		items,
		controls: feed_controls("/", sort_name, time_name, false),
		pagination,
		feed_label: "Front page posts".into(),
	})
}

pub async fn subreddit_feed(
	State(service): State<RedditService>,
	State(signer): State<MediaSigner>,
	Path(subreddit): Path<String>,
	Query(query): Query<FeedQuery>,
) -> Result<SubredditTemplate, AppError> {
	let (sort, sort_name) = parse_post_sort(query.sort.as_deref(), true)?;
	let (time, time_name) = parse_time(sort == PostSort::Top, query.t.as_deref())?;
	let (listing_query, history) = query.listing(time)?;
	let (listing, community) = tokio::try_join!(service.subreddit_posts(&subreddit, sort, &listing_query), service.subreddit_about(&subreddit),)?;
	let pagination = feed_pagination(
		&format!("/r/{subreddit}"),
		sort_name,
		time_name,
		&history,
		query.after.as_deref(),
		listing.data.after.as_deref(),
	);
	let subreddit_is_nsfw = community.data.over18 == Some(true);
	let items = listing
		.data
		.children
		.iter()
		.map(|thing| subreddit_feed_item(&thing.data, &signer, subreddit_is_nsfw))
		.collect();

	let has_wiki = if community.data.wiki_enabled == Some(true) {
		service.wiki_pages(&subreddit).await.is_ok_and(|pages| has_public_wiki_pages(&pages))
	} else {
		false
	};

	Ok(SubredditTemplate {
		community: community_view(&community.data, &signer, has_wiki),
		items,
		controls: feed_controls(&format!("/r/{subreddit}"), sort_name, time_name, true),
		pagination,
		feed_label: format!("r/{subreddit} posts"),
	})
}

#[derive(Clone, Copy)]
enum UserSection {
	Posts,
	Comments,
}

struct UserActivityPage {
	after: Option<String>,
	results: Vec<ActivityItem>,
}

pub async fn user_posts(state: State<RedditService>, signer: State<MediaSigner>, path: Path<String>, query: Query<FeedQuery>) -> Result<UserTemplate, AppError> {
	user_page(state, signer, path, query, UserSection::Posts).await
}

pub async fn user_comments(state: State<RedditService>, signer: State<MediaSigner>, path: Path<String>, query: Query<FeedQuery>) -> Result<UserTemplate, AppError> {
	user_page(state, signer, path, query, UserSection::Comments).await
}

async fn user_page(
	State(service): State<RedditService>,
	State(signer): State<MediaSigner>,
	Path(username): Path<String>,
	Query(query): Query<FeedQuery>,
	section: UserSection,
) -> Result<UserTemplate, AppError> {
	let (sort, sort_name) = parse_user_sort(query.sort.as_deref())?;
	let (time, time_name) = parse_time(sort == UserHistorySort::Top, query.t.as_deref())?;
	let (listing, history) = query.listing(time)?;
	let history_query = UserHistoryQuery {
		listing,
		sort: Some(sort),
		..UserHistoryQuery::default()
	};
	let (activity, user) = tokio::try_join!(user_activity(&service, &signer, &username, &history_query, section), service.user_about(&username),)?;
	let posts_url = format!("/user/{username}");
	let comments_url = format!("/user/{username}/comments");
	let (base, posts_active, comments_active) = match section {
		UserSection::Posts => (&posts_url, true, false),
		UserSection::Comments => (&comments_url, false, true),
	};
	let pagination = feed_pagination(base, sort_name, time_name, &history, query.after.as_deref(), activity.after.as_deref());

	Ok(UserTemplate {
		profile: user_profile(&user.data),
		results: activity.results,
		controls: user_controls(base, sort_name, time_name),
		posts_url,
		comments_url,
		posts_active,
		comments_active,
		pagination,
	})
}

async fn user_activity(
	service: &RedditService,
	signer: &MediaSigner,
	username: &str,
	query: &UserHistoryQuery,
	section: UserSection,
) -> Result<UserActivityPage, ServiceError> {
	match section {
		UserSection::Posts => {
			let listing = service.user_submitted(username, query).await?;
			let results = listing.data.children.iter().map(|thing| post_result(&thing.data, signer)).collect();
			Ok(UserActivityPage {
				after: listing.data.after,
				results,
			})
		}
		UserSection::Comments => {
			let listing = service.user_comments(username, query).await?;
			let results = listing.data.children.iter().filter_map(|item| activity_result(item, signer)).collect();
			Ok(UserActivityPage {
				after: listing.data.after,
				results,
			})
		}
	}
}

pub async fn wiki_root(
	State(service): State<RedditService>,
	State(signer): State<MediaSigner>,
	State(image_display): State<ImageDisplay>,
	Path(subreddit): Path<String>,
) -> Result<Response, AppError> {
	wiki_response(service, signer, image_display, subreddit, None, WikiQuery::default()).await
}

pub async fn wiki_page(
	State(service): State<RedditService>,
	State(signer): State<MediaSigner>,
	State(image_display): State<ImageDisplay>,
	Path((subreddit, page)): Path<(String, String)>,
	Query(query): Query<WikiQuery>,
) -> Result<Response, AppError> {
	wiki_response(service, signer, image_display, subreddit, Some(page), query).await
}

async fn wiki_response(
	service: RedditService,
	signer: MediaSigner,
	image_display: ImageDisplay,
	subreddit: String,
	requested_page: Option<String>,
	query: WikiQuery,
) -> Result<Response, AppError> {
	let query = WikiPageQuery { v: query.v, v2: query.v2 };
	let (community, pages) = tokio::try_join!(service.subreddit_about(&subreddit), service.wiki_pages(&subreddit),)?;
	let has_wiki = has_public_wiki_pages(&pages);
	let community = community_view(&community.data, &signer, has_wiki);
	let requested_page = requested_page.map(|page| page.trim_end_matches('/').to_owned());

	let Some(page) = requested_page else {
		return Ok(if has_wiki {
			Redirect::temporary(&wiki_path(&subreddit, "index")).into_response()
		} else {
			crate::error::response(axum::http::StatusCode::NOT_FOUND)
		});
	};
	if page == "index" && has_wiki && !has_wiki_page(&pages, "index") {
		return Ok(
			WikiTemplate {
				community,
				wiki: generated_wiki_index(&subreddit, &pages),
			}
			.into_response(),
		);
	}

	if !has_wiki_page(&pages, &page) {
		return Ok(crate::error::response(axum::http::StatusCode::NOT_FOUND));
	}

	let wiki = service.wiki_page(&subreddit, &page, &query).await?;

	Ok(
		WikiTemplate {
			community,
			wiki: wiki_view(&subreddit, &page, &wiki, &pages, &signer, image_display),
		}
		.into_response(),
	)
}

pub async fn post_comments(
	State(service): State<RedditService>,
	State(signer): State<MediaSigner>,
	State(image_display): State<ImageDisplay>,
	Path(path): Path<PostPath>,
	Query(query): Query<PostQuery>,
) -> Result<PostTemplate, AppError> {
	post_page(service, signer, image_display, path.subreddit, path.article, path.comment, query).await
}

pub async fn more_comments(
	State(service): State<RedditService>,
	State(signer): State<MediaSigner>,
	State(image_display): State<ImageDisplay>,
	Query(query): Query<MoreCommentsQuery>,
) -> Result<MoreCommentsTemplate, AppError> {
	let (sort, sort_name) = parse_comment_sort(query.sort.as_deref())?;
	let article = query.link_id.strip_prefix("t3_").ok_or(AppError::PostNotFound)?;
	let listing = service.graphql_more_comments(article, &query.cursor, sort).await?;
	Ok(MoreCommentsTemplate {
		comment_tree: comment_tree(&listing.data.children, &query.link_id, sort_name, markdown::Renderer::new(&signer, image_display)),
	})
}

pub async fn gallery(
	State(service): State<RedditService>,
	State(signer): State<MediaSigner>,
	State(image_display): State<ImageDisplay>,
	Path((article, index)): Path<(String, usize)>,
) -> Result<GalleryTemplate, AppError> {
	let post = post_by_id(&service, &article).await?;
	Ok(GalleryTemplate {
		gallery: gallery_view(&post, &signer, image_display, index).ok_or(AppError::PostNotFound)?,
	})
}

pub async fn post_content(
	State(service): State<RedditService>,
	State(signer): State<MediaSigner>,
	State(image_display): State<ImageDisplay>,
	Path(article): Path<String>,
) -> Result<PostContentTemplate, AppError> {
	let post = post_by_id(&service, &article).await?;
	let mut post = post_view(&post, &signer, image_display);
	post.hide_content = false;
	Ok(PostContentTemplate { post })
}

async fn post_by_id(service: &RedditService, id: &str) -> Result<Post, AppError> {
	service
		.posts_by_id(&format!("t3_{id}"))
		.await?
		.data
		.children
		.into_iter()
		.find(|thing| thing.data.id == id)
		.map(|thing| thing.data)
		.ok_or(AppError::PostNotFound)
}

async fn post_page(
	service: RedditService,
	signer: MediaSigner,
	image_display: ImageDisplay,
	subreddit: Option<String>,
	article: String,
	comment: Option<String>,
	query: PostQuery,
) -> Result<PostTemplate, AppError> {
	let renderer = markdown::Renderer::new(&signer, image_display);
	let (sort, sort_name) = parse_comment_sort(query.sort.as_deref())?;
	let comment_query = CommentQuery {
		comment,
		sort: Some(sort),
		..CommentQuery::default()
	};
	let PostComments(posts, comments) = match subreddit {
		Some(subreddit) => service.subreddit_post_comments(&subreddit, &article, &comment_query).await?,
		None => service.post_comments(&article, &comment_query).await?,
	};
	let post = posts.data.children.into_iter().next().ok_or(AppError::PostNotFound)?.data;
	let link_id = post.name.clone();
	let comment_tree = comment_tree(&comments.data.children, &link_id, sort_name, renderer);
	let comment_count = post.num_comments;
	let post = post_view(&post, &signer, image_display);

	Ok(PostTemplate {
		controls: comment_sort_controls(sort_name, &post.item.permalink),
		comments_heading: format!("{} comment{}", post.item.comments, if comment_count == 1 { "" } else { "s" }),
		empty_message: "No comments yet.",
		has_comments: !comment_tree.is_empty(),
		post,
		comment_tree,
	})
}

fn parse_post_sort(value: Option<&str>, allow_controversial: bool) -> Result<(PostSort, &'static str), AppError> {
	let sort: PostSort = value.unwrap_or("hot").parse().map_err(|()| AppError::InvalidSort)?;
	if sort == PostSort::Best || sort == PostSort::Controversial && !allow_controversial {
		return Err(AppError::InvalidSort);
	}
	Ok((sort, sort.as_str()))
}

fn parse_time(top_sort: bool, value: Option<&str>) -> Result<(Option<ListingTime>, &'static str), AppError> {
	if !top_sort {
		return if value.is_none() { Ok((None, "day")) } else { Err(AppError::InvalidFeedTime) };
	}

	let time: ListingTime = value.unwrap_or("day").parse().map_err(|()| AppError::InvalidFeedTime)?;
	Ok((Some(time), time.as_str()))
}

fn parse_comment_sort(value: Option<&str>) -> Result<(CommentSort, &'static str), AppError> {
	let sort: CommentSort = value.unwrap_or("best").parse().map_err(|()| AppError::InvalidCommentSort)?;
	if matches!(sort, CommentSort::Random | CommentSort::Qa | CommentSort::Live) {
		return Err(AppError::InvalidCommentSort);
	}
	let name = if sort == CommentSort::Confidence { "best" } else { sort.as_str() };
	Ok((sort, name))
}

fn parse_user_sort(value: Option<&str>) -> Result<(UserHistorySort, &'static str), AppError> {
	let sort: UserHistorySort = value.unwrap_or("new").parse().map_err(|()| AppError::InvalidSort)?;
	Ok((sort, sort.as_str()))
}
