//! Seam A tests for `xtask mcp check` (ADR 0070, ticket #414): each refused
//! case is a named fixture; a clean fixture passes.

use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::check;

fn write_file(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn clean_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write_file(
        root,
        "crates/goodfetch-mcp/src/revisions.rs",
        r"
pub const SUPPORTED: &[ProtocolVersion] = &[
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2025_11_25,
];
",
    );
    write_file(
        root,
        "conformance/mcp/expected-failures/2025-03-26.yaml",
        "server: []\n",
    );
    write_file(
        root,
        "conformance/mcp/expected-failures/2025-11-25.yaml",
        "server:\n  - tools-call-simple-text # Requires fixture tool test_simple_text\n",
    );
    write_file(
        root,
        "conformance/mcp/expected-failures/extensions.yaml",
        "server: []\n",
    );
    write_file(
        root,
        "conformance/mcp/expected-failures/fixture.yaml",
        "server: []\n",
    );
    write_file(
        root,
        "conformance/mcp/matrix.toml",
        r#"
[[cell]]
revision = "2025-03-26"
client = "rmcp"
transport = "stdio"

[[cell]]
revision = "2025-03-26"
client = "rmcp"
transport = "http"

[[cell]]
revision = "2025-11-25"
client = "rmcp"
transport = "stdio"

[[cell]]
revision = "2025-11-25"
client = "rmcp"
transport = "http"
"#,
    );
    dir
}

#[test]
fn clean_fixture_passes() {
    let fixture = clean_fixture();
    let findings = check(fixture.path()).unwrap();
    assert!(
        findings.is_empty(),
        "expected no findings, got: {:?}",
        findings
            .iter()
            .map(|f| (f.kind, &f.subject, &f.message))
            .collect::<Vec<_>>()
    );
}

#[test]
fn refuses_non_fixture_scenario_in_shipped_expected_failures() {
    let fixture = clean_fixture();
    write_file(
        fixture.path(),
        "conformance/mcp/expected-failures/2025-11-25.yaml",
        "server:\n  - server-initialize # Not allowed on shipped executable\n",
    );
    let findings = check(fixture.path()).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, "mcp-expected-failure-not-fixture-only");
    assert!(findings[0].subject.contains("server-initialize"));
}

#[test]
fn refuses_expected_failure_entry_without_reason() {
    let fixture = clean_fixture();
    write_file(
        fixture.path(),
        "conformance/mcp/expected-failures/2025-11-25.yaml",
        "server:\n  - tools-call-simple-text\n",
    );
    let findings = check(fixture.path()).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, "mcp-expected-failure-missing-reason");
    assert!(findings[0].subject.contains("tools-call-simple-text"));
}

#[test]
fn refuses_supported_revision_without_expected_failures_file() {
    let fixture = clean_fixture();
    fs::remove_file(
        fixture
            .path()
            .join("conformance/mcp/expected-failures/2025-11-25.yaml"),
    )
    .unwrap();
    let findings = check(fixture.path()).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, "mcp-missing-expected-failures-file");
    assert!(findings[0].subject.contains("2025-11-25.yaml"));
}

#[test]
fn refuses_supported_revision_without_matrix_cells() {
    let fixture = clean_fixture();
    write_file(
        fixture.path(),
        "conformance/mcp/matrix.toml",
        r#"
[[cell]]
revision = "2025-03-26"
client = "rmcp"
transport = "stdio"

[[cell]]
revision = "2025-03-26"
client = "rmcp"
transport = "http"
"#,
    );
    let findings = check(fixture.path()).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, "mcp-revision-missing-matrix-cells");
    assert!(findings[0].subject.contains("2025-11-25"));
}

#[test]
fn refuses_non_empty_fixture_expected_failures_file() {
    let fixture = clean_fixture();
    write_file(
        fixture.path(),
        "conformance/mcp/expected-failures/fixture.yaml",
        "server:\n  - tools-call-simple-text # Should never be expected to fail on fixture server\n",
    );
    let findings = check(fixture.path()).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].kind, "mcp-fixture-expected-failure-non-empty");
}
