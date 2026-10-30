//! The discovered-API tier's plans (spec #579; ADR 0020; pitfall B09):
//! the JSON calls a browser made while it loaded a sample, read as plans
//! the tier replays over HTTP for a later page of the same Label.
//!
//! [`infer`] reads the calls a sample's browser page made, each JSON
//! answer to a `GET`, in the order they came, a repeated URL counted once.
//! Calls to one endpoint (an origin and a path whose number segments may
//! differ) form one series, which is one of three plans, V1's (ADR 0020):
//!
//! - **once**: the endpoint was called once, and is called again as it was;
//! - **page number**: one number, in the query or the path, counts up by
//!   one from call to call, everything else the same; replayed from the
//!   first call's number until an answer holds no item (no array with an
//!   entry anywhere in it) or fails;
//! - **cursor**: from the second call on, one query parameter carries a
//!   string the call before's answer held at one path of object keys,
//!   the same for every call; replayed until an answer holds no cursor
//!   there, or the one it held before.
//!
//! An endpoint whose calls follow none of them leaves no plan at all: its
//! page stays on the browser. A request a site signs, or a token it takes
//! once, follows the plan but fails when replayed, and the comparison
//! keeps its Label on the browser (ADR 0020).
//!
//! A plan learned on one document serves another of its Label: each path
//! segment and query value of a series' first call that equals one of the
//! sample document's, the series' own counted or cursor parameter aside,
//! takes the new document's value in its place.
//!
//! Each series is replayed to its end before the next starts; the tier
//! asks for a next call only while a wait remains unanswered.

#![expect(
    clippy::disallowed_types,
    reason = "a capture's calls are pushed and read in one step each, never under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use goodfetch_page::page_value::{Kept, ObservedResponse, ResponseObserver};
use goodfetch_page::url_pattern::UrlPattern;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

/// The most calls one series replays.
const MAX_CALLS: usize = 100;

/// The most bytes of one call's answer a capture reads.
const MAX_ANSWER: usize = 4 * 1024 * 1024;

/// What a sample's browser page captures of the calls it makes (the
/// response observer of ADR 0110, registered on the sample's browser
/// run): each `2xx` answer typed as JSON whose whole body parses, in the
/// order they came. The observer is told no method, so a call another
/// method sent is captured too; replayed as a `GET`, its answer differs,
/// and the comparison keeps its Label on the browser.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl Capture {
    /// The calls captured so far.
    pub fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ResponseObserver for Capture {
    fn kept(&self) -> Kept {
        Kept {
            urls: vec![UrlPattern::glob("").expect("invariant: the empty glob compiles")],
            byte_cap: MAX_ANSWER,
        }
    }

    fn on_response(&self, response: &ObservedResponse<'_>, body: Option<&[u8]>) {
        let json = response.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-type") && value.to_ascii_lowercase().contains("json")
        });
        if !(200..300).contains(&response.status) || !json {
            return;
        }
        let Ok(url) = Url::parse(response.url) else {
            return;
        };
        // A body the browser no longer held, one past the Run's limits or
        // one past the cap, is captured without its answer: its URL still
        // tells a page number, and the same call made again may bring its
        // answer.
        let json = match body {
            Some(body) if response.body_length <= body.len() => {
                serde_json::from_slice::<Value>(body).unwrap_or(Value::Null)
            }
            Some(_) | None => Value::Null,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call { url, json });
    }
}

/// A JSON call a browser page made: its URL and its answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub url: Url,
    pub json: Value,
}

/// What a Label learned of the calls its pages make, from one sample.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// The sample's document URL.
    document: String,
    series: Vec<Series>,
}

/// The calls of one endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Series {
    Once {
        first: String,
    },
    PageNumber {
        first: String,
        at: Place,
    },
    Cursor {
        first: String,
        param: String,
        path: Vec<String>,
    },
}

/// Where a page number sits in a call's URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "in", rename_all = "snake_case")]
enum Place {
    Query { name: String },
    Segment { index: usize },
}

/// The replay of a plan for one document: the calls it asks for, one at
/// a time.
#[derive(Clone, Debug)]
pub struct Replay {
    series: Vec<Series>,
    /// The series being replayed and the call it asked for last.
    at: usize,
    asked: Option<Url>,
    /// How many calls the current series asked for.
    calls: usize,
    /// The cursors the current series sent so far.
    cursors: BTreeSet<String>,
    /// The next call of the current series, once an answer gave it.
    next: Option<Url>,
}

/// The plan the `calls` a browser made while it loaded `document` follow;
/// `None` when it made none, or one endpoint's calls follow no plan.
pub fn infer(document: &Url, calls: &[Call]) -> Option<Plan> {
    let mut seen = BTreeSet::new();
    let mut groups: Vec<(String, Vec<&Call>)> = Vec::new();
    for call in calls {
        if !seen.insert(call.url.as_str().to_owned()) {
            // The same call made again brings the answer the first lacked.
            if !call.json.is_null()
                && let Some(kept) = groups
                    .iter_mut()
                    .flat_map(|(_, group)| group.iter_mut())
                    .find(|kept| kept.url == call.url && kept.json.is_null())
            {
                *kept = call;
            }
            continue;
        }
        let key = endpoint(&call.url);
        match groups.iter_mut().find(|(group, _)| *group == key) {
            Some((_, group)) => group.push(call),
            None => groups.push((key, vec![call])),
        }
    }
    if groups.is_empty() {
        return None;
    }
    let series = groups
        .iter()
        .map(|(_, group)| series(group))
        .collect::<Option<Vec<_>>>()?;
    Some(Plan {
        document: document.to_string(),
        series,
    })
}

/// An endpoint: its origin and path, each number segment masked.
fn endpoint(url: &Url) -> String {
    let path: Vec<&str> = url
        .path()
        .split('/')
        .map(|segment| {
            if number(segment).is_some() {
                "#"
            } else {
                segment
            }
        })
        .collect();
    format!("{}{}", url.origin().ascii_serialization(), path.join("/"))
}

fn number(text: &str) -> Option<u64> {
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// The parts of `url` a plan reads: its path segments and its query pairs.
fn parts(url: &Url) -> (Vec<String>, Vec<(String, String)>) {
    (
        url.path().split('/').map(str::to_owned).collect(),
        url.query_pairs()
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect(),
    )
}

/// The plan of one endpoint's `calls`, in whatever order they were
/// captured: a page's calls are captured as their answers are read, which
/// need not be the order they were made in.
fn series(calls: &[&Call]) -> Option<Series> {
    if let [only] = calls {
        return Some(Series::Once {
            first: only.url.to_string(),
        });
    }
    if let Some(ordered) = by_page_number(calls)
        && let Some(at) = page_number(&ordered)
    {
        return Some(Series::PageNumber {
            first: ordered.first()?.url.to_string(),
            at,
        });
    }
    let ordered = by_cursor(calls)?;
    let (param, path) = cursor(&ordered)?;
    Some(Series::Cursor {
        first: ordered.first()?.url.to_string(),
        param,
        path,
    })
}

/// `calls` in the order of the one number in which the first differs
/// from each other; `None` when they differ in more than one place.
fn by_page_number<'a>(calls: &[&'a Call]) -> Option<Vec<&'a Call>> {
    let first = parts(&calls.first()?.url);
    let place = differing(&first, &parts(&calls.get(1)?.url))?;
    let mut keyed = calls
        .iter()
        .map(|call| {
            let (segments, pairs) = parts(&call.url);
            let at = match &place {
                Place::Query { name } => query(&pairs, name)?,
                Place::Segment { index } => segments.get(*index)?,
            };
            Some((number(at)?, *call))
        })
        .collect::<Option<Vec<_>>>()?;
    keyed.sort_by_key(|(number, _)| *number);
    Some(keyed.into_iter().map(|(_, call)| call).collect())
}

/// `calls` in the order their cursors chain them: first the one no other
/// call's answer leads to, then each the one before's answer leads to;
/// `None` when they form no such chain.
fn by_cursor<'a>(calls: &[&'a Call]) -> Option<Vec<&'a Call>> {
    let leads = |from: &Call, to: &Call| cursor(&[from, to]).is_some();
    let start = calls
        .iter()
        .find(|call| !calls.iter().any(|other| leads(other, call)))?;
    let mut ordered = vec![*start];
    let mut left: Vec<&Call> = calls
        .iter()
        .filter(|call| !std::ptr::eq(**call, *start))
        .copied()
        .collect();
    while !left.is_empty() {
        let last = *ordered.last()?;
        let next = left.iter().position(|call| leads(last, call))?;
        ordered.push(left.remove(next));
    }
    Some(ordered)
}

/// Where the page number of `calls` sits, when one number alone counts up
/// by one from call to call.
fn page_number(calls: &[&Call]) -> Option<Place> {
    let mut place = None;
    for pair in calls.windows(2) {
        let (before, after) = (parts(&pair[0].url), parts(&pair[1].url));
        let found = differing(&before, &after)?;
        let (was, is) = match &found {
            Place::Query { name } => (query(&before.1, name)?, query(&after.1, name)?),
            Place::Segment { index } => (before.0.get(*index)?, after.0.get(*index)?),
        };
        if number(was)?.checked_add(1)? != number(is)? {
            return None;
        }
        if place.get_or_insert_with(|| found.clone()) != &found {
            return None;
        }
    }
    place
}

/// The one place where `before` and `after` differ, when they hold the
/// same segments and query names and only one value differs.
fn differing(
    before: &(Vec<String>, Vec<(String, String)>),
    after: &(Vec<String>, Vec<(String, String)>),
) -> Option<Place> {
    if before.0.len() != after.0.len() || before.1.len() != after.1.len() {
        return None;
    }
    let mut places = before
        .0
        .iter()
        .zip(&after.0)
        .enumerate()
        .filter(|(_, (was, is))| was != is)
        .map(|(index, _)| Place::Segment { index })
        .chain(
            before
                .1
                .iter()
                .zip(&after.1)
                .filter(|((was_name, was), (is_name, is))| was_name != is_name || was != is)
                .map(|((name, _), (is_name, _))| {
                    if name == is_name {
                        Place::Query { name: name.clone() }
                    } else {
                        // A reordered query is no page number.
                        Place::Query {
                            name: String::new(),
                        }
                    }
                }),
        );
    let place = places.next()?;
    if places.next().is_some()
        || place
            == (Place::Query {
                name: String::new(),
            })
    {
        return None;
    }
    Some(place)
}

fn query<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a String> {
    pairs
        .iter()
        .find(|(pair, _)| pair == name)
        .map(|(_, value)| value)
}

/// The query parameter that carries the cursor of `calls` and the path of
/// object keys each answer holds the next one at.
fn cursor(calls: &[&Call]) -> Option<(String, Vec<String>)> {
    let mut found: Option<(String, Vec<String>)> = None;
    for pair in calls.windows(2) {
        let (before, after) = (parts(&pair[0].url), parts(&pair[1].url));
        if before.0 != after.0 {
            return None;
        }
        // Every query pair the call before sent, the cursor's aside.
        let candidates: Vec<&(String, String)> = after
            .1
            .iter()
            .filter(|pair| !before.1.contains(pair))
            .collect();
        let [(param, value)] = candidates.as_slice() else {
            return None;
        };
        let rest = |pairs: &[(String, String)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .filter(|(name, _)| name != param)
                .cloned()
                .collect()
        };
        if rest(&before.1) != rest(&after.1) {
            return None;
        }
        let path = path_to(&pair[0].json, value)?;
        let this = (param.clone(), path);
        if found.get_or_insert_with(|| this.clone()) != &this {
            return None;
        }
    }
    found
}

/// The path of object keys at which `json` holds the string `value`.
fn path_to(json: &Value, value: &str) -> Option<Vec<String>> {
    match json {
        Value::String(held) if held == value => Some(Vec::new()),
        Value::Number(held) if held.to_string() == value => Some(Vec::new()),
        Value::Object(fields) => fields.iter().find_map(|(key, field)| {
            let mut path = path_to(field, value)?;
            path.insert(0, key.clone());
            Some(path)
        }),
        _ => None,
    }
}

/// What `json` holds at `path`, as a cursor: a string, or a number.
fn at_path(json: &Value, path: &[String]) -> Option<String> {
    let held = path.iter().try_fold(json, |json, key| json.get(key))?;
    match held {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// Whether `json` holds an item: an array with an entry, anywhere in it.
fn holds_items(json: &Value) -> bool {
    match json {
        Value::Array(entries) => !entries.is_empty(),
        Value::Object(fields) => fields.values().any(holds_items),
        _ => false,
    }
}

impl Plan {
    /// The replay of the plan for `document`, a page of the sample's Label.
    pub fn replay(&self, document: &Url) -> Replay {
        let sample = Url::parse(&self.document).ok();
        let series = self
            .series
            .iter()
            .map(|series| match &sample {
                Some(sample) => series.mapped(sample, document),
                None => series.clone(),
            })
            .collect();
        Replay {
            series,
            at: 0,
            asked: None,
            calls: 0,
            cursors: BTreeSet::new(),
            next: None,
        }
    }
}

impl Series {
    fn first(&self) -> &str {
        match self {
            Self::Once { first } | Self::PageNumber { first, .. } | Self::Cursor { first, .. } => {
                first
            }
        }
    }

    /// The series as `document` calls it, learned on `sample`.
    fn mapped(&self, sample: &Url, document: &Url) -> Self {
        let Ok(mut first) = Url::parse(self.first()) else {
            return self.clone();
        };
        let (own_segment, own_query) = match self {
            Self::Once { .. } => (None, None),
            Self::PageNumber {
                at: Place::Segment { index },
                ..
            } => (Some(*index), None),
            Self::PageNumber {
                at: Place::Query { name },
                ..
            } => (None, Some(name.as_str())),
            Self::Cursor { param, .. } => (None, Some(param.as_str())),
        };
        let (sample_segments, sample_query) = parts(sample);
        let (document_segments, document_query) = parts(document);
        let (segments, pairs) = parts(&first);
        let segments: Vec<String> = segments
            .into_iter()
            .enumerate()
            .map(|(index, segment)| {
                if Some(index) == own_segment || segment.is_empty() {
                    return segment;
                }
                sample_segments
                    .iter()
                    .position(|held| *held == segment)
                    .and_then(|position| document_segments.get(position))
                    .cloned()
                    .unwrap_or(segment)
            })
            .collect();
        let pairs: Vec<(String, String)> = pairs
            .into_iter()
            .map(|(name, value)| {
                if Some(name.as_str()) == own_query {
                    return (name, value);
                }
                let mapped = query(&sample_query, &name)
                    .filter(|held| **held == value)
                    .and_then(|_| query(&document_query, &name))
                    .cloned()
                    .unwrap_or(value);
                (name, mapped)
            })
            .collect();
        first.set_path(&segments.join("/"));
        if pairs.is_empty() {
            first.set_query(None);
        } else {
            first.query_pairs_mut().clear().extend_pairs(pairs);
        }
        let first = first.to_string();
        match self {
            Self::Once { .. } => Self::Once { first },
            Self::PageNumber { at, .. } => Self::PageNumber {
                first,
                at: at.clone(),
            },
            Self::Cursor { param, path, .. } => Self::Cursor {
                first,
                param: param.clone(),
                path: path.clone(),
            },
        }
    }
}

impl Replay {
    /// The next call to replay, once the one asked for before was answered
    /// or failed; `None` once every series ended.
    pub fn next_call(&mut self) -> Option<Url> {
        loop {
            let series = self.series.get(self.at)?;
            let next = if self.asked.is_none() {
                Url::parse(series.first()).ok()
            } else {
                self.next.take()
            };
            match next {
                Some(url) if self.calls < MAX_CALLS => {
                    self.calls += 1;
                    self.asked = Some(url.clone());
                    return Some(url);
                }
                _ => self.end_series(),
            }
        }
    }

    /// Takes the answer to the call asked for last, which gives its
    /// series' next call, if any.
    pub fn answered(&mut self, answer: &Value) {
        let (Some(series), Some(asked)) = (self.series.get(self.at), &self.asked) else {
            return;
        };
        self.next = match series {
            Series::Once { .. } => None,
            Series::PageNumber { at, .. } => {
                holds_items(answer).then(|| following(asked, at)).flatten()
            }
            Series::Cursor { param, path, .. } => at_path(answer, path)
                .filter(|cursor| self.cursors.insert(cursor.clone()))
                .map(|cursor| {
                    let mut next = asked.clone();
                    let pairs: Vec<(String, String)> = asked
                        .query_pairs()
                        .filter(|(name, _)| name != param)
                        .map(|(name, value)| (name.into_owned(), value.into_owned()))
                        .chain([(param.clone(), cursor)])
                        .collect();
                    next.query_pairs_mut().clear().extend_pairs(pairs);
                    next
                }),
        };
        if self.next.is_none() {
            self.end_series();
        }
    }

    /// Takes the failure of the call asked for last, which ends its series.
    pub fn failed(&mut self) {
        self.end_series();
    }

    fn end_series(&mut self) {
        self.at += 1;
        self.asked = None;
        self.calls = 0;
        self.cursors.clear();
        self.next = None;
    }
}

/// The call after `asked`, its page number at `at` one up.
fn following(asked: &Url, at: &Place) -> Option<Url> {
    let mut next = asked.clone();
    match at {
        Place::Query { name } => {
            let pairs: Vec<(String, String)> = asked
                .query_pairs()
                .map(|(pair, value)| {
                    let value = if pair == *name {
                        number(&value)
                            .and_then(|page| page.checked_add(1))
                            .map_or_else(|| value.clone().into_owned(), |page| page.to_string())
                    } else {
                        value.into_owned()
                    };
                    (pair.into_owned(), value)
                })
                .collect();
            next.query_pairs_mut().clear().extend_pairs(pairs);
        }
        Place::Segment { index } => {
            let mut segments: Vec<String> = asked.path().split('/').map(str::to_owned).collect();
            let segment = segments.get_mut(*index)?;
            *segment = number(segment)?.checked_add(1)?.to_string();
            next.set_path(&segments.join("/"));
        }
    }
    Some(next)
}

#[cfg(test)]
mod tests;
