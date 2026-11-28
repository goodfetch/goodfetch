//! Recording a Run's responses into a WARC 1.1 file (ADRs 0105, 0116, 0118):
//! each response received under `mode: "record"` is redacted and masked before
//! it is kept in the run file's `recorded_responses` table, so a Run killed
//! and resumed produces one recording holding both halves and no credential or
//! Secret value ever reaches disk. When the Run ends, the file is replaced
//! atomically through [`Saving`], after a `warcinfo` record naming the engine,
//! the Run, the mode, the time span and the writer version.

use std::path::Path;
use std::sync::{Arc, PoisonError};
use std::time::SystemTime;

use bytes::Bytes;
use goodfetch_protocol::checkpoint::{
    FILE_WRITE_FAILED, FileWriteFailedDetails, PATH_NOT_WRITABLE, PathNotWritableDetails, WriteSide,
};
use goodfetch_protocol::run::{EndingError, RecordingReport, RunId};
use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::clock::Clock;
use crate::mask::{Masks, Scope};
use crate::response_cache::freshness::CREDENTIALS;
use crate::response_cache::{CacheRequest, Capture, Capturing, ResponseHead};
use crate::response_limits::{Decoding, Limits};
use crate::run_files::{SaveError, Saving};
use crate::run_state::{RunState, RunStateTables};
use crate::sync::Mutex;
use crate::warc::{
    self, RequestRecord, ResponseRecord, TRUNCATED_DISCONNECT, WRITER_VERSION, WarcInfo, Writer,
};

/// The run-state table holding a recording Run's redacted, masked exchanges
/// until the Run ends so a kill and resume yields one file with both halves.
pub const TABLES: RunStateTables = RunStateTables {
    name: "response-recording",
    statements: &["CREATE TABLE IF NOT EXISTS recorded_responses ( \
            position INTEGER PRIMARY KEY AUTOINCREMENT, \
            recorded_at TEXT NOT NULL, \
            method TEXT NOT NULL, \
            url TEXT NOT NULL, \
            request_headers TEXT NOT NULL, \
            request_body BLOB NOT NULL, \
            status INTEGER NOT NULL, \
            response_headers TEXT NOT NULL, \
            response_body BLOB NOT NULL, \
            truncated INTEGER NOT NULL CHECK (truncated IN (0, 1)) \
        ) STRICT"],
};

/// High-bit tag distinguishing in-memory staged sequences from SQLite rowids.
const STAGED_BIT: u64 = 1 << 63;

/// One captured HTTP exchange, already redacted and masked before keeping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SanitizedExchange {
    pub recorded_at: jiff::Timestamp,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Vec<u8>,
    pub status: u16,
    pub response_headers: Vec<(String, String)>,
    pub response_body: Vec<u8>,
    pub truncated: bool,
}

impl SanitizedExchange {
    /// Redacts credentials and masks every Secret value (`Scope::Kept`) before
    /// the exchange is staged or persisted.
    pub fn from_raw(
        masks: &Masks,
        request: CacheRequest,
        head: ResponseHead,
        body: Bytes,
        truncated: bool,
        recorded_at: SystemTime,
    ) -> Self {
        let recorded_at =
            jiff::Timestamp::try_from(recorded_at).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
        let url = masks.apply(&request.url, Scope::Kept).into_owned();
        let request_body = masks.apply_bytes(request.body(), Scope::Kept).into_owned();
        let request_headers = request
            .headers
            .into_iter()
            .filter(|(name, _)| !is_credential_header(name))
            .map(|(name, value)| (name, masks.apply(&value, Scope::Kept).into_owned()))
            .collect();

        let (raw_headers, raw_body) = maybe_decode(head.headers, body, truncated);
        let response_headers = raw_headers
            .into_iter()
            .filter(|(name, _)| !is_credential_header(name))
            .map(|(name, value)| {
                let value = if name.eq_ignore_ascii_case("set-cookie") {
                    mask_set_cookie(&value, masks)
                } else {
                    masks.apply(&value, Scope::Kept).into_owned()
                };
                (name, value)
            })
            .collect();
        let response_body = masks.apply_bytes(&raw_body, Scope::Kept).into_owned();

        Self {
            recorded_at,
            method: request.method,
            url,
            request_headers,
            request_body,
            status: head.status,
            response_headers,
            response_body,
            truncated,
        }
    }
}

fn is_credential_header(name: &str) -> bool {
    CREDENTIALS
        .iter()
        .any(|cred| name.eq_ignore_ascii_case(cred))
}

/// Masks a `Set-Cookie` header's cookie value while keeping its name and
/// attributes (with attributes also passed through `masks`).
pub(crate) fn mask_set_cookie(value: &str, masks: &Masks) -> String {
    let (pair, rest) = match value.split_once(';') {
        Some((pair, rest)) => (pair, Some(rest)),
        None => (value, None),
    };
    let mut out = match pair.split_once('=') {
        Some((name, _)) => format!("{}=[redacted]", name.trim()),
        None => "[redacted]".to_owned(),
    };
    if let Some(rest) = rest {
        out.push(';');
        out.push_str(&masks.apply(rest, Scope::Kept));
    }
    out
}

/// Decodes a whole content-encoded response body before masking so Secret
/// canaries inside gzip/deflate/brotli payloads are masked too.
fn maybe_decode(
    headers: Vec<(String, String)>,
    body: Bytes,
    truncated: bool,
) -> (Vec<(String, String)>, Bytes) {
    if truncated || body.is_empty() {
        return (headers, body);
    }
    let has_encoding = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-encoding") && !value.trim().is_empty()
    });
    if !has_encoding {
        return (headers, body);
    }
    let mut decoding = Decoding::new(
        "",
        headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
            .map(|(_, value)| value.as_str()),
        Limits::default(),
    );
    if decoding.push(&body).is_err() {
        return (headers, body);
    }
    let Ok(decoded) = decoding.finish() else {
        return (headers, body);
    };
    let stripped = headers
        .into_iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case("content-encoding")
                && !name.eq_ignore_ascii_case("content-length")
        })
        .collect();
    (stripped, Bytes::from(decoded))
}

#[derive(Debug, Default)]
struct RecorderState {
    run_state: Option<RunState>,
    staged: Vec<(u64, SanitizedExchange)>,
    next_staged: u64,
    truncated_positions: Vec<u64>,
    id_counter: u64,
}

/// Construction options for a [`Recorder`] (`mode: "record"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecorderPolicy {
    /// When true (Foxhound crawls), captured responses are staged in memory and
    /// flushed inside each attempt outcome's own transaction so recording adds
    /// no transaction to F2; otherwise (Collie Sessions) each capture is
    /// persisted through the Run's `run_state` writer.
    pub stage_for_attempt: bool,
    /// Whether the engine writes the target path on its own filesystem at Run
    /// end (`true` for Attached/Linked engines, `false` for a Daemon where the
    /// SDK streams the WARC via `recording.save_as` and writes on the program's
    /// machine).
    pub write_on_end: bool,
}

/// Captures responses for one live Run in `mode: "record"` and writes the WARC
/// file atomically when the Run ends.
#[derive(Debug)]
pub struct Recorder {
    path: String,
    run_id: RunId,
    software: String,
    masks: Masks,
    clock: Clock,
    id_seed: u64,
    stage_for_attempt: bool,
    write_on_end: bool,
    state: Mutex<RecorderState>,
}

impl Recorder {
    /// Creates a new recorder for `run_id`, targeting `path`, under `policy`.
    pub fn new(
        path: String,
        run_id: RunId,
        software: String,
        masks: Masks,
        clock: Clock,
        id_seed: u64,
        policy: RecorderPolicy,
    ) -> Arc<Self> {
        Arc::new(Self {
            path,
            run_id,
            software,
            masks,
            clock,
            id_seed,
            stage_for_attempt: policy.stage_for_attempt,
            write_on_end: policy.write_on_end,
            state: Mutex::new(RecorderState::default()),
        })
    }

    /// Attaches the Run's [`RunState`] once started or resumed.
    pub fn attach_run_state(&self, run_state: RunState) {
        self.lock().run_state = Some(run_state);
    }

    /// Inserts all staged exchanges and pending truncation updates into
    /// `connection` within the caller's existing transaction (used by crawl
    /// attempt outcomes and Run end).
    ///
    /// # Errors
    ///
    /// Returns any SQLite error from writing `recorded_responses`.
    pub fn flush_staged(&self, connection: &Connection) -> rusqlite::Result<()> {
        let (staged, truncated) = {
            let mut state = self.lock();
            (
                std::mem::take(&mut state.staged),
                std::mem::take(&mut state.truncated_positions),
            )
        };
        for (_, exchange) in &staged {
            insert_exchange(connection, exchange)?;
        }
        for position in truncated {
            mark_position_truncated(connection, position)?;
        }
        Ok(())
    }

    /// Flushes any remaining staged exchanges, reads every exchange of the Run
    /// from `connection` (including earlier halves before a kill and resume),
    /// and replaces `self.path` atomically with a WARC 1.1 file.
    ///
    /// # Errors
    ///
    /// Returns any SQLite error reading `recorded_responses`; filesystem write
    /// failures are returned as `Ok(Err(EndingError))` for ADR 0116 reporting.
    pub fn finish_and_write(
        &self,
        connection: &Connection,
    ) -> rusqlite::Result<Result<RecordingReport, EndingError>> {
        self.flush_staged(connection)?;
        let rows = read_exchanges(connection)?;
        let recorded_from = rows.iter().map(|row| row.recorded_at).min();
        let recorded_to = rows.iter().map(|row| row.recorded_at).max();
        if !self.write_on_end {
            return Ok(Ok(RecordingReport {
                mode: "record".to_owned(),
                file: self.path.clone(),
                responses_used: u64::try_from(rows.len()).unwrap_or(u64::MAX),
                recorded_from: recorded_from.map(SystemTime::from),
                recorded_to: recorded_to.map(SystemTime::from),
                written_by: Some(self.software.clone()),
                misses: Vec::new(),
            }));
        }
        let target = Path::new(&self.path);
        let gzip_per_record = target
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"));
        let filename = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let info = WarcInfo {
            record_id: self.next_uuid(),
            date: self.clock.timestamp(),
            filename,
            software: self.software.clone(),
            run_id: Some(self.run_id.to_string()),
            mode: Some("record".to_owned()),
            recorded_from,
            recorded_to,
            writer_version: WRITER_VERSION.to_owned(),
        };
        let result = (|| -> Result<(), SaveError> {
            let mut sink = SavingWriter::new(Saving::create(target)?);
            let mut writer = Writer::new(&mut sink, gzip_per_record);
            if let Err(error) = write_all_records(&mut writer, &info, &rows, || self.next_uuid()) {
                return Err(sink
                    .take_error()
                    .unwrap_or_else(|| SaveError::of_storage(&error, "encode the recording")));
            }
            sink.finish()
        })();
        match result {
            Ok(()) => Ok(Ok(RecordingReport {
                mode: "record".to_owned(),
                file: self.path.clone(),
                responses_used: u64::try_from(rows.len()).unwrap_or(u64::MAX),
                recorded_from: recorded_from.map(SystemTime::from),
                recorded_to: recorded_to.map(SystemTime::from),
                written_by: Some(self.software.clone()),
                misses: Vec::new(),
            })),
            Err(error) => Ok(Err(save_error_to_ending(&self.path, error))),
        }
    }

    fn lock(&self) -> crate::sync::MutexGuard<'_, RecorderState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_uuid(&self) -> String {
        let mut state = self.lock();
        state.id_counter = state.id_counter.wrapping_add(1);
        format_uuid(self.id_seed, state.id_counter)
    }
}

fn format_uuid(id_seed: u64, counter: u64) -> String {
    let hi = super::determinism::mix64(id_seed ^ counter);
    let lo = super::determinism::mix64(
        id_seed.wrapping_add(counter.wrapping_mul(super::determinism::SPLITMIX_INCREMENT)),
    );
    // RFC 4122 version 4, variant 1 layout.
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&hi.to_be_bytes());
    b[8..].copy_from_slice(&lo.to_be_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "urn:uuid:{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

impl Capture for Recorder {
    fn capture(
        &self,
        request: CacheRequest,
        head: ResponseHead,
        body: Bytes,
        truncated: bool,
        now: SystemTime,
    ) -> Capturing<'_> {
        let exchange =
            SanitizedExchange::from_raw(&self.masks, request, head, body, truncated, now);
        Box::pin(async move {
            let run_state = if self.stage_for_attempt {
                None
            } else {
                self.lock().run_state.clone()
            };
            if let Some(run_state) = run_state {
                let pending_truncated = {
                    let mut state = self.lock();
                    std::mem::take(&mut state.truncated_positions)
                };
                let restore = pending_truncated.clone();
                match run_state
                    .write(move |connection| {
                        for position in pending_truncated {
                            mark_position_truncated(connection, position)?;
                        }
                        insert_exchange(connection, &exchange)
                    })
                    .await
                {
                    Ok(position) => return position,
                    Err(error) => {
                        if !restore.is_empty() {
                            self.lock().truncated_positions.extend(restore);
                        }
                        tracing::warn!(
                            run_id = %self.run_id,
                            %error,
                            "could not persist a recorded response to run state"
                        );
                        return 0;
                    }
                }
            }
            let mut state = self.lock();
            state.next_staged = state.next_staged.wrapping_add(1);
            let id = STAGED_BIT | state.next_staged;
            state.staged.push((id, exchange));
            id
        })
    }

    fn mark_truncated(&self, sequence: u64) {
        if sequence == 0 {
            return;
        }
        let mut state = self.lock();
        if sequence & STAGED_BIT != 0 {
            if let Some((_, exchange)) = state.staged.iter_mut().find(|(id, _)| *id == sequence) {
                exchange.truncated = true;
            }
        } else {
            state.truncated_positions.push(sequence);
        }
    }
}

/// Adapts [`Saving`] to [`std::io::Write`] so WARC records stream directly to
/// the temporary file without double-buffering the whole recording in memory.
struct SavingWriter {
    saving: Saving,
    error: Option<SaveError>,
}

impl SavingWriter {
    fn new(saving: Saving) -> Self {
        Self {
            saving,
            error: None,
        }
    }

    fn take_error(&mut self) -> Option<SaveError> {
        self.error.take()
    }

    fn finish(self) -> Result<(), SaveError> {
        self.saving.finish()
    }
}

impl std::io::Write for SavingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Err(error) = self.saving.write(buf) {
            self.error = Some(error);
            return Err(std::io::Error::other("recording save failed"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn write_all_records<W: std::io::Write>(
    writer: &mut Writer<W>,
    info: &WarcInfo,
    rows: &[SanitizedExchange],
    mut next_id: impl FnMut() -> String,
) -> std::io::Result<()> {
    writer.write_warcinfo(info)?;
    for row in rows {
        let request_id = next_id();
        let response_id = next_id();
        writer.write_request(&RequestRecord {
            record_id: &request_id,
            concurrent_to: Some(&response_id),
            target_uri: &row.url,
            date: row.recorded_at,
            method: &row.method,
            headers: &row.request_headers,
            body: &row.request_body,
        })?;
        writer.write_response(&ResponseRecord {
            record_id: &response_id,
            concurrent_to: Some(&request_id),
            target_uri: &row.url,
            date: row.recorded_at,
            status: row.status,
            headers: &row.response_headers,
            body: &row.response_body,
            truncated: row.truncated.then_some(TRUNCATED_DISCONNECT),
        })?;
    }
    Ok(())
}

/// Encodes all committed `recorded_responses` rows of `connection` into a
/// WARC 1.1 byte buffer (gzipped per record when `gzip_per_record` is true).
/// When `rows` is empty and `allow_empty` is false, returns `Ok(None)`; when
/// `allow_empty` is true (a `mode: "record"` Run that recorded 0 responses),
/// returns a valid `warcinfo`-only WARC matching [`Recorder::finish_and_write`].
///
/// # Errors
///
/// Returns any SQLite error from reading `recorded_responses`.
#[expect(
    clippy::too_many_arguments,
    reason = "WARC encoding parameters mirror WarcInfo fields plus allow_empty for 0-response record Runs"
)]
pub fn encode_warc(
    connection: &Connection,
    run_id: &RunId,
    software: &str,
    filename: Option<String>,
    gzip_per_record: bool,
    now: jiff::Timestamp,
    id_seed: u64,
    allow_empty: bool,
) -> rusqlite::Result<Option<Vec<u8>>> {
    let rows = read_exchanges(connection)?;
    if rows.is_empty() && !allow_empty {
        return Ok(None);
    }
    let recorded_from = rows.iter().map(|row| row.recorded_at).min();
    let recorded_to = rows.iter().map(|row| row.recorded_at).max();
    let mut counter: u64 = 0;
    let mut next_uuid = || {
        counter = counter.wrapping_add(1);
        format_uuid(id_seed, counter)
    };
    let info = WarcInfo {
        record_id: next_uuid(),
        date: now,
        filename,
        software: software.to_owned(),
        run_id: Some(run_id.to_string()),
        mode: Some("record".to_owned()),
        recorded_from,
        recorded_to,
        writer_version: WRITER_VERSION.to_owned(),
    };
    let mut buffer = Vec::new();
    let mut writer = Writer::new(&mut buffer, gzip_per_record);
    write_all_records(&mut writer, &info, &rows, next_uuid)
        .expect("invariant: encoding WARC records into a Vec<u8> is infallible");
    Ok(Some(buffer))
}

/// Inserts `exchange` into `recorded_responses`, returning its row position.
pub(crate) fn insert_exchange(
    connection: &Connection,
    exchange: &SanitizedExchange,
) -> rusqlite::Result<u64> {
    let request_headers = serde_json::to_string(&exchange.request_headers)
        .expect("invariant: header list encodes as JSON");
    let response_headers = serde_json::to_string(&exchange.response_headers)
        .expect("invariant: header list encodes as JSON");
    let recorded_at = warc::warc_date(exchange.recorded_at);
    let truncated = i64::from(exchange.truncated);
    connection
        .prepare_cached(
            "INSERT INTO recorded_responses \
             (recorded_at, method, url, request_headers, request_body, status, response_headers, response_body, truncated) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?
        .execute((
            &recorded_at,
            &exchange.method,
            &exchange.url,
            &request_headers,
            &exchange.request_body,
            i64::from(exchange.status),
            &response_headers,
            &exchange.response_body,
            truncated,
        ))?;
    Ok(u64::try_from(connection.last_insert_rowid()).unwrap_or(0))
}

fn mark_position_truncated(connection: &Connection, position: u64) -> rusqlite::Result<()> {
    let Ok(rowid) = i64::try_from(position) else {
        return Ok(());
    };
    connection
        .prepare_cached("UPDATE recorded_responses SET truncated = 1 WHERE position = ?1")?
        .execute([rowid])?;
    Ok(())
}

fn read_exchanges(connection: &Connection) -> rusqlite::Result<Vec<SanitizedExchange>> {
    let mut statement = connection.prepare_cached(
        "SELECT recorded_at, method, url, request_headers, request_body, status, response_headers, response_body, truncated \
         FROM recorded_responses ORDER BY position",
    )?;
    let rows = statement.query_map([], |row| {
        let recorded_at_text: String = row.get(0)?;
        let recorded_at = recorded_at_text
            .parse::<jiff::Timestamp>()
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH);
        let request_headers_json: String = row.get(3)?;
        let response_headers_json: String = row.get(6)?;
        let request_headers: Vec<(String, String)> = serde_json::from_str(&request_headers_json)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        let response_headers: Vec<(String, String)> = serde_json::from_str(&response_headers_json)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        let status_i64: i64 = row.get(5)?;
        let truncated_i64: i64 = row.get(8)?;
        Ok(SanitizedExchange {
            recorded_at,
            method: row.get(1)?,
            url: row.get(2)?,
            request_headers,
            request_body: row.get(4)?,
            status: u16::try_from(status_i64).unwrap_or(200),
            response_headers,
            response_body: row.get(7)?,
            truncated: truncated_i64 != 0,
        })
    })?;
    rows.collect()
}

fn save_error_to_ending(path: &str, error: SaveError) -> EndingError {
    match error {
        SaveError::Path {
            reason,
            os_code,
            message,
        } => {
            let details = to_map(&PathNotWritableDetails {
                reason: reason.to_owned(),
                os_code: os_code.map(str::to_owned),
                path: path.to_owned(),
                fix: "Pass a writable file path for `recording.path`.".to_owned(),
            });
            EndingError {
                code: PATH_NOT_WRITABLE.name.to_owned(),
                message: format!("Could not write the Response recording `{path}`: {message}"),
                details,
            }
        }
        SaveError::Storage {
            reason,
            os_code,
            message,
        }
        | SaveError::Source {
            reason,
            os_code,
            message,
        } => {
            let details = to_map(&FileWriteFailedDetails {
                reason: reason.to_owned(),
                os_code: os_code.map(str::to_owned),
                path: Some(path.to_owned()),
                side: WriteSide::Program,
            });
            EndingError {
                code: FILE_WRITE_FAILED.name.to_owned(),
                message: format!("Could not write the Response recording `{path}`: {message}"),
                details,
            }
        }
        SaveError::Cancelled => EndingError {
            code: FILE_WRITE_FAILED.name.to_owned(),
            message: format!("Writing the Response recording `{path}` was cancelled."),
            details: Map::new(),
        },
    }
}

fn to_map(value: &impl serde::Serialize) -> Map<String, Value> {
    match serde_json::to_value(value) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}
