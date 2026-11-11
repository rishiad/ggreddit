use std::{
	fs,
	net::SocketAddr,
	path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

use neddit_api::media::{DomainPolicy, MediaUrlError};
use neddit_api::storage::CacheConfig;
use neddit_web::ImageDisplay;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
	pub server: ServerConfig,
	pub domains: DomainConfig,
	pub media: MediaConfig,
	pub logging: LoggingConfig,
	pub cache: CacheConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // Independent flags mirror the flat server configuration.
pub struct ServerConfig {
	pub listen: String,
	pub web: bool,
	pub api: bool,
	pub allow_nsfw: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DomainConfig {
	pub navigation: Vec<String>,
	pub shortlinks: Vec<String>,
	pub media: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaConfig {
	pub image_display: ImageDisplay,
	pub signing_key_file: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
	pub filter: String,
	pub format: LogFormat,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
	#[default]
	Compact,
	Json,
}

impl Config {
	/// Load the built-in defaults and apply an optional TOML file.
	///
	/// # Errors
	///
	/// Returns an error when the file cannot be read, parsed, or validated.
	pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
		let config = if let Some(path) = path {
			let source = fs::read_to_string(path).map_err(|source| ConfigError::Read { path: path.to_owned(), source })?;
			toml::from_str(&source).map_err(|source| ConfigError::Parse {
				name: path.display().to_string(),
				source,
			})?
		} else {
			Self::default()
		};
		config.validate()?;
		Ok(config)
	}

	fn validate(&self) -> Result<(), ConfigError> {
		if self.cache.max_bytes < 4 * 1024 * 1024 || self.cache.max_bytes > 1024 * 1024 * 1024 * 1024 {
			return Err(ConfigError::Storage("cache.max_bytes must be between 4 MiB and 1 TiB"));
		}
		if self.cache.path.as_os_str().is_empty() {
			return Err(ConfigError::Storage("cache.path must not be empty"));
		}
		if self.cache.path.to_string_lossy() == ":memory:" || self.cache.path.to_string_lossy().starts_with("file:") {
			return Err(ConfigError::Storage("storage paths must name local files, not SQLite memory databases or URIs"));
		}
		if !self.server.web && !self.server.api {
			return Err(ConfigError::NoSurface);
		}
		self
			.server
			.listen
			.parse::<SocketAddr>()
			.map_err(|_| ConfigError::InvalidListenAddress(self.server.listen.clone()))?;
		self.domain_policy()?;
		tracing_subscriber::EnvFilter::try_new(&self.logging.filter).map_err(|source| ConfigError::LogFilter { source })?;
		Ok(())
	}

	/// Compile the configured host patterns into the shared rewrite policy.
	///
	/// # Errors
	///
	/// Returns an error when a domain contains a scheme, wildcard, port, or IP address.
	pub fn domain_policy(&self) -> Result<DomainPolicy, ConfigError> {
		DomainPolicy::new(&self.domains.navigation, &self.domains.shortlinks, &self.domains.media).map_err(ConfigError::Domains)
	}
}

impl Default for ServerConfig {
	fn default() -> Self {
		Self {
			listen: "127.0.0.1:8080".into(),
			web: true,
			api: true,
			allow_nsfw: true,
		}
	}
}

impl Default for DomainConfig {
	fn default() -> Self {
		Self {
			navigation: vec!["reddit.com".into(), "*.reddit.com".into()],
			shortlinks: vec!["redd.it".into()],
			media: [
				"i.redd.it",
				"v.redd.it",
				"preview.redd.it",
				"external-preview.redd.it",
				"packaged-media.redd.it",
				"redditmedia.com",
				"*.redditmedia.com",
				"redditstatic.com",
				"*.redditstatic.com",
				"reddituploads.com",
				"*.reddituploads.com",
				"reddit-uploaded-media.s3-accelerate.amazonaws.com",
				"reddit-uploaded-video.s3-accelerate.amazonaws.com",
			]
			.into_iter()
			.map(str::to_owned)
			.collect(),
		}
	}
}

impl Default for LoggingConfig {
	fn default() -> Self {
		Self {
			filter: "info".into(),
			format: LogFormat::Compact,
		}
	}
}

#[derive(Debug, Error)]
pub enum ConfigError {
	#[error("{0}")]
	Storage(&'static str),
	#[error("failed to read configuration file `{}`", path.display())]
	Read {
		path: PathBuf,
		#[source]
		source: std::io::Error,
	},
	#[error("failed to parse {name}")]
	Parse {
		name: String,
		#[source]
		source: toml::de::Error,
	},
	#[error("web and API routes cannot both be disabled")]
	NoSurface,
	#[error("server.listen is not a socket address: `{0}`")]
	InvalidListenAddress(String),
	#[error("logging.filter is invalid")]
	LogFilter {
		#[source]
		source: tracing_subscriber::filter::ParseError,
	},
	#[error("domains contain an invalid entry")]
	Domains(#[source] MediaUrlError),
}

#[cfg(test)]
mod tests {
	use super::*;
	use neddit_api::media::MediaSigner;

	#[test]
	fn default_configuration_renders_reddit_images_without_open_proxy() {
		let config = Config::load(None).unwrap();
		assert_eq!(config.server.listen, "127.0.0.1:8080");
		assert_eq!(config.media.image_display, ImageDisplay::Inline);
		let signer = MediaSigner::from_secret(&[7; 32]).unwrap().with_domains(config.domain_policy().unwrap());
		assert!(signer.signed_media_url("https://i.redd.it/qo81j84kpl0i1.jpeg").is_some());
		assert!(signer.signed_media_url("https://evil.example/image.jpeg").is_none());
		assert!(signer.signed_media_url("http://127.0.0.1/private").is_none());
	}

	#[test]
	fn explicit_empty_media_allowlist_stays_empty() {
		let config: Config = toml::from_str("[domains]\nmedia = []").unwrap();
		let signer = MediaSigner::from_secret(&[7; 32]).unwrap().with_domains(config.domain_policy().unwrap());
		assert!(signer.signed_media_url("https://i.redd.it/photo.jpeg").is_none());
	}
}
