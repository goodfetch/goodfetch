//! The Labels a crawl declares (ADR 0093), kept in its run file as its
//! start declared them, so a resumed crawl takes each Label's settings
//! again: its `fetch.tier`, `fetch.encoding` and `session.page_check`,
//! and the Session pool its `session.source` gives it. Which requests keep
//! their URLs' fragments, by the Run's `crawl.keep_url_fragment` and each
//! Label's own, is kept beside them, so it is fixed for the Run's life
//! (ADRs 0102 and 0120).

use std::collections::BTreeMap;

use goodfetch_engine::CallError;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::LabelSettings;
use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_protocol::foxhound::frontier::CRAWL_KEEP_URL_FRAGMENT;
use goodfetch_protocol::settings::{
    SETTING_NOT_APPLICABLE, SettingNotApplicableDetails, SettingsLayer,
};
use goodfetch_session_pool::LabelPools;
use rusqlite::{Connection, params};

/// What a crawl's declared Labels set: each one's tier, encoding and Page
/// check, which have a Session pool of their own, and which requests keep
/// their URLs' fragments.
#[derive(Clone, Debug, Default)]
pub(crate) struct Declared {
    pub(crate) settings: LabelSettings,
    pub(crate) pools: LabelPools,
    pub(crate) fragments: Fragments,
}

impl Declared {
    /// What `labels` set, with `fragments`.
    ///
    /// # Errors
    ///
    /// Those of [`LabelSettings::of`] and [`LabelPools::of`].
    pub(crate) fn of(
        labels: &BTreeMap<String, LabelDeclaration>,
        fragments: Fragments,
    ) -> Result<Self, CallError> {
        Ok(Self {
            settings: LabelSettings::of(labels)?,
            pools: LabelPools::of(labels)?,
            fragments,
        })
    }
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
                .get(CRAWL_KEEP_URL_FRAGMENT.name)
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
        let mut insert = connection
            .prepare("INSERT OR REPLACE INTO url_fragments (scope, keeps) VALUES (?1, ?2)")?;
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
        let mut select = connection.prepare("SELECT scope, keeps FROM url_fragments")?;
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
    pub(crate) fn unchanged(&self, began: &Self, run_id: &str) -> Result<(), CallError> {
        if self == began {
            return Ok(());
        }
        let key = CRAWL_KEEP_URL_FRAGMENT.name;
        Err(CallError::new(
            &SETTING_NOT_APPLICABLE,
            format!(
                "The Run `{run_id}` began with another `{key}`, which is fixed for a Run's life since the keys of the requests it admitted would no longer match: start it with the value it began with, or start a new Run."
            ),
            &SettingNotApplicableDetails {
                key: key.to_owned(),
                layer: SettingsLayer::Run,
                fix: format!("the `{key}` the Run began with"),
            },
        ))
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
    let mut insert =
        connection.prepare("INSERT OR REPLACE INTO labels (name, declaration) VALUES (?1, ?2)")?;
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
    let mut select = connection.prepare("SELECT name, declaration FROM labels ORDER BY name")?;
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
