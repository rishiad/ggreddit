mod query;
mod response;
mod routes;
mod url_rewrite;

use crate::{media::MediaSigner, service::RedditService};
use axum::{middleware, Router};

pub fn router(service: RedditService) -> Router {
	Router::new().merge(routes::router()).with_state(service)
}

pub fn with_reddit_urls(router: Router, signer: MediaSigner) -> Router {
	router.layer(middleware::from_fn_with_state(signer, url_rewrite::rewrite_reddit_urls))
}
