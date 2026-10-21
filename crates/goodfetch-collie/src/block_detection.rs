//! Block detection (ADR 0035): reads a response and its document for the
//! evidence that the site refused the page, and names the Block cause.
//!
//! It is rule data, not a translation point (ADR 0127): each rule, written
//! once with its source and license, fires on evidence of a refusal, never on
//! a bot vendor's mere presence, and a response no rule classifies is no
//! block. It never produces `decoy`, Foxhound's alone (ADR 0095). It
//! classifies a response as the site's: a proxy's own refusal is told where
//! an engine module sees the proxy's answer (ADR 0127).

mod rules;

use goodfetch_protocol::block::BlockCause;

use rules::{RULES, Rule};

/// What `classify` reads of a response.
pub(crate) struct Response {
    /// Its HTTP status.
    pub(crate) status: u16,
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
    /// The markers it found in the document, one per group of its markers.
    pub(crate) markers: Vec<&'static str>,
}

/// The Block cause of a response with `document`, if a rule decides one.
pub(crate) fn classify(response: &Response, document: &Document<'_>) -> Option<Classification> {
    let markup = document.markup.to_lowercase();
    RULES
        .iter()
        .find_map(|rule| fire(rule, response.status, &markup))
}

/// `rule`'s classification of a response with `status` and the lowercased
/// `markup`, if every group of its markers has one in the markup.
fn fire(rule: &Rule, status: u16, markup: &str) -> Option<Classification> {
    if !rule.statuses.contains(&status) {
        return None;
    }
    let markers = rule
        .markers
        .iter()
        .map(|group| group.iter().copied().find(|marker| markup.contains(marker)))
        .collect::<Option<Vec<_>>>()?;
    Some(Classification {
        cause: rule.cause,
        evidence: Evidence {
            rule: rule.name,
            source: rule.source,
            license: rule.license,
            status,
            markers,
        },
    })
}

#[cfg(test)]
mod tests;
