//! A Run's admission invariants (#1896): the Scope, the Bounds with each
//! path budget's glob compiled, and each sitemap seed's rules compiled,
//! built once as the Run starts or resumes and handed to every admission,
//! so none reads them from the run file or compiles them again.
//!
//! The Scope's strategy and patterns, and the Bounds, are fixed for the
//! Run's life (ADR 0102): a resume records its new Bounds before it builds
//! these. What the Scope's seeds widen stays in the run file, where each
//! admission with no narrowing writes it in the same transaction, and a
//! check looks it up there by a URL's measures.

#![expect(
    clippy::disallowed_types,
    reason = "the sitemap rules a running crawl's seeds add, guarded for one lookup or insert inside a writer job, never held with another lock or across an await, so there is no lock order to test under loom"
)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::WriteWatch;
use goodfetch_page::url_pattern::{UnsupportedPattern, UrlPattern};
use goodfetch_protocol::foxhound::crawl::SitemapRule;
use goodfetch_protocol::foxhound::frontier::PathBudget;
use rusqlite::{Connection, OptionalExtension as _};
use url::{Position, Url};

use crate::behind::{Behind, Lesson, Watches};
use crate::bounds::Bounds;
use crate::learn::EndMerged;
use crate::rules;
use crate::scope::Scope;
use crate::unwritten::{SharedBudget, UnwrittenVisits};

/// What every admission of a Run measures a candidate against, built once.
pub struct Invariants {
    scope: Scope,
    bounds: Bounds,
    /// Each of `bounds.path_budgets`' patterns, compiled, in order.
    budgets: Vec<UrlPattern>,
    /// Each sitemap seed's rules, compiled, by their JSON as the run file
    /// keeps them with the seed's sitemaps.
    sitemap_rules: Mutex<HashMap<String, Arc<Rules>>>,
    /// The visits a batching writer has not committed, held once the Run
    /// batches its writes (#2168).
    unwritten: OnceLock<UnwrittenVisits>,
    /// The Run's counted work, which counts the evidence its admissions
    /// read and write (`F2.admission_evidence`), once the Run counts on it.
    work: OnceLock<CountedWork>,
    /// Learning's evidence, noted until a batch's end teaches it, once the
    /// Run learns behind (#2169).
    behind: OnceLock<Arc<Behind>>,
}

impl fmt::Debug for Invariants {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Invariants")
            .field("scope", &self.scope)
            .field("bounds", &self.bounds)
            .finish_non_exhaustive()
    }
}

impl Invariants {
    /// The invariants of `scope` and `bounds`, each path budget's pattern
    /// compiled.
    ///
    /// # Errors
    ///
    /// The first path budget's pattern the matcher cannot read, which the
    /// start raises as `pattern_unsupported`.
    pub fn new(scope: Scope, bounds: Bounds) -> Result<Self, UnsupportedPattern> {
        let budgets = bounds
            .path_budgets
            .iter()
            .map(|budget| UrlPattern::glob(&budget.pattern))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            scope,
            bounds,
            budgets,
            sitemap_rules: Mutex::new(HashMap::new()),
            unwritten: OnceLock::new(),
            work: OnceLock::new(),
            behind: OnceLock::new(),
        })
    }

    /// The invariants the run file `connection` holds, as a Run resumes:
    /// its Scope and Bounds, and the rules of every sitemap it keeps.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read, or holds a pattern the matcher
    /// cannot read.
    pub fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let unreadable =
            |error: UnsupportedPattern| rusqlite::Error::ToSqlConversionFailure(Box::new(error));
        let invariants =
            Self::new(Scope::read(connection)?, Bounds::read(connection)?).map_err(unreadable)?;
        for rules in kept_rules(connection)? {
            if let Ok(given) = serde_json::from_str::<Vec<SitemapRule>>(&rules)
                && let Ok(compiled) = Rules::of(&given)
            {
                invariants.known().insert(rules, Arc::new(compiled));
            }
        }
        Ok(invariants)
    }

    /// The Run's Scope.
    #[must_use]
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The Run's Bounds.
    #[must_use]
    pub const fn bounds(&self) -> &Bounds {
        &self.bounds
    }

    /// Holds, from now on, the visits the Run's writer has not committed
    /// (#2168), against `committed`, how many commits it acknowledged, and
    /// keeps those it wrote within `budget`, less what the Run state's
    /// pending work items hold (#2169), measured on `metrics`: a Run whose
    /// writer batches its writes holds them, so a redirect hop is judged on
    /// a reader. Once is enough; a second call changes nothing.
    pub fn hold_unwritten(
        &self,
        budget: SharedBudget,
        committed: impl Fn() -> u64 + Send + Sync + 'static,
        metrics: &Metrics,
    ) -> &UnwrittenVisits {
        self.unwritten
            .get_or_init(|| UnwrittenVisits::new(budget, committed, metrics))
    }

    /// Counts, from now on, the evidence the Run's admissions read and
    /// write on `work`, the Run's counted work (`F2.admission_evidence`,
    /// ADR 0140). Once is enough; a second call changes nothing.
    pub fn count_on(&self, work: CountedWork) {
        drop(self.work.set(work));
    }

    /// The Run's counted work, once the Run counts on it
    /// ([`Self::count_on`]).
    pub(crate) fn work(&self) -> Option<&CountedWork> {
        self.work.get()
    }

    /// Learns behind from now on (#2169): neither a page's lesson nor an
    /// admission's evidence is taught on its page's path, but noted, and
    /// taught by [`Self::teach_behind`] at the end of each batch the
    /// Run's writer commits, which tells [`Self::watch`] of its jobs. Once
    /// is enough; a second call changes nothing.
    pub fn learn_behind(&self) {
        let _behind = self.behind.get_or_init(Arc::default);
    }

    /// Notes `lesson`, a fetched page's, for the end of the writer's next
    /// batch to teach, once the Run learns behind ([`Self::learn_behind`]).
    pub fn note(&self, lesson: Lesson) {
        if let Some(behind) = self.behind.get() {
            behind.lesson(lesson);
        } else {
            tracing::warn!(
                "a page's lesson was noted in a Run that does not learn behind, which teaches it nowhere"
            );
        }
    }

    /// Whether evidence noted behind waits for a batch's end to teach it.
    #[must_use]
    pub fn waiting(&self) -> bool {
        self.behind.get().is_some_and(|behind| behind.waiting())
    }

    /// Teaches, in `connection`'s transaction, the writer's, at the end of
    /// a batch, every piece of evidence noted behind, in the order noted,
    /// handing `pending` the requests a rule it teaches merges; `work`
    /// counts the rows of evidence it reads and writes. What it took is
    /// no longer noted, whether it succeeds or not.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read or written: the caller rolls back
    /// what it taught.
    pub fn teach_behind(
        &self,
        connection: &Connection,
        pending: &mut dyn EndMerged,
        work: &CountedWork,
    ) -> rusqlite::Result<()> {
        self.behind
            .get()
            .map_or(Ok(()), |behind| behind.teach(connection, pending, work))
    }

    /// The watch the run-state writer tells as each of its jobs begins and
    /// when one fails: of the evidence noted behind, a failed job's taken
    /// back, and of the visits the writer has not committed, once the Run
    /// learns behind ([`Self::learn_behind`]) or holds them
    /// ([`Self::hold_unwritten`]). None before either.
    #[must_use]
    pub fn watch(&self) -> Option<Arc<dyn WriteWatch>> {
        let watches: Vec<Arc<dyn WriteWatch>> = self
            .behind
            .get()
            .map(|behind| Arc::clone(behind) as Arc<dyn WriteWatch>)
            .into_iter()
            .chain(self.unwritten().map(UnwrittenVisits::watch))
            .collect();
        (!watches.is_empty()).then(|| Arc::new(Watches(watches)) as Arc<dyn WriteWatch>)
    }

    /// Learns what the admission of the URL whose key is `key`, a link of
    /// the page whose key is `from`, teaches, on `connection`: noted for a
    /// batch's end when the Run learns behind and the admission `writes`,
    /// nothing when it does not, as its writes are rolled back; taught now
    /// otherwise, counted as a lesson on a page's path
    /// (`F2.page_learning`).
    ///
    /// # Errors
    ///
    /// When the run file cannot be read or written.
    pub(crate) fn admission_evidence(
        &self,
        connection: &Connection,
        (key, from): (&Url, Option<Url>),
        writes: bool,
    ) -> rusqlite::Result<()> {
        if let Some(behind) = self.behind.get() {
            if writes {
                behind.admitted(key.clone(), from);
            }
            return Ok(());
        }
        if let Some(work) = self.work() {
            work.count(Row::PageLearning);
        }
        rules::admitted(connection, key, from.as_ref(), self.work())
    }

    /// The visits the Run's writer has not committed, once the Run holds
    /// them ([`Self::hold_unwritten`]).
    #[must_use]
    pub fn unwritten(&self) -> Option<&UnwrittenVisits> {
        self.unwritten.get()
    }

    /// Has the Frontier's memory, when the Run holds the batches' visits
    /// ([`Self::hold_unwritten`]), read the Scope's seeds and the visits
    /// the run file `connection` holds, a reader's, as the crawl opens,
    /// within the memory budget: a lookup it then answers reads nothing
    /// (#2169).
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn fill_memory(&self, connection: &Connection) -> rusqlite::Result<()> {
        self.unwritten().map_or(Ok(()), |unwritten| {
            unwritten.read(connection, self.scope.strategy().seed_keys())
        })
    }

    /// The first path budget whose pattern the path and query of `url`
    /// match: the one that counts it.
    pub(crate) fn budget_of(&self, url: &Url) -> Option<&PathBudget> {
        self.budget_at(url).and_then(|at| self.budget(at))
    }

    /// Where, among the Run's path budgets, the one that counts `url` is:
    /// the first whose pattern its path and query match. Read from the URL
    /// alone, on its page's task (#2244).
    pub(crate) fn budget_at(&self, url: &Url) -> Option<usize> {
        let path_and_query = &url[Position::BeforePath..Position::AfterQuery];
        self.budgets
            .iter()
            .position(|pattern| pattern.matches(path_and_query))
    }

    /// The path budget at `at` among the Run's ([`Self::budget_at`]).
    pub(crate) fn budget(&self, at: usize) -> Option<&PathBudget> {
        self.bounds.path_budgets.get(at)
    }

    /// Keeps `rules`, a sitemap seed's, compiled unless they are kept, for
    /// every read of its sitemaps.
    ///
    /// # Errors
    ///
    /// The first pattern the matcher cannot read, which the call that gave
    /// it raises as `pattern_unsupported`.
    pub fn know_sitemap_rules(&self, rules: &[SitemapRule]) -> Result<(), UnsupportedPattern> {
        let json = serde_json::to_string(rules).expect("invariant: rules encode as JSON");
        if self.known().contains_key(&json) {
            return Ok(());
        }
        let compiled = Arc::new(Rules::of(rules)?);
        self.known().insert(json, compiled);
        Ok(())
    }

    /// The rules `given`, a sitemap's as the run file keeps them, compiled:
    /// those kept, or compiled now and counted on `F3.compilations.sitemap`
    /// (ADR 0140) when the seed that gave them was never known.
    ///
    /// # Errors
    ///
    /// When `given` is no rules' JSON.
    pub(crate) fn sitemap_rules(
        &self,
        given: &str,
        work: &CountedWork,
    ) -> rusqlite::Result<Arc<Rules>> {
        if let Some(rules) = self.known().get(given) {
            return Ok(Arc::clone(rules));
        }
        let rules: Vec<SitemapRule> = serde_json::from_str(given).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        for _ in &rules {
            work.count(Row::SitemapCompilations);
        }
        let compiled = Arc::new(Rules::of(&rules).unwrap_or_else(|_| Rules::none()));
        self.known().insert(given.to_owned(), Arc::clone(&compiled));
        Ok(compiled)
    }

    fn known(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Rules>>> {
        self.sitemap_rules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// A sitemap seed's rules, compiled: each a URL pattern and the Label it
/// gives the entries it matches.
#[derive(Clone, Debug)]
pub(crate) struct Rules {
    compiled: Vec<(UrlPattern, String)>,
}

impl Rules {
    /// `rules`, compiled.
    ///
    /// # Errors
    ///
    /// The first pattern the matcher cannot read, which the call that gave
    /// it raises as `pattern_unsupported`.
    pub(crate) fn of(rules: &[SitemapRule]) -> Result<Self, UnsupportedPattern> {
        let compiled = rules
            .iter()
            .map(|rule| Ok((UrlPattern::glob(&rule.pattern)?, rule.label.clone())))
            .collect::<Result<_, UnsupportedPattern>>()?;
        Ok(Self { compiled })
    }

    /// No rule: every entry is `no_rule`.
    pub(crate) const fn none() -> Self {
        Self {
            compiled: Vec::new(),
        }
    }

    /// The Label of the first rule whose pattern matches `url`, if any.
    pub(crate) fn label_of(&self, url: &Url) -> Option<&str> {
        self.compiled
            .iter()
            .find(|(pattern, _)| pattern.matches(url.as_str()))
            .map(|(_, label)| label.as_str())
    }
}

/// The rules of every sitemap the run file `connection` holds, as their
/// seeds gave them, each once; none in a run file with no sitemap table.
///
/// # Errors
///
/// When the run file cannot be read.
fn kept_rules(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let kept = connection
        .prepare("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'sitemaps'")?
        .query_row([], |_| Ok(()))
        .optional()?
        .is_some();
    if !kept {
        return Ok(Vec::new());
    }
    connection
        .prepare("SELECT DISTINCT rules FROM sitemaps")?
        .query_map([], |row| row.get(0))?
        .collect()
}
