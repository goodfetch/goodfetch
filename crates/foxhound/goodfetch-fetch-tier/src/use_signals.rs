//! The AI-use signals a site declares, judged against the Run's Purpose
//! (ADRs 0016, 0045, 0086 and 0093). A Purpose of `search` is no
//! declaration to judge them against (ADR 0086).
//!
//! Those a crawl request's response carries ([`UseSignals`]): W3C
//! `TDMRep`'s `tdm-reservation: 1`, as a header or as its document's
//! `<meta name="tdm-reservation">`, which reserves the text and data mining
//! rights of the page and so refuses every Purpose but `search`; IETF
//! aipref's `Content-Usage` header, a dictionary of the vocabulary's
//! categories each `y` or `n`, where a narrower category's preference wins
//! over the broader one it falls under (`train-ai` and `ai-use` under
//! `bulk`); and the RSL license a `Link` header or `<link>` element of type
//! `application/rsl+xml` and relation `license` names, which
//! [`rsl_refusing`] judges once it is fetched.
//!
//! Those a host declares before any request is sent: the `Content-Signal`
//! lines of the robots.txt groups that apply ([`content_signals_refusing`],
//! Cloudflare's Content Signals Policy: `ai-input` and `ai-train` each `yes`
//! or `no`), and `TDMRep`'s [`TDMREP_FILE`], whose first rule whose
//! location matches the request's path decides ([`tdmrep_file_refusing`]).
//! [`SignalFiles`] keeps a Run's fetched files, one fetch per URL.

use std::collections::HashMap;
use std::sync::Arc;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use tokio::sync::{Mutex, OnceCell};
use url::Url;

/// The path of `TDMRep`'s well-known file on an origin.
pub const TDMREP_FILE: &str = "/.well-known/tdmrep.json";

/// The media type of an RSL license.
const RSL_TYPE: &str = "application/rsl+xml";

/// What a run declares its results are for (`site_preferences.purpose`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// The default, which no signal is judged against.
    Search,
    /// Input to an AI model.
    AiInput,
    /// Training an AI model.
    AiTrain,
    /// Plain collection: text and data mining.
    Collect,
}

impl Purpose {
    /// The Purpose `site_preferences.purpose` names.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "search" => Some(Self::Search),
            "ai-input" => Some(Self::AiInput),
            "ai-train" => Some(Self::AiTrain),
            "collect" => Some(Self::Collect),
            _ => None,
        }
    }

    /// The aipref categories that govern the Purpose, the narrowest first.
    fn categories(self) -> &'static [&'static str] {
        match self {
            Self::Search => &[],
            Self::AiInput => &["ai-use", "bulk"],
            Self::AiTrain => &["train-ai", "bulk"],
            Self::Collect => &["bulk"],
        }
    }

    /// The Content Signal that governs the Purpose; `collect` has none.
    fn content_signal(self) -> Option<&'static str> {
        match self {
            Self::AiInput => Some("ai-input"),
            Self::AiTrain => Some("ai-train"),
            Self::Search | Self::Collect => None,
        }
    }

    /// The RSL usage tokens that cover the Purpose (RSL 1.0's `usage`).
    fn rsl_usages(self) -> &'static [&'static str] {
        match self {
            Self::Search => &[],
            Self::AiInput => &["ai-input", "ai-all", "all"],
            Self::AiTrain => &["ai-train", "ai-all", "all"],
            Self::Collect => &["all"],
        }
    }
}

/// The AI-use signals of one response: its headers and, read with
/// [`UseSignals::with_markup`], its document's markup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UseSignals {
    /// What reserves the text and data mining rights, if anything does:
    /// `TDMRep`'s header or `<meta>`.
    reserved: Option<&'static str>,
    /// aipref's `Content-Usage` preferences, by category: `true` for `y`.
    usage: Vec<(String, bool)>,
    /// The RSL license its `Link` header names, as written.
    license_header: Option<String>,
    /// The RSL license its `Link` header, else its document's `<link>`,
    /// names, resolved by [`UseSignals::with_markup`].
    license: Option<Url>,
}

impl UseSignals {
    /// The signals `headers` carry; a value it cannot read says nothing.
    pub fn of(headers: &[(String, String)]) -> Self {
        let mut signals = Self::default();
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("tdm-reservation") {
                if value.trim() == "1" {
                    signals.reserved = Some("TDMRep's `tdm-reservation: 1` header");
                }
            } else if name.eq_ignore_ascii_case("content-usage") {
                signals.usage.extend(value.split(',').filter_map(|member| {
                    let (category, preference) = member.split_once('=')?;
                    let allowed = match preference.trim() {
                        "y" => true,
                        "n" => false,
                        _ => return None,
                    };
                    Some((category.trim().to_ascii_lowercase(), allowed))
                }));
            } else if name.eq_ignore_ascii_case("link") && signals.license_header.is_none() {
                signals.license_header = rsl_link(value).map(str::to_owned);
            }
        }
        signals
    }

    /// These signals with those `html`, the document fetched at `url`,
    /// carries: `TDMRep`'s `<meta name="tdm-reservation" content="1">` and
    /// an RSL `<link>`, resolved against the document's base URL; the
    /// `Link` header's license is resolved against `url`. A response that
    /// holds no document is read with an empty `html`.
    #[must_use]
    pub fn with_markup(mut self, html: &str, url: &Url) -> Self {
        self.license = self
            .license_header
            .as_deref()
            .and_then(|target| url.join(target.trim()).ok());
        // Most documents name neither; they are not parsed again.
        let names = |needle: &str| {
            html.as_bytes()
                .windows(needle.len())
                .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
        };
        if !names("tdm-reservation") && !names(RSL_TYPE) {
            return self;
        }
        let document = dom_query::Document::from(html);
        let reserves = document
            .select("meta[name][content]")
            .nodes()
            .iter()
            .any(|meta| {
                meta.attr("name")
                    .is_some_and(|name| name.trim().eq_ignore_ascii_case("tdm-reservation"))
                    && meta
                        .attr("content")
                        .is_some_and(|content| content.trim() == "1")
            });
        if reserves && self.reserved.is_none() {
            self.reserved = Some("TDMRep's `<meta name=\"tdm-reservation\">`");
        }
        let base = document
            .select("base[href]")
            .attr("href")
            .and_then(|href| url.join(href.trim()).ok())
            .unwrap_or_else(|| url.clone());
        if self.license.is_some() {
            return self;
        }
        self.license = document
            .select("link[rel][type][href]")
            .nodes()
            .iter()
            .find(|link| {
                link.attr("rel").is_some_and(|rel| has_license(&rel))
                    && link
                        .attr("type")
                        .is_some_and(|kind| kind.trim().eq_ignore_ascii_case(RSL_TYPE))
            })
            .and_then(|link| link.attr("href"))
            .and_then(|href| base.join(href.trim()).ok());
        self
    }

    /// The signal that refuses `purpose`, named, if any.
    pub fn refusing(&self, purpose: Purpose) -> Option<&'static str> {
        if purpose == Purpose::Search {
            return None;
        }
        if let Some(reserved) = self.reserved {
            return Some(reserved);
        }
        let stated = purpose.categories().iter().find_map(|category| {
            self.usage
                .iter()
                .rev()
                .find(|(stated, _)| stated == category)
                .map(|(_, allowed)| *allowed)
        });
        (stated == Some(false)).then_some("aipref's `Content-Usage` header")
    }

    /// The RSL license the response names: its `Link` header's, else its
    /// document's `<link>` element's, once read with
    /// [`UseSignals::with_markup`]; before, its `Link` header's when that
    /// is an absolute URL.
    pub fn license(&self) -> Option<Url> {
        self.license.clone().or_else(|| {
            self.license_header
                .as_deref()
                .and_then(|target| Url::parse(target.trim()).ok())
        })
    }
}

/// Whether the `rel` value `rel`, a list of link types, holds `license`.
fn has_license(rel: &str) -> bool {
    rel.split_ascii_whitespace()
        .any(|kind| kind.eq_ignore_ascii_case("license"))
}

/// The target of the first link of a `Link` field value (RFC 8288) whose
/// relation is `license` and whose type is RSL's.
fn rsl_link(value: &str) -> Option<&str> {
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>')?;
        let target = &rest[start + 1..start + end];
        let after = &rest[start + end + 1..];
        let parameters = after.split_once('<').map_or(after, |(head, _)| head);
        let parameter = |wanted: &str| {
            parameters
                .split([';', ','])
                .filter_map(|parameter| parameter.split_once('='))
                .find(|(name, _)| name.trim().eq_ignore_ascii_case(wanted))
                .map(|(_, value)| value.trim().trim_matches('"'))
        };
        if parameter("rel").is_some_and(has_license)
            && parameter("type").is_some_and(|kind| kind.eq_ignore_ascii_case(RSL_TYPE))
        {
            return Some(target);
        }
        rest = after;
    }
    None
}

/// The Content Signal, of `signals` (the `Content-Signal` lines of the
/// robots.txt groups that apply, in file order), that refuses `purpose`,
/// named, if any: the last line naming its signal decides.
pub fn content_signals_refusing(
    signals: &[(String, bool)],
    purpose: Purpose,
) -> Option<&'static str> {
    let wanted = purpose.content_signal()?;
    let allowed = signals
        .iter()
        .rev()
        .find(|(name, _)| name == wanted)
        .map(|(_, allowed)| *allowed);
    (allowed == Some(false)).then_some("robots.txt's `Content-Signal` line")
}

/// One rule of a `TDMRep` file.
#[derive(serde::Deserialize)]
struct TdmRepRule {
    location: String,
    #[serde(rename = "tdm-reservation")]
    reservation: serde_json::Value,
}

/// Whether `TDMRep`'s well-known `file` reserves `url`'s rights against
/// `purpose`, named: its first rule whose location matches the URL's path
/// and query decides, as robots.txt targets them, a value other than `1`
/// or `0` unset; a file that does not
/// read reserves nothing.
pub fn tdmrep_file_refusing(file: &[u8], url: &Url, purpose: Purpose) -> Option<&'static str> {
    if purpose == Purpose::Search {
        return None;
    }
    let rules: Vec<TdmRepRule> = serde_json::from_slice(file).ok()?;
    let target = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    };
    let rule = rules
        .iter()
        .find(|rule| location_matches(rule.location.as_bytes(), target.as_bytes()))?;
    (rule.reservation.as_u64() == Some(1)).then_some("TDMRep's `/.well-known/tdmrep.json`")
}

/// Whether the `TDMRep` location `pattern` matches `path`, as a robots.txt
/// rule matches: from its start, `*` any characters, a final `$` the
/// path's end, case-sensitive.
fn location_matches(pattern: &[u8], path: &[u8]) -> bool {
    match pattern.split_first() {
        None => true,
        Some((b'$', [])) => path.is_empty(),
        Some((b'*', rest)) => (0..=path.len()).any(|at| location_matches(rest, &path[at..])),
        Some((first, rest)) => path
            .split_first()
            .is_some_and(|(byte, tail)| byte == first && location_matches(rest, tail)),
    }
}

/// Whether the RSL `file` refuses `purpose`, named: it does when it holds
/// a license and none permits the Purpose's usage. A license refuses a
/// usage it prohibits, or one its `<permits type="usage">` leaves out; a
/// prohibition wins. A file that does not read refuses nothing.
pub fn rsl_refusing(file: &[u8], purpose: Purpose) -> Option<&'static str> {
    if purpose == Purpose::Search {
        return None;
    }
    let licenses = rsl_licenses(file)?;
    let usages = purpose.rsl_usages();
    let covers = |tokens: &[String]| tokens.iter().any(|token| usages.contains(&token.as_str()));
    let refuses = |license: &RslLicense| {
        license.prohibits.as_deref().is_some_and(covers)
            || license
                .permits
                .as_deref()
                .is_some_and(|permits| !covers(permits))
    };
    (!licenses.is_empty() && licenses.iter().all(refuses)).then_some("an RSL license")
}

/// What one RSL `<license>` says of usage: its permitted and prohibited
/// tokens, when it states them.
#[derive(Default)]
struct RslLicense {
    permits: Option<Vec<String>>,
    prohibits: Option<Vec<String>>,
}

/// The licenses of the RSL `file`; `None` when it is no XML.
fn rsl_licenses(file: &[u8]) -> Option<Vec<RslLicense>> {
    let mut reader = Reader::from_reader(file);
    let mut licenses = Vec::new();
    // The usage element open in the license being read, if one is.
    let mut open: Option<(bool, String)> = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(element) => match element.local_name().as_ref() {
                b"license" => licenses.push(RslLicense::default()),
                name @ (b"permits" | b"prohibits") if is_usage(&element) => {
                    open = Some((name == b"permits", String::new()));
                }
                _ => {}
            },
            Event::Text(text) => {
                if let Some((_, tokens)) = open.as_mut() {
                    tokens.push_str(&text.decode().ok()?);
                }
            }
            Event::End(element) => {
                if matches!(element.local_name().as_ref(), b"permits" | b"prohibits")
                    && let Some((permits, tokens)) = open.take()
                    && let Some(license) = licenses.last_mut()
                {
                    let tokens = tokens.split_ascii_whitespace().map(str::to_ascii_lowercase);
                    let stated = if permits {
                        &mut license.permits
                    } else {
                        &mut license.prohibits
                    };
                    stated.get_or_insert_with(Vec::new).extend(tokens);
                }
            }
            Event::Eof => return Some(licenses),
            _ => {}
        }
    }
}

/// Whether the `<permits>` or `<prohibits>` element `element` states
/// usage.
fn is_usage(element: &BytesStart<'_>) -> bool {
    element
        .try_get_attribute("type")
        .ok()
        .flatten()
        .is_some_and(|attribute| attribute.value.as_ref() == b"usage")
}

/// A Run's fetched signal files, `TDMRep`'s well-known file and RSL
/// licenses, each fetched once by URL however many requests read it; a
/// fetch that answered no `2xx` keeps no body. Cheap to clone; every clone
/// shares the files.
#[derive(Clone, Default)]
pub struct SignalFiles {
    files: Arc<Mutex<HashMap<String, Arc<SignalFile>>>>,
}

/// One signal file of [`SignalFiles`]: its body once fetched.
type SignalFile = OnceCell<Option<Arc<[u8]>>>;

impl SignalFiles {
    /// The body at `url`: kept, or fetched with `fetch` by the first
    /// reader, every concurrent reader waiting for that fetch.
    pub async fn read<F>(&self, url: &Url, fetch: F) -> Option<Arc<[u8]>>
    where
        F: Future<Output = Option<Vec<u8>>>,
    {
        let cell = Arc::clone(
            self.files
                .lock()
                .await
                .entry(url.as_str().to_owned())
                .or_default(),
        );
        cell.get_or_init(|| async { fetch.await.map(Arc::from) })
            .await
            .clone()
    }
}

#[cfg(test)]
mod tests;
