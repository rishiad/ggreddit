#![forbid(unsafe_code)]

mod app;
mod error;
mod markdown;
mod view;

pub use error::html_error_pages;

use axum::{
	extract::FromRef,
	http::{header, HeaderValue, StatusCode},
	middleware::{self, Next},
	response::{IntoResponse, Response},
	routing::get,
	Router,
};
use neddit_api::{media::MediaSigner, server::MediaProxy, service::RedditService};
use serde::{Deserialize, Serialize};
use tower_http::compression::CompressionLayer;
#[cfg(debug_assertions)]
use tower_livereload::LiveReloadLayer;

#[cfg(debug_assertions)]
const CONTENT_SECURITY_POLICY: &str =
	"default-src 'none'; style-src 'self'; script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; connect-src 'self' ws: wss:; img-src 'self'; media-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
#[cfg(not(debug_assertions))]
const CONTENT_SECURITY_POLICY: &str =
	"default-src 'none'; style-src 'self'; script-src 'self' https://cdn.jsdelivr.net; connect-src 'self'; img-src 'self'; media-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageDisplay {
	#[default]
	Inline,
	Link,
}

#[derive(Clone)]
struct WebState {
	service: RedditService,
	signer: MediaSigner,
	media: MediaProxy,
	image_display: ImageDisplay,
}

impl FromRef<WebState> for RedditService {
	fn from_ref(state: &WebState) -> Self {
		state.service.clone()
	}
}

impl FromRef<WebState> for MediaSigner {
	fn from_ref(state: &WebState) -> Self {
		state.signer.clone()
	}
}

impl FromRef<WebState> for MediaProxy {
	fn from_ref(state: &WebState) -> Self {
		state.media.clone()
	}
}

impl FromRef<WebState> for ImageDisplay {
	fn from_ref(state: &WebState) -> Self {
		state.image_display
	}
}

pub fn router(service: RedditService, media: MediaProxy, image_display: ImageDisplay) -> Router {
	let video_enabled = media.video_enabled();
	let state = WebState {
		service,
		signer: media.signer().clone(),
		media: media.clone(),
		image_display,
	};
	let mut app = Router::new()
		.route("/", get(app::front_page))
		.route("/more-comments", get(app::more_comments))
		.route("/gallery/{article}/{index}", get(app::gallery))
		.route("/post-content/{article}", get(app::post_content))
		.route("/user/{username}", get(app::user_posts))
		.route("/user/{username}/comments", get(app::user_comments))
		.route("/r/{subreddit}", get(app::subreddit_feed))
		.route("/r/{subreddit}/", get(app::subreddit_feed))
		.route("/r/{subreddit}/wiki", get(app::wiki_root))
		.route("/r/{subreddit}/wiki/", get(app::wiki_root))
		.route("/r/{subreddit}/wiki/{*page}", get(app::wiki_page))
		.route("/comments/{article}", get(app::post_comments))
		.route("/comments/{article}/{slug}", get(app::post_comments))
		.route("/comments/{article}/{slug}/", get(app::post_comments))
		.route("/comments/{article}/{slug}/{comment}", get(app::post_comments))
		.route("/r/{subreddit}/comments/{article}", get(app::post_comments))
		.route("/r/{subreddit}/comments/{article}/{slug}", get(app::post_comments))
		.route("/r/{subreddit}/comments/{article}/{slug}/", get(app::post_comments))
		.route("/r/{subreddit}/comments/{article}/{slug}/{comment}", get(app::post_comments));
	if video_enabled {
		app = app.route("/media/video/player", get(app::video_player));
	}
	let app = app
		.with_state(state)
		.merge(system_routes())
		.merge(neddit_api::server::with_middleware(neddit_api::server::router(media)));
	#[cfg(debug_assertions)]
	let app = app.layer(LiveReloadLayer::new());
	app.layer(CompressionLayer::new()).layer(middleware::from_fn(security_headers))
}

fn system_routes() -> Router {
	health_router().route("/assets/app.css", get(stylesheet))
}

pub fn health_router() -> Router {
	Router::new().route("/healthz", get(health))
}

async fn health() -> StatusCode {
	StatusCode::NO_CONTENT
}

async fn stylesheet() -> impl IntoResponse {
	(
		[(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")],
		include_str!("../assets/app.css"),
	)
}

async fn security_headers(request: axum::extract::Request, next: Next) -> Response {
	let mut response = next.run(request).await;
	set_security_headers(&mut response);
	response
}

fn set_security_headers(response: &mut Response) {
	let headers = response.headers_mut();
	headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CONTENT_SECURITY_POLICY));
	headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
	headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
}
