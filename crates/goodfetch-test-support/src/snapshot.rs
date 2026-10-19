//! The shared snapshot redactions and filters (ADR 0062): one list of rules,
//! committed once as `test-support/fixtures/snapshot-filters.json`, which
//! insta applies here and the TypeScript and Python packages apply for
//! Vitest and inline-snapshot, so the three snapshot tools redact alike.
//!
//! A rule replaces either one of a run's values (the test's folder, the
//! executable's Product version and commit), matched literally, or every
//! match of a pattern (ULIDs and Schedule IDs, RFC 3339 instants, durations,
//! port numbers on loopback hosts). Values go first, then patterns, each in
//! the file's order. Patterns keep to the syntax Rust's `regex`, JavaScript
//! and Python's `re` read alike, and replacements are literal text, holding
//! neither `$` nor `\`.

use insta::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::env::TestEnv;

/// The committed rules, as their file holds them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotFilters {
    /// The run's values to replace, first.
    values: Vec<ValueFilter>,
    /// The patterns to replace, after the values.
    patterns: Vec<PatternFilter>,
}

/// A rule that replaces one of a run's values wherever it appears.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ValueFilter {
    /// Which value.
    value: RunValue,
    /// Its replacement, literal text.
    replacement: String,
}

/// A run's value that its snapshots redact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum RunValue {
    /// The test's folder.
    Folder,
    /// The executable's Product version.
    ProductVersion,
    /// The commit the executable was built from.
    Commit,
}

/// A rule that replaces every match of a pattern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PatternFilter {
    /// The rule's name, in snake case.
    name: String,
    /// The regular expression.
    pattern: String,
    /// Its replacement, literal text.
    replacement: String,
}

/// The committed rules' text.
const COMMITTED: &str = include_str!("../../../test-support/fixtures/snapshot-filters.json");

/// The values of one run that its snapshots redact beside the shared
/// patterns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnapshotValues {
    folder: Option<String>,
    product_version: Option<String>,
    commit: Option<String>,
}

impl SnapshotValues {
    /// The values of the test `env` sets up: its folder.
    pub fn of(env: &TestEnv) -> Self {
        Self {
            folder: Some(env.folder().display().to_string()),
            ..Self::default()
        }
    }

    /// The same values, with the Product version and commit the test's
    /// executable reports.
    #[must_use]
    pub fn with_executable(self, product_version: &str, commit: &str) -> Self {
        Self {
            product_version: Some(product_version.to_owned()),
            commit: Some(commit.to_owned()),
            ..self
        }
    }

    fn get(&self, value: RunValue) -> Option<&str> {
        match value {
            RunValue::Folder => self.folder.as_deref(),
            RunValue::ProductVersion => self.product_version.as_deref(),
            RunValue::Commit => self.commit.as_deref(),
        }
    }
}

/// insta settings with the shared filters: the current settings, then
/// `values` and the committed patterns. A test binds them around its
/// snapshot assertions.
pub fn snapshot_settings(values: &SnapshotValues) -> Settings {
    let filters: SnapshotFilters = serde_json::from_str(COMMITTED)
        .expect("invariant: the committed snapshot filters parse, which the Quick suite checks");
    let mut settings = Settings::clone_current();
    for filter in &filters.values {
        if let Some(value) = values.get(filter.value) {
            settings.add_filter(&regex::escape(value), filter.replacement.as_str());
        }
    }
    for filter in &filters.patterns {
        settings.add_filter(&filter.pattern, filter.replacement.as_str());
    }
    settings
}

#[cfg(test)]
mod tests;
