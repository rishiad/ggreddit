use crate::api::query::{ApiQuery, DefaultQuery};
use crate::api::response::respond;
use crate::service::{CommentQuery, ListingQuery, PostSort, RedditService, UserHistoryQuery, WikiPageQuery};
use axum::{
	extract::{Path, State},
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::{get, MethodRouter},
	Router,
};

fn front_page(sort: PostSort) -> MethodRouter<RedditService> {
	get(move |State(service): State<RedditService>, ApiQuery(query): ApiQuery<ListingQuery>| async move { respond(service.all_posts(sort, &query).await) })
}

fn subreddit(sort: PostSort) -> MethodRouter<RedditService> {
	get(
		move |State(service): State<RedditService>, Path(subreddit): Path<String>, ApiQuery(query): ApiQuery<ListingQuery>| async move {
			respond(service.subreddit_posts(&subreddit, sort, &query).await)
		},
	)
}

pub(super) fn router() -> Router<RedditService> {
	Router::new()
		.route("/", front_page(PostSort::Hot))
		.route("/hot", front_page(PostSort::Hot))
		.route("/new", front_page(PostSort::New))
		.route("/rising", front_page(PostSort::Rising))
		.route("/top", front_page(PostSort::Top))
		.route("/r/{subreddit}/hot", subreddit(PostSort::Hot))
		.route("/r/{subreddit}/new", subreddit(PostSort::New))
		.route("/r/{subreddit}/rising", subreddit(PostSort::Rising))
		.route("/r/{subreddit}/top", subreddit(PostSort::Top))
		.route("/r/{subreddit}/controversial", subreddit(PostSort::Controversial))
		.route("/r/{subreddit}", subreddit(PostSort::Hot))
		.route("/comments/{article}", get(post_comments))
		.route("/r/{subreddit}/comments/{article}", get(subreddit_post_comments))
		.route("/comments/{article}/{slug}", get(post_permalink))
		.route("/comments/{article}/{slug}/{comment}", get(post_comment_permalink))
		.route("/r/{subreddit}/comments/{article}/{slug}", get(subreddit_post_permalink))
		.route("/r/{subreddit}/comments/{article}/{slug}/{comment}", get(subreddit_post_comment_permalink))
		.route("/r/{subreddit}/about", get(subreddit_about))
		.route("/r/{subreddit}/wiki/pages", get(wiki_pages))
		.route("/r/{subreddit}/wiki", get(wiki_index))
		.route("/r/{subreddit}/wiki/{*page}", get(wiki_page))
		.route("/user/{username}/about", get(user_about))
		.route("/by_id/{names}", get(posts_by_id))
		.route("/user/{username}/submitted", get(user_submitted))
		.route("/user/{username}/comments", get(user_comments))
}

async fn post_comments(State(service): State<RedditService>, Path(article): Path<String>, ApiQuery(query): ApiQuery<CommentQuery>) -> Response {
	comments(service, None, article, None, query).await
}

async fn subreddit_post_comments(
	State(service): State<RedditService>,
	Path((subreddit, article)): Path<(String, String)>,
	ApiQuery(query): ApiQuery<CommentQuery>,
) -> Response {
	comments(service, Some(subreddit), article, None, query).await
}

async fn post_permalink(State(service): State<RedditService>, Path((article, _slug)): Path<(String, String)>, ApiQuery(query): ApiQuery<CommentQuery>) -> Response {
	comments(service, None, article, None, query).await
}

async fn post_comment_permalink(
	State(service): State<RedditService>,
	Path((article, _slug, comment)): Path<(String, String, String)>,
	ApiQuery(query): ApiQuery<CommentQuery>,
) -> Response {
	comments(service, None, article, Some(comment), query).await
}

async fn subreddit_post_permalink(
	State(service): State<RedditService>,
	Path((subreddit, article, _slug)): Path<(String, String, String)>,
	ApiQuery(query): ApiQuery<CommentQuery>,
) -> Response {
	comments(service, Some(subreddit), article, None, query).await
}

async fn subreddit_post_comment_permalink(
	State(service): State<RedditService>,
	Path((subreddit, article, _slug, comment)): Path<(String, String, String, String)>,
	ApiQuery(query): ApiQuery<CommentQuery>,
) -> Response {
	comments(service, Some(subreddit), article, Some(comment), query).await
}

async fn comments(service: RedditService, subreddit: Option<String>, article: String, comment: Option<String>, mut query: CommentQuery) -> Response {
	if let Some(comment) = comment {
		query.comment = Some(comment);
	}
	let result = match subreddit {
		Some(subreddit) => service.subreddit_post_comments(&subreddit, &article, &query).await,
		None => service.post_comments(&article, &query).await,
	};
	respond(result)
}

async fn subreddit_about(State(service): State<RedditService>, Path(subreddit): Path<String>) -> Response {
	respond(service.subreddit_about(&subreddit).await)
}

async fn wiki_pages(State(service): State<RedditService>, Path(subreddit): Path<String>) -> Response {
	respond(service.wiki_pages(&subreddit).await)
}

async fn wiki_page(State(service): State<RedditService>, Path((subreddit, page)): Path<(String, String)>, DefaultQuery(query): DefaultQuery<WikiPageQuery>) -> Response {
	if page == "revisions" || page.starts_with("revisions/") || page == "discussions" || page.starts_with("discussions/") {
		return StatusCode::NOT_FOUND.into_response();
	}
	respond(service.wiki_page(&subreddit, &page, &query).await)
}

async fn wiki_index(State(service): State<RedditService>, Path(subreddit): Path<String>, DefaultQuery(query): DefaultQuery<WikiPageQuery>) -> Response {
	respond(service.wiki_page(&subreddit, "index", &query).await)
}

async fn user_about(State(service): State<RedditService>, Path(username): Path<String>) -> Response {
	respond(service.user_about(&username).await)
}

async fn posts_by_id(State(service): State<RedditService>, Path(names): Path<String>) -> Response {
	respond(service.posts_by_id(&names).await)
}

async fn user_submitted(State(service): State<RedditService>, Path(username): Path<String>, ApiQuery(query): ApiQuery<UserHistoryQuery>) -> Response {
	respond(service.user_submitted(&username, &query).await)
}

async fn user_comments(State(service): State<RedditService>, Path(username): Path<String>, ApiQuery(query): ApiQuery<UserHistoryQuery>) -> Response {
	respond(service.user_comments(&username, &query).await)
}
