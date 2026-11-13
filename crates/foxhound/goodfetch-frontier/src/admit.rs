//! The Frontier's admission of a candidate (ADRs 0093, 0102 and 0120):
//! `admit(candidate, context)` answers every seed and every enqueue with an
//! `Enqueued` value, added with its ID or not added with its reason, so a
//! crawl that does nothing says why. A seed is checked as an enqueue is,
//! but takes only an absolute URL, since nothing gives it a base; an
//! enqueue resolves against its document's base URL. A candidate whose
//! identity the visited set holds is a `duplicate`; any other is added by
//! the context, where the crawl keeps its requests, and joins the set, in
//! the same transaction of the run file.
//!
//! An enqueue outside the crawl's scope, measured against every seed, is
//! `out_of_scope`; a seed is never, since seeds define the scope, and each
//! seed added widens it. A seed or an enqueue whose host the Run's domain
//! policy refuses (`egress.allowed_domains`, `egress.blocked_domains`,
//! through the engine's `egress::judge`) is then `domain_policy`, so an
//! off-site link the policy refuses too stays `out_of_scope` (ADRs 0017,
//! 0093 and 0120); both read the host as the WHATWG parser leaves it, the
//! scope alone ignoring one leading `www.`. One the crawl would otherwise
//! admit is then measured against its bounds, `crawl.max_depth` (seeds at depth 0) and
//! `crawl.max_requests`, and `crawl.path_budgets`, then the caps on a URL's
//! shape, each refusal counted by its setting or its path budget. The learned rules join behind this one
//! interface: a candidate that differs from an admitted request only by
//! parameters learned to have no effect is a `duplicate` merged by them,
//! one under a closed trap is not added with `trap_path`, and an offset
//! URL of a listing that prefers its cursor is a `duplicate`; each such
//! URL is recorded with its rule. What an admitted candidate shows, its
//! integer parameters and the page that linked it, teaches the rules in
//! turn, unless the Run set `frontier.learning` off: its admissions then
//! keep no such evidence, and no learned cursor refuses a URL.

use std::sync::LazyLock;

use encoding_rs::Encoding;
use goodfetch_engine::egress::{Judgment, Policy, Request, RequestKind, judge};
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use rusqlite::Connection;
use url::Url;

use crate::bounds::{count_admitted, count_refused};
use crate::identity::{Declared, Key, Seen, identities, keyed, seen, visit_all};
use crate::invariants::Invariants;
use crate::rules;
use crate::scope::{Narrowing, Scope};
use crate::staged::Admitting;
use crate::url::{parse, parse_link};

/// What the Frontier is asked to admit.
#[derive(Clone, Copy, Debug)]
pub enum Candidate<'a> {
    /// A seed: an absolute URL, as the program gave it.
    Seed {
        /// Its URL.
        url: &'a str,
        /// How it is sent and identified.
        declared: Declared<'a>,
    },
    /// A handler's enqueue: a URL, relative to `base` or absolute.
    Enqueue {
        /// Its URL.
        url: &'a str,
        /// The base URL of the document the handler read.
        base: &'a Url,
        /// The URL of the crawl request whose attempt enqueued it, whose
        /// page gave the link.
        from: &'a Url,
        /// How it is sent and identified.
        declared: Declared<'a>,
        /// The call's own patterns, which narrow the crawl's scope, as an
        /// `enqueueLinks` call's `include` and `exclude`.
        narrowing: Narrowing<'a>,
        /// Its depth: one deeper than the request whose attempt added it.
        depth: u32,
    },
    /// The `href` of a link an `enqueueLinks` call matched: a `GET` with no
    /// body, relative to `base` or absolute, its query percent-encoded in
    /// its document's encoding (ADR 0120).
    Href {
        /// The `href`, as written.
        href: &'a str,
        /// The base URL of the document that holds the link.
        base: &'a Url,
        /// The document's encoding.
        encoding: &'static Encoding,
        /// Whether its key keeps the fragment, as its Label's
        /// `crawl.keep_url_fragment` says (ADR 0120).
        keeps_fragment: bool,
        /// The URL of the crawl request whose attempt enqueued it.
        from: &'a Url,
        /// The call's own `include` and `exclude`, which narrow the crawl's
        /// scope.
        narrowing: Narrowing<'a>,
        /// Its depth: one deeper than the request whose attempt added it.
        depth: u32,
    },
    /// An entry of a sitemap a sitemap seed read: a `GET` with no body, at
    /// depth 0, relative to its sitemap's URL or absolute, measured against
    /// the scope and never widening it (ADR 0120).
    SitemapEntry {
        /// Its URL, as the sitemap wrote it.
        url: &'a str,
        /// The URL of the sitemap that lists it.
        sitemap: &'a Url,
    },
}

/// A link's method: `GET`, with no body and no declared key.
static LINK: LazyLock<HttpMethod> = LazyLock::new(HttpMethod::get);

/// Admits `candidate` into the run file `connection` holds, within the
/// Run's `invariants`, its Scope, which an added seed widens, and its
/// Bounds, and under the Run's domain policy
/// `policy`, through `add`, which adds it as a
/// crawl request at its URL as parsed, the URL it is fetched at, and
/// answers its ID.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    candidate: Candidate<'_>,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    admit_in(
        connection,
        invariants,
        policy,
        candidate,
        &mut Admitting::RunFile,
        add,
    )
}

/// Admits `candidate` as [`admit`] does, against what `admitting` adds to
/// the run file (ADR 0093): while an attempt stages, the visited
/// identities, the counts and the seeds of its earlier admissions, and
/// what this one does is noted there; at its commit, each URL this one
/// does not add is noted there, with why.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit_in(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    candidate: Candidate<'_>,
    admitting: &mut Admitting,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let Read {
        given,
        parsed,
        from,
        declared,
        narrowing,
        depth,
    } = Read::of(candidate);
    let url = match parsed {
        Ok(url) => url,
        Err(reason) => {
            return Ok(Enqueued::NotAdded {
                reason,
                url: given.to_owned(),
            });
        }
    };
    let not_added =
        |admitting: &mut Admitting, reason: NotAddedReason, duplicate_of: Option<&str>| {
            admitting.refused(&url, reason, duplicate_of);
            Enqueued::NotAdded {
                reason,
                url: given.to_owned(),
            }
        };
    if let Some(narrowing) = narrowing
        && !invariants
            .scope()
            .admits_in(connection, &url, narrowing, admitting.earlier())?
    {
        return Ok(not_added(admitting, NotAddedReason::OutOfScope, None));
    }
    let request = Request {
        url: url.as_str(),
        kind: RequestKind::Fetch,
    };
    if let Judgment::Refused(_) = judge(&request, policy) {
        return Ok(not_added(admitting, NotAddedReason::DomainPolicy, None));
    }
    let key = Key::of(&url, declared.keeps_fragment);
    let identities = identities(connection, &key, declared)?;
    match seen(connection, &identities, admitting.earlier())? {
        Seen::New => {}
        Seen::Visited(visitor) => {
            return Ok(not_added(
                admitting,
                NotAddedReason::Duplicate,
                Some(&visitor),
            ));
        }
        Seen::Merged {
            rules,
            crawl_request,
        } => {
            for rule in rules {
                rules::merged(connection, rule, url.as_str(), Some(identities.plain()))?;
            }
            return Ok(not_added(
                admitting,
                NotAddedReason::Duplicate,
                Some(&crawl_request),
            ));
        }
    }
    let bounds = invariants.bounds();
    if let Some((reason, rule)) = rules::refuses(connection, key.url(), bounds.learning)? {
        rules::merged(connection, rule, url.as_str(), None)?;
        return Ok(not_added(admitting, reason, None));
    }
    let budget = invariants.budget_of(&url);
    if let Some(reason) =
        bounds.refuses(connection, &url, depth.into(), budget, admitting.earlier())?
    {
        count_refused(connection, reason, budget)?;
        return Ok(not_added(admitting, reason, None));
    }
    let id = add(&url)?;
    count_admitted(connection, budget)?;
    visit_all(connection, &identities, &id)?;
    admitting.admitted(
        &id,
        identities.plain(),
        identities.learned(),
        budget.map(|budget| budget.pattern.as_str()),
    );
    if bounds.learning {
        rules::admitted(connection, key.url(), from.map(keyed).as_ref())?;
    }
    if narrowing.is_none() {
        Scope::widen(connection, &url)?;
        admitting.widened(&url);
    }
    Ok(Enqueued::Added {
        id,
        url: url.into(),
    })
}

/// What a candidate gives the admission: its URL as given and as parsed,
/// the URL of the page that gave it, how it is sent and identified, the
/// patterns narrowing the scope for it (none for a seed, which the scope
/// does not measure) and its depth.
struct Read<'a> {
    given: &'a str,
    parsed: Result<Url, NotAddedReason>,
    from: Option<&'a Url>,
    declared: Declared<'a>,
    narrowing: Option<Narrowing<'a>>,
    depth: u32,
}

impl<'a> Read<'a> {
    fn of(candidate: Candidate<'a>) -> Self {
        let (given, parsed, from, declared, narrowing, depth) = match candidate {
            Candidate::Seed { url, declared } => (url, parse(url, None), None, declared, None, 0),
            Candidate::Href {
                href,
                base,
                encoding,
                keeps_fragment,
                from,
                narrowing,
                depth,
            } => (
                href,
                parse_link(href, base, encoding),
                Some(from),
                Declared {
                    method: &LINK,
                    body: None,
                    key: None,
                    keeps_fragment,
                },
                Some(narrowing),
                depth,
            ),
            Candidate::Enqueue {
                url,
                base,
                from,
                declared,
                narrowing,
                depth,
            } => (
                url,
                parse(url, Some(base)),
                Some(from),
                declared,
                Some(narrowing),
                depth,
            ),
            Candidate::SitemapEntry { url, sitemap } => (
                url,
                parse(url, Some(sitemap)),
                None,
                Declared {
                    method: &LINK,
                    body: None,
                    key: None,
                    keeps_fragment: false,
                },
                Some(Narrowing::none()),
                0,
            ),
        };
        Self {
            given,
            parsed,
            from,
            declared,
            narrowing,
            depth,
        }
    }
}

#[cfg(test)]
mod tests;
