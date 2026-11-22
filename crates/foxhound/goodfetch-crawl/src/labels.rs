//! The Labels a crawl declares (ADR 0093), kept in its run file as its
//! start declared them, so a resumed crawl takes each Label's settings
//! again: its `fetch.tier`, `fetch.encoding` and `session.page_check`,
//! and the Session pool its `session.source` gives it. Which requests keep
//! their URLs' fragments, by the Run's `crawl.keep_url_fragment` and each
//! Label's own, is kept beside them, so it is fixed for the Run's life
//! (ADRs 0102 and 0120).
//!
//! A request whose pool holds a login, from a `session.source` or a
//! Label's own `login`, sends its HTTP from its Session's browser whatever
//! `fetch.http_from` says, so the login is never shown under a second
//! client; an explicit `transport` where such a pool's Sessions open is
//! refused at the start with `setting_locked` (ADR 0083, #2418).

use std::collections::BTreeMap;

use goodfetch_engine::CallError;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::runs::FixedSettingChanged;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::{HttpFrom, LabelSettings};
use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_protocol::foxhound::fetch::FETCH_HTTP_FROM;
use goodfetch_protocol::foxhound::frontier::CRAWL_KEEP_URL_FRAGMENT;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{
    SESSION_SOURCE, SETTING_LOCKED, SettingLimit, SettingLockedDetails, SettingsLayer,
};
use goodfetch_session_pool::{LabelPools, PoolKey};
use rusqlite::{Connection, params};

/// What a crawl's declared Labels set: each one's tier, encoding and Page
/// check, which have a Session pool of their own, and which requests keep
/// their URLs' fragments; and whether the Run's pool holds a login.
#[derive(Clone, Debug, Default)]
pub(crate) struct Declared {
    pub(crate) settings: LabelSettings,
    pub(crate) pools: LabelPools,
    pub(crate) fragments: Fragments,
    /// The Run's `session.source`, its value and the layer that set it,
    /// when the Run's pool opens its Sessions from one.
    run_login: Option<SettingLimit>,
}

impl Declared {
    /// What `labels` set, with `fragments`, under the Run's `settings`.
    ///
    /// # Errors
    ///
    /// Those of [`LabelSettings::of`] and [`LabelPools::of`].
    pub(crate) fn of(
        labels: &BTreeMap<String, LabelDeclaration>,
        fragments: Fragments,
        settings: &Resolved,
    ) -> Result<Self, CallError> {
        Ok(Self {
            settings: LabelSettings::of(labels)?,
            pools: LabelPools::of(labels)?,
            fragments,
            run_login: run_login(settings),
        })
    }

    /// No Label, under the Run's `settings`: what a resumed crawl whose
    /// Labels no longer read takes, its requests on the Run's pool.
    pub(crate) fn none(settings: &Resolved) -> Self {
        Self {
            run_login: run_login(settings),
            ..Self::default()
        }
    }

    /// The setting that gives the Sessions of a request of `label` a login,
    /// from its pool: `None` when they hold none.
    pub(crate) fn login(&self, label: &str) -> Option<&SettingLimit> {
        match self.pools.pool(label) {
            PoolKey::Run => self.run_login.as_ref(),
            pool @ PoolKey::Label(_) => self.pools.login(&pool),
        }
    }

    /// Where the HTTP of a request of `label` leaves from where the engine
    /// chooses its tier: from the browser when its pool holds a login,
    /// else as its Label's `fetch.http_from`, else `run`, the Run's, says.
    pub(crate) fn http_from(&self, label: &str, run: HttpFrom) -> HttpFrom {
        if self.login(label).is_some() {
            HttpFrom::Browser
        } else {
            self.settings.http_from(label, run)
        }
    }

    /// `setting_locked` when `fetch.http_from: transport` is set where the
    /// Sessions of a pool that holds a login open: by the Run, at its own
    /// layer, on the Run's pool, or by a Label on its pool. A value the
    /// Run's pool's Sessions inherit from configuration, or a Label's own
    /// pool from the Run, yields, and its HTTP leaves from the browser
    /// (ADR 0083).
    ///
    /// # Errors
    ///
    /// `setting_locked`, naming `fetch.http_from` and the setting that
    /// gives the pool its login.
    pub(crate) fn sends_from_login_pools(&self, settings: &Resolved) -> Result<(), CallError> {
        let runs = settings
            .effective(&FETCH_HTTP_FROM)
            .filter(|effective| effective.layer == SettingsLayer::Run)
            .and_then(|effective| effective.value.as_str().and_then(HttpFrom::named));
        if runs == Some(HttpFrom::Transport)
            && let Some(login) = &self.run_login
        {
            return Err(locked(login, "the Run"));
        }
        for (label, http_from) in self.settings.own_http_from() {
            if http_from == HttpFrom::Transport
                && let Some(login) = self.login(label)
            {
                return Err(locked(login, &format!("the Label `{label}`")));
            }
        }
        Ok(())
    }
}

/// The Run's `session.source` under `settings`, when it gives the Run's
/// pool a login.
fn run_login(settings: &Resolved) -> Option<SettingLimit> {
    settings
        .effective(&SESSION_SOURCE)
        .filter(|effective| !effective.value.is_null())
        .map(|effective| SettingLimit {
            key: SESSION_SOURCE.name.to_owned(),
            value: effective.value.clone(),
            layer: effective.layer,
            origin: effective.origin,
        })
}

/// `setting_locked` for `fetch.http_from: transport`, set by `by`, on a
/// pool whose Sessions `login` gives a login.
fn locked(login: &SettingLimit, by: &str) -> CallError {
    let key = FETCH_HTTP_FROM.name;
    let limit = &login.key;
    CallError::new(
        &SETTING_LOCKED,
        format!(
            "{by} sets `{key}: transport` on a pool whose Sessions hold a login from `{limit}`: their HTTP leaves from the browser, so the login is never shown under a second client. Set `{key}` to `browser` or `learned` there, or give that pool no `{limit}`."
        ),
        &SettingLockedDetails {
            key: key.to_owned(),
            layer: SettingsLayer::Run,
            limit: login.clone(),
            fix: format!("`{key}: browser` or `learned` on that pool, or no `{limit}`"),
        },
    )
}

/// Which crawl requests keep their URLs' fragments in their keys (ADR
/// 0120): those of each Label that sets `crawl.keep_url_fragment`, as it
/// sets it, and every other's as the Run sets it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fragments {
    run: bool,
    labels: BTreeMap<String, bool>,
}

/// How the run file names the Run's own row of [`Fragments`]; a Label's
/// is its name after [`LABEL`].
const RUN: &str = "run";

/// What starts a Label's row of [`Fragments`] in the run file.
const LABEL: &str = "label:";

impl Fragments {
    /// What the Run's `settings` and its Labels' `labels` set.
    pub(crate) fn of(settings: &Resolved, labels: &LabelSettings) -> Self {
        Self {
            run: settings
                .effective(&CRAWL_KEEP_URL_FRAGMENT)
                .is_some_and(|effective| effective.value == serde_json::Value::Bool(true)),
            labels: labels
                .fragments()
                .map(|(label, keeps)| (label.to_owned(), keeps))
                .collect(),
        }
    }

    /// Whether a request of `label` keeps its URL's fragment.
    pub(crate) fn keeps(&self, label: &str) -> bool {
        self.labels.get(label).copied().unwrap_or(self.run)
    }

    /// Records these in the run file, as the crawl's start sets them.
    ///
    /// # Errors
    ///
    /// The run file's, when the rows cannot be written.
    pub(crate) fn record(&self, connection: &Connection) -> rusqlite::Result<()> {
        let mut insert = connection.prepare_cached(
            "INSERT OR REPLACE INTO url_fragments (scope, keeps) VALUES (?1, ?2)",
        )?;
        insert.execute(params![RUN, self.run])?;
        for (label, keeps) in &self.labels {
            insert.execute(params![format!("{LABEL}{label}"), keeps])?;
        }
        Ok(())
    }

    /// What [`record`](Self::record) kept; nothing kept keeps no fragment.
    ///
    /// # Errors
    ///
    /// The run file's, when the rows cannot be read.
    pub(crate) fn read(connection: &Connection) -> rusqlite::Result<Self> {
        let mut select = connection.prepare_cached("SELECT scope, keeps FROM url_fragments")?;
        let rows = select.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })?;
        let mut fragments = Self::default();
        for row in rows {
            let (scope, keeps) = row?;
            if scope == RUN {
                fragments.run = keeps;
            } else if let Some(label) = scope.strip_prefix(LABEL) {
                fragments.labels.insert(label.to_owned(), keeps);
            }
        }
        Ok(fragments)
    }

    /// `setting_not_applicable` when a start under the Run `run_id`'s ID
    /// sets `self` while the Run began with `began`: the setting is fixed
    /// for the Run's life, since the keys of the requests it admitted would
    /// no longer match new ones (ADRs 0102 and 0120).
    pub(crate) fn unchanged(&self, began: &Self, run_id: &RunId) -> Result<(), CallError> {
        if self == began {
            return Ok(());
        }
        Err(FixedSettingChanged {
            run_id: run_id.clone(),
            key: CRAWL_KEEP_URL_FRAGMENT.name,
            layer: SettingsLayer::Run,
        }
        .into())
    }
}

/// The table that keeps a crawl's declared Labels in every run file.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-crawl-labels",
    statements: &[
        "CREATE TABLE IF NOT EXISTS labels (
            name TEXT NOT NULL PRIMARY KEY,
            declaration TEXT NOT NULL
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS url_fragments (
            scope TEXT NOT NULL PRIMARY KEY,
            keeps INTEGER NOT NULL
        ) STRICT",
    ],
};

/// Records `labels`, the Labels the crawl's start declares.
///
/// # Errors
///
/// The run file's, when the rows cannot be written.
pub(crate) fn record(
    connection: &Connection,
    labels: &BTreeMap<String, LabelDeclaration>,
) -> rusqlite::Result<()> {
    let mut insert = connection
        .prepare_cached("INSERT OR REPLACE INTO labels (name, declaration) VALUES (?1, ?2)")?;
    for (name, declaration) in labels {
        let declaration = serde_json::to_string(declaration)
            .expect("invariant: a Label's declaration encodes as JSON");
        insert.execute(params![name, declaration])?;
    }
    Ok(())
}

/// The Labels the crawl's start declared, as [`record`] kept them; a row
/// that no longer reads is left out.
///
/// # Errors
///
/// The run file's, when the rows cannot be read.
pub(crate) fn read(
    connection: &Connection,
) -> rusqlite::Result<BTreeMap<String, LabelDeclaration>> {
    let mut select =
        connection.prepare_cached("SELECT name, declaration FROM labels ORDER BY name")?;
    let rows = select.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut labels = BTreeMap::new();
    for row in rows {
        let (name, declaration) = row?;
        match serde_json::from_str(&declaration) {
            Ok(declaration) => {
                labels.insert(name, declaration);
            }
            Err(error) => {
                tracing::warn!(%error, label = %name, "a declared Label in the run file no longer reads");
            }
        }
    }
    Ok(labels)
}

#[cfg(test)]
mod tests;
