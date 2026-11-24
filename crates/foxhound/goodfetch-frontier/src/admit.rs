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
//!
//! An admission has two parts (#2244; ADR 0142). The first reads the
//! candidate and the Run's invariants alone and takes no run file: it
//! parses the URL, measures it against the Scope's patterns and the call's,
//! reads what it asks the Scope's seeds, judges its host by the domain
//! policy, makes its key and its fixed identity, finds the path budget
//! that counts it and the cap on its shape it is past. [`key_link`] runs
//! it for a page's link on the page's task, and answers a [`Keyed`]. The
//! second, [`admit_keyed`], needs the Run's state and runs where it is
//! held: whether a seed reaches the URL, its learned identity, the visited
//! set, the learned refusals, the Bounds' counts and the insert. The
//! Scope's seeds and the learned rules are state, not invariants, a seed
//! admitted while the crawl runs widening the first and a rule learned or
//! stopped changing the second, so the second part applies both as they
//! stand when it runs: a page's task sees neither, and a link is never
//! judged by seeds or rules older than its admission. The verdicts keep
//! their order, each part's judgement told in the place the whole
//! admission told it.

use std::sync::LazyLock;

use encoding_rs::Encoding;
use goodfetch_engine::egress::{Judgment, Policy, Request, RequestKind, judge};
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use rusqlite::Connection;
use url::Url;

use crate::bounds::{Bounds, SoFar, count_admitted, count_refused, keep_counts};
use crate::identity::{
    Declared, Fixed, Key, Seen, fixed_only, keep_learned, keyed as keyed_url, seen, under_rules,
    visit_all,
};
use crate::invariants::Invariants;
use crate::rules;
use crate::scope::{Asked, Narrowing, Scope};
use crate::staged::Admitting;
use crate::unwritten::{Holding, UnwrittenVisits};
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
/// does not add is noted there, with why. Both parts of the admission run
/// here, one after the other.
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
    let (given, keyed, entering) = key_candidate(invariants, policy, candidate);
    admit_keyed(
        connection,
        invariants,
        (given, &keyed),
        entering,
        admitting,
        add,
    )
}

/// Keys `candidate` within the Run's `invariants` and under its domain
/// policy `policy`, as [`key_link`] keys a page's link: the part of its
/// admission that takes no run file. Answers the candidate as given, keyed,
/// and what it enters the crawl with, which [`admit_keyed`] admits; an
/// attempt's staging keeps them for its commit (#2245).
#[must_use]
pub fn key_candidate<'a>(
    invariants: &Invariants,
    policy: &Policy,
    candidate: Candidate<'a>,
) -> (&'a str, Keyed, Entering<'a>) {
    let Read {
        given,
        parsed,
        from,
        declared,
        narrowing,
        depth,
    } = Read::of(candidate);
    let keyed = key(invariants, policy, parsed, declared, narrowing);
    (given, keyed, Entering { from, depth })
}

/// A link of a page, as the page's task reads it: an `href`, a `GET` with
/// no body, relative to `base` or absolute, its query percent-encoded in
/// its document's encoding (ADR 0120).
#[derive(Clone, Copy, Debug)]
pub struct Link<'a> {
    /// The `href`, as written.
    pub href: &'a str,
    /// The base URL of the document that holds the link.
    pub base: &'a Url,
    /// The document's encoding.
    pub encoding: &'static Encoding,
    /// Whether its key keeps the fragment, as its Label's
    /// `crawl.keep_url_fragment` says (ADR 0120).
    pub keeps_fragment: bool,
    /// The call's own `include` and `exclude`, which narrow the crawl's
    /// scope.
    pub narrowing: Narrowing<'a>,
}

/// A candidate as the first part of its admission judged it, from itself
/// and the Run's invariants alone (#2244): not parsed, not let through by
/// the Scope's patterns, refused by the domain policy, or keyed, with all
/// the second part needs of it. It holds no state of the Run, so it is as
/// true when it is admitted as when it was made, and an attempt's commit
/// adds it as its staging keyed it.
#[derive(Clone, Debug)]
pub struct Keyed(Result<Measured, Unkeyed>);

impl Keyed {
    /// Its URL's key and its fixed identity, when it was keyed.
    pub(crate) fn identity(&self) -> Option<(&Key, &Fixed)> {
        self.0
            .as_ref()
            .ok()
            .map(|measured| (&measured.key, &measured.fixed))
    }

    /// The identity its fixed rules make, when it was keyed: the one its
    /// admission looks up in the visited set first.
    #[must_use]
    pub fn plain(&self) -> Option<&str> {
        self.identity().map(|(_, fixed)| fixed.plain())
    }
}

/// A candidate the first part of admission does not key.
#[derive(Clone, Debug)]
enum Unkeyed {
    /// Its URL does not parse, or is one a crawl does not fetch.
    Unparsed(NotAddedReason),
    /// The Scope's patterns, or its call's, do not let its URL through.
    Narrowed(Url),
    /// The Run's domain policy refuses its URL's host: `domain_policy`,
    /// unless no seed reaches the URL, which is then `out_of_scope`.
    Policed {
        url: Url,
        /// What it asks the Scope's seeds; nothing for a seed.
        asked: Option<Asked>,
    },
}

/// A candidate keyed: what the second part of its admission reads of it.
#[derive(Clone, Debug)]
struct Measured {
    url: Url,
    /// What it asks the Scope's seeds; nothing for a seed, which the Scope
    /// does not measure and which widens it.
    asked: Option<Asked>,
    key: Key,
    fixed: Fixed,
    /// Where, among the Run's path budgets, the one that counts it is.
    budget: Option<usize>,
    /// The cap on its shape it is past.
    shape: Option<NotAddedReason>,
}

/// Keys `link` within the Run's `invariants` and under its domain policy
/// `policy`: the part of its admission that takes no run file, which its
/// page's task runs (#2244). [`admit_keyed`] admits what it answers.
#[must_use]
pub fn key_link(invariants: &Invariants, policy: &Policy, link: Link<'_>) -> Keyed {
    key(
        invariants,
        policy,
        parse_link(link.href, link.base, link.encoding),
        Declared {
            method: &LINK,
            body: None,
            key: None,
            keeps_fragment: link.keeps_fragment,
        },
        Some(link.narrowing),
    )
}

/// The first part of the admission of a candidate whose URL parsed as
/// `parsed`, declared so, under the `narrowing` the Scope measures it
/// with, none for a seed.
fn key(
    invariants: &Invariants,
    policy: &Policy,
    parsed: Result<Url, NotAddedReason>,
    declared: Declared<'_>,
    narrowing: Option<Narrowing<'_>>,
) -> Keyed {
    let url = match parsed {
        Ok(url) => url,
        Err(reason) => return Keyed(Err(Unkeyed::Unparsed(reason))),
    };
    let scope = invariants.scope();
    let asked = match narrowing {
        Some(narrowing) if !scope.lets_through(&url, narrowing) => {
            return Keyed(Err(Unkeyed::Narrowed(url)));
        }
        Some(_) => Some(scope.asks(&url)),
        None => None,
    };
    let request = Request {
        url: url.as_str(),
        kind: RequestKind::Fetch,
    };
    if let Judgment::Refused(_) = judge(&request, policy) {
        return Keyed(Err(Unkeyed::Policed { url, asked }));
    }
    let key = Key::of(&url, declared.keeps_fragment);
    let fixed = Fixed::of(&key, declared);
    let budget = invariants.budget_at(&url);
    let shape = invariants.bounds().shape_refuses(&url);
    Keyed(Ok(Measured {
        url,
        asked,
        key,
        fixed,
        budget,
        shape,
    }))
}

/// What a keyed candidate enters the crawl with, which its attempt holds
/// and its key does not: the page that gave it and its depth.
#[derive(Clone, Copy, Debug)]
pub struct Entering<'a> {
    /// The URL of the crawl request whose attempt gave it; none for a
    /// seed or a sitemap's entry.
    pub from: Option<&'a Url>,
    /// Its depth: 0 for a seed, else one deeper than the request whose
    /// attempt added it.
    pub depth: u32,
}

/// Admits the candidate given as `given` and keyed as `keyed` under the
/// Run's `invariants`, entering as `entering` says, into the run file
/// `connection` holds, against what `admitting` adds to it, through `add`,
/// as [`admit_in`] does: the part of an admission that needs the Run's
/// state (#2244). It asks the Scope's seeds and makes the learned identity
/// as both stand now, then checks the visited set, the learned refusals
/// and the Bounds, and adds.
///
/// # Errors
///
/// When the run file cannot be read or written, or what `add` fails with.
pub fn admit_keyed(
    connection: &Connection,
    invariants: &Invariants,
    (given, keyed): (&str, &Keyed),
    entering: Entering<'_>,
    admitting: &mut Admitting,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let scope = invariants.scope();
    match &keyed.0 {
        Ok(measured) => admit_measured(
            connection, invariants, given, measured, entering, admitting, add,
        ),
        Err(Unkeyed::Unparsed(reason)) => Ok(Enqueued::NotAdded {
            reason: reason.clone(),
            url: given.to_owned(),
        }),
        Err(Unkeyed::Narrowed(url)) => Ok(not_added(
            admitting,
            (given, url),
            NotAddedReason::OutOfScope,
            None,
        )),
        Err(Unkeyed::Policed { url, asked }) => {
            let reason = match asked {
                Some(asked)
                    if !scope.reached(
                        connection,
                        url,
                        asked,
                        admitting.earlier(),
                        invariants.unwritten(),
                    )? =>
                {
                    NotAddedReason::OutOfScope
                }
                _ => NotAddedReason::DomainPolicy,
            };
            Ok(not_added(admitting, (given, url), reason, None))
        }
    }
}

/// The answer of a candidate given as `given`, whose URL parsed as `url`,
/// that is not added for `reason`, a duplicate of the crawl request
/// `duplicate_of` when the visited set holds its identity, noted in
/// `admitting`.
fn not_added(
    admitting: &mut Admitting,
    (given, url): (&str, &Url),
    reason: NotAddedReason,
    duplicate_of: Option<&str>,
) -> Enqueued {
    admitting.refused(url, reason.clone(), duplicate_of);
    Enqueued::NotAdded {
        reason,
        url: given.to_owned(),
    }
}

/// Admits the candidate given as `given`, keyed as `measured`, as
/// [`admit_keyed`] does.
fn admit_measured(
    connection: &Connection,
    invariants: &Invariants,
    given: &str,
    measured: &Measured,
    entering: Entering<'_>,
    admitting: &mut Admitting,
    add: impl FnOnce(&Url) -> rusqlite::Result<String>,
) -> rusqlite::Result<Enqueued> {
    let Measured {
        url,
        asked,
        key,
        fixed,
        budget,
        shape,
    } = measured;
    let answered = (given, url);
    if let Some(asked) = asked
        && !invariants.scope().reached(
            connection,
            url,
            asked,
            admitting.earlier(),
            invariants.unwritten(),
        )?
    {
        let reason = NotAddedReason::OutOfScope;
        return Ok(not_added(admitting, answered, reason, None));
    }
    let bounds = invariants.bounds();
    // Held from the keying through the visit, so no reader's chain takes
    // the identity between them (#2168).
    let mut holding = invariants.unwritten().map(UnwrittenVisits::lock);
    keep_rules(connection, bounds, admitting, holding.as_mut())?;
    let identities = if bounds.learning {
        under_rules(connection, key, fixed, holding.as_ref())?
    } else {
        fixed_only(fixed)
    };
    match seen(
        connection,
        &identities,
        admitting.staged_visits(),
        holding.as_ref(),
    )? {
        Seen::New => {}
        Seen::Visited(visitor) => {
            let reason = NotAddedReason::Duplicate;
            return Ok(not_added(admitting, answered, reason, Some(&visitor)));
        }
        Seen::Merged {
            rules,
            crawl_request,
        } => {
            for rule in rules {
                rules::merged(connection, rule, url.as_str(), Some(identities.plain()))?;
            }
            admitting.leaves();
            let reason = NotAddedReason::Duplicate;
            return Ok(not_added(admitting, answered, reason, Some(&crawl_request)));
        }
    }
    let held = holding.as_ref().and_then(Holding::rules);
    if let Some((reason, rule)) = rules::refuses(connection, key.url(), bounds.learning, held)? {
        rules::merged(connection, rule, url.as_str(), None)?;
        admitting.leaves();
        return Ok(not_added(admitting, answered, reason, None));
    }
    let budget = (*budget).and_then(|at| invariants.budget(at));
    let depth = entering.depth.into();
    let mut so_far = SoFar {
        connection,
        holding: holding.as_mut(),
        admitting,
        learning: bounds.learning,
    };
    if let Some(reason) = bounds.past(depth, budget, &mut so_far)?.or(shape.clone()) {
        count_refused(connection, &reason, budget)?;
        admitting.leaves();
        return Ok(not_added(admitting, answered, reason, None));
    }
    let id = add(url)?;
    count_admitted(connection, budget)?;
    visit_all(connection, &identities, &id)?;
    if let Some(holding) = &mut holding
        && admitting.writes()
    {
        holding.counted(budget.map(|budget| budget.pattern.as_str()));
        holding.written(identities.plain(), identities.learned(), &id);
    }
    admitting.admitted(
        &id,
        identities.plain(),
        identities.learned(),
        budget.map(|budget| budget.pattern.as_str()),
    );
    if bounds.learning {
        invariants.admission_evidence(
            connection,
            (key.url(), entering.from.map(keyed_url)),
            admitting.writes(),
        )?;
    }
    if asked.is_none() {
        widen(connection, invariants, url, admitting, holding.as_mut())?;
    }
    Ok(Enqueued::Added {
        id,
        url: url.as_str().to_owned(),
    })
}

/// Has the Frontier's memory, when the Run holds the batches' visits, read
/// the learned rules and identities and the Bounds' counts, when one
/// bounds the requests, from the file on `connection`, the writer's, as
/// the crawl opens, so no admission's lookup of them reads the file while
/// memory holds them (#2169). A Run that does not learn reads the rules
/// too: a link seen again keys its request under them, and none changes.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn keep_memory(
    connection: &Connection,
    invariants: &Invariants,
) -> rusqlite::Result<()> {
    let Some(unwritten) = invariants.unwritten() else {
        return Ok(());
    };
    let bounds = invariants.bounds();
    let mut holding = unwritten.opening();
    let hosts = rules::keep_held(connection, &mut holding)?;
    keep_learned(connection, &mut holding, &hosts)?;
    keep_counts(connection, bounds, &mut holding)
}

/// Brings the learned rules memory holds, `holding` under its lock, and
/// the learned identities, up to those the writer's connection
/// `connection` holds, for an admission that writes in a Run that learns,
/// so it keys a URL, asks whether a rule refuses it and whether its
/// learned identity is a visited one's from memory (#2169).
///
/// # Errors
///
/// When the run file cannot be read.
fn keep_rules(
    connection: &Connection,
    bounds: &Bounds,
    admitting: &Admitting,
    holding: Option<&mut Holding<'_>>,
) -> rusqlite::Result<()> {
    match holding {
        Some(holding) if bounds.learning && admitting.writes() => {
            let hosts = rules::keep_held(connection, holding)?;
            keep_learned(connection, holding, &hosts)
        }
        _ => Ok(()),
    }
}

/// Widens the scope of the run file `connection` holds by the seed
/// `seed`, noted in `admitting` and in `holding`, the Frontier's memory
/// under its lock when the Run keeps one, which holds the seed's measure
/// when `admitting` writes it to the file (#2169).
///
/// # Errors
///
/// When the run file cannot be written.
pub(crate) fn widen(
    connection: &Connection,
    invariants: &Invariants,
    seed: &Url,
    admitting: &mut Admitting,
    holding: Option<&mut Holding<'_>>,
) -> rusqlite::Result<()> {
    Scope::widen(connection, seed)?;
    admitting.widened(seed);
    if let Some(holding) = holding {
        let measure = admitting
            .writes()
            .then(|| invariants.scope().seed_key(seed))
            .flatten();
        holding.widened(measure.as_deref());
    }
    Ok(())
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
