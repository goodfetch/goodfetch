//! What an attempt's staged admissions did, held beside the run file
//! (ADR 0093). A handler's enqueue, `enqueueLinks` call or seed added
//! through `ctx.run` is answered as the Frontier will admit it after the
//! attempt's earlier ones, though none of them reached the run file: each
//! is admitted once, on the run file as it stands, inside a savepoint its
//! caller rolls back, and reads what the attempt's earlier admissions did
//! from here: the identities they visited, plain and learned, with the
//! requests they added; their count against `crawl.max_requests` and each
//! path budget; the seeds that widened the scope; and the sitemaps they
//! added. What a learned rule takes from them, an offset listing's cursor,
//! stays with the commit, which adds each request they answered `added`,
//! in order, on the run file, and judges again only the candidates whose
//! refusal wrote to it, a bound's count or a learned rule's record
//! (#2245). A request the commit then does not add, because another
//! attempt's commit added it meanwhile or a rule refuses it, is told to
//! the commit, so the attempt's outcome names what became of the ID its
//! staging answered.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use goodfetch_protocol::foxhound::frontier::NotAddedReason;
use url::Url;

use crate::unwritten::Counts;

/// What an attempt's staged admissions did beside the run file: the
/// identities they visited, the requests they counted and the seeds and
/// sitemaps they added.
#[derive(Clone, Debug, Default)]
pub struct Staged {
    /// Each identity visited, with the request admitted under it.
    visited: BTreeMap<String, Added>,
    /// Each learned identity, with the rule that took its last parameter
    /// away and its crawl request.
    learned: BTreeMap<String, (i64, String)>,
    /// The requests admitted, counted against `crawl.max_requests`.
    admitted: u64,
    /// The requests admitted per path budget, by its pattern.
    budgets: BTreeMap<String, u64>,
    /// The seeds that widened the scope.
    seeds: Vec<Url>,
    /// The URLs of the sitemaps whose fetch was added.
    sitemaps: BTreeSet<String>,
    /// The candidates not added whose judgement wrote to the run file: a
    /// bound's refusal, counted, and a learned rule's merge or refusal,
    /// recorded.
    left: u64,
    /// What the attempt's earlier admissions counted that another
    /// admission added since, read once a bound nears in this call
    /// ([`Admitting::overtaken`]); never absorbed.
    overtaken: Option<Counts>,
}

/// A request an attempt's staged admission added.
#[derive(Clone, Debug)]
struct Added {
    /// Its crawl request.
    crawl_request: String,
    /// Its learned identity, with the rule that took its last parameter
    /// away, when the learned rules changed it.
    learned: Option<(String, i64)>,
    /// The pattern of the path budget that counted it.
    budget: Option<String>,
}

impl Staged {
    /// The Frontier identity of each request the admissions added, in
    /// their sort order, which a sample comparison weighs (ADR 0103).
    pub fn identities(&self) -> impl Iterator<Item = &str> + '_ {
        self.visited.keys().map(String::as_str)
    }

    /// Adds what `later` did after these admissions.
    pub fn absorb(&mut self, later: Self) {
        self.visited.extend(later.visited);
        self.learned.extend(later.learned);
        self.admitted += later.admitted;
        for (pattern, admitted) in later.budgets {
            *self.budgets.entry(pattern).or_default() += admitted;
        }
        self.seeds.extend(later.seeds);
        self.sitemaps.extend(later.sitemaps);
        self.left += later.left;
    }

    /// The crawl request `identity` was admitted for.
    pub(crate) fn visitor(&self, identity: &str) -> Option<&str> {
        self.visited
            .get(identity)
            .map(|added| added.crawl_request.as_str())
    }

    /// The rule and crawl request of the learned identity `identity`.
    pub(crate) fn learned(&self, identity: &str) -> Option<(i64, &str)> {
        self.learned
            .get(identity)
            .map(|(rule, crawl_request)| (*rule, crawl_request.as_str()))
    }

    /// The requests admitted, counted against `crawl.max_requests`.
    pub(crate) fn admitted(&self) -> u64 {
        self.admitted
    }

    /// The requests admitted against the path budget of `pattern`.
    pub(crate) fn budget(&self, pattern: &str) -> u64 {
        self.budgets.get(pattern).copied().unwrap_or_default()
    }

    /// The seeds that widened the scope.
    pub(crate) fn seeds(&self) -> &[Url] {
        &self.seeds
    }

    /// Whether a sitemap's fetch at `url` was added.
    pub(crate) fn holds_sitemap(&self, url: &str) -> bool {
        self.sitemaps.contains(url)
    }
}

/// What an admission runs against, and what it tells of itself beside the
/// run file.
#[derive(Debug, Default)]
pub enum Admitting {
    /// The run file alone: a Run's seeds, a seed added outside an attempt,
    /// a sitemap's entries.
    #[default]
    RunFile,
    /// An attempt's staging: the run file, which holds what this call did
    /// so far, and `earlier`, what the attempt's earlier calls did, while
    /// `now` keeps what this call does.
    Staging {
        /// What the attempt's earlier admissions did.
        earlier: Arc<Staged>,
        /// What this admission does.
        now: Staged,
    },
    /// An attempt's commit, which adds each request its staged calls
    /// answered `added`, in order, on the run file: each URL it does not
    /// add is kept in `refused`.
    Commit {
        /// Each URL not added, in order.
        refused: Vec<Refused>,
    },
}

/// A URL an admission at an attempt's commit did not add, with why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    /// The URL, as parsed.
    pub url: String,
    /// Why it was not added.
    pub reason: NotAddedReason,
    /// The crawl request it is a duplicate of, when the visited set holds
    /// its identity.
    pub duplicate_of: Option<String>,
}

/// The identities an attempt's staged admissions visited, its earlier
/// calls' and this call's own, while it stages. The Frontier's memory never
/// holds them, since a staging is rolled back, so an admission that finds
/// no visit there, or in the file, reads them here (#2681).
#[derive(Clone, Copy, Debug)]
pub(crate) struct StagedVisits<'a> {
    earlier: &'a Staged,
    now: &'a Staged,
}

impl<'a> StagedVisits<'a> {
    /// The crawl request `identity` was admitted for.
    pub(crate) fn visitor(self, identity: &str) -> Option<&'a str> {
        self.earlier
            .visitor(identity)
            .or_else(|| self.now.visitor(identity))
    }

    /// The rule and crawl request of the learned identity `identity`.
    pub(crate) fn learned(self, identity: &str) -> Option<(i64, &'a str)> {
        self.earlier
            .learned(identity)
            .or_else(|| self.now.learned(identity))
    }
}

impl Admitting {
    /// What the attempt's earlier admissions did, while it stages.
    pub(crate) fn earlier(&self) -> Option<&Staged> {
        match self {
            Self::Staging { earlier, .. } => Some(earlier),
            Self::RunFile | Self::Commit { .. } => None,
        }
    }

    /// The identities the attempt's staged admissions visited, its
    /// earlier calls' and this one's, while it stages.
    pub(crate) fn staged_visits(&self) -> Option<StagedVisits<'_>> {
        match self {
            Self::Staging { earlier, now } => Some(StagedVisits { earlier, now }),
            Self::RunFile | Self::Commit { .. } => None,
        }
    }

    /// The requests the attempt's staged admissions counted against
    /// `crawl.max_requests`, its earlier calls' and this one's, while it
    /// stages.
    pub(crate) fn staged_admitted(&self) -> u64 {
        match self {
            Self::Staging { earlier, now } => earlier.admitted + now.admitted,
            Self::RunFile | Self::Commit { .. } => 0,
        }
    }

    /// The requests the attempt's staged admissions counted against the
    /// path budget of `pattern`, its earlier calls' and this one's, while
    /// it stages.
    pub(crate) fn staged_budget(&self, pattern: &str) -> u64 {
        match self {
            Self::Staging { earlier, now } => earlier.budget(pattern) + now.budget(pattern),
            Self::RunFile | Self::Commit { .. } => 0,
        }
    }

    /// What the attempt's earlier staged admissions counted against the
    /// bounds that another admission added since, while it stages: the
    /// attempt's commit will not add those again, so they count once, as
    /// the run file counts them (#2169). `taken` answers whether another
    /// admission took a request's identities, its own and its learned one.
    /// Read once per call, on the first bound that nears, since nothing
    /// else admits on the writer while the call runs.
    ///
    /// # Errors
    ///
    /// `taken`'s.
    pub(crate) fn overtaken(
        &mut self,
        mut taken: impl FnMut(&str, Option<(&str, i64)>) -> rusqlite::Result<bool>,
    ) -> rusqlite::Result<Option<&Counts>> {
        let Self::Staging { earlier, now } = self else {
            return Ok(None);
        };
        if now.overtaken.is_none() {
            let mut overtaken = Counts::default();
            for (identity, added) in &earlier.visited {
                let learned = added
                    .learned
                    .as_ref()
                    .map(|(learned, rule)| (learned.as_str(), *rule));
                if taken(identity, learned)? {
                    overtaken.count(added.budget.as_deref());
                }
            }
            now.overtaken = Some(overtaken);
        }
        Ok(now.overtaken.as_ref())
    }

    /// Whether what it admits stays in the run file: a staging's is rolled
    /// back.
    pub(crate) const fn writes(&self) -> bool {
        !matches!(self, Self::Staging { .. })
    }

    /// Notes, while an attempt stages, that the request `crawl_request` was
    /// admitted under `identity`, its learned identity `learned` with its
    /// rule when it has one, counted against the path budget `budget`.
    pub(crate) fn admitted(
        &mut self,
        crawl_request: &str,
        identity: &str,
        learned: Option<(&str, i64)>,
        budget: Option<&str>,
    ) {
        if let Self::Staging { now, .. } = self {
            now.visited
                .entry(identity.to_owned())
                .or_insert_with(|| Added {
                    crawl_request: crawl_request.to_owned(),
                    learned: learned.map(|(learned, rule)| (learned.to_owned(), rule)),
                    budget: budget.map(str::to_owned),
                });
            if let Some((learned, rule)) = learned {
                now.learned
                    .entry(learned.to_owned())
                    .or_insert_with(|| (rule, crawl_request.to_owned()));
            }
            now.admitted += 1;
            if let Some(budget) = budget {
                *now.budgets.entry(budget.to_owned()).or_default() += 1;
            }
        }
    }

    /// Notes, while an attempt stages, that the judgement of a candidate
    /// not added wrote to the run file: a bound's refusal, counted, or a
    /// learned rule's merge or refusal, recorded. The staging rolls that
    /// write back, so the attempt's commit judges the candidate again
    /// (#2245).
    pub(crate) const fn leaves(&mut self) {
        if let Self::Staging { now, .. } = self {
            now.left += 1;
        }
    }

    /// How many candidates this admission left its attempt's commit to
    /// judge again, while it stages ([`Admitting::leaves`]): its caller
    /// reads it before and after a candidate to know whether it is one.
    #[must_use]
    pub const fn left(&self) -> u64 {
        match self {
            Self::Staging { now, .. } => now.left,
            Self::RunFile | Self::Commit { .. } => 0,
        }
    }

    /// Notes, while an attempt stages, that the seed `seed` widened the
    /// scope.
    pub(crate) fn widened(&mut self, seed: &Url) {
        if let Self::Staging { now, .. } = self {
            now.seeds.push(seed.clone());
        }
    }

    /// Notes, while an attempt stages, that a sitemap's fetch at `url` was
    /// added.
    pub(crate) fn sitemap_added(&mut self, url: &str) {
        if let Self::Staging { now, .. } = self {
            now.sitemaps.insert(url.to_owned());
        }
    }

    /// Notes, at an attempt's commit, that `url` was not added, with why.
    pub(crate) fn refused(
        &mut self,
        url: &Url,
        reason: NotAddedReason,
        duplicate_of: Option<&str>,
    ) {
        if let Self::Commit { refused } = self {
            refused.push(Refused {
                url: url.as_str().to_owned(),
                reason,
                duplicate_of: duplicate_of.map(str::to_owned),
            });
        }
    }
}
