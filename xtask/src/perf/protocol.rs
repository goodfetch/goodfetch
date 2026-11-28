//! MCP protocol wire-byte counts (`tools/list` per profile and named workload
//! tool results, ADRs 0065 and 0071; issue 1360), extracted from the committed
//! `goodfetch-mcp` insta snapshots on the base and head trees so `xtask perf`
//! compares them as counted protocol figures.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::figure::{Figure, Measured};

/// Relative path to the committed per-profile `tools/list` wire-byte snapshot.
pub(crate) const PROFILE_TOOLS_LIST_SNAP: &str =
    "crates/goodfetch-mcp/src/snapshots/goodfetch_mcp__tests__profile_tools_list_bytes.snap";

/// Relative path to the committed named-workload tool-result wire-byte snapshot.
pub(crate) const WORKLOAD_RESULTS_SNAP: &str =
    "crates/goodfetch-mcp/src/snapshots/goodfetch_mcp__tests__workload_tool_result_bytes.snap";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    bytes: u64,
}

/// Whether `tree` holds both MCP protocol wire-byte snapshots.
pub(crate) fn has_snapshots(tree: &Path) -> bool {
    tree.join(PROFILE_TOOLS_LIST_SNAP).is_file() && tree.join(WORKLOAD_RESULTS_SNAP).is_file()
}

/// Extracts all MCP protocol wire-byte counts from `tree`'s committed snapshots
/// and writes a JSON report to `report`.
pub(crate) fn extract_tree(tree: &Path, report: &Path) -> Result<PathBuf> {
    let mut entries = BTreeMap::new();

    let profiles = read_insta_json(&tree.join(PROFILE_TOOLS_LIST_SNAP))?;
    let profiles_obj = profiles
        .as_object()
        .ok_or_else(|| anyhow!("profile_tools_list_bytes snapshot must be a JSON object"))?;
    for (product, per_profile) in profiles_obj {
        let Some(map) = per_profile.as_object() else {
            continue;
        };
        for (profile, stats) in map {
            let bytes = stats
                .get("tools_list_bytes")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    anyhow!("missing numeric `tools_list_bytes` for {product}/{profile}")
                })?;
            entries.insert(format!("mcp/tools-list/{product}/{profile}"), bytes);
        }
    }

    let workloads = read_insta_json(&tree.join(WORKLOAD_RESULTS_SNAP))?;
    let workloads_obj = workloads
        .as_object()
        .ok_or_else(|| anyhow!("workload_tool_result_bytes snapshot must be a JSON object"))?;
    for (workload, stats) in workloads_obj {
        let bytes = stats
            .get("wire_bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("missing numeric `wire_bytes` for workload `{workload}`"))?;
        entries.insert(format!("mcp/tool-result/{workload}"), bytes);
    }

    let list: Vec<Entry> = entries
        .into_iter()
        .map(|(name, bytes)| Entry { name, bytes })
        .collect();
    if let Some(parent) = report.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(report, serde_json::to_vec_pretty(&list)?)
        .with_context(|| format!("writing {}", report.display()))?;
    Ok(report.to_path_buf())
}

fn read_insta_json(path: &Path) -> Result<Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let body = strip_insta_header(&text)
        .ok_or_else(|| anyhow!("{} has no insta snapshot frontmatter", path.display()))?;
    serde_json::from_str(body).with_context(|| format!("parsing JSON body of {}", path.display()))
}

fn strip_insta_header(text: &str) -> Option<&str> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let end = rest
        .find("\n---\n")
        .map(|pos| pos + 5)
        .or_else(|| rest.find("\r\n---\r\n").map(|pos| pos + 7))?;
    Some(&rest[end..])
}

fn entries(path: &Path) -> Result<BTreeMap<String, u64>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let entries: Vec<Entry> =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(entries
        .into_iter()
        .map(|entry| (entry.name, entry.bytes))
        .collect())
}

/// The subjects of the entries of the report at `path`.
pub(crate) fn entry_subjects(path: &Path) -> Result<BTreeSet<String>> {
    Ok(entries(path)?.into_keys().collect())
}

/// Every head protocol count, paired with the base's count for the same
/// subject when a base report is given.
pub(crate) fn read(base: Option<&Path>, head: &Path) -> Result<Vec<Measured>> {
    let base = base.map(entries).transpose()?.unwrap_or_default();
    Ok(entries(head)?
        .into_iter()
        .map(|(subject, bytes)| Measured {
            base: base.get(&subject).copied(),
            subject,
            figure: Figure::WireBytes,
            head: bytes,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::compare::{self, Verdict};

    #[test]
    fn extracts_and_compares_mcp_protocol_counts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base_report = dir.path().join("protocol-base.json");
        let head_report = dir.path().join("protocol-head.json");

        // Extract real committed snapshots from the repository root.
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");
        assert!(has_snapshots(repo_root));
        extract_tree(repo_root, &base_report).expect("extract base");

        let mut base_entries: Vec<Entry> =
            serde_json::from_slice(&std::fs::read(&base_report).unwrap()).unwrap();
        assert!(
            base_entries
                .iter()
                .any(|e| e.name == "mcp/tools-list/collie/browser"),
            "expected collie browser tools/list entry"
        );
        assert!(
            base_entries
                .iter()
                .any(|e| e.name == "mcp/tool-result/run_start"),
            "expected run_start workload entry"
        );

        // Increase one entry by +10% to prove `compare::row` trips past the +5% budget.
        for entry in &mut base_entries {
            if entry.name == "mcp/tool-result/run_start" {
                entry.bytes = entry.bytes.saturating_mul(110) / 100 + 1;
            }
        }
        std::fs::write(
            &head_report,
            serde_json::to_vec_pretty(&base_entries).unwrap(),
        )
        .unwrap();

        let measured = read(Some(&base_report), &head_report).expect("read protocol counts");
        let rows: Vec<_> = measured.into_iter().map(compare::row).collect();
        let run_start = rows
            .iter()
            .find(|row| row.measured.subject == "mcp/tool-result/run_start")
            .expect("run_start row");
        assert_eq!(run_start.verdict, Verdict::Tripped);

        let browser = rows
            .iter()
            .find(|row| row.measured.subject == "mcp/tools-list/collie/browser")
            .expect("browser row");
        assert_eq!(browser.verdict, Verdict::Within);
    }
}
