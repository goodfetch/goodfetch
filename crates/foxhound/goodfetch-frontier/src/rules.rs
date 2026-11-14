//! The Frontier's learned rules (ADR 0005; spec #581's thresholds): what
//! the pages a crawl fetched and the links they gave teach it, each rule on
//! one host, so never across a registrable domain, kept in the run file
//! with its evidence and every URL it merged.
//!
//! - **A parameter with no effect** is learned for a host once 3 pairs of
//!   pages whose URLs differ only by that query parameter have matching
//!   page signatures and no pair has differing ones; a URL is then keyed
//!   without it.
//! - **A trap** is a path pattern, the path with its numeric, date-like
//!   and hexadecimal segments generalized, closed once 100 pages under it
//!   in a row brought no new page signature on the host and no URL outside
//!   it; its later URLs are refused, which leaves the Run `unverified`.
//! - **Offset pagination** is a query parameter whose values, on one path,
//!   step by a fixed page size of 2 or more over at least 3 values; once
//!   three pages of that listing each link a cursor (a URL of the same
//!   path without the offset, with a parameter that is no offset, whose
//!   one value is each page's own), the cursor is preferred and later
//!   offset URLs are `duplicate`, counted, which leaves the Run
//!   `unverified`. A parameter one page links with two values, or two
//!   pages with one, is a sort, a filter or a view, and never a cursor. A
//!   listing paged only by its offset leaves the Run `unverified` too.
//! - **An equivalent form** (`forms`: `www.`, the scheme, an index file or
//!   a trailing slash) is learned for a host on 1 redirect from one form
//!   to the other, or 2 pairs of pages in the two forms with matching
//!   signatures and none differing, or 3 pages whose `rel=canonical`
//!   differs from their URL only by that form (ADR 0120); a canonical to
//!   another path or another registrable domain is no evidence. A URL is
//!   then keyed in the form the rule keeps.
//! - **A contradicting pair** (pages fetched in both spellings, neither a
//!   redirect, of differing signatures) keeps a parameter or a form from
//!   being learned, and stops a learned one: from then on it merges
//!   nothing more, and the report gives the pair beside the URLs it had
//!   merged. The requests it had ended are given back to the crawl; what
//!   the crawl does not get back, a request it could not make pending
//!   again or a URL the rule refused as a `duplicate` that was not added
//!   since, leaves the Run `unverified`, counted per rule.
//!
//! - **Evidence is kept while a decision needs it** (ADR 0005's note of
//!   2026-11-10). A parameter or a form that is refused, a pair of its
//!   pages having differed before it was learned, or stopped takes no
//!   pair, no residue, no canonical and no read: nothing changes it (ADR
//!   0120's note of 2026-10-24). A learned one takes no more matching
//!   pairs, only the differing one that stops it, and still leaves its
//!   residues, which that pair is read from. An offset parameter
//!   whose values step by 1 takes no more values, and one that pages keeps
//!   its three lowest. Each page still leaves its own row and its
//!   signature: a later URL with one parameter more or in another form is
//!   paired with the first, and a trap's count asks whether the second is
//!   new on the host.
//!
//! The rules read a URL by its key (`identity`'s fixed rules), so its query
//! pairs are already normalized and sorted by name. Nothing is held in
//! memory between calls (STYLE.md: the Frontier keeps its state in the run
//! file).
//!
//! Size: one concern, the evidence the learned rules keep in the run file
//! and what it teaches; every rule reads the same pages, pairs and rule
//! rows, so their thresholds stay side by side.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::runs::CompletenessReason;
use goodfetch_protocol::foxhound::frontier::NotAddedReason;
use goodfetch_protocol::foxhound::report::{
    CURSOR_PAGINATION as CURSOR_PAGINATION_REASON, EquivalentForm, LEARNED_RULE_STOPPED,
    LearnedRule, LearnedRuleKind, OFFSET_PAGINATION, TRAP_PATH as TRAP_PATH_REASON,
};
use goodfetch_protocol::run::{AxisReason, Completeness};
use rusqlite::{CachedStatement, Connection, OptionalExtension as _, Params};
use url::Url;

use crate::forms;

/// The rules learned, each with the URLs it merged, a URL it made a
/// `duplicate` at admission with its identity, and the crawl requests each
/// merging rule's merges ended; and the evidence they are learned from: the pages fetched with their signatures, by their
/// query without each of their parameters, the pairs those make, the
/// signatures seen per host, each path pattern's run of pages that brought
/// nothing new, each path's integer parameter values, and the first link
/// an offset listing's pages gave under each other parameter.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-rules",
    statements: &[
        "CREATE TABLE IF NOT EXISTS learned_rules (
            id INTEGER NOT NULL PRIMARY KEY,
            kind TEXT NOT NULL,
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            form TEXT NOT NULL,
            evidence TEXT NOT NULL,
            contradiction TEXT,
            UNIQUE (kind, host, path, parameter, form)
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS learned_merges (
            rule INTEGER NOT NULL REFERENCES learned_rules (id),
            url TEXT NOT NULL,
            identity TEXT,
            UNIQUE (rule, url)
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS rules_ended (
            crawl_request TEXT NOT NULL,
            rule INTEGER NOT NULL REFERENCES learned_rules (id),
            PRIMARY KEY (crawl_request, rule)
        ) STRICT, WITHOUT ROWID",
        "CREATE INDEX IF NOT EXISTS rules_ended_by_rule ON rules_ended (rule)",
        "CREATE TABLE IF NOT EXISTS rules_pages (
            crawl_request TEXT NOT NULL PRIMARY KEY,
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            query TEXT NOT NULL,
            url TEXT NOT NULL,
            signature BLOB NOT NULL
        ) STRICT, WITHOUT ROWID",
        "CREATE INDEX IF NOT EXISTS rules_pages_by_query ON rules_pages (host, path, query)",
        "CREATE INDEX IF NOT EXISTS rules_pages_by_url ON rules_pages (url)",
        "CREATE TABLE IF NOT EXISTS rules_residues (
            crawl_request TEXT NOT NULL,
            parameter TEXT NOT NULL,
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            residue TEXT NOT NULL,
            url TEXT NOT NULL,
            signature BLOB NOT NULL,
            PRIMARY KEY (crawl_request, parameter)
        ) STRICT, WITHOUT ROWID",
        "CREATE INDEX IF NOT EXISTS rules_residues_by_residue
            ON rules_residues (host, path, residue, parameter)",
        "CREATE TABLE IF NOT EXISTS rules_pairs (
            host TEXT NOT NULL,
            parameter TEXT NOT NULL,
            form TEXT NOT NULL,
            one TEXT NOT NULL,
            other TEXT NOT NULL,
            matching INTEGER NOT NULL,
            UNIQUE (host, parameter, form, one, other)
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS rules_canonicals (
            host TEXT NOT NULL,
            form TEXT NOT NULL,
            url TEXT NOT NULL,
            canonical TEXT NOT NULL,
            PRIMARY KEY (host, form, url)
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS rules_signatures (
            host TEXT NOT NULL,
            signature BLOB NOT NULL,
            PRIMARY KEY (host, signature)
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS rules_patterns (
            host TEXT NOT NULL,
            pattern TEXT NOT NULL,
            streak INTEGER NOT NULL,
            first TEXT,
            PRIMARY KEY (host, pattern)
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS rules_offset_values (
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            value INTEGER NOT NULL,
            url TEXT NOT NULL,
            PRIMARY KEY (host, path, parameter, value)
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS rules_offsets (
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            anchor INTEGER NOT NULL,
            step INTEGER NOT NULL,
            low INTEGER NOT NULL,
            high INTEGER NOT NULL,
            count INTEGER NOT NULL,
            paged INTEGER NOT NULL,
            PRIMARY KEY (host, path, parameter)
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS rules_cursors (
            host TEXT NOT NULL,
            path TEXT NOT NULL,
            parameter TEXT NOT NULL,
            value TEXT NOT NULL,
            page TEXT NOT NULL,
            url TEXT NOT NULL,
            second_value TEXT,
            second_page TEXT,
            second_url TEXT,
            refuted INTEGER NOT NULL,
            PRIMARY KEY (host, path, parameter)
        ) STRICT, WITHOUT ROWID",
    ],
};

/// How many pairs of matching pages teach a parameter with no effect.
const MATCHING_PAIRS: i64 = 3;

/// How many pages in a row under a path pattern that bring nothing new
/// close it as a trap.
const TRAP_PAGES: i64 = 100;

/// How many values of a parameter, on one path, show its fixed step.
const OFFSET_VALUES: i64 = 3;

/// The smallest step of an offset: a page size; page numbers step by 1.
const OFFSET_STEP: i64 = 2;

/// How many pairs of matching pages in two forms teach that they are one.
const MATCHING_FORM_PAIRS: i64 = 2;

/// How many pages whose canonical differs from their URL only by a form
/// teach that form.
const CANONICAL_PAGES: i64 = 3;

// The kinds' names as the run file writes them: the protocol's, which
// [`learned_rules`] reads them back by, so a name that drifts from the
// protocol's fails to read rather than name another kind.
const NO_EFFECT_PARAMETER: &str = "no_effect_parameter";
const TRAP_PATH: &str = "trap_path";
const CURSOR_PAGINATION: &str = "cursor_pagination";
const EQUIVALENT_FORM: &str = "equivalent_form";

/// What a page or a redirect changed of the rules that merge URLs: a
/// parameter or a form learned, or one a contradicting pair stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    /// The rule `rule` was learned and merges from now on.
    Learned(Merging),
    /// The rule `rule` met a contradicting pair and merges no more.
    Contradicted(Merging),
}

/// A rule that merges URLs, and the hosts whose URLs it reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Merging {
    /// The rule's ID.
    pub(crate) rule: i64,
    /// The hosts: its own, and for a `www` form the `www.` host too.
    pub(crate) hosts: Vec<String>,
}

/// What a merging rule is about: a query parameter, or a form.
#[derive(Clone, Copy, Debug)]
enum Subject<'a> {
    Parameter(&'a str),
    Form(EquivalentForm),
}

impl Subject<'_> {
    /// Its kind, parameter and form, as the run file writes them.
    fn columns(self) -> (&'static str, String, &'static str) {
        match self {
            Self::Parameter(parameter) => (NO_EFFECT_PARAMETER, parameter.to_owned(), ""),
            Self::Form(form) => (EQUIVALENT_FORM, String::new(), forms::name(form)),
        }
    }

    /// How many matching pairs teach it.
    fn threshold(self) -> i64 {
        match self {
            Self::Parameter(_) => MATCHING_PAIRS,
            Self::Form(_) => MATCHING_FORM_PAIRS,
        }
    }

    /// What two URLs differ by, in its evidence.
    fn difference(self) -> String {
        match self {
            Self::Parameter(parameter) => format!("`{parameter}`"),
            Self::Form(form) => described(form).to_owned(),
        }
    }

    /// The hosts its rule on `host` reads.
    fn hosts(self, host: &str) -> Vec<String> {
        match self {
            Self::Form(EquivalentForm::Www) => vec![host.to_owned(), format!("www.{host}")],
            _ => vec![host.to_owned()],
        }
    }
}

/// Where the rule about a parameter or a form stands on a host, which says
/// what evidence it still takes (ADR 0005's note of 2026-11-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standing {
    /// Not learned, and no pair of its pages differed: it takes every
    /// pair, up to the threshold that teaches it.
    Undecided,
    /// Learned and never contradicted: a matching pair changes nothing, so
    /// it takes only a differing one, which stops it.
    Learned,
    /// Refused, a pair of its pages having differed before it was learned,
    /// or stopped: no evidence changes it, so it takes none (ADR 0120's
    /// note of 2026-10-24).
    Settled,
}

impl Standing {
    /// Whether it takes a pair of pages, `matching` or differing.
    fn takes(self, matching: bool) -> bool {
        match self {
            Self::Undecided => true,
            Self::Learned => !matching,
            Self::Settled => false,
        }
    }
}

/// A form, as evidence names it.
fn described(form: EquivalentForm) -> &'static str {
    match form {
        EquivalentForm::Www => "`www.`",
        EquivalentForm::Scheme => "the scheme",
        EquivalentForm::TrailingSlash => "a trailing slash",
        EquivalentForm::IndexHtml => "`index.html`",
        EquivalentForm::IndexPhp => "`index.php`",
    }
}

/// The parts of a URL's key the rules read.
struct Parts<'a> {
    host: &'a str,
    path: &'a str,
    /// Its query pairs, as its key sorts them.
    pairs: Vec<&'a str>,
}

impl<'a> Parts<'a> {
    fn of(key: &'a Url) -> Self {
        Self {
            host: key.host_str().unwrap_or_default(),
            path: key.path(),
            pairs: key
                .query()
                .map(|query| query.split('&').filter(|pair| !pair.is_empty()).collect())
                .unwrap_or_default(),
        }
    }

    /// The names of its query parameters, each once, in order.
    fn names(&self) -> Vec<&'a str> {
        let mut names: Vec<&str> = self.pairs.iter().map(|pair| name(pair)).collect();
        names.dedup();
        names
    }

    /// Its query without the pairs of `parameter`; all of it for `None`.
    fn query_without(&self, parameter: Option<&str>) -> String {
        self.pairs
            .iter()
            .filter(|pair| Some(name(pair)) != parameter)
            .copied()
            .collect::<Vec<_>>()
            .join("&")
    }

    fn has(&self, parameter: &str) -> bool {
        self.pairs.iter().any(|pair| name(pair) == parameter)
    }
}

/// The name of a query pair: what precedes its first `=`.
fn name(pair: &str) -> &str {
    pair.split_once('=').map_or(pair, |(name, _)| name)
}

/// The value of a query pair: what follows its first `=`.
fn value(pair: &str) -> &str {
    pair.split_once('=').map_or("", |(_, value)| value)
}

/// `key` under the rules that merge on its host and still hold: without
/// the parameters learned to have no effect, then in the spelling each
/// learned form keeps; and the rules that changed it.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn without_learned(
    connection: &Connection,
    key: &Url,
) -> rusqlite::Result<(Url, Vec<i64>)> {
    without_learned_by(&mut OnRunFile(connection), key)
}

/// The rules that merge on each host and still hold, read once per host
/// from the run file and then kept: for one event that re-keys a host's
/// visited identities, a rule learned or stopped, so each identity is
/// keyed in memory rather than by queries of its own.
pub(crate) struct Holding<'c> {
    connection: &'c Connection,
    hosts: HashMap<String, HostRules>,
}

/// The rules that merge on one host and still hold: its parameters with
/// no effect, in the order of their IDs, and its forms, by name.
struct HostRules {
    parameters: Vec<(i64, String)>,
    forms: Vec<(String, i64)>,
}

impl<'c> Holding<'c> {
    /// The rules of the run file `connection` holds, none read yet.
    pub(crate) fn of(connection: &'c Connection) -> Self {
        Self {
            connection,
            hosts: HashMap::new(),
        }
    }

    /// `key` under the rules that merge on its host and still hold, as
    /// [`without_learned`] keys it.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub(crate) fn without_learned(&mut self, key: &Url) -> rusqlite::Result<(Url, Vec<i64>)> {
        without_learned_by(self, key)
    }

    fn host(&mut self, host: &str) -> rusqlite::Result<&HostRules> {
        if !self.hosts.contains_key(host) {
            let parameters = no_effect_parameters(self.connection, host)?;
            let mut statement = self.connection.prepare_cached(
                "SELECT form, id FROM learned_rules WHERE kind = ?1 AND host = ?2 AND path = ''
                 AND parameter = '' AND contradiction IS NULL",
            )?;
            let forms = statement
                .query_map((EQUIVALENT_FORM, host), |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            self.hosts
                .insert(host.to_owned(), HostRules { parameters, forms });
        }
        Ok(&self.hosts[host])
    }
}

/// Where [`without_learned_by`] reads the rules that hold on a host.
trait Lookup {
    /// What `read` answers of the parameters learned to have no effect on
    /// `host`, each with its rule, in the order of their IDs.
    fn with_parameters<T>(
        &mut self,
        host: &str,
        read: impl FnOnce(&[(i64, String)]) -> T,
    ) -> rusqlite::Result<T>;

    /// The rule about `form` on `host`, while it holds.
    fn form(&mut self, host: &str, form: EquivalentForm) -> rusqlite::Result<Option<i64>>;
}

/// The rules as the run file holds them, read at each lookup.
struct OnRunFile<'c>(&'c Connection);

impl Lookup for OnRunFile<'_> {
    fn with_parameters<T>(
        &mut self,
        host: &str,
        read: impl FnOnce(&[(i64, String)]) -> T,
    ) -> rusqlite::Result<T> {
        Ok(read(&no_effect_parameters(self.0, host)?))
    }

    fn form(&mut self, host: &str, form: EquivalentForm) -> rusqlite::Result<Option<i64>> {
        holding(self.0, host, Subject::Form(form))
    }
}

impl Lookup for Holding<'_> {
    fn with_parameters<T>(
        &mut self,
        host: &str,
        read: impl FnOnce(&[(i64, String)]) -> T,
    ) -> rusqlite::Result<T> {
        Ok(read(&self.host(host)?.parameters))
    }

    fn form(&mut self, host: &str, form: EquivalentForm) -> rusqlite::Result<Option<i64>> {
        let name = forms::name(form);
        Ok(self
            .host(host)?
            .forms
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, rule)| *rule))
    }
}

/// `key` under the rules `rules` holds, as [`without_learned`] keys it.
fn without_learned_by(rules: &mut impl Lookup, key: &Url) -> rusqlite::Result<(Url, Vec<i64>)> {
    let (mut merged, mut applied) = without_parameters(rules, key)?;
    for form in forms::FORMS {
        let Some(kept) = forms::kept(form, &merged) else {
            continue;
        };
        // A rule learned on the URL's own host still holds once a `www`
        // form took that host's `www.` away.
        let hosts = [forms::rule_host(form, &merged), forms::rule_host(form, key)];
        for host in hosts.iter().flatten() {
            if let Some(rule) = rules.form(host, form)? {
                applied.push(rule);
                merged = kept;
                break;
            }
        }
    }
    Ok((merged, applied))
}

/// The statement [`holding`] runs, which [`crate::admission_statements`]
/// names for a Run to prepare as it starts.
pub(crate) const HOLDING: &str =
    "SELECT id FROM learned_rules WHERE kind = ?1 AND host = ?2 AND path = ''
             AND parameter = ?3 AND form = ?4 AND contradiction IS NULL";

/// The ID of the rule about `subject` on `host`, while it holds: learned
/// and never contradicted.
fn holding(
    connection: &Connection,
    host: &str,
    subject: Subject<'_>,
) -> rusqlite::Result<Option<i64>> {
    let (kind, parameter, form) = subject.columns();
    connection
        .prepare_cached(HOLDING)?
        .query_row((kind, host, parameter, form), |row| row.get(0))
        .optional()
}

/// `key` without the parameters learned to have no effect on its host,
/// and the rules that took one away.
fn without_parameters(rules: &mut impl Lookup, key: &Url) -> rusqlite::Result<(Url, Vec<i64>)> {
    let parts = Parts::of(key);
    if parts.pairs.is_empty() {
        return Ok((key.clone(), Vec::new()));
    }
    let applied: Vec<(i64, String)> = rules.with_parameters(parts.host, |learned| {
        learned
            .iter()
            .filter(|(_, parameter)| parts.has(parameter))
            .cloned()
            .collect()
    })?;
    if applied.is_empty() {
        return Ok((key.clone(), Vec::new()));
    }
    let kept: Vec<&str> = parts
        .pairs
        .iter()
        .filter(|pair| !applied.iter().any(|(_, parameter)| name(pair) == parameter))
        .copied()
        .collect();
    let mut stripped = key.clone();
    if kept.is_empty() {
        stripped.set_query(None);
    } else {
        stripped.set_query(Some(&kept.join("&")));
    }
    Ok((
        stripped,
        applied.into_iter().map(|(rule, _)| rule).collect(),
    ))
}

/// The parameters learned to have no effect on `host` that still hold,
/// each with its rule, in the order of their IDs.
fn no_effect_parameters(
    connection: &Connection,
    host: &str,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut statement = connection.prepare_cached(
        "SELECT id, parameter FROM learned_rules
         WHERE kind = ?1 AND host = ?2 AND contradiction IS NULL",
    )?;
    Ok(by_id(
        statement
            .query_map((NO_EFFECT_PARAMETER, host), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    ))
}

/// `rules`, each an ID and its parameter, in the order of their IDs. The
/// queries that read them on every admission sort here rather than in
/// SQL: no index of `learned_rules` gives them in that order, so an
/// `ORDER BY id` started SQLite's sorter, and its 4 KiB buffer, for each.
fn by_id(mut rules: Vec<(i64, String)>) -> Vec<(i64, String)> {
    rules.sort_unstable_by_key(|(id, _)| *id);
    rules
}

/// The learned rule that refuses `key`, if one does, with its reason: a
/// trap closed over its path pattern refuses it with `trap_path`, and a
/// cursor learned for its listing refuses an offset URL as a `duplicate`,
/// unless the Run's admissions do not learn (`learning`,
/// `frontier.learning`).
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn refuses(
    connection: &Connection,
    key: &Url,
    learning: bool,
) -> rusqlite::Result<Option<(NotAddedReason, i64)>> {
    let parts = Parts::of(key);
    let trap = connection
        .prepare_cached("SELECT id FROM learned_rules WHERE kind = ?1 AND host = ?2 AND path = ?3")?
        .query_row((TRAP_PATH, parts.host, pattern(parts.path)), |row| {
            row.get::<_, i64>(0)
        })
        .optional()?;
    if let Some(rule) = trap {
        return Ok(Some((NotAddedReason::TrapPath, rule)));
    }
    if !learning || parts.pairs.is_empty() {
        return Ok(None);
    }
    let mut statement = connection.prepare_cached(
        "SELECT id, parameter FROM learned_rules
         WHERE kind = ?1 AND host = ?2 AND path = ?3",
    )?;
    let cursors = by_id(
        statement
            .query_map((CURSOR_PAGINATION, parts.host, parts.path), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    );
    Ok(cursors
        .into_iter()
        .find(|(_, offset)| parts.has(offset))
        .map(|(rule, _)| (NotAddedReason::Duplicate, rule)))
}

/// Records that the rule `rule` merged or refused `url`; with `identity`,
/// the URL's own, when the rule made it a `duplicate` at admission, so no
/// crawl request holds it: the visited set then says whether the crawl
/// added it since.
///
/// # Errors
///
/// When the run file cannot be written.
pub(crate) fn merged(
    connection: &Connection,
    rule: i64,
    url: &str,
    identity: Option<&str>,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO learned_merges (rule, url, identity) VALUES (?1, ?2, ?3)
             ON CONFLICT DO NOTHING",
        )?
        .execute((rule, url, identity))?;
    Ok(())
}

/// Records that a merge under `rules`, each of which it relied on, ended
/// the pending crawl request `crawl_request`.
///
/// # Errors
///
/// When the run file cannot be written.
pub(crate) fn ended(
    connection: &Connection,
    rules: &BTreeSet<i64>,
    crawl_request: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(
        "INSERT INTO rules_ended (crawl_request, rule) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
    )?;
    for rule in rules {
        statement.execute((crawl_request, rule))?;
    }
    Ok(())
}

/// The crawl requests that merges relying on the rule `rule` ended and
/// the crawl has not been given back, in the order of their IDs' text.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn ended_by(connection: &Connection, rule: i64) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection
        .prepare_cached("SELECT crawl_request FROM rules_ended WHERE rule = ?1 ORDER BY 1")?;
    statement
        .query_map([rule], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()
}

/// Records that the crawl made `crawl_request` pending again: no rule's
/// merge holds it ended any more.
///
/// # Errors
///
/// When the run file cannot be written.
pub(crate) fn given_back(connection: &Connection, crawl_request: &str) -> rusqlite::Result<()> {
    connection
        .prepare_cached("DELETE FROM rules_ended WHERE crawl_request = ?1")?
        .execute([crawl_request])?;
    Ok(())
}

/// The statement [`admitted`] ends a path pattern's streak with, which
/// [`crate::admission_statements`] has the Run prepare as it starts.
pub(crate) const STREAK_ENDED: &str =
    "UPDATE rules_patterns SET streak = 0, first = NULL WHERE host = ?1 AND pattern = ?2";

/// The statement [`offset_value`] reads what a parameter's values on a
/// path showed with, which [`crate::admission_statements`] has the Run prepare as
/// it starts.
pub(crate) const OFFSET_SEEN: &str =
    "SELECT anchor, step, low, high, count, paged FROM rules_offsets
     WHERE host = ?1 AND path = ?2 AND parameter = ?3";

/// The statement [`cursor`] reads a path's offset parameters with, which
/// [`crate::admission_statements`] has the Run prepare as it starts.
pub(crate) const PAGED_OFFSETS: &str = "SELECT parameter, step FROM rules_offsets
     WHERE host = ?1 AND path = ?2 AND paged = 1 ORDER BY parameter";

/// What the admission of the URL whose key is `key`, a link of the page
/// whose key is `from` when an attempt enqueued it, teaches: its integer
/// parameter values, which show an offset; a cursor link on an offset
/// listing; and, for a URL outside its page's path pattern, that the
/// pattern's page brought something new.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub(crate) fn admitted(
    connection: &Connection,
    key: &Url,
    from: Option<&Url>,
) -> rusqlite::Result<()> {
    let parts = Parts::of(key);
    for pair in &parts.pairs {
        if let Some(number) = integer(value(pair)) {
            offset_value(connection, &parts, name(pair), number, key)?;
        }
    }
    let Some(from) = from else {
        return Ok(());
    };
    let parent = Parts::of(from);
    if parent.host != parts.host {
        return Ok(());
    }
    if parent.path == parts.path {
        cursor(connection, &parts, from, key)?;
    }
    let from_pattern = pattern(parent.path);
    if from_pattern != pattern(parts.path) {
        connection
            .prepare_cached(STREAK_ENDED)?
            .execute((parent.host, from_pattern))?;
    }
    Ok(())
}

/// A query value read as an offset: an integer written as such, without
/// a sign or a leading zero.
fn integer(value: &str) -> Option<i64> {
    let canonical = !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'));
    canonical.then(|| value.parse().ok()).flatten()
}

/// Records `value` of `parameter` on `parts`' path, and whether the values
/// now step by one page size: every value seen is the first plus a
/// multiple of their greatest common step, and they fill every step from
/// the lowest to the highest.
///
/// A parameter takes only the values its decision still needs. One whose
/// values step by 1 is never an offset, as no later value raises their
/// common step, and takes none. One that pages keeps its step, which a
/// later value may still lower, and its [`OFFSET_VALUES`] lowest values,
/// which a cursor rule's evidence names; its lowest, highest and count
/// stay as they were when it paged.
fn offset_value(
    connection: &Connection,
    parts: &Parts<'_>,
    parameter: &str,
    value: i64,
    key: &Url,
) -> rusqlite::Result<()> {
    let listing = (parts.host, parts.path, parameter);
    let seen = connection
        .prepare_cached(OFFSET_SEEN)?
        .query_row(listing, |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, bool>(5)?,
            ))
        })
        .optional()?;
    match seen {
        Some((anchor, step, .., true)) => {
            return paged_value(connection, parts, parameter, (anchor, step), value, key);
        }
        Some((_, 1, .., false)) => return Ok(()),
        _ => {}
    }
    let inserted = connection
        .prepare_cached(
            "INSERT INTO rules_offset_values (host, path, parameter, value, url)
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
        )?
        .execute((parts.host, parts.path, parameter, value, key.as_str()))?;
    if inserted == 0 {
        return Ok(());
    }
    let (anchor, step, low, high, count) = match seen {
        None => (value, 0, value, value, 1),
        Some((anchor, step, low, high, count, _)) => (
            anchor,
            greatest_common_divisor(step, value.abs_diff(anchor)),
            low.min(value),
            high.max(value),
            count + 1,
        ),
    };
    let paged = step >= OFFSET_STEP && count >= OFFSET_VALUES && (high - low) / step + 1 == count;
    connection
        .prepare_cached(
            "INSERT INTO rules_offsets (host, path, parameter, anchor, step, low, high, count, paged)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (host, path, parameter) DO UPDATE SET step = excluded.step,
                 low = excluded.low, high = excluded.high, count = excluded.count,
                 paged = excluded.paged",
        )?
        .execute((
            parts.host,
            parts.path,
            parameter,
            anchor,
            step,
            low,
            high,
            count,
            paged,
        ))?;
    if paged {
        connection
            .prepare_cached(
                "DELETE FROM rules_offset_values
                 WHERE host = ?1 AND path = ?2 AND parameter = ?3 AND value > (
                     SELECT value FROM rules_offset_values
                     WHERE host = ?1 AND path = ?2 AND parameter = ?3
                     ORDER BY value LIMIT 1 OFFSET ?4)",
            )?
            .execute((parts.host, parts.path, parameter, OFFSET_VALUES - 1))?;
    }
    Ok(())
}

/// Records `value` of `parameter`, which pages `parts`' path from `anchor`
/// by `step`: lowers the step when the value is off it, and keeps the
/// value in place of the highest kept when it is lower.
fn paged_value(
    connection: &Connection,
    parts: &Parts<'_>,
    parameter: &str,
    (anchor, step): (i64, i64),
    value: i64,
    key: &Url,
) -> rusqlite::Result<()> {
    let listing = (parts.host, parts.path, parameter);
    let lowered = greatest_common_divisor(step, value.abs_diff(anchor));
    if lowered != step {
        connection
            .prepare_cached(
                "UPDATE rules_offsets SET step = ?4
                 WHERE host = ?1 AND path = ?2 AND parameter = ?3",
            )?
            .execute((parts.host, parts.path, parameter, lowered))?;
    }
    let highest: i64 = connection
        .prepare_cached(
            "SELECT value FROM rules_offset_values
             WHERE host = ?1 AND path = ?2 AND parameter = ?3 ORDER BY value DESC LIMIT 1",
        )?
        .query_row(listing, |row| row.get(0))?;
    if value >= highest {
        return Ok(());
    }
    let inserted = connection
        .prepare_cached(
            "INSERT INTO rules_offset_values (host, path, parameter, value, url)
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
        )?
        .execute((parts.host, parts.path, parameter, value, key.as_str()))?;
    if inserted == 1 {
        connection
            .prepare_cached(
                "DELETE FROM rules_offset_values
                 WHERE host = ?1 AND path = ?2 AND parameter = ?3 AND value = ?4",
            )?
            .execute((parts.host, parts.path, parameter, highest))?;
    }
    Ok(())
}

/// The greatest common divisor of `step` and `difference`, 0 being the
/// divisor of nothing yet.
fn greatest_common_divisor(step: i64, difference: u64) -> i64 {
    let (mut one, mut other) = (step.unsigned_abs(), difference);
    while other != 0 {
        (one, other) = (other, one % other);
    }
    i64::try_from(one).unwrap_or(i64::MAX)
}

/// Counts the link `key`, which the listing's page `from` gave to its own
/// path `parts`, as evidence of a cursor, and learns that the listing
/// prefers its cursor to each offset parameter the link lacks once three
/// of its pages each linked a value of one parameter of their own.
///
/// Evidence is a value that changes from page to page, as a token for the
/// rows after a page does: a parameter that is no offset is a cursor once
/// a third page links it with another value than the first two did, each
/// with one value of its own. The third page's first link is what shows
/// that the first two gave all their links, a page's links being admitted
/// together. One that a page links with two values, as the values of a
/// filter, or that two pages link with one value, as a sort or a view, is
/// refuted, and no later link makes it a cursor.
fn cursor(
    connection: &Connection,
    parts: &Parts<'_>,
    from: &Url,
    key: &Url,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(PAGED_OFFSETS)?;
    let offsets = statement
        .query_map((parts.host, parts.path), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if offsets.is_empty() {
        return Ok(());
    }
    let mut shown = None;
    for pair in &parts.pairs {
        let parameter = name(pair);
        if offsets.iter().any(|(offset, _)| offset == parameter) {
            continue;
        }
        if let Some(earlier) = cursor_link(connection, parts, pair, from, key)? {
            shown.get_or_insert((parameter, earlier));
        }
    }
    let Some((parameter, [(first_page, first_link), (second_page, second_link)])) = shown else {
        return Ok(());
    };
    for (offset, step) in offsets.iter().filter(|(offset, _)| !parts.has(offset)) {
        let mut statement = connection.prepare_cached(
            "SELECT url FROM rules_offset_values
             WHERE host = ?1 AND path = ?2 AND parameter = ?3 ORDER BY value LIMIT ?4",
        )?;
        let first: Vec<String> = statement
            .query_map((parts.host, parts.path, offset, OFFSET_VALUES), |row| {
                row.get(0)
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut listing = key.clone();
        listing.set_query(None);
        let evidence = format!(
            "`{offset}` steps by {step} on {listing} ({}), and three of its pages each link one value of `{parameter}` of their own: {first_page} links {first_link}, {second_page} links {second_link}, and {from} links {key}.",
            first.join(", ")
        );
        learn_rule(
            connection,
            (CURSOR_PAGINATION, parts.host, parts.path),
            (offset, ""),
            &evidence,
        )?;
    }
    Ok(())
}

/// Reads the link `key` that the page `from` gave under the query pair
/// `pair` of a listing's path `parts` against the links kept under its
/// parameter, those of the first two pages that gave one: keeps it when
/// it is the first or the second page's, answers those two pages, each
/// with its link, when a third page gave it with a value of its own, which
/// shows a cursor, and refutes the parameter when one page gave two values
/// or two pages the same one.
fn cursor_link(
    connection: &Connection,
    parts: &Parts<'_>,
    pair: &str,
    from: &Url,
    key: &Url,
) -> rusqlite::Result<Option<[(String, String); 2]>> {
    type Link = (String, String, String);
    let (parameter, value) = (name(pair), value(pair));
    let listing = (parts.host, parts.path, parameter);
    let kept = connection
        .prepare_cached(
            "SELECT value, page, url, second_value, second_page, second_url, refuted
             FROM rules_cursors WHERE host = ?1 AND path = ?2 AND parameter = ?3",
        )?
        .query_row(listing, |row| {
            let first: Link = (row.get(0)?, row.get(1)?, row.get(2)?);
            let second: Option<Link> = row
                .get::<_, Option<String>>(3)?
                .map(|value| Ok::<_, rusqlite::Error>((value, row.get(4)?, row.get(5)?)))
                .transpose()?;
            Ok((first, second, row.get::<_, bool>(6)?))
        })
        .optional()?;
    let Some((first, second, refuted)) = kept else {
        connection
            .prepare_cached(
                "INSERT INTO rules_cursors (host, path, parameter, value, page, url, refuted)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            )?
            .execute((
                parts.host,
                parts.path,
                parameter,
                value,
                from.as_str(),
                key.as_str(),
            ))?;
        return Ok(None);
    };
    if refuted {
        return Ok(None);
    }
    let pages = std::iter::once(&first).chain(&second);
    let own = pages.clone().find(|(_, page, _)| page == from.as_str());
    let consistent = match own {
        // A page that gave a link already gives the same value again.
        Some((own, ..)) => own == value,
        // Another page gives a value no page gave before.
        None => pages.clone().all(|(other, ..)| other != value),
    };
    if !consistent {
        connection
            .prepare_cached(
                "UPDATE rules_cursors SET refuted = 1
                 WHERE host = ?1 AND path = ?2 AND parameter = ?3",
            )?
            .execute(listing)?;
        return Ok(None);
    }
    if own.is_some() {
        return Ok(None);
    }
    let Some(second) = second else {
        connection
            .prepare_cached(
                "UPDATE rules_cursors SET second_value = ?4, second_page = ?5, second_url = ?6
                 WHERE host = ?1 AND path = ?2 AND parameter = ?3",
            )?
            .execute((
                parts.host,
                parts.path,
                parameter,
                value,
                from.as_str(),
                key.as_str(),
            ))?;
        return Ok(None);
    };
    Ok(Some([(first.1, first.2), (second.1, second.2)]))
}

/// Adds a rule of `kind` on `host` and `path`, about `parameter` and
/// `form`, unless it is learned already.
fn learn_rule(
    connection: &Connection,
    (kind, host, path): (&str, &str, &str),
    (parameter, form): (&str, &str),
    evidence: &str,
) -> rusqlite::Result<Option<i64>> {
    let added = connection
        .prepare_cached(
            "INSERT INTO learned_rules (kind, host, path, parameter, form, evidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
        )?
        .execute((kind, host, path, parameter, form, evidence))?;
    Ok((added == 1).then(|| connection.last_insert_rowid()))
}

/// The rows of learning's evidence that `statement` reads with `params`,
/// each as `read` makes it, counted: the one function a decision reads its
/// evidence through (`F2.evidence_reads`, ADR 0140).
fn read_evidence<T>(
    work: &CountedWork,
    mut statement: CachedStatement<'_>,
    params: impl Params,
    read: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    let rows = statement
        .query_map(params, read)?
        .collect::<rusqlite::Result<Vec<T>>>()?;
    work.count_many(
        Row::EvidenceReads,
        u64::try_from(rows.len()).unwrap_or(u64::MAX),
    );
    Ok(rows)
}

/// The evidence of the rules that merge URLs, in the run file `connection`
/// holds, whose reads `work` counts.
#[derive(Clone, Copy)]
struct Evidence<'a> {
    connection: &'a Connection,
    work: &'a CountedWork,
}

/// A page as [`learn`] reads it: the crawl request it was fetched for,
/// the key of the URL it was fetched at, its page signature and the key of
/// its `rel=canonical`, if it names one.
pub(crate) struct Page<'a> {
    pub(crate) crawl_request: &'a str,
    pub(crate) key: &'a Url,
    pub(crate) signature: &'a [u8; 32],
    pub(crate) canonical: Option<&'a Url>,
}

/// Learns from `page`, once per crawl request: its pairs with the pages
/// fetched before whose URLs differ from its own only by one parameter or
/// one form, its canonical when that differs from its URL only by a form,
/// and its path pattern's run of pages that brought nothing new; answers
/// each merging rule it taught or stopped.
///
/// # Errors
///
/// When the run file cannot be read or written.
pub(crate) fn learn(
    connection: &Connection,
    page: &Page<'_>,
    work: &CountedWork,
) -> rusqlite::Result<Vec<Change>> {
    let Page {
        crawl_request,
        key,
        signature,
        canonical,
    } = *page;
    let parts = Parts::of(key);
    let query = parts.query_without(None);
    let first_time = connection
        .prepare_cached(
            "INSERT INTO rules_pages (crawl_request, host, path, query, url, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
        )?
        .execute((
            crawl_request,
            parts.host,
            parts.path,
            &query,
            key.as_str(),
            &signature[..],
        ))?;
    if first_time == 0 {
        return Ok(Vec::new());
    }
    let new_signature = connection
        .prepare_cached(
            "INSERT INTO rules_signatures (host, signature) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        )?
        .execute((parts.host, &signature[..]))?
        == 1;
    trap(connection, &parts, key, new_signature)?;
    let mut changes = Vec::new();
    let evidence = Evidence { connection, work };
    for parameter in evidence.pairs(crawl_request, &parts, &query, key, signature)? {
        changes.extend(evidence.judge(parts.host, Subject::Parameter(&parameter))?);
    }
    for (form, host) in evidence.form_pairs(crawl_request, key, signature)? {
        changes.extend(evidence.judge(&host, Subject::Form(form))?);
    }
    if let Some(canonical) = canonical {
        changes.extend(evidence.canonical(key, canonical)?);
    }
    Ok(changes)
}

impl Evidence<'_> {
    /// Where the rule about `subject` on `host` stands.
    fn standing(self, host: &str, subject: Subject<'_>) -> rusqlite::Result<Standing> {
        let (kind, parameter, form) = subject.columns();
        let stopped = self
            .connection
            .prepare_cached(
                "SELECT contradiction IS NOT NULL FROM learned_rules
                 WHERE kind = ?1 AND host = ?2 AND path = '' AND parameter = ?3 AND form = ?4",
            )?
            .query_row((kind, host, &parameter, form), |row| row.get::<_, bool>(0))
            .optional()?;
        Ok(match stopped {
            Some(true) => Standing::Settled,
            Some(false) => Standing::Learned,
            None if self.differs(host, &parameter, form)? => Standing::Settled,
            None => Standing::Undecided,
        })
    }

    /// Pairs the page of `crawl_request` at `key` with each page fetched
    /// before at the URL that differs from `key` only by one form, as that
    /// form's [`Standing`] takes its pairs, and answers each form it paired,
    /// with the host of its rule.
    fn form_pairs(
        self,
        crawl_request: &str,
        key: &Url,
        signature: &[u8; 32],
    ) -> rusqlite::Result<Vec<(EquivalentForm, String)>> {
        let connection = self.connection;
        let mut touched = Vec::new();
        for form in forms::FORMS {
            let (Some(other), Some(host)) = (forms::other(form, key), forms::rule_host(form, key))
            else {
                continue;
            };
            let standing = self.standing(&host, Subject::Form(form))?;
            if standing == Standing::Settled {
                continue;
            }
            let signatures = read_evidence(
                self.work,
                connection.prepare_cached(
                    "SELECT signature FROM rules_pages WHERE url = ?1 AND crawl_request != ?2",
                )?,
                (other.as_str(), crawl_request),
                |row| row.get::<_, Vec<u8>>(0),
            )?;
            let mut paired = false;
            for other_signature in &signatures {
                let matching = *other_signature == signature[..];
                if !standing.takes(matching) {
                    continue;
                }
                connection
                    .prepare_cached(
                        "INSERT INTO rules_pairs (host, parameter, form, one, other, matching)
                         VALUES (?1, '', ?2, ?3, ?4, ?5) ON CONFLICT DO NOTHING",
                    )?
                    .execute((
                        &host,
                        forms::name(form),
                        other.as_str(),
                        key.as_str(),
                        matching,
                    ))?;
                paired = true;
            }
            if paired {
                touched.push((form, host));
            }
        }
        Ok(touched)
    }

    /// Counts the page at `key` toward the form its canonical, whose key is
    /// `canonical`, differs from its URL by, when it differs by one form
    /// alone, and learns that form at [`CANONICAL_PAGES`]. A canonical to
    /// another path or another registrable domain, or the page's own URL, is
    /// no evidence.
    fn canonical(self, key: &Url, canonical: &Url) -> rusqlite::Result<Option<Change>> {
        let connection = self.connection;
        let Some((form, host)) = forms::differ_by(key, canonical) else {
            return Ok(None);
        };
        if self.standing(&host, Subject::Form(form))? != Standing::Undecided {
            return Ok(None);
        }
        let name = forms::name(form);
        connection
            .prepare_cached(
                "INSERT INTO rules_canonicals (host, form, url, canonical)
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
            )?
            .execute((&host, name, key.as_str(), canonical.as_str()))?;
        let pages = read_evidence(
        self.work,
        connection.prepare_cached(
            "SELECT url, canonical FROM rules_canonicals WHERE host = ?1 AND form = ?2 ORDER BY url",
        )?,
        (&host, name),
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
        let count = i64::try_from(pages.len()).unwrap_or(i64::MAX);
        if count < CANONICAL_PAGES {
            return Ok(None);
        }
        let mut evidence = format!(
            "{count} pages named as their `rel=canonical` their own URL but for {}:",
            described(form)
        );
        for (index, (url, canonical)) in pages.iter().enumerate() {
            let separator = if index == 0 { " " } else { "; " };
            write!(evidence, "{separator}{url} names {canonical}")
                .expect("invariant: writing to a String succeeds");
        }
        evidence.push('.');
        self.learn_merging(&host, Subject::Form(form), &evidence)
    }
}

/// Learns the form a redirect from the URL whose key is `from` to the one
/// whose key is `to` shows, when they differ by one form alone: one
/// redirect is evidence enough (RFC 3986 §6.2.4).
///
/// # Errors
///
/// When the run file cannot be read or written.
pub(crate) fn learn_redirect(
    connection: &Connection,
    from: &Url,
    to: &Url,
    work: &CountedWork,
) -> rusqlite::Result<Option<Change>> {
    let Some((form, host)) = forms::differ_by(from, to) else {
        return Ok(None);
    };
    let evidence = format!(
        "{from} redirected to {to}, which differs from it only by {}.",
        described(form)
    );
    Evidence { connection, work }.learn_merging(&host, Subject::Form(form), &evidence)
}

impl Evidence<'_> {
    /// Learns the rule about `subject` on `host` from `evidence`, unless it
    /// is learned already, stopped, or a pair of its pages differs.
    fn learn_merging(
        self,
        host: &str,
        subject: Subject<'_>,
        evidence: &str,
    ) -> rusqlite::Result<Option<Change>> {
        let connection = self.connection;
        let (kind, parameter, form) = subject.columns();
        if self.differs(host, &parameter, form)? {
            return Ok(None);
        }
        Ok(
            learn_rule(connection, (kind, host, ""), (&parameter, form), evidence)?.map(|rule| {
                Change::Learned(Merging {
                    rule,
                    hosts: subject.hosts(host),
                })
            }),
        )
    }

    /// Whether a pair of pages differing only by `parameter` or `form` on
    /// `host` differed.
    fn differs(self, host: &str, parameter: &str, form: &str) -> rusqlite::Result<bool> {
        let differing = read_evidence(
            self.work,
            self.connection.prepare_cached(
                "SELECT 1 FROM rules_pairs
             WHERE host = ?1 AND parameter = ?2 AND form = ?3 AND matching = 0 LIMIT 1",
            )?,
            (host, parameter, form),
            |_| Ok(()),
        )?;
        Ok(!differing.is_empty())
    }

    /// What the pairs of `subject` on `host` now say: a rule that holds and
    /// met a differing pair stops, naming it; one not yet learned is learned
    /// once its matching pairs reach its threshold and none differs.
    fn judge(self, host: &str, subject: Subject<'_>) -> rusqlite::Result<Option<Change>> {
        let connection = self.connection;
        let (kind, parameter, form) = subject.columns();
        let pairs = read_evidence(
            self.work,
            connection.prepare_cached(
                "SELECT one, other, matching FROM rules_pairs
             WHERE host = ?1 AND parameter = ?2 AND form = ?3 ORDER BY rowid",
            )?,
            (host, &parameter, form),
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )?;
        let rule = connection
            .prepare_cached(
                "SELECT id, contradiction IS NOT NULL FROM learned_rules
             WHERE kind = ?1 AND host = ?2 AND path = '' AND parameter = ?3 AND form = ?4",
            )?
            .query_row((kind, host, &parameter, form), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?))
            })
            .optional()?;
        let differing = pairs.iter().rev().find(|(_, _, matching)| !matching);
        match (rule, differing) {
            (Some((rule, false)), Some((one, other, _))) => {
                let contradiction = format!(
                    "{one} and {other}, both fetched and neither redirected, served pages of differing signatures."
                );
                connection
                    .prepare_cached("UPDATE learned_rules SET contradiction = ?1 WHERE id = ?2")?
                    .execute((&contradiction, rule))?;
                Ok(Some(Change::Contradicted(Merging {
                    rule,
                    hosts: subject.hosts(host),
                })))
            }
            (Some(_), _) | (None, Some(_)) => Ok(None),
            (None, None) => {
                let count = i64::try_from(pairs.len()).unwrap_or(i64::MAX);
                if count < subject.threshold() {
                    return Ok(None);
                }
                let mut evidence = format!(
                    "{count} pairs of URLs differing only by {} served pages of matching signatures, and no pair pages of differing ones:",
                    subject.difference()
                );
                for (index, (one, other, _)) in pairs.iter().enumerate() {
                    let separator = if index == 0 { " " } else { "; " };
                    write!(evidence, "{separator}{one} and {other}")
                        .expect("invariant: writing to a String succeeds");
                }
                evidence.push('.');
                self.learn_merging(host, subject, &evidence)
            }
        }
    }
}

/// Counts the page at `key` in its path pattern's run of pages that
/// brought nothing new, which a new signature starts again, and closes the
/// pattern as a trap at [`TRAP_PAGES`].
fn trap(
    connection: &Connection,
    parts: &Parts<'_>,
    key: &Url,
    new_signature: bool,
) -> rusqlite::Result<()> {
    let pattern = pattern(parts.path);
    let closed: bool = connection
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM learned_rules WHERE kind = ?1 AND host = ?2 AND path = ?3)",
        )?
        .query_row((TRAP_PATH, parts.host, &pattern), |row| row.get(0))?;
    if closed {
        return Ok(());
    }
    let (streak, first): (i64, Option<String>) = connection
        .prepare_cached(
            "INSERT INTO rules_patterns (host, pattern, streak, first)
             VALUES (?1, ?2, CASE WHEN ?3 THEN 0 ELSE 1 END, CASE WHEN ?3 THEN NULL ELSE ?4 END)
             ON CONFLICT (host, pattern) DO UPDATE SET
                 streak = CASE WHEN ?3 THEN 0 ELSE streak + 1 END,
                 first = CASE WHEN ?3 THEN NULL ELSE coalesce(first, ?4) END
             RETURNING streak, first",
        )?
        .query_row((parts.host, &pattern, new_signature, key.as_str()), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    if streak >= TRAP_PAGES {
        let evidence = format!(
            "{TRAP_PAGES} pages under `{pattern}` in a row, from {} to {key}, brought no new page signature and no URL outside it.",
            first.unwrap_or_default()
        );
        learn_rule(
            connection,
            (TRAP_PATH, parts.host, &pattern),
            ("", ""),
            &evidence,
        )?;
    }
    Ok(())
}

impl Evidence<'_> {
    /// Pairs the page of `crawl_request` at `key` with each page fetched
    /// before whose URL differs from it only by one parameter, on either
    /// side, as that parameter's [`Standing`] takes its pairs, and answers
    /// each parameter it paired. A settled parameter is read and written
    /// nothing; a learned one still leaves its residue, which a differing
    /// page is read against.
    fn pairs(
        self,
        crawl_request: &str,
        parts: &Parts<'_>,
        query: &str,
        key: &Url,
        signature: &[u8; 32],
    ) -> rusqlite::Result<BTreeSet<String>> {
        let connection = self.connection;
        let mut standings: HashMap<String, Standing> = HashMap::new();
        let mut found: Vec<(String, String, Vec<u8>)> = Vec::new();
        for parameter in parts.names() {
            let standing = self.standing(parts.host, Subject::Parameter(parameter))?;
            standings.insert(parameter.to_owned(), standing);
            if standing == Standing::Settled {
                continue;
            }
            let residue = parts.query_without(Some(parameter));
            let others = read_evidence(
                self.work,
                connection.prepare_cached(
                    "SELECT url, signature FROM rules_residues
                     WHERE host = ?1 AND path = ?2 AND residue = ?3 AND parameter = ?4
                     UNION ALL
                     SELECT url, signature FROM rules_pages
                     WHERE host = ?1 AND path = ?2 AND query = ?3 AND crawl_request != ?5",
                )?,
                (parts.host, parts.path, &residue, parameter, crawl_request),
                |row| Ok((parameter.to_owned(), row.get(0)?, row.get(1)?)),
            )?;
            connection
                .prepare_cached(
                    "INSERT INTO rules_residues (crawl_request, parameter, host, path, residue, url, signature)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT DO NOTHING",
                )?
                .execute((
                    crawl_request,
                    parameter,
                    parts.host,
                    parts.path,
                    &residue,
                    key.as_str(),
                    &signature[..],
                ))?;
            found.extend(others);
        }
        let with_one_more = read_evidence(
            self.work,
            connection.prepare_cached(
                "SELECT parameter, url, signature FROM rules_residues
                 WHERE host = ?1 AND path = ?2 AND residue = ?3 AND crawl_request != ?4",
            )?,
            (parts.host, parts.path, query, crawl_request),
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        found.extend(with_one_more);
        let mut touched = BTreeSet::new();
        for (parameter, other, other_signature) in found {
            let standing = if let Some(standing) = standings.get(&parameter) {
                *standing
            } else {
                let standing = self.standing(parts.host, Subject::Parameter(&parameter))?;
                standings.insert(parameter.clone(), standing);
                standing
            };
            let matching = other_signature == signature[..];
            if !standing.takes(matching) {
                continue;
            }
            connection
                .prepare_cached(
                    "INSERT INTO rules_pairs (host, parameter, form, one, other, matching)
                     VALUES (?1, ?2, '', ?3, ?4, ?5) ON CONFLICT DO NOTHING",
                )?
                .execute((parts.host, &parameter, &other, key.as_str(), matching))?;
            touched.insert(parameter);
        }
        Ok(touched)
    }
}

/// The path pattern of `path`: each segment that is a number, a date or a
/// hexadecimal token written `{number}`, `{date}` or `{hex}`.
pub(crate) fn pattern(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if is_number(segment) {
                "{number}"
            } else if is_date(segment) {
                "{date}"
            } else if is_hexadecimal(segment) {
                "{hex}"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_number(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether `segment` is a date of numbers joined by `-`, `_` or `.`: a
/// year of 4 digits then a month and maybe a day, or a day and a month
/// then the year.
fn is_date(segment: &str) -> bool {
    let parts: Vec<&str> = segment.split(['-', '_', '.']).collect();
    let widths: Vec<usize> = parts.iter().map(|part| part.len()).collect();
    let numeric = parts.len() > 1 && parts.iter().all(|part| is_number(part));
    numeric
        && matches!(
            widths.as_slice(),
            [4, 1 | 2] | [4, 1 | 2, 1 | 2] | [1 | 2, 1 | 2, 4]
        )
}

/// Whether `segment` is a hexadecimal token: 6 or more hexadecimal digits,
/// with a decimal digit and a letter among them, so that neither a word
/// nor a number is one.
fn is_hexadecimal(segment: &str) -> bool {
    segment.len() >= 6
        && segment.bytes().all(|byte| byte.is_ascii_hexdigit())
        && segment.bytes().any(|byte| byte.is_ascii_digit())
        && segment.bytes().any(|byte| byte.is_ascii_alphabetic())
}

/// The rules the run file `connection` holds, in the order they were
/// learned, each with the URLs it merged in the order met.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn learned_rules(connection: &Connection) -> rusqlite::Result<Vec<LearnedRule>> {
    let mut statement = connection.prepare_cached(
        "SELECT id, kind, host, path, parameter, form, evidence, contradiction
         FROM learned_rules ORDER BY id",
    )?;
    let rules = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut merges = connection
        .prepare_cached("SELECT url FROM learned_merges WHERE rule = ?1 ORDER BY rowid")?;
    let mut learned = Vec::with_capacity(rules.len());
    for (id, kind, host, path, parameter, form, evidence, contradiction) in rules {
        let kind = kind_named(&kind)?;
        let form = (!form.is_empty()).then(|| form_named(&form)).transpose()?;
        let urls = merges
            .query_map([id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        learned.push(LearnedRule {
            kind,
            host,
            path: (!path.is_empty()).then_some(path),
            parameter: (!parameter.is_empty()).then_some(parameter),
            form,
            evidence,
            contradiction,
            merged: urls,
        });
    }
    Ok(learned)
}

/// The kind of learned rule the run file names `name`, as the protocol
/// spells it.
///
/// # Errors
///
/// A conversion failure for a name the protocol does not have: a row the
/// run file should not hold is never read as another kind.
fn kind_named(name: &str) -> rusqlite::Result<LearnedRuleKind> {
    serde_json::from_value(serde_json::Value::from(name)).map_err(|error| unread(1, error))
}

/// The equivalent form the run file names `name`, as the protocol spells
/// it.
///
/// # Errors
///
/// A conversion failure for a name the protocol does not have.
fn form_named(name: &str) -> rusqlite::Result<EquivalentForm> {
    serde_json::from_value(serde_json::Value::from(name)).map_err(|error| unread(5, error))
}

/// The failure to read column `column` of a learned rule, whose name
/// `error` says the protocol does not have.
fn unread(column: usize, error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, Box::new(error))
}

/// Why the learned rules leave a finished crawl `unverified`: the URLs
/// learned traps refused, the offset URLs learned cursors refused, the
/// listings paged only by an offset, and what each rule a contradicting
/// pair stopped had merged that the crawl did not get back, each a reason
/// with its count. `visited` says whether the visited set holds an
/// identity: a URL a stopped rule refused as a `duplicate` is got back
/// once the crawl added it.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn completeness_reasons(
    connection: &Connection,
    visited: impl Fn(&str) -> rusqlite::Result<bool>,
) -> rusqlite::Result<Vec<CompletenessReason>> {
    let refused_by = |kind: &str| -> rusqlite::Result<i64> {
        connection
            .prepare_cached(
                "SELECT count(*) FROM learned_merges
                 JOIN learned_rules ON learned_rules.id = learned_merges.rule
                 WHERE learned_rules.kind = ?1",
            )
            .and_then(|mut cached| cached.query_row([kind], |row| row.get(0)))
    };
    let refused = refused_by(TRAP_PATH)?;
    let by_cursor = refused_by(CURSOR_PAGINATION)?;
    let offset_only: i64 = connection
        .prepare_cached(
            "SELECT count(*) FROM rules_offsets AS offsets WHERE paged = 1 AND NOT EXISTS
             (SELECT 1 FROM learned_rules WHERE kind = ?1 AND host = offsets.host
                 AND path = offsets.path AND parameter = offsets.parameter)",
        )
        .and_then(|mut cached| cached.query_row([CURSOR_PAGINATION], |row| row.get(0)))?;
    let mut reasons = Vec::new();
    if refused > 0 {
        let urls = if refused == 1 { "URL" } else { "URLs" };
        let traps = if refused == 1 {
            "A learned trap"
        } else {
            "Learned traps"
        };
        reasons.push(unverified(
            TRAP_PATH_REASON,
            format!("{traps} refused {refused} {urls} the crawl would otherwise have admitted."),
            refused,
        ));
    }
    if by_cursor > 0 {
        let message = if by_cursor == 1 {
            "A learned cursor refused 1 offset URL of a listing the crawl pages by its cursor instead."
                .to_owned()
        } else {
            format!(
                "Learned cursors refused {by_cursor} offset URLs of listings the crawl pages by their cursor instead."
            )
        };
        reasons.push(unverified(CURSOR_PAGINATION_REASON, message, by_cursor));
    }
    if offset_only > 0 {
        let (listings, it) = if offset_only == 1 {
            ("listing was", "it")
        } else {
            ("listings were", "them")
        };
        reasons.push(unverified(
            OFFSET_PAGINATION,
            format!(
                "{offset_only} {listings} paged only by an offset parameter, so a row that moved while the crawl paged {it} may be missed."
            ),
            offset_only,
        ));
    }
    reasons.extend(stopped_reasons(connection, visited)?);
    Ok(reasons)
}

/// A reason for each rule a contradicting pair stopped that had merged
/// URLs the crawl did not get back: the requests its merges ended that
/// are not pending again, and the URLs it refused as a `duplicate` whose
/// identity `visited` does not hold.
fn stopped_reasons(
    connection: &Connection,
    visited: impl Fn(&str) -> rusqlite::Result<bool>,
) -> rusqlite::Result<Vec<CompletenessReason>> {
    let mut statement = connection.prepare_cached(
        "SELECT id, host, parameter, form FROM learned_rules
         WHERE contradiction IS NOT NULL ORDER BY id",
    )?;
    let stopped = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut reasons = Vec::new();
    for (rule, host, parameter, form) in stopped {
        let mut lost: i64 = connection
            .prepare_cached("SELECT count(*) FROM rules_ended WHERE rule = ?1")?
            .query_row([rule], |row| row.get(0))?;
        let mut refused = connection.prepare_cached(
            "SELECT identity FROM learned_merges WHERE rule = ?1 AND identity IS NOT NULL",
        )?;
        let identities = refused
            .query_map([rule], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for identity in identities {
            if !visited(&identity)? {
                lost += 1;
            }
        }
        if lost == 0 {
            continue;
        }
        let about = if form.is_empty() {
            format!("`{parameter}`")
        } else {
            described(form_named(&form)?).to_owned()
        };
        let urls = if lost == 1 { "URL" } else { "URLs" };
        reasons.push(unverified(
            LEARNED_RULE_STOPPED,
            format!(
                "A contradicting pair stopped the learned rule about {about} on {host}, which had merged {lost} {urls} the crawl did not get back."
            ),
            lost,
        ));
    }
    Ok(reasons)
}

fn unverified(code: &str, message: String, count: i64) -> CompletenessReason {
    CompletenessReason {
        completeness: Completeness::Unverified,
        reason: AxisReason {
            code: code.to_owned(),
            message,
            count: Some(u64::try_from(count).unwrap_or(0)),
            call_id: None,
        },
    }
}

#[cfg(test)]
mod tests;
