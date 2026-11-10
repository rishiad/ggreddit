#![forbid(unsafe_code)]

pub mod config;

use axum::{middleware, Router};
use neddit_api::{api, server, server::MediaProxy, service::RedditService};

pub fn router(service: RedditService, media: &MediaProxy, image_display: neddit_web::ImageDisplay, web_enabled: bool, api_enabled: bool) -> Router {
	let mut app = if web_enabled {
		neddit_web::router(service.clone(), media.clone(), image_display)
	} else {
		server::with_middleware(server::router(media.clone())).merge(neddit_web::health_router())
	};
	if api_enabled {
		let signer = media.signer().clone();
		let api_routes = api::with_reddit_urls(api::router(service), signer);
		app = app.nest("/api", server::with_middleware(api_routes));
	}
	if web_enabled {
		app = app.layer(middleware::from_fn(neddit_web::html_error_pages));
	}
	server::with_request_logging(app)
}
