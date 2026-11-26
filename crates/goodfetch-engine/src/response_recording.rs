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

use std::collections::HashMap;
use std::time::SystemTime;

use bytes::Bytes;

use crate::mask::{Masks, Scope};
use crate::response_cache::{CacheRequest, ResponseHead, Served, body_sha256};
use crate::sync::Mutex;
use crate::warc::{self, HttpRequest, HttpResponse, Record, RecordType};

#[cfg(test)]
mod tests;

/// The most recorded URLs a [`Miss`] names.
const NEAREST: usize = 5;

/// The request headers a `Vary` names that matching leaves out: a replayed
/// Run sends its own credentials, not the recorded ones.
const CREDENTIALS: [&str; 3] = ["cookie", "authorization", "proxy-authorization"];

/// The `WARC-Profile` of a revisit whose server answered `304 Not
/// Modified`, in WARC 1.1's and 1.0's spelling.
const SERVER_NOT_MODIFIED: &str = "/revisit/server-not-modified";

/// Why a file is not a Response recording the engine replays:
/// `recording_invalid`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
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
    /// The recorded response.
    Answered(Served),
    /// The recorded response, which broke off: it replays as
    /// `response_truncated`.
    Cut(Served),
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
    served: Served,
    recorded_at: Option<SystemTime>,
    cut: bool,
    /// Its place in [`Used::answered`].
    index: usize,
}

impl Entry {
    fn matches(&self, request: &CacheRequest, body_keys: &[String]) -> bool {
        self.method.eq_ignore_ascii_case(&request.method)
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
    /// engine keeps.
    pub fn open(contents: &Bytes, masks: Masks) -> Result<Self, RecordingInvalid> {
        let records = warc::read(contents)?;
        let index = Index::of(&records);
        let mut replay = Self {
            masks,
            entries: HashMap::new(),
            places: HashMap::new(),
            written_by: records
                .iter()
                .find(|record| record.record_type() == RecordType::Warcinfo)
                .and_then(software),
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
                match index.original(record) {
                    Some(original) => revisited(record, &response, &parse_response(original)?),
                    None => continue,
                }
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
        let entry = Entry {
            method: request.map_or_else(|| "GET".to_owned(), |request| request.method.clone()),
            body_key: request.map_or_else(
                || body_sha256(None, b""),
                |request| body_sha256(first(&request.headers, "content-type"), &request.body),
            ),
            vary,
            served,
            recorded_at: record.date().map(SystemTime::from),
            cut: cut || record.header("WARC-Truncated").is_some(),
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
            Answer::Cut(entry.served.clone())
        } else {
            Answer::Answered(entry.served.clone())
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

/// The value of `headers`' first `name`.
fn first<'h>(headers: &'h [(String, String)], name: &str) -> Option<&'h str> {
    headers
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.as_str())
}

/// A URL's host and path, which a [`Miss`]'s nearest URLs share.
fn place(url: &url::Url) -> (String, String) {
    (
        url.host_str().unwrap_or_default().to_owned(),
        url.path().to_owned(),
    )
}
