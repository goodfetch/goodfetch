//! A page's formats read with the Clean-up rules (ADR 0047): the extractor
//! reads its serialized document, shadow roots included, and applies,
//! through its `Context`, the cosmetic matcher this module makes of the
//! page's Session's cosmetic rules and of the prompts the Consent prompt
//! runner hid (ADR 0048), before the main-content step and the Markdown
//! converter only. Each call is a Provenance entry of the Run trace: the
//! version of the rules applied, what they removed and, when the call read
//! the main content, the share of its text the removal took, flagged above
//! a fifth (G6), and how that share was measured: `estimate` by default, an
//! upper bound read from what the removal took, or `exact` under
//! `extraction.removal_share: exact`, one more main-content step on the page
//! as served (ADR 0138).

use std::cell::RefCell;

use goodfetch_clean_up_rules::removal::{self, Removed, Rules, Share};
use goodfetch_counted_work::{CountedWork, ParsedHtml};
use goodfetch_engine::settings::{Resolved, Values};
use goodfetch_extractor::{
    BuiltIn, CleanUp, Context, Demand, Document, Extractor, Measured, Plan, Reader, Serialization,
};
use goodfetch_protocol::page::{
    CONTENT_EXTRACTOR, EXTRACTION_REMOVAL_SHARE, FormatsRequest, PageFormats, PageId, SessionId,
};
use serde_json::{Value, json};

use crate::driver::SerializedDocument;
use crate::interception::Cosmetic;

/// What one `page.formats` call's rules did, as Provenance records it.
pub(super) struct FormatsRead {
    url: String,
    /// The digest of the Clean-up rules' version applied; none when no
    /// Filter list of the page's Session is on.
    version: Option<String>,
    removed: Vec<Removed>,
    removed_text: usize,
    /// What the removal took of the main content's text; none when it took
    /// nothing.
    share: Option<Share>,
    /// How the share was measured.
    measure: ShareMeasure,
}

/// How a call measures ADR 0047's share, as `extraction.removal_share`
/// chooses (ADR 0138).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ShareMeasure {
    /// The text removed over that text plus the main content's text left:
    /// an upper bound of the exact share, so it never misses a removal the
    /// exact one flags, read with no second main-content step.
    #[default]
    Estimate,
    /// The main content's text of the page as served against the cleaned
    /// page's: one more main-content step.
    Exact,
}

impl ShareMeasure {
    /// The measure the Run's resolved settings `run` choose; the estimate
    /// without any.
    pub(super) fn of(run: Option<&Resolved>) -> Self {
        match run
            .map(|run| run.value(EXTRACTION_REMOVAL_SHARE.name))
            .and_then(Value::as_str)
        {
            Some("exact") => Self::Exact,
            _ => Self::Estimate,
        }
    }

    /// Its name, as the setting and Provenance write it.
    fn name(self) -> &'static str {
        match self {
            Self::Estimate => "estimate",
            Self::Exact => "exact",
        }
    }
}

impl FormatsRead {
    /// Its Provenance entry, for the page `page_id` of the Session
    /// `session_id`, when it has one.
    pub(super) fn entry(&self, page_id: &PageId, session_id: Option<&SessionId>) -> Value {
        let share = self.share.unwrap_or(Share::of(0, 0));
        json!({
            "action": "formats",
            "page_id": page_id,
            "session_id": session_id,
            "url": self.url,
            "clean_up_rules": self.version,
            "removed": self.removed.iter().map(|element| json!({
                "rule": element.rule,
                // A prompt the Consent prompt runner hid names no list.
                "list": element.list.map_or("consent", |list| list.name()),
                "in_shadow_root": element.in_shadow_root,
                "text": element.text,
                "links": element.links,
            })).collect::<Vec<_>>(),
            "removed_text": self.removed_text,
            "main_content": self.share.map(|share| json!({ "before": share.before, "after": share.after })),
            "share": share.taken(),
            "share_measure": self.measure.name(),
            "flagged": share.flagged(),
        })
    }
}

/// The cosmetic matcher of one call: the rules of `cosmetic`, then the
/// runner's of the selectors `hidden`, both removing in place on the page's
/// one tree, their selectors from the engine's cache on `work`; it keeps
/// what it removed.
struct Matcher<'a> {
    cosmetic: Option<&'a Cosmetic>,
    hidden: &'a [String],
    work: &'a CountedWork,
    removal: RefCell<Option<removal::Removal>>,
}

impl CleanUp for Matcher<'_> {
    fn remove(&self, document: &ParsedHtml, url: &str) -> bool {
        let mut removal = self
            .cosmetic
            .map(|cosmetic| {
                removal::remove(
                    document,
                    url,
                    Rules {
                        lists: &cosmetic.lists,
                        version: &cosmetic.version,
                        choices: &cosmetic.choices,
                    },
                    self.work,
                )
            })
            .unwrap_or_default();
        removal::remove_selected(document, &mut removal, self.hidden, self.work);
        let took = !removal.removed.is_empty();
        *self.removal.borrow_mut() = Some(removal);
        took
    }
}

/// The main-content step a page's formats read with: the
/// `content.extractor` of its own settings `page`, else the one its crawl
/// request's Label chooses, `label`, else its Session's `session`, else
/// its Run's resolved settings `run`; dom-smoothie without any. A value no
/// built-in has never reaches here: it was refused where it was set.
pub(super) fn extractor(
    run: Option<&Resolved>,
    (session, label): (&Values, Option<BuiltIn>),
    page: &Values,
) -> BuiltIn {
    let named = |values: &Values| {
        values
            .get(CONTENT_EXTRACTOR.name)
            .and_then(Value::as_str)
            .and_then(BuiltIn::named)
    };
    named(page)
        .or(label)
        .or_else(|| named(session))
        .or_else(|| {
            run.map(|run| run.value(CONTENT_EXTRACTOR.name))
                .and_then(Value::as_str)
                .and_then(BuiltIn::named)
        })
        .unwrap_or_default()
}

/// The formats `request` names, read from `document`, serialized as
/// `serialization` says, its main content found by `extractor`, the
/// cosmetic rules `cosmetic` holds and the runner's rules of the selectors
/// `hidden` applied before its main content, and what they did, its share
/// measured as `measure` says; none when the request reads nothing of the
/// cleaned page, so that no rule ran and Provenance has no line of it
/// (ADR 0138).
pub(super) fn read(
    document: &SerializedDocument,
    serialization: Serialization,
    request: FormatsRequest,
    (extractor, measure): (&dyn Extractor, ShareMeasure),
    (cosmetic, hidden): (Option<&Cosmetic>, &[String]),
    work: &CountedWork,
) -> (PageFormats, Option<FormatsRead>) {
    let of = |html| Document {
        html,
        url: &document.url,
        base_url: &document.base_url,
        serialization,
    };
    let matcher = Matcher {
        cosmetic,
        hidden,
        work,
        removal: RefCell::new(None),
    };
    let context = Context {
        clean_up: (cosmetic.is_some() || !hidden.is_empty()).then_some(&matcher as &dyn CleanUp),
        extractor,
        work: work.clone(),
    };
    let Measured {
        formats,
        main_content_length,
    } = goodfetch_extractor::measured_formats(&of(&document.html), &request, &context);
    let removal = matcher.removal.into_inner();
    let (removed, removed_text, share) = match removal {
        Some(removal) if !removal.removed.is_empty() => {
            // The formats read the main content of the document the rules
            // left. The estimate takes the text they removed as what the
            // main content lost; the exact share reads the served
            // document's main content too. A call that read no main content
            // measures no share (story 8).
            let share = main_content_length.map(|after| {
                let before = match measure {
                    ShareMeasure::Estimate => removal.removed_text + after,
                    ShareMeasure::Exact => goodfetch_extractor::main_content_length(
                        &of(&document.html),
                        extractor,
                        work,
                    ),
                };
                Share::of(before, after)
            });
            (removal.removed, removal.removed_text, share)
        }
        Some(_) | None => (Vec::new(), 0, None),
    };
    let cleaned_up = Plan::of(&Demand {
        formats: &request,
        learning: false,
    })
    .runs(Reader::CleanUp);
    let read = cleaned_up.then(|| FormatsRead {
        url: document.url.clone(),
        version: cosmetic.map(|cosmetic| cosmetic.version.clone()),
        removed,
        removed_text,
        share,
        measure,
    });
    (formats, read)
}

#[cfg(test)]
mod tests;
