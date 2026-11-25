//! The Identity consistency test's helper (ADR 0074): the Proving ground's
//! consistency probe pages (#328) read each surface of the browser's
//! Identity profile in every context a page can run script in, the main
//! frame, a cross-origin iframe and dedicated, shared and service workers,
//! and post each value to the site, whose observations keep it beside the
//! request that carried it. [`PageValues::read`] gathers one surface's
//! values from every context and from the headers of each context's
//! requests, where a header carries the surface, and [`identity_report`]
//! compares them with the main frame's.
//!
//! A context's value is `null` where the context lacks the surface, such as
//! a worker's screen, and is then left out; a context that reported nothing
//! is flagged, since a probe that failed in it would hide the very
//! difference the test looks for. A header is compared where the request
//! carried it: the client hints beyond the default three are sent only to
//! a site that asks for them.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::proving_ground_format::{ObservedRequest, ReceivedRequests};

/// The host name of the main frame and the workers.
pub const CONSISTENCY_HOST: &str = "consistency.test";

/// The host name of the cross-origin iframe.
pub const CONSISTENCY_FRAME_HOST: &str = "consistency-frame.test";

/// A context the consistency probes run script in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProbeContext {
    /// The top-level page.
    MainFrame,
    /// The iframe of another site, out of process.
    CrossOriginFrame,
    /// A dedicated worker the main frame starts.
    DedicatedWorker,
    /// A shared worker the main frame starts.
    SharedWorker,
    /// A service worker the main frame registers.
    ServiceWorker,
}

impl ProbeContext {
    /// Every context, the main frame first.
    pub const ALL: [Self; 5] = [
        Self::MainFrame,
        Self::CrossOriginFrame,
        Self::DedicatedWorker,
        Self::SharedWorker,
        Self::ServiceWorker,
    ];

    /// The page it reports as: its document's or its script's path.
    pub fn page(self) -> &'static str {
        match self {
            Self::MainFrame => "/",
            Self::CrossOriginFrame => "/frame",
            Self::DedicatedWorker => "/dedicated-worker.js",
            Self::SharedWorker => "/shared-worker.js",
            Self::ServiceWorker => "/service-worker.js",
        }
    }

    /// The host its requests go to.
    pub fn host(self) -> &'static str {
        match self {
            Self::CrossOriginFrame => CONSISTENCY_FRAME_HOST,
            _ => CONSISTENCY_HOST,
        }
    }

    /// The context reporting as `page`, if any.
    fn of_page(page: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|context| context.page() == page)
    }
}

impl fmt::Display for ProbeContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MainFrame => "the main frame",
            Self::CrossOriginFrame => "the cross-origin iframe",
            Self::DedicatedWorker => "the dedicated worker",
            Self::SharedWorker => "the shared worker",
            Self::ServiceWorker => "the service worker",
        })
    }
}

/// Where a value was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueSource {
    /// What a context's script read.
    Script(ProbeContext),
    /// What a header of a context's requests sent.
    Headers(ProbeContext),
}

impl fmt::Display for ValueSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Script(context) => write!(formatter, "{context}"),
            Self::Headers(context) => write!(formatter, "the headers of {context}'s requests"),
        }
    }
}

/// One surface's values in every context, and in the headers of each
/// context's requests.
#[derive(Clone, Debug, PartialEq)]
pub struct PageValues {
    /// The surface, as the probes name it, such as `user_agent`.
    pub surface: String,
    /// Each value read, with where; a header's once per value it sent.
    pub values: Vec<(ValueSource, Value)>,
}

/// Why the probes' reports could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConsistencyError {
    /// The observations hold no reports, or a report this helper cannot
    /// read.
    #[error("the observations' reports cannot be read: {0}")]
    Reports(String),
    /// A context reported a value that is not JSON text.
    #[error("{context} reported {surface} as {observed:?}, which is not JSON")]
    NotJson {
        /// The context.
        context: ProbeContext,
        /// The surface.
        surface: String,
        /// What it reported.
        observed: String,
    },
}

/// One report a page's script posted, as the observations list it.
#[derive(Deserialize)]
struct ReportRecord {
    task: Option<String>,
    page: String,
    check: String,
    observed: String,
}

impl PageValues {
    /// Every surface the probes reported in a Proving ground's
    /// `observations`, each with its values, by the surface's name.
    ///
    /// # Errors
    ///
    /// `Reports` when the observations hold no list of reports, and
    /// `NotJson` when a context reported a value that is not JSON text.
    ///
    /// # Panics
    ///
    /// When the observations hold no list of requests, as
    /// [`ReceivedRequests::of`] does.
    pub fn read(observations: &Value) -> Result<Vec<Self>, ConsistencyError> {
        let reports = Vec::<ReportRecord>::deserialize(&observations["reports"])
            .map_err(|error| ConsistencyError::Reports(error.to_string()))?;
        let requests = ReceivedRequests::of(observations);
        let mut surfaces: BTreeMap<String, Vec<(ValueSource, Value)>> = BTreeMap::new();
        for report in reports {
            // The consistency pages have no task; a task's page of the same
            // path is another family's.
            let Some(context) = report
                .task
                .is_none()
                .then(|| ProbeContext::of_page(&report.page))
                .flatten()
            else {
                continue;
            };
            let value =
                serde_json::from_str(&report.observed).map_err(|_| ConsistencyError::NotJson {
                    context,
                    surface: report.check.clone(),
                    observed: report.observed.clone(),
                })?;
            surfaces
                .entry(report.check)
                .or_default()
                .push((ValueSource::Script(context), value));
        }
        Ok(surfaces
            .into_iter()
            .map(|(surface, mut values)| {
                for context in ProbeContext::ALL {
                    let target = format!("/reports?page={}", context.page());
                    let sent = requests.of_the_case().filter(|request| {
                        request.host == context.host() && request.target == target
                    });
                    for value in sent.filter_map(|request| header_value(&surface, request)) {
                        let entry = (ValueSource::Headers(context), value);
                        if !values.contains(&entry) {
                            values.push(entry);
                        }
                    }
                }
                Self { surface, values }
            })
            .collect())
    }
}

/// What a surface's values in every context and their headers show.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityReport {
    /// The surface.
    pub surface: String,
    /// The main frame's value, which every other is compared with; `None`
    /// when the main frame reported none.
    pub main_frame: Option<Value>,
    /// Each value that differs from the main frame's, with where it was
    /// read.
    pub differing: Vec<(ValueSource, Value)>,
    /// Each context that reported no value of the surface.
    pub unreported: Vec<ProbeContext>,
}

impl IdentityReport {
    /// Whether every context reported the surface, and every value agrees
    /// with the main frame's.
    pub fn is_consistent(&self) -> bool {
        self.main_frame.is_some() && self.differing.is_empty() && self.unreported.is_empty()
    }
}

impl fmt::Display for IdentityReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let main_frame = self
            .main_frame
            .as_ref()
            .map_or_else(|| "nothing".to_owned(), Value::to_string);
        write!(
            formatter,
            "{}: the main frame reports {main_frame}",
            self.surface
        )?;
        for (source, value) in &self.differing {
            write!(formatter, "; {source} gives {value}")?;
        }
        for context in &self.unreported {
            write!(formatter, "; {context} reported nothing")?;
        }
        if self.is_consistent() {
            formatter.write_str(", and every context agrees")?;
        }
        Ok(())
    }
}

/// Compares one surface's values in every context and in the headers of
/// each context's requests with the main frame's: each that differs, and
/// each context that reported none, is flagged. A context's `null`, which
/// says it lacks the surface, is left out, and a header is compared with
/// the main frame's value as Chromium sends it, a language list expanded.
pub fn identity_report(page_values: &PageValues) -> IdentityReport {
    let main_frame = page_values
        .values
        .iter()
        .find(|(source, _)| *source == ValueSource::Script(ProbeContext::MainFrame))
        .map(|(_, value)| value.clone());
    let unreported = ProbeContext::ALL
        .into_iter()
        .filter(|context| {
            !page_values
                .values
                .iter()
                .any(|(source, _)| *source == ValueSource::Script(*context))
        })
        .collect();
    let differing = main_frame
        .as_ref()
        .map(|expected| {
            page_values
                .values
                .iter()
                .filter(|(source, value)| match source {
                    ValueSource::Script(_) => !value.is_null() && value != expected,
                    ValueSource::Headers(_) => {
                        *value != as_headers_send(&page_values.surface, expected)
                    }
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    IdentityReport {
        surface: page_values.surface.clone(),
        main_frame,
        differing,
        unreported,
    }
}

/// How a header spells a surface's value.
#[derive(Clone, Copy)]
enum Spelling {
    /// The value as it is.
    Text,
    /// The first language tag of a language list.
    FirstLanguage,
    /// Every language tag of a language list, without their weights.
    Languages,
    /// A structured-field string.
    String,
    /// A structured-field boolean.
    Boolean,
    /// A structured-field list of brands, each `"brand";v="version"`, as
    /// `brand/version`, as the probes write a brand list.
    Brands,
}

/// The header that carries each surface a header carries, and how it
/// spells it.
const HEADERS: [(&str, &str, Spelling); 12] = [
    ("user_agent", "user-agent", Spelling::Text),
    ("language", "accept-language", Spelling::FirstLanguage),
    ("languages", "accept-language", Spelling::Languages),
    ("ua_brands", "sec-ch-ua", Spelling::Brands),
    (
        "ua_full_version_list",
        "sec-ch-ua-full-version-list",
        Spelling::Brands,
    ),
    ("ua_mobile", "sec-ch-ua-mobile", Spelling::Boolean),
    ("ua_wow64", "sec-ch-ua-wow64", Spelling::Boolean),
    ("ua_platform", "sec-ch-ua-platform", Spelling::String),
    (
        "ua_platform_version",
        "sec-ch-ua-platform-version",
        Spelling::String,
    ),
    ("ua_architecture", "sec-ch-ua-arch", Spelling::String),
    ("ua_bitness", "sec-ch-ua-bitness", Spelling::String),
    ("ua_model", "sec-ch-ua-model", Spelling::String),
];

/// The value of `surface` that `request`'s header sent, spelled as the
/// probes report it; `None` when no header carries the surface or the
/// request sent none. A header this cannot read is kept as it was sent,
/// so it differs from every value and is flagged.
fn header_value(surface: &str, request: &ObservedRequest) -> Option<Value> {
    let (_, name, spelling) = HEADERS.iter().find(|(known, _, _)| *known == surface)?;
    let sent = request.header(name)?;
    let read = match spelling {
        Spelling::Text => Some(json!(sent)),
        Spelling::FirstLanguage => languages(sent).into_iter().next().map(Value::from),
        Spelling::Languages => Some(json!(languages(sent))),
        Spelling::String => {
            string(sent.trim()).and_then(|(text, rest)| rest.trim().is_empty().then(|| json!(text)))
        }
        Spelling::Boolean => match sent.trim() {
            "?0" => Some(json!(false)),
            "?1" => Some(json!(true)),
            _ => None,
        },
        Spelling::Brands => brands(sent).map(Value::from),
    };
    Some(read.unwrap_or_else(|| json!({ "header": name, "unread": sent })))
}

/// The main frame's value of `surface` as the headers carry it: Chromium
/// expands a language list into `Accept-Language`, adding each regional
/// tag's base language after it unless the next tag is of the same
/// language or the base is the private `x` or `i`, and never twice
/// (`net::HttpUtil::ExpandLanguageList`), so
/// `["en-US"]` is sent as `en-US,en`. Every other value is sent as it is.
fn as_headers_send(surface: &str, value: &Value) -> Value {
    let spelling = HEADERS
        .iter()
        .find(|(known, _, _)| *known == surface)
        .map(|(_, _, spelling)| *spelling);
    let (Some(Spelling::Languages), Some(tags)) = (spelling, value.as_array()) else {
        return value.clone();
    };
    let Some(tags) = tags.iter().map(Value::as_str).collect::<Option<Vec<_>>>() else {
        return value.clone();
    };
    let base = |tag: &str| tag.split('-').next().unwrap_or_default().to_owned();
    let mut sent: Vec<String> = Vec::new();
    for (index, tag) in tags.iter().enumerate() {
        let private = matches!(base(tag).as_str(), "x" | "i");
        let next_shares_language = tags
            .get(index + 1)
            .is_some_and(|next| base(next) == base(tag));
        let added = if private || next_shares_language {
            vec![(*tag).to_owned()]
        } else {
            vec![(*tag).to_owned(), base(tag)]
        };
        for added in added {
            if !sent.contains(&added) {
                sent.push(added);
            }
        }
    }
    json!(sent)
}

/// The language tags of an `Accept-Language` list, in order, without their
/// weights.
fn languages(list: &str) -> Vec<String> {
    list.split(',')
        .filter_map(|item| {
            let tag = item.split(';').next().unwrap_or_default().trim();
            (!tag.is_empty()).then(|| tag.to_owned())
        })
        .collect()
}

/// The structured-field string `input` starts with, and what follows it.
fn string(input: &str) -> Option<(String, &str)> {
    let mut characters = input.strip_prefix('"')?.char_indices();
    let mut text = String::new();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' => return Some((text, &input[index + 2..])),
            '\\' => text.push(characters.next()?.1),
            _ => text.push(character),
        }
    }
    None
}

/// A brand list's brands as `brand/version`; `None` when it is not one.
fn brands(list: &str) -> Option<Vec<String>> {
    let mut brands = Vec::new();
    let mut rest = list.trim();
    while !rest.is_empty() {
        let (brand, after) = string(rest)?;
        let (version, after) = string(after.trim_start().strip_prefix(";v=")?)?;
        brands.push(format!("{brand}/{version}"));
        rest = after.trim_start();
        if let Some(next) = rest.strip_prefix(',') {
            rest = next.trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(brands)
}

#[cfg(test)]
mod tests;
