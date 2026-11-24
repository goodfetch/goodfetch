//! The sample comparison's weighing (spec #579's `comparison`; ADRs 0020,
//! 0103, 0118 and 0119): whether a sample's cheaper run and its browser
//! run, the reference, staged the same results.
//!
//! Two runs match when the Records they staged are equal as multisets,
//! each compared as canonical JSON (RFC 8785) after its strings are put in
//! Unicode NFC and their whitespace normalized (each run of whitespace one
//! space, none at either end), the fields ADR 0047's removal took out of
//! the cheaper run's content left out of both; and when the requests they
//! staged are equal as sets of Frontier identities (method, URL key and
//! body hash, ADR 0120). A browser run that ended on another URL than the
//! cheaper run's document, or read its document in another
//! `document.characterSet`, never matches. Any difference keeps the
//! browser.

use std::collections::BTreeSet;

use goodfetch_protocol::foxhound::crawl::PushedRecord;
use icu_normalizer::ComposingNormalizerBorrowed;
use serde_json::Value;

/// What one run of a sample staged, as the weighing reads it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StagedRun<'a> {
    /// The Records it staged: those it pushed, then those its answer held.
    pub(crate) records: &'a [PushedRecord],
    /// The Frontier identities of the requests it staged.
    pub(crate) enqueues: &'a BTreeSet<String>,
    /// The URL of the document it ended on, when it names one.
    pub(crate) ended_at: Option<&'a str>,
    /// The encoding it read its document in, when it names one.
    pub(crate) encoding: Option<&'a str>,
}

/// Whether the sample's `cheaper` run matched its `browser` run, the
/// fields in `removed`, those ADR 0047's removal took out of the cheaper
/// run's content, left out of every Record.
pub(crate) fn weigh(
    cheaper: &StagedRun<'_>,
    browser: &StagedRun<'_>,
    removed: &BTreeSet<String>,
) -> bool {
    same_url(cheaper.ended_at, browser.ended_at)
        && same_encoding(cheaper.encoding, browser.encoding)
        && cheaper.enqueues == browser.enqueues
        && canonical(cheaper.records, removed) == canonical(browser.records, removed)
}

/// Whether two runs read their documents alike (ADR 0119): names compare
/// without regard to ASCII case, and a run that names none leaves the rest
/// to decide.
fn same_encoding(cheaper: Option<&str>, browser: Option<&str>) -> bool {
    match (cheaper, browser) {
        (Some(cheaper), Some(browser)) => cheaper.eq_ignore_ascii_case(browser),
        _ => true,
    }
}

/// Whether two runs ended on one URL, compared by its Frontier key (ADR
/// 0118): a run that names none leaves the rest to decide.
fn same_url(cheaper: Option<&str>, browser: Option<&str>) -> bool {
    let key = |url: &str| {
        goodfetch_frontier::Url::parse(url)
            .map_or_else(|_| url.to_owned(), |url| goodfetch_frontier::url_key(&url))
    };
    match (cheaper, browser) {
        (Some(cheaper), Some(browser)) => key(cheaper) == key(browser),
        _ => true,
    }
}

/// `records` as a multiset of canonical texts, sorted.
fn canonical(records: &[PushedRecord], removed: &BTreeSet<String>) -> Vec<String> {
    let mut texts: Vec<String> = records
        .iter()
        .map(|record| {
            let mut data = record.data.value();
            if let Value::Object(fields) = &mut data {
                fields.retain(|name, _| !removed.contains(name));
            }
            let normalized = serde_json::json!({
                "dataset": record.dataset,
                "data": normalize(data),
            });
            serde_json_canonicalizer::to_string(&normalized)
                .expect("invariant: a JSON value canonicalizes")
        })
        .collect();
    texts.sort_unstable();
    texts
}

/// `value` with every string, keys included, in NFC with its whitespace
/// normalized.
fn normalize(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(normalized(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(name, value)| (normalized(&name), normalize(value)))
                .collect(),
        ),
        other => other,
    }
}

/// `text` in NFC, each run of whitespace one space, none at either end.
fn normalized(text: &str) -> String {
    let composed = ComposingNormalizerBorrowed::new_nfc().normalize(text);
    composed.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;
