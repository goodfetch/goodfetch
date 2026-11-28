//! A Response recording replayed (ADR 0105): [`Replay::open`] indexes a
//! WARC file's responses once, at the Run's start, and [`Replay::answer`]
//! answers each request the Run sends from them, without the network.
//!
//! A request is answered by a recorded response of the same method, the
//! same URL without its fragment, the same canonical body, the body `replay`
//! keys the Response cache by, and the same values of the request headers
//! the response's `Vary` names, except `Cookie`, `Authorization` and
//! `Proxy-Authorization`. A Secret's value sent in a body is compared as its
//! `%name%`: a recording written with redaction on holds the placeholder, so
//! the live body is compared both as it is and masked. A key recorded
//! several times is answered in the order recorded, then by its last
//! response, so a login form and the page after it replay in turn.
//!
//! A request no response answers is a [`Miss`], naming the recorded URLs of
//! the same host and path, and the Run's report lists every one. A response
//! recorded `WARC-Truncated`, or whose body broke off, is answered as
//! [`Answer::Cut`], which replays as `response_truncated` (ADR 0118). A
//! `revisit` record is answered with the payload of the record it refers
//! to.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, PoisonError};
use std::time::SystemTime;

use bytes::Bytes;
use goodfetch_protocol::run::{
    RECORDING_INVALID, RESPONSE_NOT_RECORDED, RecordingInvalidDetails, RecordingReport,
    ResponseNotRecordedDetails, RunId,
};

use crate::call_error::CallError;
use crate::dns::HostMapping;
use crate::mask::{Masks, Scope};
use crate::response_cache::freshness::{CREDENTIALS, first};
use crate::response_cache::{
    CacheRequest, NotRecorded, Recorded, Recording, Replaying, ResponseHead, Served, body_sha256,
};
use crate::sync::Mutex;
use crate::tasks::TaskOwner;
use crate::warc::{self, HttpRequest, HttpResponse, Record, RecordType};

mod determinism;
mod record;
#[cfg(test)]
mod tests;

pub use determinism::{
    SeededRandom, http_date_from_headers, init_script, routed_date_script, seed_of,
    system_time_millis,
};
pub use record::{Recorder, SanitizedExchange, TABLES, encode_warc};

/// The most recorded URLs a [`Miss`] names.
const NEAREST: usize = 5;

/// The `WARC-Profile` of a revisit whose server answered `304 Not
/// Modified`, in WARC 1.1's and 1.0's spelling.
const SERVER_NOT_MODIFIED: &str = "/revisit/server-not-modified";

/// Why a file is not a Response recording the engine replays:
/// `recording_invalid`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RecordingInvalid {
    /// The file is not WARC records the engine reads.
    #[error(transparent)]
    Unreadable(#[from] warc::Unreadable),
    /// A record says it holds an HTTP message and does not.
    #[error("record {record} says it holds an HTTP message: {source}")]
    NotHttp {
        /// The record's `WARC-Record-ID`.
        record: String,
        /// What is wrong with its block.
        #[source]
        source: warc::NotHttp,
    },
}

/// What a Response recording answers a request with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The recorded response, shared with the index rather than copied, along
    /// with when it was recorded.
    Answered {
        /// The recorded head and body.
        served: Arc<Served>,
        /// When the response was recorded (`WARC-Date` or HTTP `Date`).
        recorded_at: Option<SystemTime>,
    },
    /// The recorded response, which broke off: it replays as
    /// `response_truncated`.
    Cut(Arc<Served>),
    /// No recorded response: the request fails as `response_not_recorded`.
    Missed(Miss),
}

/// A request no recorded response answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Miss {
    /// Its method.
    pub method: String,
    /// Its URL, without its fragment.
    pub url: String,
    /// The recorded URLs of its host and path, in the order recorded, at
    /// most [`NEAREST`].
    pub nearest: Vec<String>,
}

impl std::fmt::Display for Miss {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} {}", self.method, self.url)
    }
}

/// What a replay did, for the Run's report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// How many recorded responses answered a request.
    pub responses_used: u64,
    /// When the oldest of them was fetched.
    pub recorded_from: Option<SystemTime>,
    /// When the newest of them was fetched.
    pub recorded_to: Option<SystemTime>,
    /// The software that wrote the file, its `warcinfo`'s `software`.
    pub written_by: Option<String>,
    /// Each request no recorded response answered, in the order sent.
    pub misses: Vec<Miss>,
}

/// A recorded response and what it answers.
#[derive(Debug)]
struct Entry {
    /// The recorded request's method.
    method: String,
    /// The hash of the recorded request's canonical body.
    body_key: String,
    /// Each request header the response's `Vary` names, but credentials,
    /// with the recorded request's value; empty when no request was
    /// recorded.
    vary: Vec<(String, Option<String>)>,
    served: Arc<Served>,
    recorded_at: Option<SystemTime>,
    cut: bool,
    /// Its place in [`Used::answered`].
    index: usize,
}

impl Entry {
    fn matches(&self, request: &CacheRequest, body_keys: &[String]) -> bool {
        self.method == request.method
            && body_keys.contains(&self.body_key)
            && self
                .vary
                .iter()
                .all(|(name, value)| joined(&request.headers, name) == *value)
    }
}

/// A Response recording's responses, indexed for a Run.
#[derive(Debug)]
pub struct Replay {
    masks: Masks,
    /// Each URL's recorded responses, in the order recorded.
    entries: HashMap<String, Vec<Entry>>,
    /// Each host and path's recorded URLs, in the order recorded.
    places: HashMap<(String, String), Vec<String>>,
    /// The `WARC-Record-ID` of the recording's `warcinfo` record, if any.
    warcinfo_id: String,
    written_by: Option<String>,
    used: Mutex<Used>,
}

/// What a replay used so far.
#[derive(Debug, Default)]
struct Used {
    /// Whether each recorded response answered a request, by its
    /// [`Entry::index`].
    answered: Vec<bool>,
    misses: Vec<Miss>,
}

impl Replay {
    /// The responses of the WARC file `contents`, a Secret's value in a
    /// live body compared as its `%name%` too, as `masks` masks what the
    /// engine keeps; a gzipped file is decoded within
    /// `max_decompression_ratio`, the Run's
    /// `network.max_decompression_ratio`.
    pub fn open(
        contents: &Bytes,
        masks: Masks,
        max_decompression_ratio: u64,
    ) -> Result<Self, RecordingInvalid> {
        let records = warc::read(contents, max_decompression_ratio)?;
        let index = Index::of(&records);
        let warcinfo = records
            .iter()
            .find(|record| record.record_type() == RecordType::Warcinfo);
        let mut replay = Self {
            masks,
            entries: HashMap::new(),
            places: HashMap::new(),
            warcinfo_id: warcinfo
                .and_then(Record::record_id)
                .unwrap_or_default()
                .to_owned(),
            written_by: warcinfo.and_then(software),
            used: Mutex::new(Used::default()),
        };
        for record in &records {
            let kind = record.record_type();
            if !matches!(kind, RecordType::Response | RecordType::Revisit) || !record.holds_http() {
                continue;
            }
            let Some(url) = record
                .target_uri()
                .and_then(|uri| url::Url::parse(uri).ok())
            else {
                continue;
            };
            let response = parse_response(record)?;
            let (served, cut) = if kind == RecordType::Revisit {
                let Some(original) = index.original(record) else {
                    continue;
                };
                let (served, cut) = revisited(record, &response, &parse_response(original)?);
                (served, cut || truncated(original))
            } else {
                let cut = response.cut;
                (served(response), cut)
            };
            let request = index.request_of(record).map(parse_request).transpose()?;
            replay.add(&url, record, request.as_ref(), served, cut);
        }
        Ok(replay)
    }

    fn lock(&self) -> crate::sync::MutexGuard<'_, Used> {
        self.used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn add(
        &mut self,
        url: &url::Url,
        record: &Record,
        request: Option<&HttpRequest>,
        served: Served,
        cut: bool,
    ) {
        let mut url = url.clone();
        url.set_fragment(None);
        let vary = request.map_or_else(Vec::new, |request| {
            vary_names(&served.head.headers)
                .map(|name| {
                    let value = joined(&request.headers, &name);
                    (name, value)
                })
                .collect()
        });
        let index = {
            let mut used = self.lock();
            let index = used.answered.len();
            used.answered.push(false);
            index
        };
        let recorded_at = record
            .date()
            .map(SystemTime::from)
            .or_else(|| http_date_from_headers(&served.head.headers));
        let entry = Entry {
            method: request.map_or_else(|| "GET".to_owned(), |request| request.method.clone()),
            body_key: request.map_or_else(
                || body_sha256(None, b""),
                |request| body_sha256(first(&request.headers, "content-type"), &request.body),
            ),
            vary,
            served: Arc::new(served),
            recorded_at,
            cut: cut || truncated(record),
            index,
        };
        let urls = self.places.entry(place(&url)).or_default();
        let url = String::from(url);
        if !urls.contains(&url) {
            urls.push(url.clone());
        }
        self.entries.entry(url).or_default().push(entry);
    }

    /// What answers `request`: the first recorded response of its key not
    /// yet answered, else the last; a [`Miss`], listed in the report, when
    /// none.
    pub fn answer(&self, request: &CacheRequest) -> Answer {
        let matching: Vec<&Entry> = self
            .entries
            .get(&request.url)
            .map(|entries| {
                let body_keys = self.body_keys(request);
                entries
                    .iter()
                    .filter(|entry| entry.matches(request, &body_keys))
                    .collect()
            })
            .unwrap_or_default();
        let mut used = self.lock();
        let found = matching
            .iter()
            .find(|entry| !used.answered.get(entry.index).copied().unwrap_or(true))
            .or_else(|| matching.last());
        let Some(entry) = found else {
            let miss = self.miss(request);
            used.misses.push(miss.clone());
            return Answer::Missed(miss);
        };
        if let Some(answered) = used.answered.get_mut(entry.index) {
            *answered = true;
        }
        if entry.cut {
            Answer::Cut(Arc::clone(&entry.served))
        } else {
            Answer::Answered {
                served: Arc::clone(&entry.served),
                recorded_at: entry.recorded_at,
            }
        }
    }

    /// The hashes `request`'s body may be recorded under: of its canonical
    /// body, and of it with every Secret's value masked when that differs.
    fn body_keys(&self, request: &CacheRequest) -> Vec<String> {
        let mut keys = vec![request.replay_key().to_owned()];
        if let std::borrow::Cow::Owned(masked) = self.masks.apply_bytes(request.body(), Scope::Kept)
        {
            keys.push(body_sha256(
                first(&request.headers, "content-type"),
                &masked,
            ));
        }
        keys
    }

    fn miss(&self, request: &CacheRequest) -> Miss {
        let nearest = url::Url::parse(&request.url)
            .ok()
            .and_then(|url| self.places.get(&place(&url)))
            .map(|urls| urls.iter().take(NEAREST).cloned().collect())
            .unwrap_or_default();
        Miss {
            method: request.method.clone(),
            url: request.url.clone(),
            nearest,
        }
    }

    /// The `WARC-Record-ID` of the recording's `warcinfo` record (empty when
    /// the file has no `warcinfo` record).
    pub fn warcinfo_id(&self) -> &str {
        &self.warcinfo_id
    }

    /// Each recorded URL's first recorded fetch timestamp in Unix epoch
    /// milliseconds (`WARC-Date`, falling back to its HTTP `Date` header).
    pub fn document_times(&self) -> BTreeMap<String, i64> {
        self.entries
            .iter()
            .filter_map(|(url, entries)| {
                let time = entries.iter().find_map(|entry| entry.recorded_at)?;
                Some((url.clone(), system_time_millis(time)?))
            })
            .collect()
    }

    /// What the replay did so far.
    pub fn report(&self) -> Report {
        let used = self.lock();
        let answered: Vec<&Entry> = self
            .entries
            .values()
            .flatten()
            .filter(|entry| used.answered.get(entry.index).copied().unwrap_or(false))
            .collect();
        let times = answered.iter().filter_map(|entry| entry.recorded_at);
        Report {
            responses_used: u64::try_from(answered.len()).unwrap_or(u64::MAX),
            recorded_from: times.clone().min(),
            recorded_to: times.max(),
            written_by: self.written_by.clone(),
            misses: used.misses.clone(),
        }
    }
}

/// What `response_not_recorded`'s `fix` advises (ADR 0105).
const NOT_RECORDED_FIX: &str = "Record the Response recording again so it holds this request, or answer the request with a route.";

/// `response_not_recorded` (`RefusedError`, not retryable) for a request
/// `miss` that a replaying Run's Response recording had no response for
/// (ADR 0105).
pub fn not_recorded_error(miss: &NotRecorded) -> CallError {
    CallError::new(
        &RESPONSE_NOT_RECORDED,
        format!(
            "The Response recording `{}` has no response for {} {}; nothing was sent.",
            miss.recording, miss.method, miss.url
        ),
        &ResponseNotRecordedDetails {
            method: miss.method.clone(),
            url: miss.url.clone(),
            recording: miss.recording.clone(),
            nearest: miss.nearest.clone(),
            fix: NOT_RECORDED_FIX.to_owned(),
        },
    )
}

/// `recording_invalid` (`ProgramError`, not retryable) for a Response
/// recording at `recording` that the engine cannot read or replay (ADR
/// 0105).
pub fn recording_invalid_error(recording: &str, reason: impl Into<String>) -> CallError {
    let reason = reason.into();
    CallError::new(
        &RECORDING_INVALID,
        format!("The Response recording `{recording}` cannot be replayed: {reason}."),
        &RecordingInvalidDetails {
            recording: recording.to_owned(),
            reason,
        },
    )
}

/// A live Run's opened replay recording: its start path and its indexed
/// WARC responses, shared across every tier and Session of the Run.
#[derive(Clone, Debug)]
pub struct Replayed {
    path: String,
    replay: Arc<Replay>,
}

impl Replayed {
    /// Its path, as the Run's start named it.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The init script that sets `Date` and seeded `Math.random` for pages
    /// replayed from this recording, skipping hosts `resolve` maps to loopback
    /// (ADR 0105, issue 801).
    pub fn init_script(&self, resolve: &HostMapping) -> String {
        init_script(
            self.replay.warcinfo_id(),
            &self.replay.document_times(),
            &resolve.loopback_hosts(),
        )
    }

    /// What the replay did so far, in the Run report's wire shape.
    pub fn report(&self) -> RecordingReport {
        let report = self.replay.report();
        RecordingReport {
            mode: "replay".to_owned(),
            file: self.path.clone(),
            responses_used: report.responses_used,
            recorded_from: report.recorded_from,
            recorded_to: report.recorded_to,
            written_by: report.written_by,
            misses: report
                .misses
                .into_iter()
                .map(|miss| miss.to_string())
                .collect(),
        }
    }
}

impl Recording for Replayed {
    fn recorded<'a>(&'a self, request: &'a CacheRequest) -> Replaying<'a> {
        let answer = match self.replay.answer(request) {
            Answer::Answered {
                served,
                recorded_at,
            } => Recorded::Answered {
                served,
                recorded_at,
            },
            Answer::Cut(served) => Recorded::Cut(served),
            Answer::Missed(miss) => Recorded::Missed(NotRecorded {
                method: miss.method,
                url: miss.url,
                recording: self.path.clone(),
                nearest: miss.nearest,
            }),
        };
        Box::pin(async move { answer })
    }
}

/// What a live Run has attached for its Response recording.
#[derive(Clone, Debug)]
enum Attached {
    Replay(Replayed),
    Record(Arc<Recorder>),
}

/// Each live Run's Response recording, keyed by its [`RunId`]: opened once
/// when the Run starts so every request of every tier and Session shares
/// the same index or recorder without reopening the file (ADRs 0105 and 0140).
#[derive(Clone, Debug, Default)]
pub struct Recordings {
    runs: Arc<Mutex<HashMap<RunId, Attached>>>,
}

impl Recordings {
    /// Reads and indexes the WARC file at `path` on `owner`'s blocking
    /// threads, failing with `recording_invalid` if the file cannot be
    /// read or is not a WARC recording the engine replays.
    ///
    /// # Errors
    ///
    /// `recording_invalid` when reading, decoding or parsing the file fails.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the blocking read to finish without
    /// attaching anything to any Run.
    pub async fn open_replay(
        &self,
        path: &str,
        masks: Masks,
        max_decompression_ratio: u64,
        owner: &TaskOwner,
    ) -> Result<Replayed, CallError> {
        let display = path.to_owned();
        let file = PathBuf::from(path);
        let parsed = owner
            .blocking(move || {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "runs inside the TaskOwner's blocking policy (STYLE.md)"
                )]
                let bytes = std::fs::read(&file).map_err(|error| error.to_string())?;
                Replay::open(&Bytes::from(bytes), masks, max_decompression_ratio)
                    .map_err(|error| error.to_string())
            })
            .await;
        match parsed {
            Ok(replay) => Ok(Replayed {
                path: display,
                replay: Arc::new(replay),
            }),
            Err(reason) => Err(recording_invalid_error(&display, reason)),
        }
    }

    /// Parses and indexes the WARC bytes `bytes` (sent by the SDK over the
    /// connection, or empty for a test engine's empty replay) on `owner`'s
    /// blocking threads, failing with `recording_invalid` if `bytes` is not
    /// a WARC recording the engine replays.
    ///
    /// # Errors
    ///
    /// `recording_invalid` when decoding or parsing `bytes` fails.
    pub async fn open_replay_bytes(
        &self,
        display: &str,
        bytes: Bytes,
        masks: Masks,
        max_decompression_ratio: u64,
        owner: &TaskOwner,
    ) -> Result<Replayed, CallError> {
        let display = display.to_owned();
        let parsed = owner
            .blocking(move || {
                Replay::open(&bytes, masks, max_decompression_ratio)
                    .map_err(|error| error.to_string())
            })
            .await;
        match parsed {
            Ok(replay) => Ok(Replayed {
                path: display,
                replay: Arc::new(replay),
            }),
            Err(reason) => Err(recording_invalid_error(&display, reason)),
        }
    }

    /// Attaches a replaying recording to the live Run `run_id`.
    pub fn attach(&self, run_id: RunId, replayed: Replayed) {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run_id, Attached::Replay(replayed));
    }

    /// Attaches a live [`Recorder`] (`mode: "record"`) to `run_id`.
    pub fn attach_recorder(&self, run_id: RunId, recorder: Arc<Recorder>) {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run_id, Attached::Record(recorder));
    }

    /// The Run's replay recording as a [`Recording`], if it replays one.
    pub fn get(&self, run_id: &RunId) -> Option<Arc<dyn Recording>> {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .and_then(|attached| match attached {
                Attached::Replay(replayed) => {
                    Some(Arc::new(replayed.clone()) as Arc<dyn Recording>)
                }
                Attached::Record(_) => None,
            })
    }

    /// The Run's [`Recorder`], if it records one (`mode: "record"`).
    pub fn recorder(&self, run_id: &RunId) -> Option<Arc<Recorder>> {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .and_then(|attached| match attached {
                Attached::Record(recorder) => Some(Arc::clone(recorder)),
                Attached::Replay(_) => None,
            })
    }

    /// Whether `run_id` replays a Response recording.
    pub fn is_replaying(&self, run_id: &RunId) -> bool {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .is_some_and(|attached| matches!(attached, Attached::Replay(_)))
    }

    /// The page init script for `run_id`'s replay recording, or `None` when
    /// `run_id` is not replaying (ADR 0105, issue 801).
    pub fn replay_init_script(&self, run_id: &RunId, resolve: &HostMapping) -> Option<String> {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .and_then(|attached| match attached {
                Attached::Replay(replayed) => Some(replayed.init_script(resolve)),
                Attached::Record(_) => None,
            })
    }

    /// Removes `run_id`'s recording (replay or record) without writing a file,
    /// returning its replay [`RecordingReport`] if it was replaying.
    pub fn take_report_of(&self, run_id: &RunId) -> Option<RecordingReport> {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(run_id)
            .and_then(|attached| match attached {
                Attached::Replay(replayed) => Some(replayed.report()),
                Attached::Record(_) => None,
            })
    }

    /// Finishes `run_id`'s recording when the Run ends: returns a replay
    /// report, or writes a `mode: "record"` WARC file atomically from
    /// `connection`'s `recorded_responses` table (ADRs 0105 and 0116).
    ///
    /// # Errors
    ///
    /// Returns any SQLite error from reading/flushing `recorded_responses`.
    pub(crate) fn finish_run(
        &self,
        run_id: &RunId,
        connection: &rusqlite::Connection,
    ) -> rusqlite::Result<Option<Result<RecordingReport, goodfetch_protocol::run::EndingError>>>
    {
        let attached = self
            .runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(run_id);
        match attached {
            Some(Attached::Replay(replayed)) => Ok(Some(Ok(replayed.report()))),
            Some(Attached::Record(recorder)) => recorder.finish_and_write(connection).map(Some),
            None => Ok(None),
        }
    }
}

/// The records a response's request and a revisit's original are found
/// among.
struct Index<'r> {
    by_id: HashMap<&'r str, &'r Record>,
    /// Each record's request, by the id of the record it is concurrent to.
    requests: HashMap<&'r str, &'r Record>,
    /// The first response of each payload digest.
    by_payload: HashMap<&'r str, &'r Record>,
}

impl<'r> Index<'r> {
    fn of(records: &'r [Record]) -> Self {
        let mut index = Self {
            by_id: HashMap::new(),
            requests: HashMap::new(),
            by_payload: HashMap::new(),
        };
        for record in records {
            if let Some(id) = record.record_id() {
                index.by_id.entry(id).or_insert(record);
            }
            match record.record_type() {
                RecordType::Request => {
                    for id in concurrent_to(record) {
                        index.requests.entry(id).or_insert(record);
                    }
                }
                RecordType::Response => {
                    if let Some(digest) = record.header("WARC-Payload-Digest") {
                        index.by_payload.entry(digest).or_insert(record);
                    }
                }
                _ => {}
            }
        }
        for record in records {
            let Some(id) = record.record_id() else {
                continue;
            };
            let request = concurrent_to(record)
                .filter_map(|other| index.by_id.get(other).copied())
                .find(|other| other.record_type() == RecordType::Request);
            if let Some(request) = request {
                index.requests.entry(id).or_insert(request);
            }
        }
        index
    }

    /// The request `response` answered, linked by `WARC-Concurrent-To`
    /// either way.
    fn request_of(&self, response: &Record) -> Option<&'r Record> {
        self.requests.get(response.record_id()?).copied()
    }

    /// The response whose payload the revisit `revisit` repeats: the one
    /// its `WARC-Refers-To` names, else the first of its payload digest.
    fn original(&self, revisit: &Record) -> Option<&'r Record> {
        let refers_to = revisit
            .header("WARC-Refers-To")
            .and_then(|id| self.by_id.get(id).copied())
            .filter(|record| record.record_type() == RecordType::Response);
        refers_to.or_else(|| {
            revisit
                .header("WARC-Payload-Digest")
                .and_then(|digest| self.by_payload.get(digest).copied())
        })
    }
}

/// The ids each of `record`'s `WARC-Concurrent-To` headers names.
fn concurrent_to(record: &Record) -> impl Iterator<Item = &str> {
    record
        .headers()
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("WARC-Concurrent-To"))
        .map(|(_, value)| value.as_str())
}

fn parse_response(record: &Record) -> Result<HttpResponse, RecordingInvalid> {
    HttpResponse::parse(record.block()).map_err(|source| not_http(record, source))
}

fn parse_request(record: &Record) -> Result<HttpRequest, RecordingInvalid> {
    HttpRequest::parse(record.block()).map_err(|source| not_http(record, source))
}

fn not_http(record: &Record, source: warc::NotHttp) -> RecordingInvalid {
    RecordingInvalid::NotHttp {
        record: record.record_id().unwrap_or_default().to_owned(),
        source,
    }
}

fn served(response: HttpResponse) -> Served {
    Served {
        head: ResponseHead {
            status: response.status,
            headers: response.headers,
        },
        body: response.body,
    }
}

/// What a revisit answers, and whether it broke off: when its server
/// answered `304 Not Modified`, the original's status and body under the
/// original's headers updated by the revisit's, as a cache updates them
/// (RFC 9111 §4.3.4); else the revisit's status and headers with the
/// original's body.
fn revisited(revisit: &Record, response: &HttpResponse, original: &HttpResponse) -> (Served, bool) {
    let not_modified = revisit
        .header("WARC-Profile")
        .is_some_and(|profile| profile.ends_with(SERVER_NOT_MODIFIED));
    let (status, headers) = if not_modified {
        let mut headers: Vec<(String, String)> = original
            .headers
            .iter()
            .filter(|(name, _)| !response.headers.iter().any(|(other, _)| other == name))
            .cloned()
            .collect();
        headers.extend(response.headers.iter().cloned());
        (original.status, headers)
    } else {
        (response.status, response.headers.clone())
    };
    let served = Served {
        head: ResponseHead { status, headers },
        body: original.body.clone(),
    };
    (served, original.cut)
}

/// Whether `record` says its payload broke off, for any `WARC-Truncated`
/// reason.
fn truncated(record: &Record) -> bool {
    record.header("WARC-Truncated").is_some()
}

/// The `software` field of a `warcinfo` record's block.
fn software(warcinfo: &Record) -> Option<String> {
    String::from_utf8_lossy(warcinfo.block())
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("software"))
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The request headers a response's `Vary` names, in lower case, but `*`
/// and credentials.
fn vary_names(headers: &[(String, String)]) -> impl Iterator<Item = String> {
    let mut names: Vec<String> = headers
        .iter()
        .filter(|(name, _)| name == "vary")
        .flat_map(|(_, value)| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty() && name != "*" && !CREDENTIALS.contains(&name.as_str()))
        .collect();
    names.sort();
    names.dedup();
    names.into_iter()
}

/// The values of `headers`' `name`, trimmed and joined with `, `, as a
/// header sent several times reads; `None` when it is not sent.
fn joined(headers: &[(String, String)], name: &str) -> Option<String> {
    let values: Vec<&str> = headers
        .iter()
        .filter(|(header, _)| header == name)
        .map(|(_, value)| value.trim())
        .collect();
    (!values.is_empty()).then(|| values.join(", "))
}

/// A URL's host and path, which a [`Miss`]'s nearest URLs share.
fn place(url: &url::Url) -> (String, String) {
    (
        url.host_str().unwrap_or_default().to_owned(),
        url.path().to_owned(),
    )
}
