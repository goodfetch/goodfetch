//! The Response cache (ADR 0046): one per user and per major, in the
//! per-major folder, which an upgrade never copies, so it starts empty. A
//! SQLite index keys each response by its partition, the method, the URL
//! without its fragment and the request headers its `Vary` names; its body
//! lies beside it in a file named by its SHA-256, which one body shared by
//! several entries is stored once as.
//!
//! A response sent with no cookie or credential, which sets no cookie and
//! is not `private`, is kept in the public partition; any other in its
//! Session's Saved identity's, or for the life of a Session with none in
//! the Session's own ([`partition`]; [`freshness`] says which are
//! storable). Each partition also carries the person, the proxy's exit
//! country, the language and the device class it was fetched with, so a
//! page fetched for one never answers another. A
//! fresh entry is served with no request, a stale one revalidated with its
//! validators, its stored body served on a 304. Under `replay` every
//! response in the partition is stored, whatever its method, status or
//! freshness, its key adding the SHA-256 of its request's canonical body
//! ([`canonical`]), and served again as stored with no request; the two
//! modes' entries never answer each other. What became of a fetch keeps
//! its response out ([`KeptOut`], which [`ResponseSource`] reads): a Block
//! cause or a soft block, a failed Page check, a cut body, a Paid fetch,
//! which is never served either, a resolved secret in the request, or a
//! certificate exception; its callers never store an Effect's. Each entry
//! records the Run that fetched it.
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
//! Its bounds ([`bounds`], ADR 0046) are the Run's: past
//! `cache.max_size`, each store evicts the least recently used entries, a
//! hit counting as a use, and past `cache.max_age_ceiling` an entry is
//! served in no mode and the next store deletes it. The index counts the
//! bytes its bodies hold, each once, as it changes. While the filesystem
//! holding the cache is at `low` or `critical` it stores nothing new (ADR
//! 0122), and still serves and revalidates what it holds.
//!
//! Its upkeep ([`upkeep`]) counts, clears and verifies one person's
//! entries, and deletes a discarded Run's (ADR 0001).
//!
//! Counted work (ADR 0140): a lookup that queries the index counts once on
//! `C5.cache_lookups`, a body file opened to serve a hit once on
//! `C5.cache_file_opens`, and a body file written, or the bodies' folder
//! made, on `C5.cache_body_writes`; a miss opens no body file. A hit's use
//! is written with the others in one transaction later, never one each.
//!
//! Size: one concern, every statement of the one index and the body files
//! they name, so that a store's transaction, which inserts, evicts and
//! frees body files, reads beside the lookups it answers.

mod bounds;
mod canonical;
pub mod freshness;
mod partition;
pub mod source;
mod upkeep;

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::formats::{Compatibility, RESPONSE_CACHE};
use goodfetch_protocol::point_in_time;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::storage::StorageLevel;
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use tokio::sync::watch;

pub use self::bounds::Bounds;
use self::bounds::{Weighed, evictions, index_time};
pub(crate) use self::canonical::body_sha256;
pub use self::freshness::{CacheRequest, ResponseHead};
use self::freshness::{Freshness, Stored};
pub use self::partition::Keeper;
pub use self::source::{KeptOut, Recording, Replaying, ResponseSource, Served, Source};
pub use self::upkeep::Clearing;
use crate::governor::{Held, StorageSample};
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

/// The tables of this version: the entries; each body once, with how many
/// entries hold it; and the bytes the bodies hold, which triggers keep as
/// the entries change. The writer deletes a body no entry holds, then its
/// file.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS entries (
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
    ) STRICT",
    "CREATE INDEX IF NOT EXISTS entries_by_use ON entries (used_at)",
    "CREATE INDEX IF NOT EXISTS entries_by_fetch ON entries (fetched_at)",
    "CREATE TABLE IF NOT EXISTS bodies (
        sha256 TEXT PRIMARY KEY,
        size INTEGER NOT NULL,
        holders INTEGER NOT NULL
    ) STRICT",
    "CREATE INDEX IF NOT EXISTS bodies_unheld ON bodies (sha256) WHERE holders = 0",
    "CREATE TABLE IF NOT EXISTS held (
        one INTEGER PRIMARY KEY CHECK (one = 1),
        bytes INTEGER NOT NULL
    ) STRICT",
    "INSERT OR IGNORE INTO held (one, bytes) VALUES (1, 0)",
    "CREATE TRIGGER IF NOT EXISTS entry_holds_its_body AFTER INSERT ON entries BEGIN
        INSERT OR IGNORE INTO bodies (sha256, size, holders) VALUES (new.body_sha256, new.body_size, 0);
        UPDATE bodies SET holders = holders + 1 WHERE sha256 = new.body_sha256;
    END",
    "CREATE TRIGGER IF NOT EXISTS entry_changes_its_body AFTER UPDATE OF body_sha256 ON entries
        WHEN old.body_sha256 <> new.body_sha256 BEGIN
        UPDATE bodies SET holders = holders - 1 WHERE sha256 = old.body_sha256;
        INSERT OR IGNORE INTO bodies (sha256, size, holders) VALUES (new.body_sha256, new.body_size, 0);
        UPDATE bodies SET holders = holders + 1 WHERE sha256 = new.body_sha256;
    END",
    "CREATE TRIGGER IF NOT EXISTS entry_lets_its_body_go AFTER DELETE ON entries BEGIN
        UPDATE bodies SET holders = holders - 1 WHERE sha256 = old.body_sha256;
    END",
    "CREATE TRIGGER IF NOT EXISTS body_held AFTER INSERT ON bodies BEGIN
        UPDATE held SET bytes = bytes + new.size;
    END",
    "CREATE TRIGGER IF NOT EXISTS body_let_go AFTER DELETE ON bodies BEGIN
        UPDATE held SET bytes = bytes - old.size;
    END",
];

/// The bytes the cache holds: its bodies', each once, and its index's
/// pages in use.
const HELD: &str = "SELECT bytes + (page_count - freelist_count) * page_size FROM held, pragma_page_count(), pragma_freelist_count(), pragma_page_size()";

/// The least recently used entries, as eviction weighs them, the oldest
/// first.
const OLDEST: &str = "SELECT entries.rowid, body_sha256, size, holders FROM entries JOIN bodies ON sha256 = body_sha256 ORDER BY used_at, entries.rowid LIMIT ?1";

/// How many entries eviction reads at a time.
const PAGE: i64 = 64;

/// Records a hit's use.
const USED: &str = "UPDATE entries SET used_at = ?1 WHERE partition = ?2 AND person = ?3 AND country = ?4 AND language = ?5 AND device = ?6 AND method = ?7 AND url = ?8 AND vary_key = ?9 AND replay_key = ?10 AND used_at < ?1";

/// A URL's entries in two partitions, the same one twice to read one,
/// under one replay key, empty for `revalidate`'s, the newest first.
const FIND: &str = "SELECT vary_key, request_headers, status, response_headers, fetched_at, body_sha256, body_size, run_id, partition FROM entries WHERE partition IN (?1, ?9) AND person = ?2 AND country = ?3 AND language = ?4 AND device = ?5 AND method = ?6 AND url = ?7 AND replay_key = ?8 ORDER BY fetched_at DESC";

/// The replay key of `revalidate`'s entries.
const REVALIDATED: &str = "";

/// Who a response was fetched for and how, which keeps one partition's
/// entries from answering another's requests (ADR 0046).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Partition {
    /// Whose partition a response that is not public is kept in: the
    /// Session's Saved identity, or the Session itself; none outside a
    /// Session, such a response then kept nowhere.
    pub keeper: Option<Keeper>,
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
    /// The partition's name in the index ([`partition`]).
    kept_in: String,
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
/// its writer and the connection its lookups keep, which its clones share,
/// under the bounds of the Run that uses it.
#[derive(Clone, Debug)]
pub struct ResponseCache {
    store: Arc<KeptStore<CacheFile>>,
    /// Set once the index was found of a version this engine cannot read.
    without: Arc<OnceLock<()>>,
    /// Where a flush of the hits' uses runs.
    owner: Arc<TaskOwner>,
    bounds: Bounds,
    /// The governor's samples of the filesystem holding the cache; none
    /// when nothing samples it, which then stores at every level.
    storage: Option<Arc<Watched>>,
}

/// The cache's folder, which the governor samples while it is held, and
/// its samples.
#[derive(Debug)]
struct Watched {
    held: Held,
    samples: watch::Receiver<Arc<StorageSample>>,
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
    /// The hits' uses not written yet.
    uses: Mutex<Uses>,
}

/// The hits' uses not written yet, and whether a flush of them is on its
/// way.
#[derive(Debug, Default)]
struct Uses {
    pending: Vec<Use>,
    flushing: bool,
}

/// A hit's use of an entry, which makes it the most recently used.
#[derive(Debug)]
struct Use {
    partition: Partition,
    kept_in: String,
    method: String,
    url: String,
    vary_key: String,
    replay_key: String,
    at: SystemTime,
}

impl ResponseCache {
    /// The cache in `folder`, its index opened through the SQLite VFS named
    /// `vfs` (none in production, ADR 0060), its writer an actor of `owner`
    /// and its lookups on `owner`'s blocking threads, its work counted on
    /// `work`, under the catalog's bounds. Nothing is opened before its
    /// first call.
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
            uses: Mutex::new(Uses::default()),
        };
        Self {
            store: Arc::new(KeptStore::of(
                file,
                "response-cache-writer",
                Arc::clone(&owner),
            )),
            without: Arc::default(),
            owner,
            bounds: Bounds::default(),
            storage: None,
        }
    }

    /// This cache under `bounds`, a Run's.
    #[must_use]
    pub fn bounded(&self, bounds: Bounds) -> Self {
        Self {
            bounds,
            ..self.clone()
        }
    }

    /// This cache, storing nothing new while the filesystem holding its
    /// folder, which `held` has the governor sample, is at `low` or
    /// `critical` in the last of `samples` (ADR 0122).
    #[must_use]
    pub fn watching(self, held: Held, samples: watch::Receiver<Arc<StorageSample>>) -> Self {
        Self {
            storage: Some(Arc::new(Watched { held, samples })),
            ..self
        }
    }

    /// Whether this engine runs without the cache, having found it of a
    /// format version it cannot read.
    pub fn running_without(&self) -> bool {
        self.without.get().is_some()
    }

    /// Whether it stores what it is handed: not while the engine runs
    /// without it, nor while the governor's last sample found its
    /// filesystem at `low` or `critical`.
    pub fn storing(&self) -> bool {
        !self.running_without()
            && self.storage.as_ref().is_none_or(|watched| {
                watched
                    .samples
                    .borrow()
                    .of(watched.held.folder())
                    .is_none_or(|filesystem| filesystem.level < StorageLevel::Low)
            })
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
        if self.running_without() || !freshness::revalidates(request) {
            return Lookup::Miss;
        }
        let Some(read) = partition::read_from(partition.keeper.as_ref(), request) else {
            return Lookup::Miss;
        };
        let (partition, request, bounds) = (partition.clone(), request.clone(), self.bounds);
        let looked = self
            .store
            .read(Some((Lookup::Miss, false)), move |file, connection| {
                file.lookup(connection, &partition, &read, &request, now, bounds)
            })
            .await;
        let (lookup, flush) = self.settled(looked).unwrap_or((Lookup::Miss, false));
        if flush {
            self.flush_uses();
        }
        lookup
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
    /// held for its variant, in the public partition or its keeper's
    /// ([`Keeper`]); whether it was stored. A response that is not
    /// storable, or that is personal with no keeper, is skipped, as every
    /// one is while the engine runs without the cache.
    pub async fn store(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        head: &ResponseHead,
        body: Bytes,
        run_id: Option<&RunId>,
        now: SystemTime,
    ) -> bool {
        let Some(variant) = freshness::storable(request, head) else {
            return false;
        };
        let Some(kept_in) = partition::kept_in(partition.keeper.as_ref(), request, head) else {
            return false;
        };
        let keyed = Keyed {
            kept_in,
            variant,
            replay_key: REVALIDATED.to_owned(),
        };
        self.put(entry(partition, request, head, keyed, run_id, now), body)
            .await
    }

    /// The response `replay` stored for `request` in `partition`, served
    /// as it was stored with no request, whatever its freshness, at `now`:
    /// the entry of its method, its URL, its canonical body and the variant
    /// its stored `Vary` names; none when the cache holds no such entry, or
    /// it has outlived the ceiling.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves a read begun to finish on its own, its
    /// hit lost; the use it recorded waits for the next flush.
    pub async fn replayed(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        now: SystemTime,
    ) -> Option<Hit> {
        if self.running_without() {
            return None;
        }
        let read = partition::read_from(partition.keeper.as_ref(), request)?;
        // Hashed once, before the request is handed to the reader.
        request.replay_key();
        let (partition, request, bounds) = (partition.clone(), request.clone(), self.bounds);
        let looked = self
            .store
            .read(Some((None, false)), move |file, connection| {
                file.replayed(connection, &partition, &read, &request, now, bounds)
            })
            .await;
        let (hit, flush) = self.settled(looked).unwrap_or((None, false));
        if flush {
            self.flush_uses();
        }
        hit
    }

    /// Stores `head` and `body` under `replay`, the response to `request`
    /// in `partition` received at `now` in the Run `run_id`, keyed by its
    /// canonical body and in place of what `replay` held for it; whether
    /// it was stored, in the public partition or its keeper's ([`Keeper`]).
    /// Any method, status and freshness is stored, but never `Vary: *` nor
    /// a personal response with no keeper.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the writer took the store still stores
    /// the response; only the answer is lost.
    pub async fn store_replayed(
        &self,
        partition: &Partition,
        request: &CacheRequest,
        head: &ResponseHead,
        body: Bytes,
        run_id: Option<&RunId>,
        now: SystemTime,
    ) -> bool {
        let Some(variant) = freshness::variant(request, head) else {
            return false;
        };
        let Some(kept_in) = partition::kept_in(partition.keeper.as_ref(), request, head) else {
            return false;
        };
        let keyed = Keyed {
            kept_in,
            variant,
            replay_key: request.replay_key().to_owned(),
        };
        self.put(entry(partition, request, head, keyed, run_id, now), body)
            .await
    }

    /// Stores `entry` with `body`, in place of what the cache held under
    /// its key, and evicts what its bounds then leave no room for; whether
    /// it was stored. Nothing is while the cache is not [`storing`], nor a
    /// body larger than the cap.
    ///
    /// [`storing`]: Self::storing
    async fn put(&self, entry: Entry, body: Bytes) -> bool {
        let entry = Entry {
            body_size: u64::try_from(body.len()).unwrap_or(u64::MAX),
            ..entry
        };
        if !self.storing() || entry.body_size > self.bounds.max_size {
            return false;
        }
        let bounds = self.bounds;
        let stored = self
            .store
            .write(None, move |file, connection| {
                // Hashed on the writer's thread, off the caller's runtime.
                let entry = Entry {
                    body_sha256: data_encoding::HEXLOWER.encode(&Sha256::digest(&body)),
                    ..entry
                };
                file.store(connection, &entry, &body, bounds)
            })
            .await;
        self.settled(stored).is_some()
    }

    /// Writes the hits' uses recorded so far, in one transaction on the
    /// writer, after the lookup that recorded the first of them answered.
    fn flush_uses(&self) {
        let cache = self.clone();
        self.owner.spawn(async move {
            let flushed = cache
                .store
                .write(None, |file, connection| {
                    sqlite_store::immediate(connection, |connection| file.flush(connection))
                        .map_err(|error| file.failed(&error))
                })
                .await;
            cache.settled(flushed);
        });
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
    /// What the cache holds for `request` at `now` in the partitions
    /// `read`, an entry past the ceiling left out, and whether a fresh
    /// hit's use, recorded, needs a flush started.
    fn lookup(
        &self,
        connection: &Connection,
        partition: &Partition,
        read: &[String; 2],
        request: &CacheRequest,
        now: SystemTime,
        bounds: Bounds,
    ) -> Result<(Lookup, bool), ResponseCacheError> {
        self.work.count(Row::CacheLookups);
        let rows = self.find(connection, partition, read, request, REVALIDATED)?;
        for entry in rows {
            if bounds.outlived(entry.stored.fetched_at, now) {
                continue;
            }
            match freshness::freshness(&entry.stored, request, now) {
                Freshness::Other => {}
                Freshness::Fresh { age } => {
                    let Some(body) = self.body(&entry)? else {
                        return Ok((Lookup::Miss, false));
                    };
                    let flush = self.used(&entry, now);
                    return Ok((
                        Lookup::Fresh(Hit {
                            head: ResponseHead {
                                status: entry.stored.head.status,
                                headers: freshness::served_headers(&entry.stored.head.headers, age),
                            },
                            body,
                            run_id: entry.run_id,
                        }),
                        flush,
                    ));
                }
                Freshness::Stale { conditions } => {
                    return Ok((
                        Lookup::Stale(Stale {
                            conditions,
                            entry: Box::new(entry),
                        }),
                        false,
                    ));
                }
            }
        }
        Ok((Lookup::Miss, false))
    }

    /// The entry `replay` stored for `request` in the partitions `read`:
    /// its stored head, served as it is, and its body, whatever their
    /// freshness, and whether its use, recorded, needs a flush started;
    /// none when no entry's variant is the one `request` asks for, it has
    /// outlived the ceiling at `now`, or its body is gone.
    fn replayed(
        &self,
        connection: &Connection,
        partition: &Partition,
        read: &[String; 2],
        request: &CacheRequest,
        now: SystemTime,
        bounds: Bounds,
    ) -> Result<(Option<Hit>, bool), ResponseCacheError> {
        self.work.count(Row::CacheLookups);
        let rows = self.find(connection, partition, read, request, request.replay_key())?;
        for entry in rows {
            let asked = freshness::variant(request, &entry.stored.head);
            if asked.is_none_or(|asked| asked.vary_key != entry.vary_key)
                || bounds.outlived(entry.stored.fetched_at, now)
            {
                continue;
            }
            let Some(body) = self.body(&entry)? else {
                return Ok((None, false));
            };
            let flush = self.used(&entry, now);
            return Ok((
                Some(Hit {
                    head: entry.stored.head,
                    body,
                    run_id: entry.run_id,
                }),
                flush,
            ));
        }
        Ok((None, false))
    }

    /// Records `entry`'s use at `now`; whether no flush is on its way yet,
    /// the caller then starting one.
    fn used(&self, entry: &Entry, now: SystemTime) -> bool {
        let mut uses = self.uses.lock().unwrap_or_else(PoisonError::into_inner);
        uses.pending.push(Use {
            partition: entry.partition.clone(),
            kept_in: entry.kept_in.clone(),
            method: entry.stored.method.clone(),
            url: entry.stored.url.clone(),
            vary_key: entry.vary_key.clone(),
            replay_key: entry.replay_key.clone(),
            at: now,
        });
        !std::mem::replace(&mut uses.flushing, true)
    }

    /// Writes the uses recorded so far, each making its entry the most
    /// recently used unless a later use did.
    fn flush(&self, connection: &Connection) -> rusqlite::Result<()> {
        // Cleared with the taking: a use recorded from now on starts
        // another flush.
        let uses =
            std::mem::take(&mut *self.uses.lock().unwrap_or_else(PoisonError::into_inner)).pending;
        let mut statement = connection.prepare_cached(USED)?;
        for used in uses {
            statement.execute(rusqlite::params![
                index_time(used.at),
                used.kept_in,
                used.partition.person,
                used.partition.country,
                used.partition.language,
                used.partition.device,
                used.method,
                used.url,
                used.vary_key,
                used.replay_key,
            ])?;
        }
        Ok(())
    }

    /// `request`'s entries in `partition` under `replay_key`, kept in
    /// either of the partitions `read`, the newest first; a row this engine
    /// cannot read is left out.
    fn find(
        &self,
        connection: &Connection,
        partition: &Partition,
        [first, second]: &[String; 2],
        request: &CacheRequest,
        replay_key: &str,
    ) -> Result<Vec<Entry>, ResponseCacheError> {
        let mut statement = connection
            .prepare_cached(FIND)
            .map_err(|error| self.failed(&error))?;
        let rows = statement
            .query_map(
                (
                    first,
                    &partition.person,
                    &partition.country,
                    &partition.language,
                    &partition.device,
                    &request.method,
                    &request.url,
                    replay_key,
                    second,
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
                        row.get::<_, String>(8)?,
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
                kept_in,
            ) = row.map_err(|error| self.failed(&error))?;
            let read = || {
                Some(Entry {
                    partition: partition.clone(),
                    kept_in: kept_in.clone(),
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
        let at = index_time(now);
        let updated = connection
            .prepare_cached(
                "UPDATE entries SET response_headers = ?1, fetched_at = ?2, used_at = ?2 WHERE partition = ?3 AND person = ?4 AND country = ?5 AND language = ?6 AND device = ?7 AND method = ?8 AND url = ?9 AND vary_key = ?10 AND body_sha256 = ?11 AND replay_key = ?12",
            )
            .and_then(|mut statement| {
                statement.execute(rusqlite::params![
                    json(headers),
                    at,
                    entry.kept_in,
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

    /// Stores `entry` with `body` under `bounds`, in one transaction with
    /// the uses recorded so far, the deletion of the entries past the
    /// ceiling and the eviction of the least recently used past the cap;
    /// then deletes the files of the bodies no entry holds any more.
    fn store(
        &self,
        connection: &Connection,
        entry: &Entry,
        body: &[u8],
        bounds: Bounds,
    ) -> Result<(), ResponseCacheError> {
        self.write_body(&entry.body_sha256, body)?;
        let let_go = sqlite_store::immediate(connection, |connection| {
            self.flush(connection)?;
            if let Some(oldest) = entry.stored.fetched_at.checked_sub(bounds.max_age_ceiling) {
                connection
                    .prepare_cached("DELETE FROM entries WHERE fetched_at < ?1")?
                    .execute([index_time(oldest)])?;
            }
            insert(connection, entry)?;
            evict(connection, bounds.max_size)
        })
        .map_err(|error| self.failed(&error))?;
        self.remove_bodies(&let_go);
        Ok(())
    }

    /// Deletes the files of `let_go`, bodies no entry holds any more, and
    /// answers how many were there and the bytes their bodies weighed; one
    /// that cannot be deleted stays, counts in neither, and is logged.
    fn remove_bodies(&self, let_go: &[(String, u64)]) -> (u64, u64) {
        let bodies = self.folder.join(BODIES);
        let (mut removed, mut bytes) = (0, 0_u64);
        for (sha256, size) in let_go.iter().filter(|(sha256, _)| is_sha256(sha256)) {
            let path = bodies.join(sha256);
            #[expect(
                clippy::disallowed_methods,
                reason = "the writer's job is blocking work, on its own thread"
            )]
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    removed += 1;
                    bytes = bytes.saturating_add(*size);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(error = %body_failed(&path, &error), "a body file the Response cache let go stays");
                }
            }
        }
        (removed, bytes)
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
    /// half a body. Another engine's clear may delete a file found there
    /// before this store's entry is written; that entry then misses, as
    /// one whose file is gone does, until a later store of the same body
    /// writes the file again, a race accepted for the cost of a miss.
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
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('format', 'entries', 'bodies', 'held')",
            )
            .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
            .map_err(|error| self.failed(&error))?;
        if tables < 4 {
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

/// Where an entry is stored beyond its [`Partition`]'s person, country,
/// language and device.
struct Keyed {
    /// The partition that keeps it ([`partition`]).
    kept_in: String,
    variant: freshness::Variant,
    /// [`REVALIDATED`], or its request's replay key under `replay`.
    replay_key: String,
}

/// The entry `head`, the response to `request` in `partition` received at
/// `now` in the Run `run_id`, is stored as under `keyed`; its body's hash
/// and length set as it is stored.
fn entry(
    partition: &Partition,
    request: &CacheRequest,
    head: &ResponseHead,
    keyed: Keyed,
    run_id: Option<&RunId>,
    now: SystemTime,
) -> Entry {
    let Keyed {
        kept_in,
        variant,
        replay_key,
    } = keyed;
    Entry {
        partition: partition.clone(),
        kept_in,
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

/// Inserts `entry`, in place of what the index held under its key.
fn insert(connection: &Connection, entry: &Entry) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO entries (partition, person, country, language, device, method, url, vary_key, request_headers, status, response_headers, fetched_at, body_sha256, body_size, run_id, used_at, replay_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?12, ?16)
             ON CONFLICT (partition, person, country, language, device, method, url, vary_key, replay_key) DO UPDATE SET
             request_headers = excluded.request_headers, status = excluded.status, response_headers = excluded.response_headers, fetched_at = excluded.fetched_at, body_sha256 = excluded.body_sha256, body_size = excluded.body_size, run_id = excluded.run_id, used_at = excluded.used_at",
        )?
        .execute(rusqlite::params![
            entry.kept_in,
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
            index_time(entry.stored.fetched_at),
            entry.body_sha256,
            i64::try_from(entry.body_size).unwrap_or(i64::MAX),
            entry.run_id.as_ref().map(RunId::as_str),
            entry.replay_key,
        ])
        .map(drop)
}

/// Deletes the least recently used entries, a page at a time, until the
/// bytes held are within `cap`, or no entry is left, and answers the
/// bodies no entry holds any more with their sizes. Those bodies are
/// deleted before each weighing, so the bytes held never count a body
/// already let go: the entries a deletion before it took, or a page
/// taken before.
fn evict(connection: &Connection, cap: u64) -> rusqlite::Result<Vec<(String, u64)>> {
    let mut gone = Vec::new();
    loop {
        gone.extend(let_go(connection)?);
        let held: i64 = connection
            .prepare_cached(HELD)?
            .query_row([], |row| row.get(0))?;
        let held = u64::try_from(held).unwrap_or(0);
        if held <= cap {
            return Ok(gone);
        }
        let (rowids, oldest): (Vec<i64>, Vec<Weighed>) = connection
            .prepare_cached(OLDEST)?
            .query_map([PAGE], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    Weighed {
                        body: row.get(1)?,
                        size: u64::try_from(row.get::<_, i64>(2)?).unwrap_or(0),
                        holders: u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .unzip();
        let taken = evictions(held, cap, &oldest);
        let mut delete = connection.prepare_cached("DELETE FROM entries WHERE rowid = ?1")?;
        for rowid in &rowids[..taken] {
            delete.execute([rowid])?;
        }
        if taken < oldest.len() || oldest.is_empty() {
            gone.extend(let_go(connection)?);
            return Ok(gone);
        }
    }
}

/// Deletes the bodies no entry holds any more, and answers them with their
/// sizes.
fn let_go(connection: &Connection) -> rusqlite::Result<Vec<(String, u64)>> {
    let unheld = connection
        .prepare_cached("SELECT sha256, size FROM bodies WHERE holders = 0")?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    connection
        .prepare_cached("DELETE FROM bodies WHERE holders = 0")?
        .execute([])?;
    Ok(unheld)
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
