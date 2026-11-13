//! A crawl's scope (ADRs 0093, 0104 and 0120; pitfall U15): the site a
//! crawl was given, which it never leaves. Seeds define it, each widening
//! it by its own URL, and every enqueue, redirect hop and Hook rewrite is
//! measured against every seed under `crawl.scope.strategy`: the same host
//! name (the default), the same registrable domain under the Public Suffix
//! List, the same origin, or every URL. Hosts are compared as the WHATWG
//! parser leaves them, but one leading `www.` label is ignored, and under
//! `same_origin` a seed's `http` origin also admits its host's `https`
//! origin on the same port, so the scope never waits for evidence.
//! `crawl.scope.include` and `crawl.scope.exclude`, and an `enqueueLinks`
//! call's own patterns, only narrow the strategy: a URL must match an
//! `include` pattern when there are any, and no `exclude` pattern.
//!
//! The scope lives in the run file: its strategy and patterns, recorded
//! when the Run begins and read back from it, so they are fixed for the
//! Run's life whatever a resume's settings say (ADR 0102), and each seed's
//! measures, its host, registrable domain and origin, indexed, so a check
//! looks a URL's measures up rather than reading every seed.
//!
//! A check has two parts (#2244). The patterns, and what a URL asks the
//! seeds, are read from the URL and the Run's invariants alone, on its
//! page's task ([`Scope::lets_through`], [`Scope::asks`]). Whether a seed
//! reaches it is asked where the seeds are held ([`Scope::reached`]): they
//! are the Run's state, since each seed admitted while the crawl runs
//! widens them.

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::settings::Resolved;
use goodfetch_page::url_pattern::{UnsupportedPattern, UrlPattern};
use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::foxhound::frontier::{
    CRAWL_SCOPE_EXCLUDE, CRAWL_SCOPE_INCLUDE, CRAWL_SCOPE_STRATEGY,
};
use rusqlite::{Connection, OptionalExtension as _};
use url::{Host, Url};

use crate::staged::Staged;

/// The scope's strategy and patterns, one row, and the measures of the
/// seeds that widened it.
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-frontier-scope",
    statements: &[
        "CREATE TABLE IF NOT EXISTS scope (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            strategy TEXT NOT NULL,
            include TEXT NOT NULL,
            exclude TEXT NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS scope_seeds (
            host TEXT NOT NULL,
            port INTEGER NOT NULL,
            scheme TEXT NOT NULL,
            domain TEXT NOT NULL,
            PRIMARY KEY (host, port, scheme)
        ) STRICT, WITHOUT ROWID",
        "CREATE INDEX IF NOT EXISTS scope_seeds_by_domain ON scope_seeds (domain)",
    ],
};

/// What a URL is measured by against a seed (`crawl.scope.strategy`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Strategy {
    /// The seed's host name, its scheme and port aside.
    #[default]
    SameHostname,
    /// The seed's registrable domain, its subdomains included.
    SameDomain,
    /// The seed's origin, and its host's `https` origin on the same port
    /// for an `http` seed.
    SameOrigin,
    /// Every URL.
    All,
}

impl Strategy {
    /// The strategy named `name`, as `crawl.scope.strategy` spells it.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "same_hostname" => Some(Self::SameHostname),
            "same_domain" => Some(Self::SameDomain),
            "same_origin" => Some(Self::SameOrigin),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// Its name, as `crawl.scope.strategy` spells it.
    pub fn name(self) -> &'static str {
        match self {
            Self::SameHostname => "same_hostname",
            Self::SameDomain => "same_domain",
            Self::SameOrigin => "same_origin",
            Self::All => "all",
        }
    }

    /// The query that answers whether a seed of the run file reaches a URL
    /// under this strategy, by the URL's measures; none for `all`, which
    /// reaches every URL. [`crate::prepare`] prepares it as the Run starts.
    pub(crate) fn query(self) -> Option<&'static str> {
        match self {
            Self::SameHostname => Some("SELECT EXISTS (SELECT 1 FROM scope_seeds WHERE host = ?1)"),
            Self::SameDomain => Some("SELECT EXISTS (SELECT 1 FROM scope_seeds WHERE domain = ?1)"),
            Self::SameOrigin => Some(
                "SELECT EXISTS (SELECT 1 FROM scope_seeds
                    WHERE host = ?1 AND port = ?2
                      AND (scheme = ?3 OR (scheme = 'http' AND ?3 = 'https')))",
            ),
            Self::All => None,
        }
    }

    /// Whether `url` is within this strategy's reach of the seed `seed`.
    pub fn reaches(self, seed: &Url, url: &Url) -> bool {
        match (self, Measures::of(seed), Measures::of(url)) {
            (Self::All, _, _) => true,
            (_, Some(seed), Some(url)) => match self {
                Self::SameHostname => seed.host == url.host,
                Self::SameDomain => seed.domain == url.domain,
                _ => {
                    seed.host == url.host
                        && seed.port == url.port
                        && (seed.scheme == url.scheme
                            || (seed.scheme == "http" && url.scheme == "https"))
                }
            },
            _ => false,
        }
    }
}

/// A URL's measures, as the scope compares them.
struct Measures<'a> {
    scheme: &'a str,
    /// Its host as the WHATWG parser serializes it, without one leading
    /// `www.` label of a domain.
    host: String,
    /// Its port, or 0 for its scheme's default.
    port: u16,
    /// Its host's registrable domain under the full Public Suffix List,
    /// private entries included, as a domain key is (ADR 0004); an
    /// address, or a host with no registrable domain, is its own.
    domain: String,
}

impl Measures<'_> {
    /// The measures of `url`; none for a URL with no host.
    fn of(url: &Url) -> Option<Measures<'_>> {
        let host = compared_host(url)?;
        let domain = registrable(url, &host);
        Some(Measures {
            scheme: url.scheme(),
            host,
            port: url.port().unwrap_or(0),
            domain,
        })
    }
}

/// The host of `url` as the scope compares it: as the WHATWG parser
/// serializes it, without one leading `www.` label of a domain; none for a
/// URL with no host.
fn compared_host(url: &Url) -> Option<String> {
    Some(match url.host()? {
        Host::Domain(domain) => domain
            .strip_prefix("www.")
            .filter(|rest| !rest.is_empty())
            .unwrap_or(domain)
            .to_owned(),
        address => address.to_string(),
    })
}

/// The registrable domain of `url`, whose compared host is `host`, under
/// the full Public Suffix List; an address, or a host with no registrable
/// domain, is its own.
fn registrable(url: &Url, host: &str) -> String {
    match url.host() {
        Some(Host::Domain(_)) => psl::domain(host.as_bytes()).map_or_else(
            || host.to_owned(),
            |registrable| String::from_utf8_lossy(registrable.as_bytes()).into_owned(),
        ),
        _ => host.to_owned(),
    }
}

/// What a URL asks the Scope's seeds, read from it once, on its page's
/// task, with no run file (#2244): the one measure its strategy compares,
/// so the part of admission that holds the seeds only looks it up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Asked {
    /// Nothing, under `all`: every URL is reached.
    Nothing,
    /// A URL with no host, which no seed reaches.
    NoHost,
    /// Under `same_hostname`: its host, without one leading `www.`.
    Host(String),
    /// Under `same_domain`: its registrable domain.
    Domain(String),
    /// Under `same_origin`: its host, its port, or 0 for its scheme's
    /// default, and its scheme.
    Origin {
        /// Its host, without one leading `www.`.
        host: String,
        /// Its port, or 0 for its scheme's default.
        port: u16,
        /// Its scheme.
        scheme: String,
    },
}

/// URL patterns, each a glob read as Playwright reads one (ADR 0081),
/// matched in the engine against a URL as the WHATWG parser serializes it.
#[derive(Clone, Debug, Default)]
pub struct Patterns {
    globs: Vec<String>,
    compiled: Vec<UrlPattern>,
}

impl Patterns {
    /// The patterns `globs`, compiled.
    ///
    /// # Errors
    ///
    /// The first glob the matcher cannot read, which the call that gave it
    /// raises as `pattern_unsupported`.
    pub fn globs(globs: &[String]) -> Result<Self, UnsupportedPattern> {
        let compiled = globs
            .iter()
            .map(|glob| UrlPattern::glob(glob))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            globs: globs.to_vec(),
            compiled,
        })
    }

    /// The patterns `globs`, each compiled from the engine's cache `work`
    /// keeps for the Run, and counted on `row` only when it does not keep it
    /// (ADR 0140): a second call with the same globs compiles nothing.
    ///
    /// # Errors
    ///
    /// As [`Self::globs`].
    pub fn cached(
        work: &CountedWork,
        row: Row,
        globs: &[String],
    ) -> Result<Self, UnsupportedPattern> {
        let compiled = globs
            .iter()
            .map(|glob| UrlPattern::cached_glob(work, row, glob))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            globs: globs.to_vec(),
            compiled,
        })
    }

    fn is_empty(&self) -> bool {
        self.compiled.is_empty()
    }

    fn any_matches(&self, url: &Url) -> bool {
        self.compiled
            .iter()
            .any(|pattern| pattern.matches(url.as_str()))
    }
}

/// Patterns that narrow the scope for one call, as an `enqueueLinks`
/// call's own `include` and `exclude`.
#[derive(Clone, Copy, Debug)]
pub struct Narrowing<'a> {
    /// A URL must match one of these when there are any.
    pub include: &'a Patterns,
    /// A URL must match none of these.
    pub exclude: &'a Patterns,
}

impl Narrowing<'_> {
    /// No narrowing.
    pub fn none() -> Narrowing<'static> {
        static NONE: Patterns = Patterns {
            globs: Vec::new(),
            compiled: Vec::new(),
        };
        Narrowing {
            include: &NONE,
            exclude: &NONE,
        }
    }

    fn admits(&self, url: &Url) -> bool {
        (self.include.is_empty() || self.include.any_matches(url)) && !self.exclude.any_matches(url)
    }
}

/// A crawl's scope: its strategy and its patterns, measured against the
/// seeds the run file holds.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    strategy: Strategy,
    include: Patterns,
    exclude: Patterns,
}

impl Scope {
    /// The scope under `strategy`, narrowed by `include` and `exclude`.
    pub fn new(strategy: Strategy, include: Patterns, exclude: Patterns) -> Self {
        Self {
            strategy,
            include,
            exclude,
        }
    }

    /// The scope a Run's `settings` give: `crawl.scope.strategy`,
    /// `crawl.scope.include` and `crawl.scope.exclude`.
    ///
    /// # Errors
    ///
    /// The first pattern the matcher cannot read, which the start raises as
    /// `pattern_unsupported`.
    pub fn of_settings(settings: &Resolved) -> Result<Self, UnsupportedPattern> {
        let globs = |setting: &Setting| -> Vec<String> {
            settings
                .value(setting)
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|glob| glob.as_str().map(str::to_owned))
                .collect()
        };
        Ok(Self::new(
            Strategy::named(settings.get(&CRAWL_SCOPE_STRATEGY)).expect(
                "invariant: the settings resolver lets only `crawl.scope.strategy`'s choices through",
            ),
            Patterns::globs(&globs(&CRAWL_SCOPE_INCLUDE))?,
            Patterns::globs(&globs(&CRAWL_SCOPE_EXCLUDE))?,
        ))
    }

    /// Its strategy.
    pub fn strategy(&self) -> Strategy {
        self.strategy
    }

    /// Whether `url` is in the scope the seeds of the run file `connection`
    /// holds define, narrowed further by `narrowing`: the patterns let it
    /// through, and the strategy reaches it from a seed.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn admits(
        &self,
        connection: &Connection,
        url: &Url,
        narrowing: Narrowing<'_>,
    ) -> rusqlite::Result<bool> {
        self.admits_in(connection, url, narrowing, None)
    }

    /// Whether `url` is in the scope as [`Scope::admits`] measures it, the
    /// seeds of `staged`, an attempt's earlier staged admissions, widening
    /// it too.
    pub(crate) fn admits_in(
        &self,
        connection: &Connection,
        url: &Url,
        narrowing: Narrowing<'_>,
        staged: Option<&Staged>,
    ) -> rusqlite::Result<bool> {
        Ok(self.lets_through(url, narrowing)
            && self.reached(connection, url, &self.asks(url), staged)?)
    }

    /// Whether the scope's patterns, and `narrowing`'s, let `url` through:
    /// the part of the scope that is fixed for the Run's life, which reads
    /// no run file (#2244).
    pub(crate) fn lets_through(&self, url: &Url, narrowing: Narrowing<'_>) -> bool {
        let patterns = Narrowing {
            include: &self.include,
            exclude: &self.exclude,
        };
        patterns.admits(url) && narrowing.admits(url)
    }

    /// What `url` asks the scope's seeds under its strategy, read from the
    /// URL alone (#2244).
    pub(crate) fn asks(&self, url: &Url) -> Asked {
        let measured = |measure: fn(&Url, String) -> Asked| {
            compared_host(url).map_or(Asked::NoHost, |host| measure(url, host))
        };
        match self.strategy {
            Strategy::All => Asked::Nothing,
            Strategy::SameHostname => measured(|_, host| Asked::Host(host)),
            Strategy::SameDomain => measured(|url, host| Asked::Domain(registrable(url, &host))),
            Strategy::SameOrigin => measured(|url, host| Asked::Origin {
                host,
                port: url.port().unwrap_or(0),
                scheme: url.scheme().to_owned(),
            }),
        }
    }

    /// Whether a seed reaches `url`, which asks `asked` ([`Scope::asks`]):
    /// a seed of the run file `connection` holds, or one of `staged`, an
    /// attempt's earlier staged admissions. The seeds are the Run's state,
    /// each seed admitted while the crawl runs widening them, so this is
    /// asked where that state is held (#2244).
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub(crate) fn reached(
        &self,
        connection: &Connection,
        url: &Url,
        asked: &Asked,
        staged: Option<&Staged>,
    ) -> rusqlite::Result<bool> {
        if staged.is_some_and(|staged| {
            staged
                .seeds()
                .iter()
                .any(|seed| self.strategy.reaches(seed, url))
        }) {
            return Ok(true);
        }
        let port;
        let params: &[&dyn rusqlite::ToSql] = match asked {
            Asked::Nothing => return Ok(true),
            Asked::NoHost => return Ok(false),
            Asked::Host(host) => &[host],
            Asked::Domain(domain) => &[domain],
            Asked::Origin {
                host,
                port: asked,
                scheme,
            } => {
                port = i64::from(*asked);
                &[host, &port, scheme]
            }
        };
        let Some(query) = self.strategy.query() else {
            return Ok(true);
        };
        connection
            .prepare_cached(query)
            .and_then(|mut cached| cached.query_row(params, |row| row.get::<_, bool>(0)))
    }

    /// Widens the scope of the run file `connection` holds by the seed
    /// `seed`.
    ///
    /// # Errors
    ///
    /// When the run file cannot be written.
    pub fn widen(connection: &Connection, seed: &Url) -> rusqlite::Result<()> {
        let Some(measures) = Measures::of(seed) else {
            return Ok(());
        };
        // Called for every seed: the connection keeps the statement
        // prepared rather than compiling it again each time.
        connection
            .prepare_cached(
                "INSERT INTO scope_seeds (host, port, scheme, domain) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT DO NOTHING",
            )?
            .execute((
                &measures.host,
                i64::from(measures.port),
                measures.scheme,
                &measures.domain,
            ))?;
        Ok(())
    }

    /// Records the scope's strategy and patterns in the run file
    /// `connection` holds, once, as the Run begins.
    ///
    /// # Errors
    ///
    /// When the run file cannot be written.
    pub fn record(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection
            .prepare_cached(
                "INSERT INTO scope (id, strategy, include, exclude) VALUES (1, ?1, ?2, ?3)",
            )
            .and_then(|mut cached| {
                cached.execute((
                    self.strategy.name(),
                    serde_json::to_string(&self.include.globs)
                        .expect("invariant: globs encode as JSON"),
                    serde_json::to_string(&self.exclude.globs)
                        .expect("invariant: globs encode as JSON"),
                ))
            })?;
        Ok(())
    }

    /// The scope the run file `connection` holds; the default one in a run
    /// file whose Run recorded none.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read, or holds a scope it cannot.
    pub fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let invalid = |error: Box<dyn std::error::Error + Send + Sync>| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error)
        };
        let row = connection
            .prepare_cached("SELECT strategy, include, exclude FROM scope WHERE id = 1")
            .and_then(|mut cached| {
                cached.query_row([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
            })
            .optional()?;
        let Some((strategy, include, exclude)) = row else {
            return Ok(Self::default());
        };
        let patterns = |globs: &str| -> rusqlite::Result<Patterns> {
            let globs: Vec<String> =
                serde_json::from_str(globs).map_err(|error| invalid(Box::new(error)))?;
            Patterns::globs(&globs).map_err(|error| invalid(Box::new(error)))
        };
        Ok(Self::new(
            Strategy::named(&strategy)
                .ok_or_else(|| invalid(format!("no scope strategy `{strategy}`").into()))?,
            patterns(&include)?,
            patterns(&exclude)?,
        ))
    }
}

#[cfg(test)]
mod tests;
