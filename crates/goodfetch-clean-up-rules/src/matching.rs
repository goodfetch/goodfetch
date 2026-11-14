//! What the interception does with one request (ADRs 0047 and 0117): the
//! never-block list first, then each Filter list that is on, then
//! `browser.block_resources`. A request a Filter list blocks fails as
//! `BlockedByClient`, or, when its rule names a stand-in the alias table
//! has, is answered with that stand-in.

use crate::list::{LISTS, List, ListSet};
use crate::never_block::NeverBlock;
use crate::rules::FilterLists;

/// The kind of resource a request loads, as the browser names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ResourceKind {
    /// A top-level document.
    Document,
    /// A frame's document.
    Subdocument,
    Stylesheet,
    Script,
    Image,
    Font,
    Media,
    /// An `XMLHttpRequest` or a `fetch()`.
    Xhr,
    WebSocket,
    /// A beacon or a hyperlink's ping.
    Ping,
    /// An `<object>` or `<embed>`.
    Object,
    /// A Content Security Policy report.
    CspReport,
    /// Anything else, such as a manifest or a prefetch.
    Other,
}

impl ResourceKind {
    /// The name `adblock`'s matching takes.
    fn adblock_name(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Subdocument => "subdocument",
            Self::Stylesheet => "stylesheet",
            Self::Script => "script",
            Self::Image => "image",
            Self::Font => "font",
            Self::Media => "media",
            Self::Xhr => "xmlhttprequest",
            Self::WebSocket => "websocket",
            Self::Ping => "ping",
            Self::Object => "object",
            Self::CspReport => "csp_report",
            Self::Other => "other",
        }
    }
}

/// One request as the interception holds it.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    /// Its URL, as the last route sent it on.
    pub url: &'a str,
    /// The URL of the document that sent it: the request's own for a
    /// top-level document.
    pub source_url: &'a str,
    /// What it loads.
    pub kind: ResourceKind,
    /// Its method, such as `GET`.
    pub method: &'a str,
}

/// What a Session's settings choose (ADRs 0047 and 0117): which Filter
/// lists are on, whether a site's own scripts pass them, and which
/// resource kinds `browser.block_resources` blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one boolean setting of `filter_lists.*`, set apart"
)]
pub struct Choices {
    /// `filter_lists.ads`.
    pub ads: bool,
    /// `filter_lists.trackers`.
    pub trackers: bool,
    /// `filter_lists.annoyances`.
    pub annoyances: bool,
    /// `filter_lists.first_party_scripts`: a script of the document's own
    /// site passes every Filter list.
    pub first_party_scripts: bool,
    /// `browser.block_resources`.
    pub block_resources: Vec<ResourceKind>,
}

impl Choices {
    /// Whether any Filter list is on, so that the embedded lists are needed.
    pub fn needs_lists(&self) -> bool {
        self.ads || self.trackers || self.annoyances
    }

    /// The Filter lists it turns on.
    pub(crate) fn lists(&self) -> ListSet {
        LISTS
            .into_iter()
            .zip([self.ads, self.trackers, self.annoyances])
            .filter(|(_, on)| *on)
            .fold(ListSet::default(), |set, (list, _)| set.with(list))
    }

    /// Whether any request may be blocked at all.
    pub fn blocks_anything(&self) -> bool {
        self.needs_lists() || !self.block_resources.is_empty()
    }
}

/// What the interception does with a request.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verdict {
    /// It goes on: nothing blocks it.
    Allowed,
    /// It goes on by the never-block list, whatever the Filter lists and
    /// `browser.block_resources` say.
    NeverBlocked,
    /// A Filter list blocks it: it fails as `BlockedByClient`.
    Blocked(List),
    /// A Filter list answers it with a stand-in.
    StandIn {
        /// The list whose rule named it.
        list: List,
        /// Its media type.
        mime: String,
        /// Its body, in Base64.
        body: String,
    },
    /// `browser.block_resources` blocks its kind: it fails as
    /// `BlockedByClient`.
    ResourceBlocked,
}

impl Verdict {
    /// Whether the request goes on to the network.
    pub fn sends_on(&self) -> bool {
        matches!(self, Self::Allowed | Self::NeverBlocked)
    }
}

/// What the interception does with `request` under `choices`: the
/// never-block list first, then each Filter list that is on (`lists`, which
/// the caller loaded when [`Choices::needs_lists`]), then
/// `browser.block_resources`. A request `adblock` cannot read, such as a
/// `data:` URL, goes on.
pub fn judge(
    request: &Request<'_>,
    never_block: &NeverBlock,
    lists: Option<&FilterLists>,
    choices: &Choices,
) -> Verdict {
    if !choices.blocks_anything() {
        return Verdict::Allowed;
    }
    let Ok(read) = adblock::request::Request::new(
        request.url,
        request.source_url,
        request.kind.adblock_name(),
        request.method,
    ) else {
        return Verdict::Allowed;
    };
    if never_block.names(&read) {
        return Verdict::NeverBlocked;
    }
    let own_script =
        choices.first_party_scripts && request.kind == ResourceKind::Script && !read.is_third_party;
    if let Some(lists) = lists
        && !own_script
    {
        let enabled = [
            (List::Ads, choices.ads, &lists.ads),
            (List::Trackers, choices.trackers, &lists.trackers),
            (List::Annoyances, choices.annoyances, &lists.annoyances),
        ];
        for (list, on, engine) in enabled {
            if !on {
                continue;
            }
            let result = engine.check_network_request(&read);
            if !result.should_block() {
                continue;
            }
            return match result.redirect.as_deref().and_then(data_url) {
                Some((mime, body)) => Verdict::StandIn {
                    list,
                    mime: mime.to_owned(),
                    body: body.to_owned(),
                },
                None => Verdict::Blocked(list),
            };
        }
    }
    if choices.block_resources.contains(&request.kind) {
        return Verdict::ResourceBlocked;
    }
    Verdict::Allowed
}

/// The media type and Base64 body of a `data:<mime>;base64,<body>` URL, as
/// `adblock` gives a stand-in.
fn data_url(url: &str) -> Option<(&str, &str)> {
    let (mime, body) = url.strip_prefix("data:")?.split_once(";base64,")?;
    Some((mime, body))
}
