//! The Response cache (ADR 0046): one per user and per major, in the
//! per-major folder, which an upgrade never copies, so it starts empty. A
//! SQLite index keys each response by its partition, the method, the URL
//! without its fragment and the request headers its `Vary` names; its body
//! lies beside it in a file named by its SHA-256, which one body shared by
//! several entries is stored once as.
//!
//! This minor builds the public partition: GET and HEAD responses sent with
//! no cookie or credential, which set no cookie and are not `private`
//! ([`freshness`] says which are storable). Each partition also carries the
//! person, the proxy's exit country, the language and the device class it
//! was fetched with, so a page fetched for one never answers another. A
//! fresh entry is served with no request, a stale one revalidated with its
//! validators, its stored body served on a 304. Under `replay` every
//! response in the partition is stored, whatever its method, status or
//! freshness, its key adding the SHA-256 of its request's canonical body
//! ([`canonical`]), and served again as stored with no request; the two
//! modes' entries never answer each other. Its callers never store a
//! blocked request, an Effect, a payment, a Secret's response, one fetched
//! under a certificate exception or a cut body; each entry records the Run
//! that fetched it.
//!
//! The index opens through a VFS named by a value, none in production, as
//! every store does, once for its writes, on a writer, and once for its
//! lookups, on its owner's blocking threads. An engine that finds an index
//! of a format version it cannot read runs without the cache (rule
//! `run_without`, ADR 0030): it neither reads nor writes it, says so once
//! in its log, and every lookup misses and every store is skipped, so the
//! Run goes on. A cache failure never fails a fetch: it is logged, and the
//! lookup misses or the store is skipped.
//!
//! Counted work (ADR 0140): a lookup that queries the index counts once on
//! `C5.cache_lookups`, a body file opened to serve a hit once on
//! `C5.cache_file_opens`, and a body file written, or the bodies' folder
//! made, on `C5.cache_body_writes`; a miss opens no body file.

mod canonical;
pub mod freshness;
pub mod source;

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::formats::{Compatibility, RESPONSE_CACHE};
use goodfetch_protocol::point_in_time;
use goodfetch_protocol::run::RunId;
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};

pub use self::freshness::{CacheRequest, ResponseHead};
use self::freshness::{Freshness, Stored};
pub use self::source::{Recording, Replaying, ResponseSource, Served, Source};
use crate::sqlite_store::{
    self, Access, Durability, File, Folder, FormatRefusal, FullFsync, KeptStore, OpenError,
    Opening, Schema, StoreFile, Synchronous, Wait, WalOnClose,
};
use crate::sync::Mutex;
use crate::tasks::{ActorStopped, TaskOwner};

/// The index's file in the cache's folder.
const INDEX: &str = "index.sqlite";

/// The body files' folder in the cache's folder.
const BODIES: &str = "bodies";

/// The one partition this minor stores (ADR 0046); the Session partition
/// joins with #795.
const PUBLIC: &str = "public";

/// How long a write waits for another engine's, and a lookup while SQLite
/// answers busy.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// A cache's commits survive a process crash; a power cut may roll the
/// last back, which loses entries a later fetch stores again, never the
/// file.
const DURABILITY: Durability = Durability {
    synchronous: Synchronous::Normal,
    opening: Synchronous::Normal,
    full_fsync: FullFsync::Off,
    wal_on_close: WalOnClose::Kept,
};

/// The tables of this version.
const SCHEMA: &[&str] = &["CREATE TABLE IF NOT EXISTS entries (
        partition TEXT NOT NULL,
        person TEXT NOT NULL,
        country TEXT NOT NULL,
        language TEXT NOT NULL,
        device TEXT NOT NULL,
        method TEXT NOT NULL,
        url TEXT NOT NULL,
        vary_key TEXT NOT NULL,
        replay_key TEXT NOT NULL,
        request_headers TEXT NOT NULL,
        status INTEGER NOT NULL,
        response_headers TEXT NOT NULL,
        fetched_at TEXT NOT NULL,
        body_sha256 TEXT NOT NULL,
        body_size INTEGER NOT NULL,
        run_id TEXT,
        used_at TEXT NOT NULL,
        PRIMARY KEY (partition, person, country, language, device, method, url, vary_key, replay_key)
    ) STRICT"];

/// A URL's entries in one partition under one replay key, empty for
/// `revalidate`'s, the newest first.
const FIND: &str = "SELECT vary_key, request_headers, status, response_headers, fetched_at, body_sha256, body_size, run_id FROM entries WHERE partition = ?1 AND person = ?2 AND country = ?3 AND language = ?4 AND device = ?5 AND method = ?6 AND url = ?7 AND replay_key = ?8 ORDER BY fetched_at DESC";

/// The replay key of `revalidate`'s entries.
const REVALIDATED: &str = "";

/// Who a response was fetched for and how, which keeps one partition's
/// entries from answering another's requests (ADR 0046).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Partition {
    /// The person it was fetched for.
    pub person: String,
    /// The proxy's exit country, as the Proxy declares it; empty when it
    /// left directly or the Proxy declares none.
    pub country: String,
    /// The Identity profile's language; empty without one.
    pub language: String,
    /// The Identity profile's device class; empty without one.
    pub device: String,
}

/// A response the cache serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Its head: the stored headers, its `Age` its own.
    pub head: ResponseHead,
    /// Its body, as stored.
    pub body: Bytes,
    /// The Run that fetched it; none outside a Run.
    pub run_id: Option<RunId>,
}

/// What the cache holds for a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// Nothing that answers it: the request is sent as it is.
    Miss,
    /// A fresh response, served with no request.
    Fresh(Hit),
    /// A stale response: the request is sent with its conditions, and a 304
    /// answering it is handed to [`ResponseCache::revalidated`].
    Stale(Stale),
}

/// A stale entry, and the conditions its revalidation sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stale {
    /// The conditional headers to add to the request, names in lower case:
    /// `if-none-match` from its `ETag`, `if-modified-since` from its
    /// `Last-Modified`.
    pub conditions: Vec<(String, String)>,
    entry: Box<Entry>,
}

/// An entry of the index, as a lookup read it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    partition: Partition,
    vary_key: String,
    /// The SHA-256 of the request's canonical body under `replay`;
    /// [`REVALIDATED`] for `revalidate`'s.
    replay_key: String,
    stored: Stored,
    body_sha256: String,
    body_size: u64,
    run_id: Option<RunId>,
}

/// Why the cache could not be read or written: logged, never a fetch's
/// failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ResponseCacheError {
    /// SQLite failed; a write committed nothing.
    #[error("the Response cache's index at `{path}`: {message}")]
    Sqlite {
        /// The index's file.
        path: PathBuf,
        /// SQLite's message.
        message: String,
    },
    /// A body file could not be read or written.
    #[error("the Response cache's body file `{path}`: {message}")]
    Body {
        /// The body's file.
        path: PathBuf,
        /// The OS's message.
        message: String,
    },
    /// The engine's writer of the index stopped with its owner.
    #[error("the Response cache's index at `{path}` is not running: {message}")]
    NotRunning {
        /// The index's file.
        path: PathBuf,
        /// Why.
        message: String,
    },
    /// The index is of a format version this engine cannot read (ADR 0030).
    #[error("the Response cache is {found}, which this engine cannot read")]
    Format {
        /// The index's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
}

/// The Response cache of one user and one major, as one engine opens it:
/// its writer and the connection its lookups keep, which its clones share.
#[derive(Clone, Debug)]
pub struct ResponseCache {
    store: Arc<KeptStore<CacheFile>>,
    /// Set once the index was found of a version this engine cannot read.
    without: Arc<OnceLock<()>>,
}

/// The cache's folder, its index and the VFS the index opens through.
#[derive(Debug)]
struct CacheFile {
    folder: PathBuf,
    index: PathBuf,
    vfs: Option<String>,
    work: CountedWork,
    /// Set once the bodies' folder is known to be there.
    bodies_made: OnceLock<()>,
    /// Numbers this engine's partial body files.
    partials: Mutex<u64>,
}

impl ResponseCache {
    /// The cache in `folder`, its index opened through the SQLite VFS named
    /// `vfs` (none in production, ADR 0060), its writer an actor of `owner`
    /// and its lookups on `owner`'s blocking threads, its work counted on
    /// `work`. Nothing is opened before its first call.
    pub fn of(
        folder: PathBuf,
        vfs: Option<String>,
        owner: Arc<TaskOwner>,
        work: CountedWork,
    ) -> Self {
        let file = CacheFile {
            index: folder.join(INDEX),
            folder,
            vfs,
            work,
            bodies_made: OnceLock::new(),
            partials: Mutex::new(0),
        };
        Self {
            store: Arc::new(KeptStore::of(file, "response-cache-writer", owner)),
            without: Arc::default(),
        }
    }

    /// Whether this engine runs without the cache, having found it of a
    /// format version it cannot read.
    pub fn running_without(&self) -> bool {
        self.without.get().is_some()
    }

    /// What the cache holds for `request` in `partition` at `now`: a
    /// fresh response, served with no request, a stale one to revalidate,
    /// or nothing. A cache never written holds nothing, and is not created.
    pub async fn lookup(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        now: SystemTime,
    ) -> Lookup {
        if self.running_without() || !freshness::public(request) {
            return Lookup::Miss;
        }
        let (partition, request) = (partition.clone(), request.clone());
        let looked = self
            .store
            .read(Some(Lookup::Miss), move |file, connection| {
                file.lookup(connection, &partition, &request, now)
            })
            .await;
        self.settled(looked).unwrap_or(Lookup::Miss)
    }

    /// The hit `head` makes of `stale` when it is a 304 answering the
    /// revalidation of `request` at `now`: the stored body with the
    /// stored headers the 304 updates, which the entry keeps from now on.
    /// None when `head` is no such answer, or the entry or its body is
    /// gone: the caller then stores the response it got, or sends the
    /// request again without the conditions.
    pub async fn revalidated(
        &self,
        stale: Stale,
        request: &CacheRequest,
        head: &ResponseHead,
        now: SystemTime,
    ) -> Option<Hit> {
        if self.running_without() {
            return None;
        }
        let entry = stale.entry;
        let headers = freshness::not_modified(&entry.stored, request, head, now)?;
        let updated = self
            .store
            .write(None, move |file, connection| {
                file.revalidated(connection, &entry, &headers, now)
            })
            .await;
        self.settled(updated).flatten()
    }

    /// Stores `head` and `body`, the response to `request` in `partition`
    /// received at `now` in the Run `run_id`, in place of what the cache
    /// held for its variant; whether it was stored. A response the public
    /// partition does not store is skipped, as every one is while the
    /// engine runs without the cache.
    pub async fn store(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        head: &ResponseHead,
        body: Bytes,
        run_id: Option<&RunId>,
        now: SystemTime,
    ) -> bool {
        if self.running_without() {
            return false;
        }
        let Some(variant) = freshness::storable(request, head) else {
            return false;
        };
        let keyed = (variant, REVALIDATED.to_owned());
        self.put(entry(partition, request, head, keyed, run_id, now), body)
            .await
    }

    /// The response `replay` stored for `request` in `partition`, served
    /// as it was stored with no request, whatever its age: the entry of
    /// its method, its URL, its canonical body and the variant its stored
    /// `Vary` names; none when the cache holds no such entry.
    pub async fn replayed(&self, partition: &Partition, request: &CacheRequest) -> Option<Hit> {
        if self.running_without() || freshness::credentialed(request) {
            return None;
        }
        // Hashed once, before the request is handed to the reader.
        request.replay_key();
        let (partition, request) = (partition.clone(), request.clone());
        let looked = self
            .store
            .read(Some(None), move |file, connection| {
                file.replayed(connection, &partition, &request)
            })
            .await;
        self.settled(looked).flatten()
    }

    /// Stores `head` and `body` under `replay`, the response to `request`
    /// in `partition` received at `now` in the Run `run_id`, keyed by its
    /// canonical body and in place of what `replay` held for it; whether
    /// it was stored. Any method, status and freshness is stored, but
    /// never a response the public partition does not hold.
    pub async fn store_replayed(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        head: &ResponseHead,
        body: Bytes,
        run_id: Option<&RunId>,
        now: SystemTime,
    ) -> bool {
        if self.running_without() {
            return false;
        }
        let Some(variant) = freshness::replayable(request, head) else {
            return false;
        };
        let keyed = (variant, request.replay_key().to_owned());
        self.put(entry(partition, request, head, keyed, run_id, now), body)
            .await
    }

    /// Stores `entry` with `body`, in place of what the cache held under
    /// its key; whether it was stored.
    async fn put(&self, entry: Entry, body: Bytes) -> bool {
        let entry = Entry {
            body_size: u64::try_from(body.len()).unwrap_or(u64::MAX),
            ..entry
        };
        let stored = self
            .store
            .write(None, move |file, connection| {
                // Hashed on the writer's thread, off the caller's runtime.
                let entry = Entry {
                    body_sha256: data_encoding::HEXLOWER.encode(&Sha256::digest(&body)),
                    ..entry
                };
                file.store(connection, &entry, &body)
            })
            .await;
        self.settled(stored).is_some()
    }

    /// `answer`'s value; none for an error, which is logged, and after
    /// which a format refusal has the engine run without the cache.
    fn settled<T>(&self, answer: Result<T, ResponseCacheError>) -> Option<T> {
        match answer {
            Ok(value) => Some(value),
            Err(error @ ResponseCacheError::Format { .. }) => {
                if self.without.set(()).is_ok() {
                    tracing::warn!(%error, "running without the Response cache");
                }
                None
            }
            Err(error) => {
                tracing::warn!(%error, "the Response cache failed: the fetch goes on without it");
                None
            }
        }
    }
}

impl CacheFile {
    fn lookup(
        &self,
        connection: &Connection,
        partition: &Partition,
        request: &CacheRequest,
        now: SystemTime,
    ) -> Result<Lookup, ResponseCacheError> {
        self.work.count(Row::CacheLookups);
        let rows = self.find(connection, partition, request, REVALIDATED)?;
        for entry in rows {
            match freshness::freshness(&entry.stored, request, now) {
                Freshness::Other => {}
                Freshness::Fresh { age } => {
                    let Some(body) = self.body(&entry)? else {
                        return Ok(Lookup::Miss);
                    };
                    return Ok(Lookup::Fresh(Hit {
                        head: ResponseHead {
                            status: entry.stored.head.status,
                            headers: freshness::served_headers(&entry.stored.head.headers, age),
                        },
                        body,
                        run_id: entry.run_id,
                    }));
                }
                Freshness::Stale { conditions } => {
                    return Ok(Lookup::Stale(Stale {
                        conditions,
                        entry: Box::new(entry),
                    }));
                }
            }
        }
        Ok(Lookup::Miss)
    }

    /// The entry `replay` stored for `request`: its stored head, served as
    /// it is, and its body, whatever their age; none when no entry's
    /// variant is the one `request` asks for, or its body is gone.
    fn replayed(
        &self,
        connection: &Connection,
        partition: &Partition,
        request: &CacheRequest,
    ) -> Result<Option<Hit>, ResponseCacheError> {
        self.work.count(Row::CacheLookups);
        let rows = self.find(connection, partition, request, request.replay_key())?;
        for entry in rows {
            let asked = freshness::variant(request, &entry.stored.head);
            if asked.is_none_or(|asked| asked.vary_key != entry.vary_key) {
                continue;
            }
            let Some(body) = self.body(&entry)? else {
                return Ok(None);
            };
            return Ok(Some(Hit {
                head: entry.stored.head,
                body,
                run_id: entry.run_id,
            }));
        }
        Ok(None)
    }

    /// `request`'s entries in `partition` under `replay_key`, the newest
    /// first; a row this engine cannot read is left out.
    fn find(
        &self,
        connection: &Connection,
        partition: &Partition,
        request: &CacheRequest,
        replay_key: &str,
    ) -> Result<Vec<Entry>, ResponseCacheError> {
        let mut statement = connection
            .prepare_cached(FIND)
            .map_err(|error| self.failed(&error))?;
        let rows = statement
            .query_map(
                (
                    PUBLIC,
                    &partition.person,
                    &partition.country,
                    &partition.language,
                    &partition.device,
                    &request.method,
                    &request.url,
                    replay_key,
                ),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1).ok(),
                        row.get::<_, i64>(2).ok(),
                        row.get::<_, String>(3).ok(),
                        row.get::<_, String>(4).ok(),
                        row.get::<_, String>(5).ok(),
                        row.get::<_, i64>(6).ok(),
                        row.get::<_, Option<String>>(7).ok(),
                    ))
                },
            )
            .map_err(|error| self.failed(&error))?;
        let mut entries = Vec::new();
        for row in rows {
            let (
                vary_key,
                request_headers,
                status,
                response_headers,
                fetched_at,
                sha256,
                size,
                run_id,
            ) = row.map_err(|error| self.failed(&error))?;
            let read = || {
                Some(Entry {
                    partition: partition.clone(),
                    stored: Stored {
                        method: request.method.clone(),
                        url: request.url.clone(),
                        request_headers: serde_json::from_str(&request_headers?).ok()?,
                        head: ResponseHead {
                            status: u16::try_from(status?).ok()?,
                            headers: serde_json::from_str(&response_headers?).ok()?,
                        },
                        fetched_at: point_in_time::from_rfc3339(&fetched_at?).ok()?,
                    },
                    vary_key: vary_key.clone(),
                    replay_key: replay_key.to_owned(),
                    body_sha256: sha256.filter(|sha256| is_sha256(sha256))?,
                    body_size: u64::try_from(size?).ok()?,
                    run_id: match run_id? {
                        Some(run_id) => Some(RunId::parse(&run_id).ok()?),
                        None => None,
                    },
                })
            };
            entries.extend(read());
        }
        Ok(entries)
    }

    fn revalidated(
        &self,
        connection: &Connection,
        entry: &Entry,
        headers: &[(String, String)],
        now: SystemTime,
    ) -> Result<Option<Hit>, ResponseCacheError> {
        let at = rfc3339(now);
        let updated = connection
            .prepare_cached(
                "UPDATE entries SET response_headers = ?1, fetched_at = ?2, used_at = ?2 WHERE partition = ?3 AND person = ?4 AND country = ?5 AND language = ?6 AND device = ?7 AND method = ?8 AND url = ?9 AND vary_key = ?10 AND body_sha256 = ?11 AND replay_key = ?12",
            )
            .and_then(|mut statement| {
                statement.execute(rusqlite::params![
                    json(headers),
                    at,
                    PUBLIC,
                    entry.partition.person,
                    entry.partition.country,
                    entry.partition.language,
                    entry.partition.device,
                    entry.stored.method,
                    entry.stored.url,
                    entry.vary_key,
                    entry.body_sha256,
                    entry.replay_key,
                ])
            })
            .map_err(|error| self.failed(&error))?;
        if updated == 0 {
            return Ok(None);
        }
        let Some(body) = self.body(entry)? else {
            return Ok(None);
        };
        Ok(Some(Hit {
            head: ResponseHead {
                status: entry.stored.head.status,
                headers: freshness::served_headers(headers, Duration::ZERO),
            },
            body,
            run_id: entry.run_id.clone(),
        }))
    }

    fn store(
        &self,
        connection: &Connection,
        entry: &Entry,
        body: &[u8],
    ) -> Result<(), ResponseCacheError> {
        self.write_body(&entry.body_sha256, body)?;
        let at = rfc3339(entry.stored.fetched_at);
        connection
            .prepare_cached(
                "INSERT INTO entries (partition, person, country, language, device, method, url, vary_key, request_headers, status, response_headers, fetched_at, body_sha256, body_size, run_id, used_at, replay_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?12, ?16)
                 ON CONFLICT (partition, person, country, language, device, method, url, vary_key, replay_key) DO UPDATE SET
                 request_headers = excluded.request_headers, status = excluded.status, response_headers = excluded.response_headers, fetched_at = excluded.fetched_at, body_sha256 = excluded.body_sha256, body_size = excluded.body_size, run_id = excluded.run_id, used_at = excluded.used_at",
            )
            .and_then(|mut statement| {
                statement.execute(rusqlite::params![
                    PUBLIC,
                    entry.partition.person,
                    entry.partition.country,
                    entry.partition.language,
                    entry.partition.device,
                    entry.stored.method,
                    entry.stored.url,
                    entry.vary_key,
                    json(&entry.stored.request_headers),
                    entry.stored.head.status,
                    json(&entry.stored.head.headers),
                    at,
                    entry.body_sha256,
                    i64::try_from(entry.body_size).unwrap_or(i64::MAX),
                    entry.run_id.as_ref().map(RunId::as_str),
                    entry.replay_key,
                ])
            })
            .map(drop)
            .map_err(|error| self.failed(&error))
    }

    /// The body of `entry`, read whole from its file; none when its file is
    /// gone or holds another length, which then misses.
    fn body(&self, entry: &Entry) -> Result<Option<Bytes>, ResponseCacheError> {
        let path = self.folder.join(BODIES).join(&entry.body_sha256);
        let mut file = match self.work.open_file(
            Row::CacheFileOpens,
            std::fs::OpenOptions::new().read(true),
            &path,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(body_failed(&path, &error)),
        };
        let mut body = Vec::with_capacity(usize::try_from(entry.body_size).unwrap_or(0));
        file.read_to_end(&mut body)
            .map_err(|error| body_failed(&path, &error))?;
        Ok((u64::try_from(body.len()).ok() == Some(entry.body_size)).then(|| Bytes::from(body)))
    }

    /// Writes `body` as the file `sha256` names, unless it is there: to a
    /// partial file of this engine's, then renamed, so a reader never meets
    /// half a body.
    fn write_body(&self, sha256: &str, body: &[u8]) -> Result<(), ResponseCacheError> {
        let bodies = self.folder.join(BODIES);
        let path = bodies.join(sha256);
        if path.exists() {
            return Ok(());
        }
        if self.bodies_made.get().is_none() {
            self.work
                .create_dir_all(Row::CacheBodyWrites, &bodies)
                .map_err(|error| body_failed(&bodies, &error))?;
            let _ = self.bodies_made.set(());
        }
        let number = {
            let mut partials = self.partials.lock().unwrap_or_else(PoisonError::into_inner);
            *partials += 1;
            *partials
        };
        let partial = bodies.join(format!("{sha256}.{}-{number}.partial", std::process::id()));
        let written = self
            .work
            .open_file(
                Row::CacheBodyWrites,
                std::fs::OpenOptions::new().write(true).create_new(true),
                &partial,
            )
            .and_then(|mut file| file.write_all(body));
        #[expect(
            clippy::disallowed_methods,
            reason = "the writer's job is blocking work, on its own thread"
        )]
        let renamed = written.and_then(|()| std::fs::rename(&partial, &path));
        if let Err(error) = renamed {
            #[expect(
                clippy::disallowed_methods,
                reason = "the writer's job is blocking work, on its own thread"
            )]
            let _ = std::fs::remove_file(&partial);
            return Err(body_failed(&path, &error));
        }
        Ok(())
    }

    /// A connection to the index opened with `access`, waiting up to
    /// [`BUSY_TIMEOUT`] where SQLite answers busy.
    fn connect(&self, access: Access) -> Result<Connection, ResponseCacheError> {
        let connection = File::new(&self.index, self.vfs.as_deref())
            .connect(access)
            .map_err(|error| self.failed(&error))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| self.failed(&error))?;
        Ok(connection)
    }

    fn failed(&self, error: &dyn std::fmt::Display) -> ResponseCacheError {
        ResponseCacheError::Sqlite {
            path: self.index.clone(),
            message: error.to_string(),
        }
    }
}

impl StoreFile for CacheFile {
    type Error = ResponseCacheError;

    fn path(&self) -> &Path {
        &self.index
    }

    /// Opens the index, creating its folder, it and its tables when it does
    /// not exist, and refuses one of a version this engine cannot read,
    /// whose transaction is rolled back, so nothing is written to it.
    fn open(&self) -> Result<Connection, ResponseCacheError> {
        File::new(&self.index, self.vfs.as_deref())
            .open(&Opening {
                folder: Folder::Make,
                access: Access::Create,
                wait: Wait::Busy(BUSY_TIMEOUT),
                durability: DURABILITY,
                counting: None,
                schema: Some(Schema {
                    format: &RESPONSE_CACHE,
                    statements: &[SCHEMA],
                }),
            })
            .map_err(|error| match error {
                OpenError::Format(refusal) => refused(refusal),
                other => self.failed(&other),
            })
    }

    /// Opens the existing index for its lookups, refusing one of a version
    /// this engine cannot read: read-only, so it takes no write lock and
    /// never waits for a writer. An index whose tables are not all there
    /// yet has them made, through [`StoreFile::open`], whose connection
    /// then reads.
    fn open_read(&self) -> Result<Connection, ResponseCacheError> {
        let connection = self.connect(Access::ReadOnly)?;
        let tables: i64 = connection
            .prepare_cached(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('format', 'entries')",
            )
            .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
            .map_err(|error| self.failed(&error))?;
        if tables < 2 {
            drop(connection);
            return self.open();
        }
        let found: String = connection
            .prepare_cached("SELECT version FROM format WHERE name = ?1")
            .and_then(|mut statement| statement.query_row([RESPONSE_CACHE.name], |row| row.get(0)))
            .map_err(|error| self.failed(&error))?;
        sqlite_store::check_format(&RESPONSE_CACHE, &found).map_err(refused)?;
        Ok(connection)
    }

    fn stopped(&self, stopped: &ActorStopped) -> ResponseCacheError {
        ResponseCacheError::NotRunning {
            path: self.index.clone(),
            message: stopped.to_string(),
        }
    }
}

/// The entry `head`, the response to `request` in `partition` received at
/// `now` in the Run `run_id`, is stored as under `keyed`, its variant and
/// its replay key; its body's hash and length set as it is stored.
fn entry(
    partition: &Partition,
    request: &CacheRequest,
    head: &ResponseHead,
    keyed: (freshness::Variant, String),
    run_id: Option<&RunId>,
    now: SystemTime,
) -> Entry {
    let (variant, replay_key) = keyed;
    Entry {
        partition: partition.clone(),
        vary_key: variant.vary_key,
        replay_key,
        stored: Stored {
            method: request.method.clone(),
            url: request.url.clone(),
            request_headers: variant.request_headers,
            head: head.clone(),
            fetched_at: now,
        },
        body_sha256: String::new(),
        body_size: 0,
        run_id: run_id.cloned(),
    }
}

/// Whether `text` is a SHA-256 as lower-case hex, so a damaged row never
/// names a file outside the bodies' folder.
fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn json(headers: &[(String, String)]) -> String {
    serde_json::to_string(headers).expect("invariant: pairs of strings serialize")
}

fn rfc3339(at: SystemTime) -> String {
    crate::clock::rfc3339(at)
}

fn body_failed(path: &Path, error: &std::io::Error) -> ResponseCacheError {
    ResponseCacheError::Body {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

/// The error of an index of a version this engine cannot read (ADR 0030).
fn refused(refusal: FormatRefusal) -> ResponseCacheError {
    ResponseCacheError::Format {
        found: refusal.found,
        compatibility: refusal.compatibility,
    }
}

#[cfg(test)]
mod tests;
