//! The Response cache's upkeep (ADRs 0001 and 0046): what `cache.stats`
//! counts, what `cache.clear` deletes and what `cache.verify` reads, each
//! over one person's entries, and the deletion of a discarded Run's
//! entries, which keeps ADR 0001's rule that nothing of a Run outlives its
//! discard but a Saved identity.
//!
//! A clear deletes its entries in one transaction, then the body files no
//! entry holds any more; SQLite's `secure_delete` overwrites what the rows
//! held, and a checkpoint that truncates the write-ahead log drops its
//! older copies, so nothing of a deleted entry stays in the cache's files.
//! A verify lists the entries on the reads' connection, then reads and
//! hashes their body files on the cache owner's blocking threads, so the
//! Runs' lookups never wait for it. None of them creates a cache never
//! written, and each refuses one of a format version the engine cannot
//! read.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::response_cache::{
    CacheBodyProblem, CacheCleared, CacheDamage, CachePartitionStats, CacheStats, CacheVerified,
};
use goodfetch_protocol::run::RunId;
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};

use super::{
    BODIES, CacheFile, HELD, ResponseCache, ResponseCacheError, is_sha256, let_go, partition,
};
use crate::host_name;
use crate::sqlite_store;

/// How many bytes a verify reads of a body file at a time.
const CHUNK: usize = 64 * 1024;

/// Which entries a clear deletes: those every given filter keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clearing {
    /// Only this person's; every person's when absent, as a discarded
    /// Run's deletion is.
    pub person: Option<String>,
    /// Only those of this host and its subdomains, keyed as
    /// [`host_name::key`] keys a host.
    pub domain: Option<String>,
    /// Only those this Run fetched.
    pub run_id: Option<RunId>,
    /// Only those of this partition: `public`, a Saved identity's name, or
    /// a Session's own ([`Keeper`](super::Keeper)).
    pub partition: Option<String>,
    /// Only those fetched through this proxy exit country.
    pub country: Option<String>,
    /// Only those fetched with this Identity profile language.
    pub language: Option<String>,
    /// Only those fetched with this device class.
    pub device: Option<String>,
}

impl Clearing {
    /// The entries the Run `run_id` fetched, whoever it fetched them for:
    /// what its discard deletes.
    pub fn of_run(run_id: &RunId) -> Self {
        Self {
            run_id: Some(run_id.clone()),
            ..Self::default()
        }
    }

    /// The entries kept in the own partition of the Session `session_id`
    /// of the Run `run_id`: what its end deletes.
    pub fn of_session(run_id: &RunId, session_id: &SessionId) -> Self {
        Self {
            partition: Some(partition::of_session(run_id, session_id)),
            ..Self::default()
        }
    }

    /// Whether it keeps the entry of `url`, by its host.
    fn keeps_url(&self, url: &str) -> bool {
        let Some(domain) = &self.domain else {
            return true;
        };
        url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().and_then(host_name::key))
            .is_some_and(|host| host_name::covering(&host).any(|covered| covered == domain))
    }
}

/// An entry as a verify reads it.
struct Held {
    partition: String,
    method: String,
    url: String,
    run_id: Option<String>,
    body_sha256: String,
    body_size: u64,
}

impl ResponseCache {
    /// What the cache holds of `person`'s: its entries, the bodies they
    /// hold and the bytes those weigh, and its entries by partition; and
    /// the bytes the whole cache holds. A cache never written counts
    /// nothing, and is not created.
    ///
    /// # Errors
    ///
    /// When the index cannot be read, or is of a format version this
    /// engine cannot read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves a read begun to finish on its own, its
    /// counts lost.
    pub async fn stats(&self, person: &str) -> Result<CacheStats, ResponseCacheError> {
        let person = person.to_owned();
        self.store
            .read(Some(CacheStats::default()), move |file, connection| {
                file.stats(connection, &person)
            })
            .await
    }

    /// Deletes the entries `clearing` keeps, then the body files no entry
    /// holds any more, and answers what left. A cache never written is
    /// not created.
    ///
    /// # Errors
    ///
    /// When the index cannot be written, or is of a format version this
    /// engine cannot read: nothing is deleted then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the writer took the clear still clears;
    /// only its counts are lost.
    pub async fn clear(&self, clearing: Clearing) -> Result<CacheCleared, ResponseCacheError> {
        self.store
            .write(Some(CacheCleared::default()), move |file, connection| {
                file.clear(connection, &clearing)
            })
            .await
    }

    /// Deletes every entry fetched by `run_id` whose URL, headers or stored
    /// body contains any non-empty string in `needles`, with `secure_delete`
    /// on, removing unheld body files and truncating the WAL, and answers how
    /// many entries were removed (ADR 0016). A cache never written is not
    /// created.
    ///
    /// # Errors
    ///
    /// When the index cannot be written, or is of a format version this
    /// engine cannot read.
    ///
    /// # Cancel safety
    ///
    /// As [`ResponseCache::clear`]'s.
    pub async fn erase_matching(
        &self,
        run_id: &RunId,
        needles: &[String],
    ) -> Result<u64, ResponseCacheError> {
        let needles: Vec<String> = needles
            .iter()
            .filter(|needle| !needle.is_empty())
            .cloned()
            .collect();
        if needles.is_empty() {
            return Ok(0);
        }
        let read_run_id = run_id.clone();
        let read_needles = needles.clone();
        let Some((bodies_dir, meta_matched, body_candidates)) = self
            .store
            .read(Some(None), move |file, connection| {
                file.erase_candidates(connection, &read_run_id, &read_needles)
                    .map(Some)
            })
            .await?
        else {
            return Ok(0);
        };
        let matched = self
            .owner
            .blocking(move || select_erased(&bodies_dir, meta_matched, body_candidates, &needles))
            .await?;
        if matched.is_empty() {
            return Ok(0);
        }
        let write_run_id = run_id.clone();
        self.store
            .write(Some(0), move |file, connection| {
                file.delete_erased(connection, &write_run_id, &matched)
            })
            .await
    }

    /// Deletes the entries the discarded Run `run_id` fetched, whoever it
    /// fetched them for, so nothing of it outlives its discard (ADR 0001).
    /// A cache this engine cannot read keeps them, which a warning says.
    ///
    /// # Errors
    ///
    /// When the index cannot be written: nothing is deleted then.
    ///
    /// # Cancel safety
    ///
    /// As [`ResponseCache::clear`]'s.
    pub async fn forget_run(&self, run_id: &RunId) -> Result<(), ResponseCacheError> {
        match self.clear(Clearing::of_run(run_id)).await {
            Ok(_) => Ok(()),
            Err(error @ ResponseCacheError::Format { .. }) => {
                tracing::warn!(%run_id, %error, "the discarded Run's entries stay in a Response cache this engine cannot read");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Deletes the entries kept in the own partition of the Session
    /// `session_id` of the Run `run_id`, which has no Saved identity, so
    /// nothing personal it fetched outlives it (ADR 0046). A cache this
    /// engine cannot read keeps them, which a warning says; the Run's
    /// discard deletes them still.
    ///
    /// # Errors
    ///
    /// When the index cannot be written: nothing is deleted then.
    ///
    /// # Cancel safety
    ///
    /// As [`ResponseCache::clear`]'s.
    pub async fn forget_session(
        &self,
        run_id: &RunId,
        session_id: &SessionId,
    ) -> Result<(), ResponseCacheError> {
        match self.clear(Clearing::of_session(run_id, session_id)).await {
            Ok(_) => Ok(()),
            Err(error @ ResponseCacheError::Format { .. }) => {
                tracing::warn!(%run_id, session_id = %session_id.0, %error, "the ended Session's entries stay in a Response cache this engine cannot read");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Reads the body file of each of `person`'s entries, each file once,
    /// and lists the entries whose file is missing or no longer hashes to
    /// the body stored. It changes nothing; a cache never written holds
    /// nothing to verify, and is not created.
    ///
    /// # Errors
    ///
    /// When the index cannot be read, or is of a format version this
    /// engine cannot read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the read or the hashing begun to finish
    /// on its own, its list lost; it changes nothing either way.
    pub async fn verify(&self, person: &str) -> Result<CacheVerified, ResponseCacheError> {
        let person = person.to_owned();
        let (entries, bodies) = self
            .store
            .read(
                Some((Vec::new(), PathBuf::new())),
                move |file, connection| {
                    Ok((file.held(connection, &person)?, file.folder.join(BODIES)))
                },
            )
            .await?;
        self.owner
            .blocking(move || verified(&entries, &bodies))
            .await
    }
}

impl CacheFile {
    fn stats(
        &self,
        connection: &Connection,
        person: &str,
    ) -> Result<CacheStats, ResponseCacheError> {
        let partitions = connection
            .prepare_cached(
                "SELECT partition, country, language, device, count(*) FROM entries WHERE person = ?1
                 GROUP BY partition, country, language, device
                 ORDER BY count(*) DESC, partition, country, language, device",
            )
            .and_then(|mut statement| {
                statement
                    .query_map([person], |row| {
                        Ok(CachePartitionStats {
                            partition: row.get(0)?,
                            country: row.get(1)?,
                            language: row.get(2)?,
                            device: row.get(3)?,
                            entries: count(row.get(4)?),
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|error| self.failed(&error))?;
        let (bodies, body_bytes) = connection
            .prepare_cached(
                "SELECT count(*), coalesce(sum(size), 0) FROM bodies
                 WHERE sha256 IN (SELECT body_sha256 FROM entries WHERE person = ?1)",
            )
            .and_then(|mut statement| {
                statement.query_row([person], |row| Ok((count(row.get(0)?), count(row.get(1)?))))
            })
            .map_err(|error| self.failed(&error))?;
        let held_bytes = connection
            .prepare_cached(HELD)
            .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
            .map(count)
            .map_err(|error| self.failed(&error))?;
        Ok(CacheStats {
            entries: partitions.iter().map(|partition| partition.entries).sum(),
            bodies,
            body_bytes,
            held_bytes,
            partitions,
        })
    }

    /// Deletes what `clearing` keeps in one transaction with the uses
    /// recorded so far, `secure_delete` on for that clear alone, then the
    /// body files no entry holds any more, then truncates the write-ahead
    /// log.
    fn clear(
        &self,
        connection: &Connection,
        clearing: &Clearing,
    ) -> Result<CacheCleared, ResponseCacheError> {
        self.secure_delete_and_truncate(connection, "cleared", |connection| {
            let rows = connection
                .prepare_cached(
                    "SELECT rowid, url FROM entries WHERE (?1 IS NULL OR person = ?1)
                     AND (?2 IS NULL OR run_id = ?2) AND (?3 IS NULL OR partition = ?3)
                     AND (?4 IS NULL OR country = ?4) AND (?5 IS NULL OR language = ?5)
                     AND (?6 IS NULL OR device = ?6)",
                )?
                .query_map(
                    rusqlite::params![
                        clearing.person,
                        clearing.run_id.as_ref().map(RunId::as_str),
                        clearing.partition,
                        clearing.country,
                        clearing.language,
                        clearing.device,
                    ],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut delete = connection.prepare_cached("DELETE FROM entries WHERE rowid = ?1")?;
            let mut entries = 0;
            for (rowid, url) in rows {
                if clearing.keeps_url(&url) {
                    entries += delete.execute([rowid])?;
                }
            }
            Ok(entries)
        })
    }

    /// Lists `run_id`'s entries for `erase_matching` on the reads connection
    /// without acquiring the writer lock, partitioning those already matched
    /// by metadata from those whose body files need streaming inspection.
    #[expect(
        clippy::type_complexity,
        reason = "private helper passing candidate `(rowid, sha256)` lists between read and scan phases"
    )]
    fn erase_candidates(
        &self,
        connection: &Connection,
        run_id: &RunId,
        needles: &[String],
    ) -> Result<(PathBuf, Vec<(i64, String)>, Vec<(i64, String)>), ResponseCacheError> {
        let rows = connection
            .prepare_cached(
                "SELECT rowid, url, request_headers, response_headers, body_sha256
                 FROM entries WHERE run_id = ?1",
            )
            .and_then(|mut statement| {
                statement
                    .query_map([run_id.as_str()], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|error| self.failed(&error))?;
        let mut meta_matched = Vec::new();
        let mut body_candidates = Vec::new();
        for (rowid, url, request_headers, response_headers, body_sha256) in rows {
            let matched_meta = needles.iter().any(|needle| {
                url.contains(needle)
                    || request_headers.contains(needle)
                    || response_headers.contains(needle)
            });
            if matched_meta {
                meta_matched.push((rowid, body_sha256));
            } else if is_sha256(&body_sha256) {
                body_candidates.push((rowid, body_sha256));
            }
        }
        Ok((self.folder.join(BODIES), meta_matched, body_candidates))
    }

    /// Deletes the verified `(rowid, body_sha256)` entries of `run_id` under
    /// `secure_delete`, re-checking `run_id` and `body_sha256` inside the
    /// write transaction, then removes unheld bodies and truncates the WAL.
    fn delete_erased(
        &self,
        connection: &Connection,
        run_id: &RunId,
        matched: &[(i64, String)],
    ) -> Result<u64, ResponseCacheError> {
        let cleared = self.secure_delete_and_truncate(connection, "erased", |connection| {
            let mut delete = connection.prepare_cached(
                "DELETE FROM entries WHERE rowid = ?1 AND run_id = ?2 AND body_sha256 = ?3",
            )?;
            let mut entries = 0;
            for (rowid, body_sha256) in matched {
                entries +=
                    delete.execute(rusqlite::params![rowid, run_id.as_str(), body_sha256])?;
            }
            Ok(entries)
        })?;
        Ok(cleared.entries)
    }

    /// Runs `delete_entries` in one immediate transaction with `secure_delete`
    /// enabled, removes unheld body files, disables `secure_delete`, and
    /// truncates the WAL so no deleted entry remains on disk.
    fn secure_delete_and_truncate<F>(
        &self,
        connection: &Connection,
        label: &str,
        delete_entries: F,
    ) -> Result<CacheCleared, ResponseCacheError>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<usize>,
    {
        connection
            .pragma_update(None, "secure_delete", true)
            .map_err(|error| self.failed(&error))?;
        let cleared: rusqlite::Result<(usize, Vec<(String, u64)>)> =
            sqlite_store::immediate(connection, |connection| {
                self.flush(connection)?;
                let entries = delete_entries(connection)?;
                Ok((entries, let_go(connection)?))
            });
        // A failed clear lets its connection go, `secure_delete` with it;
        // a clear done turns it off, so the stores and evictions after it
        // zero no pages.
        let (entries, let_go) = cleared.map_err(|error| self.failed(&error))?;
        let (bodies, bytes) = self.remove_bodies(&let_go);
        if let Err(error) = connection.pragma_update(None, "secure_delete", false) {
            tracing::warn!(index = %self.index.display(), %error, "the Response cache's writer keeps zeroing the pages it frees");
        }
        // A read on the index, this engine's or another's, keeps the log
        // from its truncation; the next clear's then drops what this one
        // left.
        let truncated: rusqlite::Result<i64> = connection
            .prepare_cached("PRAGMA wal_checkpoint(TRUNCATE)")
            .and_then(|mut statement| statement.query_row([], |row| row.get(0)));
        match truncated {
            Ok(0) => {}
            Ok(_) => tracing::warn!(
                index = %self.index.display(),
                "a reader held the Response cache's index: its write-ahead log keeps the {label} entries until the next clear"
            ),
            Err(error) => return Err(self.failed(&error)),
        }
        Ok(CacheCleared {
            entries: u64::try_from(entries).unwrap_or(u64::MAX),
            bodies,
            bytes,
        })
    }

    /// `person`'s entries, as a verify reads them.
    fn held(&self, connection: &Connection, person: &str) -> Result<Vec<Held>, ResponseCacheError> {
        connection
            .prepare_cached(
                "SELECT partition, method, url, run_id, body_sha256, body_size FROM entries
                 WHERE person = ?1 ORDER BY url, method, partition",
            )
            .and_then(|mut statement| {
                statement
                    .query_map([person], |row| {
                        Ok(Held {
                            partition: row.get(0)?,
                            method: row.get(1)?,
                            url: row.get(2)?,
                            run_id: row.get(3)?,
                            body_sha256: row.get(4)?,
                            body_size: count(row.get(5)?),
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|error| self.failed(&error))
    }
}

/// What a verify of `entries`, their body files in `bodies`, finds.
fn verified(entries: &[Held], bodies: &Path) -> Result<CacheVerified, ResponseCacheError> {
    let mut read: BTreeMap<&str, Option<(String, u64)>> = BTreeMap::new();
    for entry in entries {
        if !read.contains_key(entry.body_sha256.as_str()) {
            let found = if is_sha256(&entry.body_sha256) {
                hashed(&bodies.join(&entry.body_sha256))?
            } else {
                None
            };
            read.insert(&entry.body_sha256, found);
        }
    }
    let damaged = entries
        .iter()
        .filter_map(|entry| {
            let problem = match read
                .get(entry.body_sha256.as_str())
                .and_then(Option::as_ref)
            {
                None => CacheBodyProblem::Missing,
                Some((sha256, size))
                    if *sha256 == entry.body_sha256 && *size == entry.body_size =>
                {
                    return None;
                }
                Some(_) => CacheBodyProblem::Changed,
            };
            Some(CacheDamage {
                partition: entry.partition.clone(),
                method: entry.method.clone(),
                url: entry.url.clone(),
                run_id: entry
                    .run_id
                    .as_deref()
                    .and_then(|run_id| RunId::parse(run_id).ok()),
                body_sha256: entry.body_sha256.clone(),
                problem,
            })
        })
        .collect();
    Ok(CacheVerified {
        entries: u64::try_from(entries.len()).unwrap_or(u64::MAX),
        bodies: u64::try_from(read.values().filter(|found| found.is_some()).count())
            .unwrap_or(u64::MAX),
        damaged,
    })
}

/// The SHA-256, as lower-case hex, and the length of the file at `path`;
/// none when it is not there.
fn hashed(path: &Path) -> Result<Option<(String, u64)>, ResponseCacheError> {
    #[expect(
        clippy::disallowed_methods,
        reason = "a verify's reads are blocking work, on the cache owner's blocking threads, and no fetch's: they count on no Shared path"
    )]
    let opened = std::fs::File::open(path);
    let mut file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(super::body_failed(path, &error)),
    };
    let (mut digest, mut size, mut chunk) = (Sha256::new(), 0_u64, vec![0; CHUNK]);
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| super::body_failed(path, &error))?;
        if read == 0 {
            break;
        }
        digest.update(&chunk[..read]);
        size += u64::try_from(read).unwrap_or(u64::MAX);
    }
    Ok(Some((
        data_encoding::HEXLOWER.encode(&digest.finalize()),
        size,
    )))
}

/// Streams each unique candidate body file in chunks outside the writer lock
/// and combines body matches with metadata matches.
fn select_erased(
    bodies_dir: &Path,
    mut matched: Vec<(i64, String)>,
    body_candidates: Vec<(i64, String)>,
    needles: &[String],
) -> Result<Vec<(i64, String)>, ResponseCacheError> {
    let mut scanned: BTreeMap<String, bool> = BTreeMap::new();
    for (rowid, body_sha256) in body_candidates {
        let hit = if let Some(&cached) = scanned.get(&body_sha256) {
            cached
        } else {
            let found = body_contains_streaming(&bodies_dir.join(&body_sha256), needles)?;
            scanned.insert(body_sha256.clone(), found);
            found
        };
        if hit {
            matched.push((rowid, body_sha256));
        }
    }
    Ok(matched)
}

/// Whether the body file at `path` contains any of `needles`, streaming in
/// [`CHUNK`]-sized reads with overlap across chunk boundaries.
fn body_contains_streaming(path: &Path, needles: &[String]) -> Result<bool, ResponseCacheError> {
    #[expect(
        clippy::disallowed_methods,
        reason = "erasure body scans run on the cache owner's blocking threads, outside any Shared path"
    )]
    let opened = std::fs::File::open(path);
    let mut file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(super::body_failed(path, &error)),
    };
    let max_needle = needles.iter().map(String::len).max().unwrap_or(0);
    if max_needle == 0 {
        return Ok(false);
    }
    let overlap = max_needle - 1;
    let mut buffer = vec![0_u8; overlap + CHUNK];
    let mut carry = 0_usize;
    loop {
        let read = file
            .read(&mut buffer[carry..])
            .map_err(|error| super::body_failed(path, &error))?;
        if read == 0 {
            return Ok(false);
        }
        let valid = carry + read;
        let window = &buffer[..valid];
        if needles.iter().any(|needle| {
            let bytes = needle.as_bytes();
            !bytes.is_empty() && window.windows(bytes.len()).any(|slice| slice == bytes)
        }) {
            return Ok(true);
        }
        if overlap > 0 {
            let keep = valid.min(overlap);
            buffer.copy_within(valid - keep..valid, 0);
            carry = keep;
        }
    }
}

/// A count SQLite answers, which is never negative.
fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

#[cfg(test)]
mod tests;
