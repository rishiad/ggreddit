#![forbid(unsafe_code)]

use std::{env, io::IsTerminal as _, path::PathBuf};

use clap::Parser;
use neddit::config::{Config, LogFormat, LoggingConfig};
use neddit_api::{
	client::RedditClient,
	media::{video::VideoResolver, MediaSigner},
	server,
	server::MediaProxy,
	service::{ContentPolicy, RedditService},
	storage::Cache,
};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "Neddit", version, about = "Private Reddit-compatible read service")]
struct Args {
	#[arg(short, long, value_name = "FILE", help = "Read configuration from this TOML file")]
	config: Option<PathBuf>,

	#[arg(long, help = "Validate the configuration and exit")]
	check_config: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
	let args = Args::parse();
	let config = Config::load(args.config.as_deref())?;
	if args.check_config {
		println!("configuration is valid");
		return Ok(());
	}

	init_logging(&config.logging)?;
	info!(
		event = "service.starting",
		version = env!("CARGO_PKG_VERSION"),
		web = config.server.web,
		api = config.server.api,
		allow_nsfw = config.server.allow_nsfw,
		video_enabled = config.video.enabled,
		"starting neddit"
	);
	let listener = server::bind(&config.server.listen).await?;
	let address = listener.local_addr()?;

	let domains = config.domain_policy()?;
	let signer = if let Some(path) = &config.media.signing_key_file {
		MediaSigner::from_file(path)?
	} else {
		warn!(
			event = "media.ephemeral_signing_key",
			"no media key file configured; signed media URLs will expire after restart"
		);
		MediaSigner::random()?
	}
	.with_domains(domains.clone())
	.with_content_policy(config.server.allow_nsfw);
	let cache = Cache::open(&config.cache).await?;
	let reddit = RedditClient::new().await?.with_cache(cache.clone());
	let service = RedditService::with_content_policy(reddit.clone(), ContentPolicy::new(config.server.allow_nsfw));
	let video_exclusions = config.video_exclusions()?;
	let video = config.video.enabled.then(|| VideoResolver::new(&config.video.executable, video_exclusions));
	let media = MediaProxy::new(reddit.clone(), signer, video);
	let app = neddit::router(service, &media, config.media.image_display, config.server.web, config.server.api);

	info!(event = "service.ready", %address, version = env!("CARGO_PKG_VERSION"), "neddit is ready");
	let result = server::serve(app, listener).await;
	if let Err(error) = &result {
		error!(event = "service.failed", %error, "HTTP server failed");
	}
	reddit.shutdown().await;
	if result.is_ok() {
		info!(event = "service.stopped", "neddit stopped");
	}
	result?;
	Ok(())
}

fn init_logging(config: &LoggingConfig) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
	let filter = match env::var("RUST_LOG") {
		Ok(filter) => EnvFilter::try_new(filter)?,
		Err(env::VarError::NotPresent) => EnvFilter::try_new(&config.filter)?,
		Err(error) => return Err(Box::new(error)),
	};
	match config.format {
		LogFormat::Compact => tracing_subscriber::fmt()
			.with_env_filter(filter)
			.with_writer(std::io::stderr)
			.with_ansi(std::io::stderr().is_terminal() && env::var_os("NO_COLOR").is_none())
			.compact()
			.try_init()?,
		LogFormat::Json => tracing_subscriber::fmt()
			.with_env_filter(filter)
			.with_writer(std::io::stderr)
			.with_ansi(false)
			.json()
			.try_init()?,
	}
	Ok(())
}
