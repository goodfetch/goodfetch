//! Generated per-Product reference pages and `xtask docs-samples check`
//! (ADRs 0050, 0068, 0095 and 0097; issue 150).

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::workspace::Workspace;

/// `xtask docs-site render` generates per-Product reference pages for every
/// Operation, Setting, and Metric: Foxhound's section carries both Collie's
/// and Foxhound's entries, while Collie's section carries only Collie's
/// entries (no Foxhound-only setting like `crawl.max_pages`), and each
/// Product's reference index links `TypeDoc`, Griffe, docs.rs and clap-markdown.
///
/// Covers: story:1-41, story:1-42, story:1-43, story:1-47, story:1-119
#[test]
fn docs_site_render_generates_scoped_reference_pages_for_each_product() {
    let workspace = Workspace::from_fixture("docs-site");

    let output = workspace.xtask(&["docs-site", "render"]);
    assert_eq!(output.code, 0, "{}", output.stderr);

    let docs = workspace.path().join("target/docs-site/source/docs");

    // Both Products get a reference index linking TypeDoc, Griffe, docs.rs and clap-markdown.
    for (product, pkg) in [
        ("collie", "goodfetch-collie"),
        ("foxhound", "goodfetch-foxhound"),
    ] {
        let index = fs::read_to_string(docs.join(product).join("reference/index.md")).unwrap();
        assert!(index.contains("TypeDoc"), "{product}: {index}");
        assert!(index.contains("Griffe"), "{product}: {index}");
        assert!(index.contains("docs.rs"), "{product}: {index}");
        assert!(index.contains("clap-markdown"), "{product}: {index}");
        assert!(index.contains(pkg), "{product}: {index}");
    }

    // Collie operation (`run.start`) appears in both Collie and Foxhound references.
    let collie_op =
        fs::read_to_string(docs.join("collie/reference/operations/run-start.md")).unwrap();
    let foxhound_op =
        fs::read_to_string(docs.join("foxhound/reference/operations/run-start.md")).unwrap();
    assert!(collie_op.contains("Goodfetch Collie"));
    assert!(collie_op.contains("Added in:"));
    assert!(collie_op.contains("### TypeScript"));
    assert!(collie_op.contains("### Python"));
    assert!(collie_op.contains("### Rust"));
    assert!(collie_op.contains("goodfetch-collie run start"));
    assert!(foxhound_op.contains("goodfetch-foxhound run start"));

    // Foxhound-only setting (`crawl.max_requests`) appears ONLY in Foxhound's reference.
    assert!(
        !docs
            .join("collie/reference/settings/crawl-max-requests.md")
            .exists(),
        "Collie reference must not include Foxhound-only setting crawl.max_requests"
    );
    let fox_setting =
        fs::read_to_string(docs.join("foxhound/reference/settings/crawl-max-requests.md")).unwrap();
    assert!(fox_setting.contains("Goodfetch Foxhound"));
    assert!(fox_setting.contains("Added in:"));

    // Metrics appear under `reference/metrics/`.
    let collie_metrics: Vec<_> = fs::read_dir(docs.join("collie/reference/metrics"))
        .unwrap()
        .collect();
    assert!(!collie_metrics.is_empty(), "Collie metrics generated");

    // Sidebars register the Reference category for both Products.
    let sidebars: Value = serde_json::from_str(
        &fs::read_to_string(
            workspace
                .path()
                .join("target/docs-site/source/sidebars.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let collie_bar = sidebars["collie"].as_array().unwrap();
    let fox_bar = sidebars["foxhound"].as_array().unwrap();
    assert!(collie_bar.iter().any(|item| item["label"] == "Reference"));
    assert!(fox_bar.iter().any(|item| item["label"] == "Reference"));
}

/// `xtask docs-samples check` passes when `manifest.json` matches the docs and
/// README sample count, and fails when a published package README sample is
/// missing from `manifest.json` (issue 150, comment of 2026-10-17).
///
/// Covers: story:1-43, story:1-119
#[test]
fn docs_samples_check_fails_when_a_readme_sample_is_missing_from_manifest() {
    let workspace = Workspace::from_fixture("docs-samples");
    // Add a published package with a README containing a sample block.
    let pkg = workspace.path().join("sdks/typescript/collie");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("package.json"), r#"{"name":"@goodfetch/collie"}"#).unwrap();
    fs::write(
        pkg.join("README.md"),
        "# Collie\n\n```ts\nimport { Collie } from \"@goodfetch/collie\";\n```\n",
    )
    .unwrap();

    let extract = workspace.xtask(&["docs-samples", "extract"]);
    assert_eq!(extract.code, 0, "{}", extract.stderr);

    let ok = workspace.xtask(&["docs-samples", "check"]);
    assert_eq!(ok.code, 0, "{}", ok.stderr);

    // Drop the README sample from manifest.json and verify `docs-samples check` refuses it.
    let manifest_path = workspace.path().join("target/docs-samples/manifest.json");
    let mut manifest: Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let samples = manifest["samples"].as_array_mut().unwrap();
    let before = samples.len();
    samples.retain(|sample| sample["page"].as_str() != Some("sdks/typescript/collie/README.md"));
    assert!(
        samples.len() < before,
        "README sample was extracted into manifest.json"
    );
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let red = workspace.xtask(&["docs-samples", "check"]);
    assert_ne!(red.code, 0, "missing README sample must fail");
    let combined = format!("{}{}", red.stdout, red.stderr);
    assert!(
        combined.contains("sdks/typescript/collie/README.md"),
        "failure should name the missing README sample: {combined}"
    );

    // Also verify `--language-check` catches an extra sample missing from manifest.json.
    let lc = workspace.path().join("target/language-check.json");
    write_json(
        &lc,
        &json!({
            "samples": [
                {
                    "page": "sdks/typescript/collie/README.md",
                    "rendering": "collie",
                    "line": 3,
                    "language": "ts"
                }
            ]
        }),
    );
    let red_lc = workspace.xtask(&[
        "docs-samples",
        "check",
        "--language-check",
        lc.to_str().unwrap(),
    ]);
    assert_ne!(red_lc.code, 0, "mismatch with language-check fails");
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}
