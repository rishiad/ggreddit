use crate::client::{error::ClientError, RedditClient};
use crate::media::{MediaSigner, MediaUrlError};
use axum::{
	body::{to_bytes, Body},
	extract::{MatchedPath, Path, Request, State},
	http::{header, Extensions, HeaderMap, HeaderValue, StatusCode, Version},
	middleware::{self, Next},
	response::{IntoResponse, Response},
	routing::get,
	Router,
};
use std::{
	future::{Future, IntoFuture},
	io, str,
	time::{Duration, Instant},
};
use thiserror::Error;
use tower_http::compression::{
	predicate::{Predicate, SizeAbove},
	CompressionLayer,
};
use tracing::Instrument as _;
use url::Url;

const COMPRESSION_MIN_SIZE: u64 = 1452;
const MANIFEST_SIZE_LIMIT: usize = 8 * 1024 * 1024;
const MAX_MEDIA_REDIRECTS: usize = 4;
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const MEDIA_CACHE_CONTROL: &str = "public, max-age=604800, immutable";
const HTTP_TARGET: &str = "ggreddit::http";
const REQUEST_ID_HEADER: &str = "x-request-id";

#[derive(Clone, Debug)]
pub(crate) struct RequestId(String);

impl RequestId {
	pub(crate) fn as_str(&self) -> &str {
		&self.0
	}
}

#[derive(Clone)]
pub struct MediaProxy {
	client: RedditClient,
	signer: MediaSigner,
}

impl MediaProxy {
	pub fn new(client: RedditClient, signer: MediaSigner) -> Self {
		Self { client, signer }
	}

	pub fn signer(&self) -> &MediaSigner {
		&self.signer
	}
}

pub fn router(media: MediaProxy) -> Router {
	Router::new().route("/media/{signature}/{encoded}", get(proxy_media)).with_state(media)
}

pub fn with_middleware(app: Router) -> Router {
	app
		.layer(CompressionLayer::new().compress_when(SizeAbove::new(COMPRESSION_MIN_SIZE).and(compressible_content)))
		.layer(middleware::from_fn(default_headers))
}

pub fn with_request_logging(app: Router) -> Router {
	app.layer(middleware::from_fn(log_request))
}

pub async fn bind(address: &str) -> io::Result<tokio::net::TcpListener> {
	tokio::net::TcpListener::bind(address).await
}

pub async fn serve(app: Router, listener: tokio::net::TcpListener) -> io::Result<()> {
	serve_until(app, listener, shutdown_signal(), SHUTDOWN_GRACE).await
}

async fn serve_until(app: Router, listener: tokio::net::TcpListener, signal: impl Future<Output = ()> + Send + 'static, grace: Duration) -> io::Result<()> {
	let (started, received) = tokio::sync::oneshot::channel();
	let server = axum::serve(listener, app)
		.with_graceful_shutdown(async move {
			signal.await;
			let _ = started.send(());
		})
		.into_future();
	tokio::pin!(server);
	tokio::select! {
		biased;
		result = &mut server => result,
		_ = async { let _ = received.await; tokio::time::sleep(grace).await } => {
			tracing::warn!(event = "service.shutdown_timeout", "stalled HTTP requests exceeded the shutdown grace period");
			Ok(())
		}
	}
}

async fn log_request(mut request: Request, next: Next) -> Response {
	let started = Instant::now();
	let request_id = RequestId(uuid::Uuid::new_v4().to_string());
	let method = request.method().clone();
	let route = route_label(&request);
	let quiet = quiet_route(&route);
	request.extensions_mut().insert(request_id.clone());

	let span = tracing::info_span!(target: HTTP_TARGET, "http.request", request_id = %request_id.as_str(), %method, %route);
	async move {
		let mut response = next.run(request).await;
		let status = response.status();
		let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
		if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
			tracing::warn!(target: HTTP_TARGET, event = "http.response", status = status.as_u16(), latency_ms, "request completed");
		} else if quiet {
			tracing::debug!(target: HTTP_TARGET, event = "http.response", status = status.as_u16(), latency_ms, "request completed");
		} else {
			tracing::info!(target: HTTP_TARGET, event = "http.response", status = status.as_u16(), latency_ms, "request completed");
		}
		response
			.headers_mut()
			.insert(REQUEST_ID_HEADER, HeaderValue::from_str(request_id.as_str()).expect("UUID is a valid header value"));
		response
	}
	.instrument(span)
	.await
}

fn route_label(request: &Request) -> String {
	request
		.extensions()
		.get::<MatchedPath>()
		.map(|path| path.as_str().to_owned())
		.unwrap_or_else(|| "<unmatched>".to_owned())
}

fn quiet_route(route: &str) -> bool {
	route == "/healthz" || route.starts_with("/assets/") || route.starts_with("/media/")
}

async fn proxy_media(State(media): State<MediaProxy>, Path((signature, encoded)): Path<(String, String)>, headers: HeaderMap) -> Result<Response, ProxyError> {
	let target = media.signer.decode_target(&signature, &encoded)?;
	let etag = format!("\"{signature}\"");
	if request_has_etag(&headers, &etag) {
		return not_modified(&etag, MEDIA_CACHE_CONTROL);
	}
	let (response, target) = follow_redirects(&media, target, &headers).await?;
	finish_proxy_response(response, &target, &media.signer, &etag, MEDIA_CACHE_CONTROL).await
}

async fn follow_redirects(media: &MediaProxy, mut target: Url, headers: &HeaderMap) -> Result<(Response, Url), ProxyError> {
	for redirect in 0..=MAX_MEDIA_REDIRECTS {
		let response = media.client.proxy(target.to_string(), headers).await?;
		if response.status() == StatusCode::NOT_MODIFIED || !response.status().is_redirection() {
			return Ok((response, target));
		}
		if redirect == MAX_MEDIA_REDIRECTS {
			return Err(ProxyError::TooManyRedirects);
		}
		let location = response
			.headers()
			.get(header::LOCATION)
			.ok_or(ProxyError::MissingRedirectLocation)?
			.to_str()
			.map_err(|_| ProxyError::InvalidRedirect)?;
		let redirected = target.join(location).map_err(|_| ProxyError::InvalidRedirect)?;
		media.signer.validate_media_target(&redirected).map_err(|_| ProxyError::ForbiddenRedirect)?;
		target = redirected;
	}
	Err(ProxyError::TooManyRedirects)
}

async fn finish_proxy_response(mut response: Response, target: &Url, signer: &MediaSigner, etag: &str, cache_control: &'static str) -> Result<Response, ProxyError> {
	match response.status() {
		StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE => return Err(ProxyError::UpstreamUnavailable),
		StatusCode::GATEWAY_TIMEOUT => return Err(ProxyError::UpstreamTimeout),
		status
			if status != StatusCode::NOT_MODIFIED && !status.is_success() && !matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE | StatusCode::RANGE_NOT_SATISFIABLE) =>
		{
			return Err(ProxyError::UpstreamFailure);
		}
		_ => {}
	}
	if response.status().is_success() && is_hls(&response, target) {
		response = rewrite_manifest(response, |manifest| signer.rewrite_hls(manifest, target)).await?;
	} else if response.status().is_success() && is_dash(&response, target) {
		response = rewrite_manifest(response, |manifest| signer.rewrite_dash(manifest, target)).await?;
	}

	if response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED {
		let headers = response.headers_mut();
		headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control));
		headers.insert(header::ETAG, HeaderValue::from_str(etag).map_err(|_| ProxyError::InvalidEtag)?);
		headers.remove(header::LOCATION);
	}
	Ok(response)
}

async fn rewrite_manifest<F, E>(response: Response, rewrite: F) -> Result<Response, ProxyError>
where
	F: FnOnce(&str) -> Result<String, E>,
	ProxyError: From<E>,
{
	let (mut parts, body) = response.into_parts();
	let body = to_bytes(body, MANIFEST_SIZE_LIMIT).await.map_err(ProxyError::ManifestBody)?;
	let manifest = str::from_utf8(&body).map_err(|_| ProxyError::InvalidManifest)?;
	let body = rewrite(manifest).map_err(ProxyError::from)?;
	parts.headers.remove(header::CONTENT_LENGTH);
	parts.headers.remove(header::CONTENT_ENCODING);
	Ok(Response::from_parts(parts, Body::from(body)))
}

fn is_hls(response: &Response, target: &Url) -> bool {
	target.path().ends_with(".m3u8") || content_type(response).is_some_and(|value| value.contains("mpegurl"))
}

fn is_dash(response: &Response, target: &Url) -> bool {
	target.path().ends_with(".mpd") || content_type(response).is_some_and(|value| value.contains("dash+xml"))
}

fn content_type(response: &Response) -> Option<&str> {
	response.headers().get(header::CONTENT_TYPE)?.to_str().ok()
}

fn request_has_etag(headers: &HeaderMap, etag: &str) -> bool {
	headers
		.get(header::IF_NONE_MATCH)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value.split(',').any(|value| value.trim() == etag))
}

fn not_modified(etag: &str, cache_control: &'static str) -> Result<Response, ProxyError> {
	let mut response = Response::new(Body::empty());
	*response.status_mut() = StatusCode::NOT_MODIFIED;
	response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control));
	response
		.headers_mut()
		.insert(header::ETAG, HeaderValue::from_str(etag).map_err(|_| ProxyError::InvalidEtag)?);
	Ok(response)
}

async fn default_headers(request: axum::extract::Request, next: Next) -> Response {
	let mut response = next.run(request).await;
	response.headers_mut().insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
	response.headers_mut().insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
	response
}

fn compressible_content(_: StatusCode, _: Version, headers: &HeaderMap, _: &Extensions) -> bool {
	headers
		.get(header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| value.starts_with("text/") || value.starts_with("application/json"))
}

#[derive(Debug, Error)]
enum ProxyError {
	#[error(transparent)]
	MediaUrl(#[from] MediaUrlError),
	#[error("Reddit media request failed")]
	Client(#[from] ClientError),
	#[error("Reddit media redirect omitted its destination")]
	MissingRedirectLocation,
	#[error("Reddit returned an invalid media redirect")]
	InvalidRedirect,
	#[error("Reddit redirected media outside its allowed hosts")]
	ForbiddenRedirect,
	#[error("Reddit media exceeded the redirect limit")]
	TooManyRedirects,
	#[error("Reddit returned an invalid media manifest")]
	InvalidManifest,
	#[error("failed to read Reddit media manifest")]
	ManifestBody(#[source] axum::Error),
	#[error("failed to construct media ETag")]
	InvalidEtag,
	#[error("Reddit media is temporarily unavailable")]
	UpstreamUnavailable,
	#[error("Reddit media timed out")]
	UpstreamTimeout,
	#[error("Reddit media returned an unexpected status")]
	UpstreamFailure,
}

impl IntoResponse for ProxyError {
	fn into_response(self) -> Response {
		let status = match self {
			Self::MediaUrl(MediaUrlError::InvalidSignature | MediaUrlError::ForbiddenTarget) => StatusCode::FORBIDDEN,
			Self::MediaUrl(MediaUrlError::InvalidEncoding | MediaUrlError::InvalidTarget) => StatusCode::BAD_REQUEST,
			Self::UpstreamTimeout | Self::Client(ClientError::Timeout | ClientError::Auth(crate::client::AuthError::Timeout)) => StatusCode::GATEWAY_TIMEOUT,
			Self::UpstreamUnavailable | Self::Client(ClientError::Unauthorized | ClientError::RateLimited { .. } | ClientError::Auth(_)) => StatusCode::SERVICE_UNAVAILABLE,
			Self::InvalidEtag
			| Self::MediaUrl(MediaUrlError::ShortSecret | MediaUrlError::InvalidDomainPattern(_))
			| Self::Client(ClientError::BuildClient(_) | ClientError::ResponseBuild(_)) => StatusCode::INTERNAL_SERVER_ERROR,
			_ => StatusCode::BAD_GATEWAY,
		};
		(status, self.to_string()).into_response()
	}
}

async fn shutdown_signal() {
	#[cfg(windows)]
	{
		match tokio::signal::ctrl_c().await {
			Ok(()) => tracing::info!(event = "service.shutdown_requested", signal = "CTRL+C", "shutdown requested"),
			Err(error) => tracing::error!(event = "service.signal_failed", %error, signal = "CTRL+C", "failed to receive shutdown signal"),
		}
	}

	#[cfg(unix)]
	{
		let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
		match terminate {
			Ok(mut terminate) => {
				tokio::select! {
					result = tokio::signal::ctrl_c() => {
						match result {
							Ok(()) => tracing::info!(event = "service.shutdown_requested", signal = "CTRL+C", "shutdown requested"),
							Err(error) => tracing::error!(event = "service.signal_failed", %error, signal = "CTRL+C", "failed to receive shutdown signal"),
						}
					}
					_ = terminate.recv() => tracing::info!(event = "service.shutdown_requested", signal = "SIGTERM", "shutdown requested")
				}
			}
			Err(error) => {
				tracing::error!(event = "service.signal_failed", %error, signal = "SIGTERM", "failed to install shutdown signal handler");
				match tokio::signal::ctrl_c().await {
					Ok(()) => tracing::info!(event = "service.shutdown_requested", signal = "CTRL+C", "shutdown requested"),
					Err(error) => tracing::error!(event = "service.signal_failed", %error, signal = "CTRL+C", "failed to receive shutdown signal"),
				}
			}
		}
	}
}
