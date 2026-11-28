//! `xtask mcp check` (Quick, ADR 0070, ticket #414): verifies that every
//! supported MCP revision in `crates/goodfetch-mcp/src/revisions.rs` has:
//! - its shipped-executable expected-failures file
//!   `conformance/mcp/expected-failures/<revision>.yaml`,
//! - only fixture-only scenarios (or fixture-only per-check entries) listed in
//!   each shipped expected-failures file, each with a non-empty `# <reason>`
//!   comment,
//! - an empty `conformance/mcp/expected-failures/fixture.yaml` for the
//!   test-only fixture server,
//! - a valid `conformance/mcp/expected-failures/extensions.yaml`,
//! - both `stdio` and `http` client matrix cells in `conformance/mcp/matrix.toml`.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::{self, Finding};
use crate::environment::Environment;

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Verifies MCP conformance expected-failures files and client matrix
    /// coverage for every supported revision (ADR 0070).
    Check,
}

const REVISIONS_SOURCE: &str = "crates/goodfetch-mcp/src/revisions.rs";
const EXPECTED_FAILURES_DIR: &str = "conformance/mcp/expected-failures";
const MATRIX_FILE: &str = "conformance/mcp/matrix.toml";

/// Scenarios (or per-check entries `<scenario>:<check-id>`) allowed in the
/// shipped Collie executable's expected-failures files: ONLY those that require
/// fixture tools, fixture resources, fixture prompts, or server-initiated
/// capabilities that Collie's shipped MCP server does not expose (ADR 0070).
const FIXTURE_ONLY_ENTRIES: &[&str] = &[
    // Logging / completions / prompts / subscriptions (not advertised by Collie)
    "logging-set-level",
    "completion-complete",
    "prompts-list",
    "prompts-get-simple",
    "prompts-get-with-args",
    "prompts-get-embedded-resource",
    "prompts-get-with-image",
    "resources-subscribe",
    "resources-unsubscribe",
    "caching:sep-2549-prompts-list-caching-hints",
    // Fixture tools (test_*)
    "tools-call-simple-text",
    "tools-call-image",
    "tools-call-audio",
    "tools-call-embedded-resource",
    "tools-call-mixed-content",
    "tools-call-with-logging",
    "tools-call-error",
    "tools-call-with-progress",
    "tools-call-sampling",
    "tools-call-elicitation",
    "elicitation-sep1034-defaults",
    "elicitation-sep1330-enums",
    // Fixture resources (test://*)
    "resources-read-text",
    "resources-read-binary",
    "resources-templates-read",
    // Fixture-tool sub-checks of `server-stateless`
    "server-stateless:sep-2575-server-rejects-undeclared-capability",
    "server-stateless:sep-2575-missing-capability-http-400",
    "server-stateless:sep-2575-http-server-no-independent-requests-on-stream",
    "server-stateless:sep-2575-server-no-log-without-loglevel",
    // Fixture input-required-result tools/prompts (2026-07-28)
    "input-required-result-basic-elicitation",
    "input-required-result-basic-sampling",
    "input-required-result-basic-list-roots",
    "input-required-result-request-state",
    "input-required-result-multiple-input-requests",
    "input-required-result-multi-round",
    "input-required-result-missing-input-response",
    "input-required-result-non-tool-request",
    "input-required-result-result-type",
    "input-required-result-unsupported-methods",
    "input-required-result-tampered-state",
    "input-required-result-capability-check",
    "input-required-result-ignore-extra-params",
    "input-required-result-validate-input",
];

/// Extension checks allowed in `conformance/mcp/expected-failures/extensions.yaml`.
const ALLOWED_EXTENSION_ENTRIES: &[&str] =
    &["sep-2640-skills-enumeration:sep-2640-skill-uri-scheme"];

#[derive(Debug, Deserialize)]
struct ExpectedFailuresYaml {
    #[serde(default)]
    server: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MatrixToml {
    #[serde(default)]
    cell: Vec<MatrixCell>,
}

#[derive(Debug, Deserialize)]
struct MatrixCell {
    revision: String,
    client: String,
    transport: String,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Check => Ok(check::report(&check(&environment.root)?)),
    }
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    let revisions = read_supported_revisions(root)?;
    if revisions.is_empty() {
        findings.push(Finding::new(
            "mcp-no-supported-revisions",
            REVISIONS_SOURCE,
            "no `ProtocolVersion::V_*` entries found in `SUPPORTED`",
        ));
        return Ok(findings);
    }

    let expected_dir = root.join(EXPECTED_FAILURES_DIR);

    for revision in &revisions {
        let rel = format!("{EXPECTED_FAILURES_DIR}/{revision}.yaml");
        let path = expected_dir.join(format!("{revision}.yaml"));
        if !path.is_file() {
            findings.push(Finding::new(
                "mcp-missing-expected-failures-file",
                rel,
                format!("supported MCP revision `{revision}` has no expected-failures file"),
            ));
            continue;
        }
        check_expected_failures_file(&path, &rel, FIXTURE_ONLY_ENTRIES, false, &mut findings)?;
    }

    let extensions_rel = format!("{EXPECTED_FAILURES_DIR}/extensions.yaml");
    let extensions_path = expected_dir.join("extensions.yaml");
    if extensions_path.is_file() {
        check_expected_failures_file(
            &extensions_path,
            &extensions_rel,
            ALLOWED_EXTENSION_ENTRIES,
            false,
            &mut findings,
        )?;
    } else {
        findings.push(Finding::new(
            "mcp-missing-expected-failures-file",
            extensions_rel,
            "missing `extensions.yaml` expected-failures file",
        ));
    }

    let fixture_rel = format!("{EXPECTED_FAILURES_DIR}/fixture.yaml");
    let fixture_path = expected_dir.join("fixture.yaml");
    if fixture_path.is_file() {
        check_expected_failures_file(&fixture_path, &fixture_rel, &[], true, &mut findings)?;
    } else {
        findings.push(Finding::new(
            "mcp-missing-expected-failures-file",
            fixture_rel,
            "missing `fixture.yaml` expected-failures file for the fixture server",
        ));
    }

    check_matrix(root, &revisions, &mut findings)?;

    Ok(findings)
}

fn read_supported_revisions(root: &Path) -> Result<Vec<String>> {
    let path = root.join(REVISIONS_SOURCE);
    let source =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut revisions = Vec::new();
    let mut in_supported = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("pub const SUPPORTED") {
            in_supported = true;
            continue;
        }
        if in_supported {
            if trimmed.starts_with("];") {
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("ProtocolVersion::V_") {
                let raw = rest.trim_end_matches(',').trim();
                let date = raw.replace('_', "-");
                if !date.is_empty() {
                    revisions.push(date);
                }
            }
        }
    }
    Ok(revisions)
}

fn check_expected_failures_file(
    path: &Path,
    rel: &str,
    allowed: &[&str],
    must_be_empty: bool,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed: ExpectedFailuresYaml = match serde_saphyr::from_str(&raw) {
        Ok(parsed) => parsed,
        Err(error) => {
            findings.push(Finding::new(
                "mcp-invalid-expected-failures-file",
                rel,
                format!("invalid YAML: {error}"),
            ));
            return Ok(());
        }
    };

    if must_be_empty {
        for entry in &parsed.server {
            findings.push(Finding::new(
                "mcp-fixture-expected-failure-non-empty",
                format!("{rel}#{entry}"),
                "the fixture server's expected-failures file must be empty (ADR 0070)",
            ));
        }
        return Ok(());
    }

    let reasons = extract_inline_reasons(&raw);
    for entry in &parsed.server {
        if !allowed.contains(&entry.as_str()) {
            findings.push(Finding::new(
                "mcp-expected-failure-not-fixture-only",
                format!("{rel}#{entry}"),
                format!(
                    "`{entry}` does not require fixture tools or resources and must not be baselined on the shipped executable"
                ),
            ));
        }
        if reasons
            .get(entry)
            .is_none_or(|reason| reason.trim().is_empty())
        {
            findings.push(Finding::new(
                "mcp-expected-failure-missing-reason",
                format!("{rel}#{entry}"),
                format!(
                    "expected-failure entry `{entry}` must carry an inline `# <reason>` comment"
                ),
            ));
        }
    }
    Ok(())
}

/// Maps each `- <entry>` line in a YAML list to its inline `# <reason>` comment.
fn extract_inline_reasons(raw: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("- ") else {
            continue;
        };
        let (entry_raw, reason) = match rest.split_once('#') {
            Some((left, right)) => (left.trim(), right.trim().to_owned()),
            None => (rest.trim(), String::new()),
        };
        let entry = entry_raw.trim_matches('"').trim_matches('\'').to_owned();
        if !entry.is_empty() {
            map.insert(entry, reason);
        }
    }
    map
}

fn check_matrix(root: &Path, revisions: &[String], findings: &mut Vec<Finding>) -> Result<()> {
    let path = root.join(MATRIX_FILE);
    if !path.is_file() {
        findings.push(Finding::new(
            "mcp-missing-matrix-file",
            MATRIX_FILE,
            "missing MCP client matrix manifest `conformance/mcp/matrix.toml`",
        ));
        return Ok(());
    }
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let matrix: MatrixToml = match toml::from_str(&raw) {
        Ok(matrix) => matrix,
        Err(error) => {
            findings.push(Finding::new(
                "mcp-invalid-matrix-file",
                MATRIX_FILE,
                format!("invalid TOML: {error}"),
            ));
            return Ok(());
        }
    };

    let mut transports_by_rev: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for cell in &matrix.cell {
        if cell.client.trim().is_empty() {
            findings.push(Finding::new(
                "mcp-invalid-matrix-file",
                MATRIX_FILE,
                format!(
                    "matrix cell for `{}` has an empty client name",
                    cell.revision
                ),
            ));
        }
        if !matches!(cell.transport.as_str(), "stdio" | "http") {
            findings.push(Finding::new(
                "mcp-invalid-matrix-file",
                MATRIX_FILE,
                format!(
                    "matrix cell for `{}` has unknown transport `{}` (expected `stdio` or `http`)",
                    cell.revision, cell.transport
                ),
            ));
        }
        transports_by_rev
            .entry(cell.revision.as_str())
            .or_default()
            .insert(cell.transport.as_str());
    }

    for revision in revisions {
        let covered = transports_by_rev.get(revision.as_str());
        let has_stdio = covered.is_some_and(|set| set.contains("stdio"));
        let has_http = covered.is_some_and(|set| set.contains("http"));
        if !has_stdio || !has_http {
            findings.push(Finding::new(
                "mcp-revision-missing-matrix-cells",
                format!("{MATRIX_FILE}#{revision}"),
                format!(
                    "supported MCP revision `{revision}` must declare both `stdio` and `http` client matrix cells"
                ),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
