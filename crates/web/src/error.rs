use askama::Template;
use axum::{
	extract::Request,
	http::{header, StatusCode},
	middleware::Next,
	response::{Html, IntoResponse, Response},
};
use neddit_api::service::ServiceError;
use thiserror::Error;

const ERROR_DESCRIPTIONS: &[(u16, &str)] = &[
	(400, "The request could not be understood."),
	(401, "Authentication is required to view this page."),
	(403, "You do not have permission to view this page."),
	(404, "The requested page could not be found."),
	(405, "This request method is not supported."),
	(408, "The request timed out."),
	(409, "The request conflicts with the current state."),
	(410, "The requested page is no longer available."),
	(413, "The request is too large."),
	(429, "Too many requests. Please try again later."),
	(500, "The server could not complete this request."),
	(502, "Reddit did not return the data needed to load this page."),
	(503, "The service is temporarily unavailable."),
	(504, "Reddit took too long to respond."),
];

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate {
	code: u16,
	description: &'static str,
}

#[derive(Debug, Error)]
pub enum AppError {
	#[error("invalid feed sort")]
	InvalidSort,
	#[error("invalid feed time")]
	InvalidFeedTime,
	#[error("invalid comment sort")]
	InvalidCommentSort,
	#[error("post not found")]
	PostNotFound,
	#[error("failed to load Reddit data")]
	Service(#[from] ServiceError),
}

impl IntoResponse for AppError {
	fn into_response(self) -> Response {
		if matches!(&self, Self::Service(ServiceError::ContentBlocked)) {
			return response_with_description(StatusCode::FORBIDDEN, "This server has NSFW disabled.");
		}
		let status = match self {
			Self::InvalidSort | Self::InvalidFeedTime | Self::InvalidCommentSort => StatusCode::BAD_REQUEST,
			Self::PostNotFound => StatusCode::NOT_FOUND,
			Self::Service(_) => StatusCode::BAD_GATEWAY,
		};
		response(status)
	}
}

pub(crate) fn response(status: StatusCode) -> Response {
	let description = ERROR_DESCRIPTIONS
		.iter()
		.find_map(|&(code, description)| (code == status.as_u16()).then_some(description))
		.unwrap_or("The request could not be completed.");
	response_with_description(status, description)
}

fn response_with_description(status: StatusCode, description: &'static str) -> Response {
	let template = ErrorTemplate {
		code: status.as_u16(),
		description,
	};
	let mut response = match template.render() {
		Ok(body) => (status, Html(body)).into_response(),
		Err(_) => (status, description).into_response(),
	};
	crate::set_security_headers(&mut response);
	response
}

pub async fn html_error_pages(request: Request, next: Next) -> Response {
	let accepts_html = request
		.headers()
		.get(header::ACCEPT)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value.split(',').any(|media_type| media_type.trim_start().starts_with("text/html")));
	let response = next.run(request).await;
	let status = response.status();
	let already_html = response
		.headers()
		.get(header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value.starts_with("text/html"));
	if accepts_html && (status.is_client_error() || status.is_server_error()) && !already_html {
		return self::response(status);
	}
	response
}
