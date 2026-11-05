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
//!
//! When no reader the Run asks for needs the tree (no Page signature, so
//! `frontier.learning: false`, no `XPath` field, and every selector one
//! lol-html reads), the readers read a document in one streaming pass
//! instead ([`UntouchedReaders::stream`], `goodfetch_extractor::Streaming`):
//! the same `<base>`, refresh directive, links with their trap-link
//! judgement, fields, Page check and robots `meta`, with no tree. A
//! document whose markup names an AI-use signal, or whose tree the pass
//! cannot vouch for, is parsed as before.

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use goodfetch_extractor::{Query, StreamSelector, Streamed, Streaming};
use goodfetch_protocol::foxhound::crawl::FieldLocator;
use goodfetch_protocol::settings::page_check::PageCheck;
use url::Url;

use crate::canonical;
use crate::document::{
    self, CONTROLS, FieldElement, Locating, LocatorInvalid, Matched, SelectorInvalid,
};
use crate::expected_page::{self, Verdict};
use crate::refresh::{self, Refresh};
use crate::use_signals::{self, SignalReaders, UseSignals};

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
    /// The streaming path's selectors, when it serves every reader.
    stream: Option<StreamReaders>,
}

/// The selectors the streaming path reads the readers' elements with,
/// parsed once per Run, each beside its reader's tree matcher.
#[derive(Debug)]
struct StreamReaders {
    streaming: Streaming,
    base: StreamSelector,
    refresh: StreamSelector,
    controls: StreamSelector,
    robots: StreamSelector,
    /// For each of the readers' links, its selector, none for one the
    /// engine does not read.
    links: Vec<Option<StreamSelector>>,
    /// For each field's CSS selector, its selector; none when a locator
    /// is invalid.
    fields: Option<Vec<StreamSelector>>,
    /// For each selector the Page check names, its selector, none for one
    /// the engine does not read.
    page_check: Vec<Option<StreamSelector>>,
}

impl StreamReaders {
    /// The streaming path's selectors of `readers`, none when it cannot
    /// serve one of them: the Page signature, an `XPath` field, or a
    /// selector lol-html does not read.
    fn of(readers: &UntouchedReaders, work: &CountedWork) -> Option<Self> {
        if readers.signs {
            return None;
        }
        let literal = |selector| StreamSelector::of(selector, work);
        // A selector the engine refuses reads nothing on either path.
        let each = |selector: &str, read: bool| -> Option<Option<StreamSelector>> {
            if read {
                StreamSelector::of(selector, work).map(Some)
            } else {
                Some(None)
            }
        };
        let links = readers
            .links
            .iter()
            .map(|(selector, matcher)| each(selector, matcher.is_ok()))
            .collect::<Option<_>>()?;
        let fields = match &readers.fields {
            Some((locators, Ok(_))) => Some(
                locators
                    .iter()
                    .map(|locator| match locator {
                        FieldLocator::Css { selector } => StreamSelector::of(selector, work),
                        FieldLocator::Xpath { .. } => None,
                    })
                    .collect::<Option<_>>()?,
            ),
            Some((_, Err(_))) | None => None,
        };
        let page_check = readers
            .page_check
            .iter()
            .flat_map(|(_, matchers)| matchers)
            .map(|(selector, matcher)| each(selector, matcher.is_some()))
            .collect::<Option<_>>()?;
        Some(Self {
            streaming: Streaming::new(work),
            base: literal(BASE)?,
            refresh: literal(refresh::META)?,
            controls: literal(CONTROLS)?,
            robots: literal(expected_page::ROBOTS)?,
            links,
            fields,
            page_check,
        })
    }
}

/// What the readers of the untouched page read of a document in one
/// streaming pass, before the read of a crawl request's own document.
#[derive(Debug)]
pub(crate) struct StreamRead {
    base: Vec<Streamed>,
    refresh: Vec<Streamed>,
    controls: Vec<Streamed>,
    robots: Vec<Streamed>,
    links: Vec<Vec<Streamed>>,
    fields: Vec<Vec<Streamed>>,
    page_check: Vec<Vec<Streamed>>,
}

impl StreamRead {
    /// The refresh directive the document at `url` holds in its markup.
    pub(crate) fn refresh(&self, url: &Url) -> Option<Refresh> {
        refresh::first_of(
            url,
            self.refresh.iter().filter_map(|meta| {
                let own = meta.held.first()?;
                Some((own.attr("http-equiv"), own.attr("content")))
            }),
        )
    }

    /// The `href` of its first `<base>` with one, as written.
    pub(crate) fn base_href(&self) -> Option<String> {
        self.base
            .first()
            .and_then(|base| base.held.first()?.attr("href"))
            .map(str::to_owned)
    }
}

impl UntouchedReaders {
    /// The readers of `demand`, each selector compiled once on `work`.
    pub fn of(demand: UntouchedDemand, work: &CountedWork) -> Self {
        let literal = |selector| work.literal_selector(Row::PageSelectorCompilations, selector);
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
        let mut readers = Self {
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
            stream: None,
        };
        readers.stream = StreamReaders::of(&readers, work);
        readers
    }

    /// What the readers read of `html`, the untouched HTML document, in
    /// one streaming pass; none when the pass cannot serve them all, its
    /// caller then parsing the document: when a reader the Run asks for
    /// needs a tree, its markup names an AI-use signal, or the pass cannot
    /// vouch for its tree.
    pub(crate) fn stream(&self, html: &str) -> Option<StreamRead> {
        let stream = self.stream.as_ref()?;
        if use_signals::names_signals(html) {
            return None;
        }
        let all = |selector| Query {
            selector,
            first: false,
        };
        let first = |selector| Query {
            selector,
            first: true,
        };
        let mut queries = vec![
            first(&stream.base),
            all(&stream.refresh),
            all(&stream.controls),
            all(&stream.robots),
        ];
        queries.extend(stream.links.iter().flatten().map(all));
        let links_end = queries.len();
        queries.extend(stream.fields.iter().flatten().map(first));
        let fields_end = queries.len();
        queries.extend(stream.page_check.iter().flatten().map(first));
        let mut read = stream.streaming.read(html, &queries)?.into_iter();
        let mut next = |count: usize| read.by_ref().take(count).collect::<Vec<_>>();
        let mut fixed = next(FIXED).into_iter();
        let (base, refresh, controls, robots) = (
            fixed.next().unwrap_or_default(),
            fixed.next().unwrap_or_default(),
            fixed.next().unwrap_or_default(),
            fixed.next().unwrap_or_default(),
        );
        let links = next(links_end - FIXED);
        let fields = next(fields_end - links_end);
        let page_check = next(usize::MAX);
        Some(StreamRead {
            base,
            refresh,
            controls,
            robots,
            links,
            fields,
            page_check,
        })
    }

    /// `signals` with those the markup of a document the streaming path
    /// read, fetched at `url`, carries: none, since a document whose markup
    /// names one is not streamed.
    pub(crate) fn streamed_use_signals(signals: UseSignals, url: &Url) -> UseSignals {
        signals.with_header_license(url)
    }

    /// [`UntouchedReaders::read`] of a crawl request's own document, which
    /// the streaming path read as `streamed`: the document fetched at
    /// `url`. It holds no Page signature and no `rel=canonical`, which
    /// only a Run that learns reads, on the tree.
    pub(crate) fn read_streamed(&self, streamed: StreamRead, url: &str) -> Read {
        let StreamRead {
            controls,
            robots,
            links: mut streamed_links,
            fields: streamed_fields,
            page_check: checked,
            ..
        } = streamed;
        let mut streamed_links = streamed_links.drain(..);
        let links = self
            .links
            .iter()
            .map(|(selector, matcher)| {
                let matched = matcher.as_ref().map_err(Clone::clone).map(|_| {
                    document::matched_streamed(streamed_links.next().unwrap_or_default(), &controls)
                });
                (selector.clone(), matched)
            })
            .collect();
        let fields = self.fields.as_ref().map(|(locators, locating)| {
            let found = locating.as_ref().map_err(Clone::clone).map(|_| {
                streamed_fields
                    .into_iter()
                    .map(|mut found| found.drain(..).next().and_then(document::field_streamed))
                    .collect()
            });
            (locators.clone(), found)
        });
        let page_check = self.page_check.as_ref().map(|(check, matchers)| {
            let mut checked = checked.into_iter();
            let found: Vec<(&str, Option<bool>)> = matchers
                .iter()
                .map(|(selector, matcher)| {
                    let found = matcher
                        .as_ref()
                        .map(|_| !checked.next().unwrap_or_default().is_empty());
                    (selector.as_str(), found)
                })
                .collect();
            let verdict = expected_page::judge_with(check, url, |selector| {
                found
                    .iter()
                    .find(|(named, _)| *named == selector)
                    .and_then(|(_, found)| *found)
            });
            let robots = robots
                .iter()
                .filter_map(|meta| meta.held.first())
                .filter(|meta| {
                    meta.attr("name")
                        .is_some_and(|name| name.eq_ignore_ascii_case("robots"))
                })
                .map(|meta| meta.attr("content").unwrap_or_default().to_owned())
                .collect();
            (check.clone(), verdict, robots)
        });
        Read {
            links,
            fields,
            page_check,
            canonical: None,
            signature: None,
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

/// How many of the streaming path's queries every pass reads: the
/// `<base>`, refresh, `aria-controls` and robots `meta` elements.
const FIXED: usize = 4;

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
