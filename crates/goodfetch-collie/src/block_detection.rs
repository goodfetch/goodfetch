//! Block detection (ADR 0035): reads a response and its document for the
//! evidence that the site refused the page, and names the Block cause.
//!
//! It is rule data, not a translation point (ADR 0127): each rule, written
//! once with its source and license, fires on evidence of a refusal (a
//! challenge or a refusal page, a vendor's header that marks a challenge, a
//! rate-limit status), never on a bot vendor's mere presence, and a response
//! no rule classifies is no block. The outside sources the rules are seeded
//! from are declared in `block_detection/third-party.toml` for the notice
//! inventory. It never produces `decoy`, Foxhound's alone (ADR 0095). It
//! classifies a response as the site's: a proxy's own refusal is told where
//! an engine module sees the proxy's answer (ADR 0127). [`Detector`] gives
//! it to the page crate, whose pages ask it for a document's Block cause.

mod rules;

use goodfetch_page::blocks::{Block, BlockClassifier};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::page::PageResponse;

use rules::{HeaderMarker, RULES, Rule};

/// What `classify` reads of a response.
pub(crate) struct Response<'a> {
    /// Its HTTP status.
    pub(crate) status: u16,
    /// Its headers, each `(name, value)`, as received.
    pub(crate) headers: &'a [(&'a str, &'a str)],
}

/// What `classify` reads of the document a response carries.
pub(crate) struct Document<'a> {
    /// Its markup: a response's body, or a page's serialized document.
    pub(crate) markup: &'a str,
}

/// A Block cause with the evidence that gave it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Classification {
    pub(crate) cause: BlockCause,
    pub(crate) evidence: Evidence,
}

/// Why a rule fired: the rule, where it comes from, and what it matched.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Evidence {
    /// The rule's name.
    pub(crate) rule: &'static str,
    /// Where the rule comes from.
    pub(crate) source: &'static str,
    /// The license it is used under.
    pub(crate) license: &'static str,
    /// The status it matched.
    pub(crate) status: u16,
    /// The headers it found in the response, one per group of its header
    /// markers.
    pub(crate) headers: Vec<HeaderMarker>,
    /// The markers it found in the document, one per group of its markers.
    pub(crate) markers: Vec<&'static str>,
}

/// Collie's Block classification, as the page crate asks for it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Detector;

impl BlockClassifier for Detector {
    fn classify(&self, response: &PageResponse, markup: &str) -> Option<Block> {
        let headers: Vec<(&str, &str)> = response
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let response = Response {
            status: response.status,
            headers: &headers,
        };
        classify(&response, &Document { markup }).map(|classification| Block {
            cause: classification.cause,
            rule: classification.evidence.rule,
        })
    }

    fn markers(&self, status: u16) -> Vec<&'static str> {
        markers(status)
    }
}

/// The markers, lowercase, that the rules reading a response with `status`
/// look for in its document, each once: none when no rule reads the
/// document of such a response. Since a rule only asks whether the markup
/// holds each, the markers a document holds, each on a line of its own,
/// classify as its whole markup does.
pub(crate) fn markers(status: u16) -> Vec<&'static str> {
    let mut markers: Vec<&'static str> = RULES
        .iter()
        .filter(|rule| rule.statuses.contains(&status))
        .flat_map(|rule| {
            rule.markers
                .iter()
                .flat_map(|group| group.iter())
                .chain(rule.unless)
        })
        .copied()
        .collect();
    markers.sort_unstable();
    markers.dedup();
    markers
}

/// The Block cause of a response with `document`, if a rule decides one.
pub(crate) fn classify(response: &Response<'_>, document: &Document<'_>) -> Option<Classification> {
    let headers: Vec<(String, String)> = response
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_lowercase()))
        .collect();
    let markup = document.markup.to_lowercase();
    RULES
        .iter()
        .find_map(|rule| fire(rule, response.status, &headers, &markup))
}

/// `rule`'s classification of a response with `status`, the lowercased
/// `headers` and the lowercased `markup`, if every group of its header
/// markers has one in the headers, every group of its markers has one in
/// the markup, and the markup holds none of what it is `unless`.
fn fire(
    rule: &Rule,
    status: u16,
    headers: &[(String, String)],
    markup: &str,
) -> Option<Classification> {
    if !rule.statuses.contains(&status) || rule.unless.iter().any(|text| markup.contains(text)) {
        return None;
    }
    let matched_headers = rule
        .headers
        .iter()
        .map(|group| {
            group.iter().copied().find(|marker| {
                headers
                    .iter()
                    .any(|(name, value)| name == marker.name && value.contains(marker.value))
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let markers = rule
        .markers
        .iter()
        .map(|group| group.iter().copied().find(|marker| markup.contains(marker)))
        .collect::<Option<Vec<_>>>()?;
    Some(Classification {
        cause: rule.cause.clone(),
        evidence: Evidence {
            rule: rule.name,
            source: rule.source.name(),
            license: rule.source.license(),
            status,
            headers: matched_headers,
            markers,
        },
    })
}

#[cfg(test)]
mod tests;
