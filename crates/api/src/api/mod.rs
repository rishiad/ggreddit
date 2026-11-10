mod query;
mod response;
mod routes;
mod url_rewrite;

use crate::{media::MediaSigner, service::RedditService};
use axum::{
	extract::{rejection::QueryRejection, Query},
	http::StatusCode,
	response::{IntoResponse, Response},
	routing::get,
	Json, Router,
};

type DiagnosticResponse = (StatusCode, Json<crate::search::Diagnostic>);

fn decode_query<T>(query: Result<Query<T>, QueryRejection>, message: &'static str) -> Result<T, DiagnosticResponse> {
	query.map(|Query(value)| value).map_err(|_| {
		diagnostic(crate::search::Diagnostic {
			code: "invalid_value",
			message: message.into(),
			start: 0,
			end: 0,
		})
	})
}

fn diagnostic(error: crate::search::Diagnostic) -> DiagnosticResponse {
	(error.status(), Json(error))
}

pub fn router(service: RedditService) -> Router {
	Router::new().merge(routes::router()).route("/search/ql", get(ql_search)).with_state(service)
}

async fn ql_search(axum::extract::State(service): axum::extract::State<RedditService>, query: Result<Query<crate::search::Request>, QueryRejection>) -> Response {
	let result: Result<Json<crate::search::Page>, DiagnosticResponse> = async {
		let request = decode_query(query, "Invalid or obsolete search request controls")?;
		service.search_ql(&request).await.map(Json).map_err(diagnostic)
	}
	.await;
	result.into_response()
}

pub fn with_reddit_urls(router: Router, signer: MediaSigner) -> Router {
	url_rewrite::with_reddit_urls(router, signer)
}
