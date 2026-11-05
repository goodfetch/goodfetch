//! The readers of the untouched page (ADR 0138; spec #1577): what Foxhound
//! reads of a fetched document as it was served, all from one tree parsed
//! once per document, given to the Page analysis as its first stage.
//!
//! - [`UntouchedReaders`] is that stage, built once per Run for each Label: the
//!   selectors of its fixed readers compiled once, and what the Run asks of
//!   its crawl requests' own documents: a code-free crawl's links and
//!   fields, the Label's Page check, and, while the Run learns, the Page
//!   signature.
//! - [`Read`] is what they read of one document, which every later step
//!   shares: its `<base>`, its refresh directive, its `rel=canonical`, its
//!   links with their markup's trap-link judgement, its fields, its Page
//!   check's verdict and its robots `meta`, its AI-use signals' markup and
//!   its Page signature. The signature's main-content step reads last, as
//!   it consumes the tree.
//!
//! A redirect hop's document is read in turn: the chain reads its refresh
//! directive on its tree, and only the document the chain ends on is read
//! whole (the `chain` module's [`crate::chain::Reading`]).

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_protocol::foxhound::crawl::FieldLocator;
use goodfetch_protocol::settings::page_check::PageCheck;
use url::Url;

use crate::canonical;
use crate::document::{
    self, CONTROLS, FieldElement, Locating, LocatorInvalid, Matched, SelectorInvalid,
};
use crate::expected_page::{self, Verdict};
use crate::refresh::{self, Refresh};
use crate::use_signals::{SignalReaders, UseSignals};

/// What a Run asks of its crawl requests' own documents, beyond what every
/// document's readers read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UntouchedDemand {
    /// The CSS selectors whose links a code-free crawl reads.
    pub links: Vec<String>,
    /// The locators of a code-free crawl's fields, in order; none for a
    /// crawl reading no field.
    pub fields: Vec<FieldLocator>,
    /// The Page check judging the Label's documents: its own, else the
    /// Run's.
    pub page_check: Option<PageCheck>,
    /// Whether the Run learns from its pages (`frontier.learning`): their
    /// `rel=canonical` and Page signature.
    pub signs: bool,
}

/// The readers of the untouched page, built once per Run for a Label.
#[derive(Debug)]
pub struct UntouchedReaders {
    base: dom_query::Matcher,
    refresh: dom_query::Matcher,
    canonical: dom_query::Matcher,
    controls: dom_query::Matcher,
    robots: dom_query::Matcher,
    signals: SignalReaders,
    links: Vec<(String, Result<dom_query::Matcher, SelectorInvalid>)>,
    fields: Option<(Vec<FieldLocator>, Located)>,
    page_check: Option<(PageCheck, CheckMatchers)>,
    signs: bool,
}

impl UntouchedReaders {
    /// The readers of `demand`, each selector compiled once on `work`, in
    /// the engine's cache. So are the selectors of the main-content step a
    /// Run that learns reads its Page signature with, so that no page
    /// compiles them (ADRs 0138 and 0140).
    pub fn of(demand: UntouchedDemand, work: &CountedWork) -> Self {
        let literal = |selector| work.literal_selector(Row::PageSelectorCompilations, selector);
        if demand.signs {
            goodfetch_extractor::compile_selectors(work);
        }
        let UntouchedDemand {
            links,
            fields,
            page_check,
            signs,
        } = demand;
        let page_check = page_check.map(|check| {
            let selectors = expected_page::selectors(&check)
                .map(|selector| (selector.to_owned(), document::matcher(selector, work).ok()))
                .collect();
            (check, selectors)
        });
        Self {
            base: literal(BASE),
            refresh: literal(refresh::META),
            canonical: literal(canonical::LINK),
            controls: literal(CONTROLS),
            robots: literal(expected_page::ROBOTS),
            signals: SignalReaders {
                meta: literal(SignalReaders::META),
                base: literal(SignalReaders::BASE),
                link: literal(SignalReaders::LINK),
            },
            links: links
                .into_iter()
                .map(|selector| {
                    let matcher = document::matcher(&selector, work);
                    (selector, matcher)
                })
                .collect(),
            fields: (!fields.is_empty()).then(|| {
                let locating = document::locating(&fields, work);
                (fields, locating)
            }),
            page_check,
            signs,
        }
    }

    /// The refresh directive `document`, the untouched tree of the HTML
    /// document at `url`, holds in its markup.
    pub(crate) fn refresh(&self, url: &Url, document: &ParsedHtml) -> Option<Refresh> {
        refresh::in_tree(url, document, &self.refresh)
    }

    /// The `href` of `document`'s first `<base>` with one, as written.
    pub(crate) fn base_href(&self, document: &ParsedHtml) -> Option<String> {
        document
            .select_matcher(&self.base)
            .attr("href")
            .map(|href| href.to_string())
    }

    /// `signals` with those the markup `html`, whose untouched tree is
    /// `document`, carries, fetched at `url`.
    pub(crate) fn use_signals(
        &self,
        signals: UseSignals,
        (html, document): (&str, &ParsedHtml),
        url: &Url,
    ) -> UseSignals {
        signals.with_tree(html, document, url, &self.signals)
    }

    /// What the readers of a crawl request's own document read of
    /// `document`, its untouched tree, which they consume: the document
    /// fetched at `url`, whose base URL is `base`. `signed` reads its Page
    /// signature once the others read it, on the same tree.
    pub(crate) fn read(
        &self,
        document: ParsedHtml,
        (url, base): (&str, &str),
        signed: impl FnOnce(ParsedHtml) -> [u8; 32],
    ) -> Read {
        let links = self
            .links
            .iter()
            .map(|(selector, matcher)| {
                let matched = matcher
                    .as_ref()
                    .map(|matcher| document::matched_in(&document, matcher, &self.controls))
                    .map_err(Clone::clone);
                (selector.clone(), matched)
            })
            .collect();
        let fields = self.fields.as_ref().map(|(locators, locating)| {
            let found = locating
                .as_ref()
                .map(|locating| document::first_elements_in(&document, locating))
                .map_err(Clone::clone);
            (locators.clone(), found)
        });
        let page_check = self.page_check.as_ref().map(|(check, matchers)| {
            let verdict = expected_page::judge_with(check, url, |selector| {
                let (_, matcher) = matchers.iter().find(|(named, _)| named == selector)?;
                let matcher = matcher.as_ref()?;
                Some(!document::matched_in(&document, matcher, &self.controls).is_empty())
            });
            let robots = expected_page::robots_in(&document, &self.robots);
            (check.clone(), verdict, robots)
        });
        let canonical = if self.signs {
            Url::parse(base)
                .ok()
                .and_then(|base| canonical::in_head(&document, &self.canonical, &base))
        } else {
            None
        };
        let signature = self.signs.then(|| signed(document));
        Read {
            links,
            fields,
            page_check,
            canonical,
            signature,
        }
    }
}

/// A code-free crawl's fields' locators, read once.
type Located = Result<Vec<Locating>, LocatorInvalid>;

/// The matcher of each CSS selector a Page check names, by selector; none
/// for one the engine does not read.
type CheckMatchers = Vec<(String, Option<dom_query::Matcher>)>;

/// The first element each field's locator found.
type Found = Result<Vec<Option<FieldElement>>, LocatorInvalid>;

/// What a document's `<base>` is found by.
const BASE: &str = "base[href]";

/// What the readers of the untouched page read of a crawl request's own
/// document, for the steps after them.
#[derive(Debug, Default)]
pub(crate) struct Read {
    /// The links each selector the Run asked for matched, by selector.
    links: Vec<(String, Result<Vec<Matched>, SelectorInvalid>)>,
    /// The first element each field's locator found, with the locators.
    fields: Option<(Vec<FieldLocator>, Found)>,
    /// The Page check's verdict, with the check, and the `content` of each
    /// robots `meta` element.
    page_check: Option<(PageCheck, Verdict, Vec<String>)>,
    /// The URL its head's `rel=canonical` names, resolved, while the Run
    /// learns.
    canonical: Option<Url>,
    /// Its Page signature, while the Run learns.
    signature: Option<[u8; 32]>,
}

impl Read {
    /// The links `selector` matched, when the Run asked for them.
    pub(crate) fn links(&self, selector: &str) -> Option<Result<Vec<Matched>, SelectorInvalid>> {
        self.links
            .iter()
            .find(|(asked, _)| asked == selector)
            .map(|(_, matched)| matched.clone())
    }

    /// The first element each of `locators` found, when the Run asked for
    /// exactly those.
    pub(crate) fn fields(&self, locators: &[FieldLocator]) -> Option<Found> {
        self.fields
            .as_ref()
            .filter(|(asked, _)| asked.as_slice() == locators)
            .map(|(_, found)| found.clone())
    }

    /// The verdict of `check` and the robots `meta` elements' `content`,
    /// when the Run asked for that check.
    pub(crate) fn judged(&self, check: &PageCheck) -> Option<(Verdict, &[String])> {
        self.page_check
            .as_ref()
            .filter(|(asked, _, _)| asked == check)
            .map(|(_, verdict, robots)| (*verdict, robots.as_slice()))
    }

    /// Its Page signature and its head's `rel=canonical`, while the Run
    /// learns.
    pub(crate) fn signed(&self) -> Option<([u8; 32], Option<&Url>)> {
        self.signature
            .map(|signature| (signature, self.canonical.as_ref()))
    }
}

#[cfg(test)]
mod tests;
