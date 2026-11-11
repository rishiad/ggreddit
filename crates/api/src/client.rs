pub mod error;
mod graphql;
mod oauth;

use axum::{body::Body, http::HeaderMap, response::Response};
use percent_encoding::{percent_encode, CONTROLS};
use serde_json::Value;
use std::{result::Result, time::Duration};
use tracing::{debug, trace, warn};
use url::form_urlencoded;
use wreq::redirect::Policy;
use wreq::{header as wreq_header, Client as WreqClient, EmulationFactory, Method, Response as WreqResponse};
use wreq_util::{Emulation, EmulationOS, EmulationOption};

use self::error::ClientError;
pub use self::oauth::AuthError;
use self::oauth::{CredentialLease, OAuthHandle};

const REDDIT_URL_BASE: &str = "https://oauth.reddit.com";
const REDDIT_URL_BASE_HOST: &str = "oauth.reddit.com";
const REDDIT_GRAPHQL_URL: &str = "https://gql-fed.reddit.com/";
const POST_COMMENTS_HASH: &str = "314f29c2494fd78c7844ef2c6b09acb4bef5b37ab50b4ddc01691d6d5c8433f8";
const MAX_UPSTREAM_BYTES: usize = 8 * 1024 * 1024;
const MAX_REDDIT_REDIRECTS: usize = 4;

const ALTERNATIVE_REDDIT_URL_BASE: &str = "https://www.reddit.com";

#[derive(Clone)]
pub struct RedditClient {
	http: WreqClient,
	oauth: OAuthHandle,
	cache: Option<crate::storage::Cache>,
}

struct UpstreamResponse {
	response: WreqResponse,
	lease: CredentialLease,
	rate_limit_reset: Option<String>,
}

impl RedditClient {
	pub async fn new() -> Result<Self, ClientError> {
		let http = Self::build_http_client()?;
		let oauth = OAuthHandle::start(http.clone()).await?;
		Ok(Self { http, oauth, cache: None })
	}

	pub fn with_cache(mut self, cache: crate::storage::Cache) -> Self {
		self.cache = Some(cache);
		self
	}

	pub async fn shutdown(&self) {
		self.oauth.shutdown().await;
	}

	/// Build the HTTP client used for Reddit and media requests.
	fn build_http_client() -> Result<WreqClient, ClientError> {
		// Keeping this list short to aid in privacy.
		// The more emulations, the more unique a fingerprint each instance has.
		// But some emulations should increase evasiveness.
		let emulation = [Emulation::Chrome145, Emulation::Firefox147];
		let emulation_os = [EmulationOS::Android, EmulationOS::Windows];

		let rand = fastrand::usize(..);
		let emulation = EmulationOption::builder()
			.emulation(emulation[rand % emulation.len()])
			.emulation_os(emulation_os[rand % emulation_os.len()])
			.build()
			.emulation();

		debug!(?emulation, "HTTP client initialized");
		WreqClient::builder()
			.emulation(emulation)
			.redirect(Policy::none())
			.build()
			.map_err(ClientError::BuildClient)
	}

	pub async fn proxy(&self, url: String, request_headers: &HeaderMap) -> Result<Response, ClientError> {
		let credentials = self.oauth.current();
		self.proxy_with(&self.http, url, request_headers, Some(credentials.user_agent())).await
	}

	async fn proxy_with(&self, client: &WreqClient, url: String, request_headers: &HeaderMap, user_agent: Option<&str>) -> Result<Response, ClientError> {
		// First parameter is target URL (mandatory).
		let wreq_uri = wreq::Uri::try_from(&url).map_err(|_| ClientError::InvalidProxyUrl { url: url.clone() })?;

		let mut builder = client.get(wreq_uri);
		// Copy useful headers from original request
		for &key in &["Range", "If-Modified-Since", "Cache-Control"] {
			if let Some(value) = request_headers.get(key) {
				builder = builder.header(key, value.as_bytes());
			}
		}

		if let Some(user_agent) = user_agent {
			builder = builder.header("User-Agent", user_agent);
		}

		// Request the media body rather than a provider landing page.
		builder = builder.header(wreq_header::ACCEPT, "*/*");

		let response = builder.send().await.map_err(|source| ClientError::Request { url, source })?;
		response.into_http_response()
	}

	/// Makes a GET request to Reddit at `path`. By default, this will honor HTTP
	/// 3xx codes Reddit returns and will automatically redirect.
	async fn reddit_get(&self, path: String) -> Result<UpstreamResponse, ClientError> {
		self.request(&Method::GET, with_raw_json(path), REDDIT_URL_BASE, REDDIT_URL_BASE_HOST).await
	}

	async fn request(&self, method: &Method, mut path: String, base_path: &str, host: &str) -> Result<UpstreamResponse, ClientError> {
		for redirects in 0..=MAX_REDDIT_REDIRECTS {
			let url = format!("{base_path}{path}");
			let lease = self.oauth.acquire().await?;
			let generation = lease.generation();
			let mut headers = Vec::with_capacity(lease.headers().len() + 1);
			headers.push(("Host", host));
			headers.extend(lease.headers().iter().map(|(name, value)| (name.as_str(), value.as_str())));
			fastrand::shuffle(&mut headers);

			let mut builder = self.http.request(method.clone(), &url);
			for (key, value) in headers {
				builder = builder.header(key, value);
			}
			let result = builder.send().await;
			let response = result.map_err(|source| ClientError::Request { url: url.clone(), source })?;
			let rate_limit_reset = self.observe_rate_limit_headers(generation, &response).await;
			if !response.status().is_redirection() {
				return Ok(UpstreamResponse {
					response,
					lease,
					rate_limit_reset,
				});
			}

			let location = response
				.headers()
				.get(wreq::header::LOCATION)
				.ok_or_else(|| ClientError::MissingRedirectLocation { path: path.clone() })?;
			if location.to_str().ok() == Some(ALTERNATIVE_REDDIT_URL_BASE) {
				return Err(ClientError::InvalidRedirect { path });
			}
			if redirects == MAX_REDDIT_REDIRECTS {
				return Err(ClientError::TooManyRedirects { path });
			}
			path = percent_encode(location.as_bytes(), CONTROLS)
				.to_string()
				.trim_start_matches(REDDIT_URL_BASE)
				.trim_start_matches(ALTERNATIVE_REDDIT_URL_BASE)
				.to_string();
			path = with_raw_json(path);
			drop(response);
			drop(lease);
		}
		Err(ClientError::TooManyRedirects { path })
	}

	/// Make a request to a Reddit API and parse the JSON response.
	pub async fn json(&self, path: String) -> Result<Value, ClientError> {
		let Some(cache) = &self.cache else {
			return self.fetch_json(path).await;
		};
		let key = cache_key(&path);
		let ttl = cache_ttl(&path);
		tokio::time::timeout(Duration::from_secs(20), cache.json(key, ttl, self.fetch_json(path)))
			.await
			.map_err(|_| ClientError::Timeout)?
	}

	/// Fetch the Android app's persisted post-comment query with the current mobile token.
	pub(crate) async fn post_comments_graphql(&self, variables: Value) -> Result<Value, ClientError> {
		if let Some(cache) = &self.cache {
			let key = format!("reddit-graphql-v1:{}", crate::storage::fingerprint(variables.to_string().as_bytes()));
			return tokio::time::timeout(Duration::from_secs(20), cache.json(key, 60, self.fetch_post_comments_graphql(variables)))
				.await
				.map_err(|_| ClientError::Timeout)?;
		}
		tokio::time::timeout(Duration::from_secs(20), self.fetch_post_comments_graphql(variables))
			.await
			.map_err(|_| ClientError::Timeout)?
	}

	async fn fetch_post_comments_graphql(&self, variables: Value) -> Result<Value, ClientError> {
		let json = self.graphql("PostComments", POST_COMMENTS_HASH, variables).await?;
		if json.pointer("/data/postInfoById/commentForest/trees").and_then(Value::as_array).is_none() {
			return Err(ClientError::InvalidEnvelope);
		}
		Ok(json)
	}

	async fn graphql(&self, operation: &'static str, hash: &'static str, variables: Value) -> Result<Value, ClientError> {
		let lease = self.oauth.acquire().await?;
		let generation = lease.generation();
		let mut request = self.http.post(REDDIT_GRAPHQL_URL);
		for (key, value) in lease.headers() {
			if !key.eq_ignore_ascii_case("content-type") {
				request = request.header(key, value);
			}
		}
		let payload = serde_json::json!({
			"operationName": operation,
			"variables": variables,
			"extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
		});
		let response = request.json(&payload).send().await.map_err(|source| ClientError::Request {
			url: REDDIT_GRAPHQL_URL.into(),
			source,
		})?;
		let status = response.status();
		let reset = self.observe_rate_limit_headers(generation, &response).await;
		if status.as_u16() == 429 {
			self.oauth.invalidate(generation).await;
			return Err(ClientError::RateLimited { reset });
		}
		if status.as_u16() == 401 {
			self.oauth.invalidate(generation).await;
			return Err(ClientError::Unauthorized);
		}
		if !status.is_success() {
			return Err(ClientError::UnexpectedStatus {
				path: operation.into(),
				status: status.as_u16(),
			});
		}
		let mut response = response;
		if response.content_length().is_some_and(|length| length > MAX_UPSTREAM_BYTES as u64) {
			return Err(ClientError::BodyTooLarge);
		}
		let mut body = Vec::new();
		while let Some(chunk) = response.chunk().await.map_err(|source| ClientError::Body { path: operation.into(), source })? {
			append_body_chunk(&mut body, &chunk)?;
		}
		let json: Value = serde_json::from_slice(&body).map_err(|source| ClientError::Decode { path: operation.into(), source })?;
		if json["errors"].as_array().is_some_and(|errors| !errors.is_empty()) {
			warn!(event = "upstream.graphql_errors", operation, codes = ?json["errors"].as_array().map(|errors| errors.iter().map(|error| error["extensions"]["code"].as_str().unwrap_or("unknown")).collect::<Vec<_>>()), "Reddit GraphQL rejected a request");
			return Err(ClientError::InvalidEnvelope);
		}
		if json["data"].is_null() {
			return Err(ClientError::InvalidEnvelope);
		}
		Ok(json)
	}

	async fn fetch_json(&self, path: String) -> Result<Value, ClientError> {
		if let Some(result) = graphql::route(self, &path).await {
			if let Err(error) = &result {
				warn!(event = "upstream.graphql_failed", %path, %error, "Reddit GraphQL request failed");
			}
			return result;
		}
		let UpstreamResponse {
			mut response,
			lease,
			rate_limit_reset,
		} = self.reddit_get(path.clone()).await?;
		let generation = lease.generation();
		let status = response.status();
		if response.content_length().is_some_and(|n| n > MAX_UPSTREAM_BYTES as u64) {
			return Err(ClientError::BodyTooLarge);
		}
		let mut body = Vec::new();
		while let Some(chunk) = response.chunk().await.map_err(|source| ClientError::Body { path: path.clone(), source })? {
			append_body_chunk(&mut body, &chunk)?;
		}
		drop(lease);

		if body.is_empty() {
			self.oauth.invalidate(generation).await;
			return Err(ClientError::RateLimited { reset: rate_limit_reset });
		}
		if status.as_u16() == 429 {
			self.oauth.invalidate(generation).await;
			return Err(ClientError::RateLimited { reset: rate_limit_reset });
		}
		if status.as_u16() == 401 {
			self.oauth.invalidate(generation).await;
			return Err(ClientError::Unauthorized);
		}

		let json: Value = serde_json::from_slice(&body).map_err(|source| {
			warn!(event = "upstream.invalid_response", upstream = "reddit", status = status.as_u16(), error = %source, "Reddit returned invalid JSON");
			if status.is_server_error() {
				ClientError::UpstreamUnavailable { status: status.as_u16(), source }
			} else {
				ClientError::Decode { path: path.clone(), source }
			}
		})?;

		if json["data"]["is_suspended"].as_bool() == Some(true) {
			return Err(ClientError::Suspended);
		}
		validate_json_envelope(status.as_u16(), &json)?;

		let Some(code) = json["error"].as_i64() else {
			return Ok(json);
		};
		let reason = json["reason"].as_str().unwrap_or_default();
		let message = json["message"].as_str().unwrap_or_default();
		if message == "Unauthorized" {
			debug!(event = "oauth.refresh_requested", reason = "unauthorized", "requesting OAuth refresh");
			self.oauth.invalidate(generation).await;
			return Err(ClientError::Unauthorized);
		}

		match reason {
			"quarantined" => Err(ClientError::Quarantined),
			"gated" => Err(ClientError::Gated),
			"private" => Err(ClientError::Private),
			"banned" => Err(ClientError::Banned),
			_ => Err(ClientError::Reddit {
				code,
				reason: reason.to_string(),
				message: message.to_string(),
				path,
			}),
		}
	}

	async fn observe_rate_limit_headers(&self, generation: u64, response: &WreqResponse) -> Option<String> {
		let remaining = response.headers().get("x-ratelimit-remaining")?.to_str().ok()?;
		let reset = response.headers().get("x-ratelimit-reset")?.to_str().ok()?;
		let used = response.headers().get("x-ratelimit-used")?.to_str().ok()?;
		trace!(
			event = "upstream.rate_limit",
			upstream = "reddit",
			remaining,
			reset_seconds = reset,
			used,
			"Reddit rate limit observed"
		);

		if let Some(remaining) = parse_rate_limit_remaining(remaining) {
			let reset_after = reset
				.parse::<f64>()
				.ok()
				.filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
				.and_then(|seconds| Duration::try_from_secs_f64(seconds).ok());
			self.oauth.observe_rate_limit(generation, remaining, reset_after).await;
		}

		Some(reset.to_string())
	}
}

pub(crate) fn cache_key(path: &str) -> String {
	let (path, query) = path.split_once('?').unwrap_or((path, ""));
	let mut pairs: Vec<_> = form_urlencoded::parse(query.as_bytes()).collect();
	// Stable key sort preserves the order of duplicate parameters.
	pairs.sort_by(|a, b| a.0.cmp(&b.0));
	let query = form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish();
	format!("reddit-json-v1:{}", crate::storage::fingerprint(format!("{path}?{query}").as_bytes()))
}

fn cache_ttl(path: &str) -> i64 {
	let path = path.split('?').next().unwrap_or(path);
	if path.contains("/wiki/") {
		600
	} else if path.ends_with("/about") {
		120
	} else if path.contains("/comments/") || path.starts_with("/comments/") || path.starts_with("/by_id/") {
		60
	} else {
		30
	}
}

fn parse_rate_limit_remaining(value: &str) -> Option<u16> {
	let value = value.parse::<f64>().ok()?.floor();
	if !value.is_finite() || value < 0.0 {
		return None;
	}
	Some(value.min(f64::from(u16::MAX)) as u16)
}

fn append_body_chunk(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), ClientError> {
	if chunk.len() > MAX_UPSTREAM_BYTES.saturating_sub(body.len()) {
		return Err(ClientError::BodyTooLarge);
	}
	body.extend_from_slice(chunk);
	Ok(())
}

fn validate_json_envelope(status: u16, json: &Value) -> Result<(), ClientError> {
	let numeric_error = json.get("error").is_some_and(Value::is_i64);
	let error = json.get("error").is_some_and(|v| !v.is_null() && !v.is_i64());
	let errors = json.get("errors").is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()));
	if (!(200..300).contains(&status) && !numeric_error) || error || errors {
		return Err(ClientError::InvalidEnvelope);
	}
	Ok(())
}

fn with_raw_json(path: String) -> String {
	let (base, query) = path.split_once('?').unwrap_or((&path, ""));
	let mut serializer = form_urlencoded::Serializer::new(String::new());
	for (key, value) in form_urlencoded::parse(query.as_bytes()) {
		if key != "raw_json" {
			serializer.append_pair(&key, &value);
		}
	}
	serializer.append_pair("raw_json", "1");
	format!("{base}?{}", serializer.finish())
}

trait IntoHttpResponse {
	fn into_http_response(self) -> Result<Response, ClientError>;
}

impl IntoHttpResponse for WreqResponse {
	fn into_http_response(self) -> Result<Response, ClientError> {
		let status = self.status();
		let version = self.version();

		let mut builder = Response::builder().status(status).version(version);

		for (name, value) in self.headers() {
			if is_safe_proxy_header(name.as_str()) {
				builder = builder.header(name, value);
			}
		}

		builder.body(Body::from_stream(self.bytes_stream())).map_err(ClientError::ResponseBuild)
	}
}

fn is_safe_proxy_header(name: &str) -> bool {
	matches!(
		name,
		"accept-ranges"
			| "cache-control"
			| "content-disposition"
			| "content-encoding"
			| "content-language"
			| "content-length"
			| "content-range"
			| "content-type"
			| "expires"
			| "last-modified"
			| "location"
	)
}
