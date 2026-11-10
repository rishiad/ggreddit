//! Local SQLite stores. Each bounded queue owns one database thread.
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
	path::PathBuf,
	sync::Arc,
	time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, oneshot, Mutex, OwnedMutexGuard};

const MAX_ENTRY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("storage is unavailable")]
	Unavailable,
	#[error("storage capacity reached")]
	Capacity,
	#[error("unsupported database schema version")]
	Schema,
	#[error(transparent)]
	Sql(#[from] rusqlite::Error),
	#[error(transparent)]
	Io(#[from] std::io::Error),
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Clone)]
struct Database(mpsc::Sender<Job>);

impl Database {
	async fn open(path: PathBuf) -> Result<Self, Error> {
		if path.as_os_str().is_empty() || path.to_string_lossy() == ":memory:" || path.to_string_lossy().starts_with("file:") {
			return Err(Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "storage requires a local file path")));
		}
		let (sender, mut receiver) = mpsc::channel::<Job>(64);
		let (ready, opened) = oneshot::channel();
		std::thread::Builder::new().name("neddit-sqlite".into()).spawn(move || {
			let connection = (|| {
				if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
					std::fs::create_dir_all(parent)?;
				}
				private_file(&path)?;
				let mut db = Connection::open(path)?;
				db.busy_timeout(Duration::from_millis(250))?;
				db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
				let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
				let kind: u32 = db.query_row("PRAGMA application_id", [], |r| r.get(0))?;
				let expected = 0x4E444343;
				if version > 1 {
					return Err(Error::Schema);
				}
				if version != 0 && kind != expected {
					return Err(Error::Schema);
				}
				if version == 0 {
					let tx = db.transaction()?;
					tx.execute_batch(
						"CREATE TABLE entries (
                            key TEXT PRIMARY KEY, payload BLOB NOT NULL,
                            created INTEGER NOT NULL, expires INTEGER NOT NULL, bytes INTEGER NOT NULL
                        ); CREATE INDEX entries_expiry ON entries(expires);
                        CREATE INDEX entries_created ON entries(created, key);
                        CREATE TABLE cache_size (bytes INTEGER NOT NULL);
                        INSERT INTO cache_size VALUES (0);
                        CREATE TRIGGER cache_insert AFTER INSERT ON entries BEGIN
                            UPDATE cache_size SET bytes=bytes+new.bytes;
                        END;
                        CREATE TRIGGER cache_delete AFTER DELETE ON entries BEGIN
                            UPDATE cache_size SET bytes=bytes-old.bytes;
                        END;",
					)?;
					tx.execute_batch("PRAGMA user_version=1;")?;
					tx.pragma_update(None, "application_id", expected)?;
					tx.commit()?;
				}
				Ok::<_, Error>(db)
			})();
			match connection {
				Ok(mut db) => {
					if ready.send(Ok(())).is_err() {
						return;
					}
					while let Some(job) = receiver.blocking_recv() {
						job(&mut db);
					}
				}
				Err(error) => {
					let _ = ready.send(Err(error));
				}
			}
		})?;
		opened.await.map_err(|_| Error::Unavailable)??;
		Ok(Self(sender))
	}

	async fn call<T: Send + 'static>(&self, job: impl FnOnce(&mut Connection) -> Result<T, Error> + Send + 'static) -> Result<T, Error> {
		let (send, receive) = oneshot::channel();
		self
			.0
			.try_send(Box::new(move |db| {
				let _ = send.send(job(db));
			}))
			.map_err(|_| Error::Unavailable)?;
		receive.await.map_err(|_| Error::Unavailable)?
	}
}

fn private_file(path: &std::path::Path) -> std::io::Result<()> {
	let mut options = std::fs::OpenOptions::new();
	options.read(true).write(true).create(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}
	options.open(path)?;
	Ok(())
}

pub(crate) fn now() -> i64 {
	SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

pub(crate) fn fingerprint(value: &[u8]) -> String {
	format!("{:x}", Sha256::digest(value))
}

/// Fixed stripes bound lock memory even for adversarial unique requests.
#[derive(Clone)]
pub(crate) struct Flights(Arc<[Arc<Mutex<()>>; 64]>);

impl Default for Flights {
	fn default() -> Self {
		Self(Arc::new(std::array::from_fn(|_| Arc::new(Mutex::new(())))))
	}
}

impl Flights {
	pub(crate) async fn lock(&self, key: &str) -> OwnedMutexGuard<()> {
		let digest = Sha256::digest(key.as_bytes());
		self.0[usize::from(digest[0]) % 64].clone().lock_owned().await
	}
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheConfig {
	pub path: PathBuf,
	pub max_bytes: u64,
}

impl Default for CacheConfig {
	fn default() -> Self {
		Self {
			path: "data/cache.sqlite".into(),
			max_bytes: 256 * 1024 * 1024,
		}
	}
}

#[derive(Clone)]
pub struct Cache {
	db: Database,
	max_bytes: u64,
	flights: Flights,
}

impl Cache {
	pub async fn open(config: &CacheConfig) -> Result<Self, Error> {
		let db = Database::open(config.path.clone()).await?;
		let disk_limit = config.max_bytes.saturating_mul(2).saturating_add(4 * 1024 * 1024);
		db.call(move |db| {
			let page_size: u64 = db.query_row("PRAGMA page_size", [], |r| r.get::<_, u32>(0))?.into();
			db.pragma_update(None, "max_page_count", (disk_limit / page_size).min(u64::from(u32::MAX)) as u32)?;
			db.execute_batch("PRAGMA journal_size_limit=4194304;")?;
			Ok(())
		})
		.await?;
		let cache = Self {
			db,
			max_bytes: config.max_bytes,
			flights: Default::default(),
		};
		let weak = cache.db.0.downgrade();
		tokio::spawn(async move {
			loop {
				tokio::time::sleep(Duration::from_secs(60)).await;
				let Some(sender) = weak.upgrade() else {
					break;
				};
				let result = Database(sender)
					.call(|db| {
						db.execute("DELETE FROM entries WHERE key IN (SELECT key FROM entries WHERE expires <= ?1 LIMIT 256)", [now()])?;
						db.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")?;
						Ok(())
					})
					.await;
				if let Err(error) = result {
					tracing::warn!(event = "cache.maintenance_failed", error = %error, "cache maintenance failed");
				}
			}
		});
		Ok(cache)
	}

	pub(crate) async fn json<E>(&self, key: String, ttl: i64, fetch: impl std::future::Future<Output = Result<serde_json::Value, E>>) -> Result<serde_json::Value, E> {
		let _guard = self.flights.lock(&key).await;
		if let Some(bytes) = self.get(&key).await {
			if let Ok(value) = serde_json::from_slice(&bytes) {
				return Ok(value);
			}
		}
		let value = fetch.await?;
		if let Ok(bytes) = serde_json::to_vec(&value) {
			if let Err(error) = self.put(key, bytes, ttl).await {
				tracing::warn!(event = "cache.write_failed", error = %error, "cache write failed");
			}
		}
		Ok(value)
	}

	pub(crate) async fn get(&self, key: &str) -> Option<Vec<u8>> {
		let key = key.to_owned();
		match self
			.db
			.call(move |db| {
				Ok(
					db.query_row("SELECT payload FROM entries WHERE key=?1 AND expires>?2", params![key, now()], |r| r.get(0))
						.optional()?,
				)
			})
			.await
		{
			Ok(value) => {
				tracing::debug!(event = "cache.read", hit = value.is_some(), "cache read completed");
				value
			}
			Err(error) => {
				tracing::warn!(event = "cache.read_failed", error = %error, "cache read failed");
				None
			}
		}
	}

	pub(crate) async fn put(&self, key: String, payload: Vec<u8>, ttl: i64) -> Result<(), Error> {
		let max = self.max_bytes;
		let charge = (payload.len() + key.len() + 128) as u64;
		if payload.len() > MAX_ENTRY_BYTES || charge > max {
			return Err(Error::Capacity);
		}
		self
			.db
			.call(move |db| {
				let tx = db.transaction()?;
				tx.execute(
					"DELETE FROM entries WHERE key=?1 OR key IN (SELECT key FROM entries WHERE expires<=?2 LIMIT 256)",
					params![key, now()],
				)?;
				let mut size: i64 = tx.query_row("SELECT bytes FROM cache_size", [], |r| r.get(0))?;
				while size as u64 + charge > max {
					tx.execute("DELETE FROM entries WHERE key IN (SELECT key FROM entries ORDER BY created, key LIMIT 64)", [])?;
					size = tx.query_row("SELECT bytes FROM cache_size", [], |r| r.get(0))?;
				}
				tx.execute("INSERT INTO entries VALUES (?1,?2,?3,?4,?5)", params![key, payload, now(), now() + ttl, charge as i64])?;
				tx.commit()?;
				Ok(())
			})
			.await
	}
}
