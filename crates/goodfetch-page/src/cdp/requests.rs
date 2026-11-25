//! The values a paused request's answer is made of, as the page's logic
//! builds them and the browser's protocol takes them: the ID a request is
//! paused at, the headers it carries and is sent on with, and the commands
//! that send it on or answer it. Free of the rest of the crate, so the
//! per-request benchmark builds them as the pipeline does (ADR 0071).

use goodfetch_cdp::generated::{fetch, network};
use goodfetch_clean_up_rules::matching::ResourceKind;
use serde_json::{Map, Value};

/// The ID the browser paused a request or its response at, which every
/// answer to it names.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PausedId(String);

impl PausedId {
    /// The ID the browser named `id`.
    pub(crate) fn new(id: String) -> Self {
        Self(id)
    }

    /// The ID as the browser names it.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// One header a request is sent on with, or a response answered with, as
/// given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub(crate) name: String,
    pub(crate) value: String,
}

impl From<Header> for fetch::HeaderEntry {
    fn from(header: Header) -> Self {
        Self {
            name: header.name,
            value: header.value,
        }
    }
}

/// The headers a paused request carries, borrowed from the browser's
/// event, each by the name it was given.
#[derive(Clone, Copy)]
pub(crate) struct RequestHeaders<'a>(&'a Map<String, Value>);

impl<'a> RequestHeaders<'a> {
    /// The headers the browser reports as `headers`.
    pub(crate) fn of(headers: &'a Map<String, Value>) -> Self {
        Self(headers)
    }

    /// The value of the header `name`, whatever its case: the first given.
    pub(crate) fn get(self, name: &str) -> Option<&'a str> {
        self.0
            .iter()
            .find(|(given, _)| given.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| value.as_str())
    }

    /// Each header whose value is a string, by its name as given.
    pub(crate) fn iter(self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.0
            .iter()
            .filter_map(|(name, value)| Some((name.as_str(), value.as_str()?)))
    }

    /// The browser's own view of them.
    pub(super) fn map(self) -> &'a Map<String, Value> {
        self.0
    }
}

/// The command that sends a paused request on to the network: each part
/// given replaces the request's own.
#[derive(Debug)]
pub(crate) struct ContinueRequest {
    pub(crate) id: PausedId,
    pub(crate) url: Option<String>,
    pub(crate) method: Option<String>,
    /// In Base64.
    pub(crate) post_data: Option<String>,
    pub(crate) headers: Option<Vec<Header>>,
    /// Whether its response pauses too.
    pub(crate) intercept_response: bool,
}

impl ContinueRequest {
    /// The command as the browser takes it.
    pub(crate) fn into_params(self) -> fetch::ContinueRequestParams {
        fetch::ContinueRequestParams {
            request_id: self.id.0,
            url: self.url,
            method: self.method,
            post_data: self.post_data,
            headers: self
                .headers
                .map(|headers| headers.into_iter().map(fetch::HeaderEntry::from).collect()),
            intercept_response: self.intercept_response.then_some(true),
        }
    }
}

/// The command that answers a paused request with a response of the
/// engine's.
#[derive(Debug)]
pub(crate) struct Fulfill {
    pub(crate) id: PausedId,
    pub(crate) status: u16,
    pub(crate) headers: Vec<Header>,
    /// In Base64.
    pub(crate) body: String,
}

impl Fulfill {
    /// The command as the browser takes it.
    pub(crate) fn into_params(self) -> fetch::FulfillRequestParams {
        fetch::FulfillRequestParams {
            request_id: self.id.0,
            response_code: i64::from(self.status),
            response_headers: Some(
                self.headers
                    .into_iter()
                    .map(fetch::HeaderEntry::from)
                    .collect(),
            ),
            binary_response_headers: None,
            body: Some(self.body),
            response_phrase: None,
        }
    }
}

/// What a request of `resource_type` loads, as the Filter lists match it;
/// a document is a frame's unless `main_frame`.
pub(crate) fn kind_of(resource_type: network::ResourceType, main_frame: bool) -> ResourceKind {
    use network::ResourceType as Browser;
    match resource_type {
        Browser::Document if main_frame => ResourceKind::Document,
        Browser::Document => ResourceKind::Subdocument,
        Browser::Stylesheet => ResourceKind::Stylesheet,
        Browser::Image => ResourceKind::Image,
        Browser::Media => ResourceKind::Media,
        Browser::Font => ResourceKind::Font,
        Browser::Script => ResourceKind::Script,
        Browser::Xhr | Browser::Fetch | Browser::EventSource => ResourceKind::Xhr,
        Browser::WebSocket => ResourceKind::WebSocket,
        Browser::Ping => ResourceKind::Ping,
        Browser::CspViolationReport => ResourceKind::CspReport,
        _ => ResourceKind::Other,
    }
}
