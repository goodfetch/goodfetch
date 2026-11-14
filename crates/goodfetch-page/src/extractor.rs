//! The one resolver of a page's main-content step (spec #2164): the
//! built-in `content.extractor` names, with the attempts
//! `extraction.readability` gives dom-smoothie. A page's formats, a
//! browser page's and a served document's, the exact removal share's
//! second run and Foxhound's Page signature all resolve their step here,
//! so that no two of them read a page with different steps.

use goodfetch_engine::settings::{Resolved, Values};
use goodfetch_extractor::{Attempts, BuiltIn, Chosen};
use goodfetch_protocol::page::{CONTENT_EXTRACTOR, EXTRACTION_READABILITY};
use serde_json::Value;

/// The main-content step a page reads with: the `content.extractor` of its
/// own settings `page`, else the one its crawl request's Label chooses,
/// `label`, else its Session's `session`, else its Run's resolved settings
/// `run`, dom-smoothie without any; with the `extraction.readability` of
/// `run`, a per-Run setting, `retry` without one. A value no built-in has
/// never reaches here: it was refused where it was set.
#[must_use]
pub fn chosen(
    run: Option<&Resolved>,
    (session, label): (&Values, Option<BuiltIn>),
    page: &Values,
) -> Chosen {
    let named = |values: &Values| {
        values
            .get(CONTENT_EXTRACTOR.name)
            .and_then(Value::as_str)
            .and_then(BuiltIn::named)
    };
    let built_in = named(page)
        .or(label)
        .or_else(|| named(session))
        .or_else(|| run.and_then(|run| BuiltIn::named(run.get(&CONTENT_EXTRACTOR))))
        .unwrap_or_default();
    let attempts = run
        .and_then(|run| Attempts::named(run.get(&EXTRACTION_READABILITY)))
        .unwrap_or_default();
    Chosen { built_in, attempts }
}

/// The main-content step of a document a fetch tier served, which has no
/// Session's or page's settings: its Label's `label`, else its Run's `run`.
#[must_use]
pub fn served(run: Option<&Resolved>, label: Option<BuiltIn>) -> Chosen {
    chosen(run, (&Values::default(), label), &Values::default())
}

#[cfg(test)]
mod tests;
