//! The shared snapshot redactions and filters (ADR 0062): one list of rules,
//! committed once as `test-support/fixtures/snapshot-filters.json`, which
//! insta applies here and the TypeScript and Python packages apply for
//! Vitest and inline-snapshot, so the three snapshot tools redact alike.
//! The CLI's wire record (`goodfetch-collie`'s `wire_record` module) reads
//! the same file's patterns, and measures each message's bytes after them.
//!
//! A rule replaces either one of a run's values (the test's folder, the
//! executable's Product version and commit), matched literally, or every
//! match of a pattern (a Daemon service's name, socket folder and unit
//! folder, ULIDs and Schedule IDs, RFC 3339 instants, durations, port
//! numbers on loopback hosts). Values go first, then patterns, each in
//! the file's order. Patterns keep to the syntax Rust's `regex`, JavaScript
//! and Python's `re` read alike, with ASCII's word boundary in all three
//! (JavaScript's own), and replacements are literal text, holding neither
//! `$` nor `\`. The test's folder is matched as its path, its canonical path
//! and each written inside a JSON string, the forms an output prints it in.
//! A test also names the paths its own output prints, such as an
//! executable's, which are matched in the same forms before every other
//! rule: an executable names itself by the path the OS gives it, which is
//! canonical on Linux, so a build folder reached through a link prints in
//! another form than the path the test started it by. On Windows, which
//! joins a path with `\`, the separators of a path under the test's folder
//! or a named path are then written as `/`, as every other host joins them,
//! so one snapshot serves every host; insta's rules alone hold this one.

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
    /// The paths the test names, each with its replacement.
    paths: Vec<(String, String)>,
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

    /// The same values, with `path` (an executable's, say) replaced by
    /// `replacement` in each form an output prints it in, as the test's
    /// folder is.
    #[must_use]
    pub fn with_path(mut self, path: &str, replacement: &str) -> Self {
        self.paths.push((path.to_owned(), replacement.to_owned()));
        self
    }

    fn get(&self, value: RunValue) -> Option<&str> {
        match value {
            RunValue::Folder => self.folder.as_deref(),
            RunValue::ProductVersion => self.product_version.as_deref(),
            RunValue::Commit => self.commit.as_deref(),
        }
    }
}

/// `text` under the rules [`snapshot_settings`] gives insta for `values`,
/// applied as insta applies them: for a value a test compares without a
/// snapshot.
pub fn redact(text: &str, values: &SnapshotValues) -> String {
    apply(text, rules(values))
}

/// `text` with each of `rules` applied in turn, as insta applies its
/// filters.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn apply(text: &str, rules: Vec<(String, String)>) -> String {
    rules
        .into_iter()
        .fold(text.to_owned(), |text, (pattern, replacement)| {
            regex::Regex::new(&pattern)
                .expect("invariant: each rule's pattern compiles, which the Quick suite checks")
                .replace_all(&text, replacement.as_str())
                .into_owned()
        })
}

/// insta settings with the shared filters: the current settings, then
/// the paths the test names, `values` and the committed patterns. A test binds them around its
/// snapshot assertions.
pub fn snapshot_settings(values: &SnapshotValues) -> Settings {
    let mut settings = Settings::clone_current();
    for (pattern, replacement) in rules(values) {
        settings.add_filter(&pattern, replacement);
    }
    settings
}

/// The shared rules for `values` as insta's filters take them, a pattern
/// and its replacement each, in the order they apply, on this host.
fn rules(values: &SnapshotValues) -> Vec<(String, String)> {
    rules_joined_with(values, std::path::MAIN_SEPARATOR)
}

/// The shared rules for `values` on a host that joins paths with
/// `separator`: each path the test names, then each value, matched
/// literally, then, where the separator is `\`, the separators of the
/// paths under them written as `/`, then the committed patterns.
fn rules_joined_with(values: &SnapshotValues, separator: char) -> Vec<(String, String)> {
    let filters: SnapshotFilters = serde_json::from_str(COMMITTED)
        .expect("invariant: the committed snapshot filters parse, which the Quick suite checks");
    let mut rules = Vec::new();
    let mut path_replacements = Vec::new();
    for (path, replacement) in &values.paths {
        for form in path_forms(path) {
            rules.push((regex::escape(&form), replacement.clone()));
        }
        path_replacements.push(replacement.clone());
    }
    for filter in &filters.values {
        if let Some(value) = values.get(filter.value) {
            let forms = match filter.value {
                RunValue::Folder => {
                    path_replacements.push(filter.replacement.clone());
                    path_forms(value)
                }
                RunValue::ProductVersion | RunValue::Commit => vec![value.to_owned()],
            };
            for form in forms {
                rules.push((regex::escape(&form), filter.replacement.clone()));
            }
        }
    }
    if separator == '\\' {
        for replacement in &path_replacements {
            rules.extend(slash_separators(replacement));
        }
    }
    // `(?-u)`: a word boundary is ASCII's, as JavaScript's and Python's
    // under `re.ASCII` are.
    for filter in filters.patterns {
        rules.push((format!("(?-u){}", filter.pattern), filter.replacement));
    }
    rules
}

/// The most separators a path under a replaced one holds that its rules
/// write as `/`; a deeper path keeps its last `\`, which fails its
/// snapshot rather than passing it.
const DEEPEST_PATH: usize = 16;

/// The rules that write each separator of a path under `replacement` (the
/// test's folder's, say) as `/`, one separator a rule: Windows joins it with
/// `\`, written `\\` inside a JSON string, and every other host with `/`.
/// A path ends at a space, a quote or a backtick, and a backslash that no
/// path character follows, such as JSON's escaped quote, is no separator.
fn slash_separators(replacement: &str) -> Vec<(String, String)> {
    let pattern = format!(
        r#"({}[^\s"'`\\]*)\\{{1,2}}([^\s"'`\\])"#,
        regex::escape(replacement)
    );
    vec![(pattern, "${1}/${2}".to_owned()); DEEPEST_PATH]
}

/// The forms an output prints a path in, longest first so a longer form
/// is replaced before a shorter one inside it: the path, its canonical path
/// when it has another, and each written inside a JSON string.
fn path_forms(path: &str) -> Vec<String> {
    let mut paths = vec![path.to_owned()];
    if let Ok(canonical) = std::fs::canonicalize(path) {
        paths.push(canonical.display().to_string());
    }
    let mut forms: Vec<String> = paths
        .iter()
        .flat_map(|path| {
            let escaped = serde_json::to_string(path)
                .expect("invariant: a string serializes to JSON")
                .trim_matches('"')
                .to_owned();
            [path.clone(), escaped]
        })
        .collect();
    forms.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    forms.dedup();
    forms
}

#[cfg(test)]
mod tests;
