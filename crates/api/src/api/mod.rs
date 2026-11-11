mod query;
mod response;
mod routes;
mod url_rewrite;

use crate::{media::MediaSigner, service::RedditService};
use axum::Router;

pub fn router(service: RedditService) -> Router {
	Router::new().merge(routes::router()).with_state(service)
}

pub fn with_reddit_urls(router: Router, signer: MediaSigner) -> Router {
	url_rewrite::with_reddit_urls(router, signer)
}
