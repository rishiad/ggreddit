use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use regex::Regex;
use serde_json::Value;
use sha2::Sha256;
use std::{
	path::Path,
	sync::{Arc, LazyLock},
};
use thiserror::Error;
use url::{Host, Url};

type HmacSha256 = Hmac<Sha256>;

static ABSOLUTE_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)(?:https?:)?//[^\s<>\"']+"#).unwrap());
static HLS_URI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"URI="([^"]+)""#).unwrap());
static DASH_BASE_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<BaseURL([^>]*)>([^<]+)</BaseURL>").unwrap());

const MEDIA_ROUTE: &str = "/media";

#[derive(Clone)]
pub struct MediaSigner {
	key: Arc<[u8]>,
	domains: DomainPolicy,
	policy_scope: &'static str,
}

#[derive(Clone, Debug, Default)]
pub struct DomainPolicy {
	navigation: DomainSet,
	shortlinks: DomainSet,
	media: DomainSet,
}

/// A compiled set of exact domains and `*.` subdomain patterns.
#[derive(Clone, Debug, Default)]
pub struct DomainSet(Arc<[HostPattern]>);

#[derive(Clone, Debug)]
struct HostPattern {
	domain: String,
	subdomains_only: bool,
}

impl DomainPolicy {
	pub fn new(navigation: &[String], shortlinks: &[String], media: &[String]) -> Result<Self, MediaUrlError> {
		Ok(Self {
			navigation: DomainSet::new(navigation)?,
			shortlinks: DomainSet::new(shortlinks)?,
			media: DomainSet::new(media)?,
		})
	}

	pub fn rewrite_navigation(&self, url: &str) -> Option<String> {
		rewrite_navigation(url, self)
	}

	fn is_navigation_host(&self, host: &str) -> bool {
		self.navigation.matches(host)
	}

	fn is_shortlink_host(&self, host: &str) -> bool {
		self.shortlinks.matches(host)
	}

	fn is_media_host(&self, host: &str) -> bool {
		self.media.matches(host)
	}
}

impl DomainSet {
	/// Compile domain patterns.
	///
	/// # Errors
	///
	/// Returns an error when a pattern contains a scheme, port, IP address, or invalid wildcard.
	pub fn new(values: &[String]) -> Result<Self, MediaUrlError> {
		Ok(Self(parse_patterns(values)?.into()))
	}

	/// Return whether a host matches this set.
	pub fn matches(&self, host: &str) -> bool {
		let host = host.trim_end_matches('.').to_ascii_lowercase();
		self.0.iter().any(|pattern| pattern.matches(&host))
	}
}

impl MediaSigner {
	pub fn random() -> Result<Self, std::io::Error> {
		let mut key = [0_u8; 32];
		getrandom::fill(&mut key).map_err(|error| std::io::Error::other(format!("failed to generate media signing key: {error}")))?;
		Ok(Self {
			key: Arc::from(key),
			domains: DomainPolicy::default(),
			policy_scope: "nsfw-allowed",
		})
	}

	pub fn from_secret(secret: &[u8]) -> Result<Self, MediaUrlError> {
		if secret.len() < 32 {
			return Err(MediaUrlError::ShortSecret);
		}
		Ok(Self {
			key: Arc::from(secret),
			domains: DomainPolicy::default(),
			policy_scope: "nsfw-allowed",
		})
	}

	pub fn from_file(path: impl AsRef<Path>) -> Result<Self, MediaSignerLoadError> {
		let secret = std::fs::read(path)?;
		let start = secret.iter().position(|byte| !byte.is_ascii_whitespace()).unwrap_or(secret.len());
		let end = secret.iter().rposition(|byte| !byte.is_ascii_whitespace()).map_or(start, |index| index + 1);
		Self::from_secret(&secret[start..end]).map_err(MediaSignerLoadError::from)
	}

	pub fn with_domains(mut self, domains: DomainPolicy) -> Self {
		self.domains = domains;
		self
	}

	pub fn with_content_policy(mut self, allow_nsfw: bool) -> Self {
		self.policy_scope = if allow_nsfw { "nsfw-allowed" } else { "nsfw-blocked" };
		self
	}

	pub fn rewrite_value(&self, value: &mut Value) {
		match value {
			Value::Array(values) => {
				for value in values {
					self.rewrite_value(value);
				}
			}
			Value::Object(values) => {
				for value in values.values_mut() {
					self.rewrite_value(value);
				}
			}
			Value::String(value) => {
				*value = self.rewrite_text(value);
			}
			_ => {}
		}
	}

	pub fn rewrite_text(&self, text: &str) -> String {
		if !text.contains("//") {
			return text.to_string();
		}
		if let Some(rewritten) = self.rewrite_absolute_url(text) {
			return rewritten;
		}

		ABSOLUTE_URL
			.replace_all(text, |captures: &regex::Captures<'_>| {
				let matched = &captures[0];
				let (candidate, suffix) = trim_url_punctuation(matched);
				self
					.rewrite_absolute_url(candidate)
					.map_or_else(|| matched.to_string(), |rewritten| format!("{rewritten}{suffix}"))
			})
			.into_owned()
	}

	pub fn media_url(&self, target: &Url) -> Result<String, MediaUrlError> {
		self.validate_media_target(target)?;
		let target = target.as_str();
		let signature = self.sign(target);
		let encoded = URL_SAFE_NO_PAD.encode(target);
		Ok(format!("{MEDIA_ROUTE}/{signature}/{encoded}"))
	}

	/// Validate an upstream media URL and turn it into a signed local URL.
	pub fn signed_media_url(&self, source: &str) -> Option<String> {
		let decoded = if source.starts_with("//") { format!("https:{source}") } else { source.to_owned() }.replace("&amp;", "&");
		let mut target = Url::parse(&decoded).ok()?;
		target.set_fragment(None);
		self.media_url(&target).ok()
	}

	pub fn decode_target(&self, signature: &str, encoded: &str) -> Result<Url, MediaUrlError> {
		let target = self.decode_signed_target(signature, encoded)?;
		self.validate_media_target(&target)?;
		Ok(target)
	}

	pub fn rewrite_navigation(&self, url: &str) -> Option<String> {
		rewrite_navigation(url, &self.domains)
	}

	pub fn validate_media_target(&self, target: &Url) -> Result<(), MediaUrlError> {
		validate_media_target_with(target, &self.domains)
	}

	fn decode_signed_target(&self, signature: &str, encoded: &str) -> Result<Url, MediaUrlError> {
		let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| MediaUrlError::InvalidEncoding)?;
		let target = std::str::from_utf8(&bytes).map_err(|_| MediaUrlError::InvalidEncoding)?;
		let supplied = URL_SAFE_NO_PAD.decode(signature).map_err(|_| MediaUrlError::InvalidSignature)?;
		let verifier = self.authenticator(target);
		verifier.verify_slice(&supplied).map_err(|_| MediaUrlError::InvalidSignature)?;
		Url::parse(target).map_err(|_| MediaUrlError::InvalidTarget)
	}

	pub fn rewrite_hls(&self, manifest: &str, source: &Url) -> Result<String, MediaUrlError> {
		self.rewrite_hls_with(manifest, |reference| self.rewrite_media_reference(reference, source))
	}

	pub(crate) fn rewrite_hls_with<E, F>(&self, manifest: &str, mut rewrite: F) -> Result<String, E>
	where
		F: FnMut(&str) -> Result<String, E>,
	{
		let manifest = rewrite_captures(manifest, &HLS_URI, 1, &mut rewrite)?;
		let mut rewritten = String::with_capacity(manifest.len());
		for line in manifest.split_inclusive('\n') {
			let (content, ending) = line
				.strip_suffix("\r\n")
				.map_or_else(|| line.strip_suffix('\n').map_or((line, ""), |content| (content, "\n")), |content| (content, "\r\n"));
			if content.is_empty() || content.starts_with('#') {
				rewritten.push_str(content);
			} else {
				rewritten.push_str(&rewrite(content)?);
			}
			rewritten.push_str(ending);
		}
		Ok(rewritten)
	}

	pub fn rewrite_dash(&self, manifest: &str, source: &Url) -> Result<String, MediaUrlError> {
		rewrite_captures(manifest, &DASH_BASE_URL, 2, |reference| self.rewrite_media_reference(reference, source))
	}

	fn rewrite_absolute_url(&self, original: &str) -> Option<String> {
		let mut target = parse_http_url(original)?;
		let host = target.host_str()?.to_ascii_lowercase();

		if self.domains.is_media_host(&host) {
			let fragment = target.fragment().map(str::to_owned);
			target.set_fragment(None);
			target.set_scheme("https").ok()?;
			target.set_port(None).ok()?;
			let mut rewritten = self.media_url(&target).ok()?;
			if let Some(fragment) = fragment {
				rewritten.push('#');
				rewritten.push_str(&fragment);
			}
			return Some(rewritten);
		}
		rewrite_navigation_target(&mut target, &self.domains)
	}

	fn rewrite_media_reference(&self, reference: &str, source: &Url) -> Result<String, MediaUrlError> {
		if reference.is_empty() || reference.starts_with("data:") {
			return Ok(reference.to_string());
		}
		let reference = reference.replace("&amp;", "&");
		let target = source.join(&reference).map_err(|_| MediaUrlError::InvalidManifestUrl)?;
		self.media_url(&target)
	}

	fn sign(&self, target: &str) -> String {
		URL_SAFE_NO_PAD.encode(self.authenticator(target).finalize().into_bytes())
	}

	fn authenticator(&self, target: &str) -> HmacSha256 {
		let mut signer = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts keys of any size");
		signer.update(self.policy_scope.as_bytes());
		signer.update(&[0]);
		signer.update(target.as_bytes());
		signer
	}
}

fn rewrite_navigation(url: &str, domains: &DomainPolicy) -> Option<String> {
	let mut target = parse_http_url(url)?;
	rewrite_navigation_target(&mut target, domains)
}

fn parse_http_url(value: &str) -> Option<Url> {
	let decoded = if value.starts_with("//") { format!("https:{value}") } else { value.to_owned() }.replace("&amp;", "&");
	let target = Url::parse(&decoded).ok()?;
	(matches!(target.scheme(), "http" | "https") && target.username().is_empty() && target.password().is_none()).then_some(target)
}

fn rewrite_navigation_target(target: &mut Url, domains: &DomainPolicy) -> Option<String> {
	let host = target.host_str()?.to_ascii_lowercase();
	if domains.is_navigation_host(&host) {
		Some(local_path(target))
	} else if domains.is_shortlink_host(&host) {
		let path = target.path().trim_start_matches('/');
		target.set_path(&format!("/comments/{path}"));
		Some(local_path(target))
	} else {
		None
	}
}

fn validate_media_target_with(target: &Url, domains: &DomainPolicy) -> Result<(), MediaUrlError> {
	let host = target.host_str().ok_or(MediaUrlError::InvalidTarget)?.to_ascii_lowercase();
	if target.scheme() != "https" || !target.username().is_empty() || target.password().is_some() || target.port().is_some() || !domains.is_media_host(&host) {
		return Err(MediaUrlError::ForbiddenTarget);
	}
	Ok(())
}

fn parse_patterns(values: &[String]) -> Result<Vec<HostPattern>, MediaUrlError> {
	values.iter().map(|value| HostPattern::parse(value)).collect()
}

impl HostPattern {
	fn parse(value: &str) -> Result<Self, MediaUrlError> {
		let (subdomains_only, value) = value.strip_prefix("*.").map_or((false, value), |value| (true, value));
		if value.is_empty() || value.contains('*') {
			return Err(MediaUrlError::InvalidDomainPattern(value.to_owned()));
		}
		let Host::Domain(domain) = Host::parse(value).map_err(|_| MediaUrlError::InvalidDomainPattern(value.to_owned()))? else {
			return Err(MediaUrlError::InvalidDomainPattern(value.to_owned()));
		};
		Ok(Self {
			domain: domain.to_ascii_lowercase(),
			subdomains_only,
		})
	}

	fn matches(&self, host: &str) -> bool {
		if self.subdomains_only {
			host.strip_suffix(&self.domain).is_some_and(|prefix| prefix.ends_with('.') && prefix.len() > 1)
		} else {
			host == self.domain
		}
	}
}

fn local_path(target: &Url) -> String {
	let mut path = target
		.path()
		.strip_prefix("/gallery/")
		.map_or_else(|| target.path().to_string(), |id| format!("/comments/{id}"));
	if let Some(query) = target.query() {
		path.push('?');
		path.push_str(query);
	}
	if let Some(fragment) = target.fragment() {
		path.push('#');
		path.push_str(fragment);
	}
	path
}

fn trim_url_punctuation(url: &str) -> (&str, &str) {
	let mut split = url.len();
	while let Some(character) = url[..split].chars().next_back() {
		let unmatched_closer = match character {
			')' => url[..split].matches(')').count() > url[..split].matches('(').count(),
			']' => url[..split].matches(']').count() > url[..split].matches('[').count(),
			'}' => url[..split].matches('}').count() > url[..split].matches('{').count(),
			_ => false,
		};
		if unmatched_closer || matches!(character, '.' | ',' | ';' | '!') {
			split -= character.len_utf8();
		} else {
			break;
		}
	}
	url.split_at(split)
}

fn rewrite_captures<F, E>(input: &str, pattern: &Regex, capture: usize, mut rewrite: F) -> Result<String, E>
where
	F: FnMut(&str) -> Result<String, E>,
{
	let mut output = String::with_capacity(input.len());
	let mut end = 0;
	for captures in pattern.captures_iter(input) {
		let matched = captures.get(capture).expect("the media pattern contains the requested capture");
		output.push_str(&input[end..matched.start()]);
		output.push_str(&rewrite(matched.as_str())?);
		end = matched.end();
	}
	output.push_str(&input[end..]);
	Ok(output)
}

#[derive(Debug, Error)]
pub enum MediaUrlError {
	#[error("media signing secrets must contain at least 32 bytes")]
	ShortSecret,
	#[error("invalid media URL encoding")]
	InvalidEncoding,
	#[error("invalid media URL signature")]
	InvalidSignature,
	#[error("invalid media target URL")]
	InvalidTarget,
	#[error("media target is not an allowed Reddit host")]
	ForbiddenTarget,
	#[error("invalid URL in Reddit media manifest")]
	InvalidManifestUrl,
	#[error("invalid domain pattern `{0}`")]
	InvalidDomainPattern(String),
}

#[derive(Debug, Error)]
pub enum MediaSignerLoadError {
	#[error("failed to read the media signing secret")]
	Read(#[from] std::io::Error),
	#[error(transparent)]
	InvalidSecret(#[from] MediaUrlError),
}
