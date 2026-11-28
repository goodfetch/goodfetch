//! Deployment template set's three readers (spec #583, issue 1262; ADRs 0049,
//! 0062, 0094 and 0097): a single template change propagates identically into
//! `xtask codegen`'s zstd-embedded `init --deploy` payload, `xtask docs-site
//! render`'s generated guide block, and `xtask deploy-templates smoke`'s
//! rendered files.

use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::deploy::{DeployImagePin, DeployTarget, DeployTemplateFile, render};

use crate::workspace::{Workspace, finding};

const JSON2TS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../node_modules/.bin/json2ts");
const DATAMODEL_CODEGEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../.venv/bin/datamodel-codegen"
);

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives under repository root")
}

/// Copies the repository's deployment template sources and a docs page with a
/// `{/* deploy: compose */}` directive into a fixture workspace.
fn copy_deploy_sources(workspace: &Workspace) {
    for rel in [
        "packaging/deploy/image-digests.toml",
        "packaging/deploy/compose/compose.yaml.tmpl",
        "packaging/deploy/compose/operator-policy.toml.tmpl",
        "packaging/oci/seccomp-profile.json",
    ] {
        let dest = workspace.path().join(rel);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(repo_root().join(rel), &dest).unwrap();
    }
    workspace.write(
        "docs/operations.md",
        b"---\ntitle: Operations\n---\n\n# Operations\n\n{/* deploy: compose */}\n",
    );
}

/// Extracts the three string arguments (`repository`, `default_digest`,
/// `nightly_digest`) emitted by `xtask codegen` into `generated/deploy.rs`.
fn parse_generated_pin(deploy_rs: &str) -> DeployImagePin {
    let mut quoted = deploy_rs
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('"') && line.ends_with("\","))
        .map(|line| {
            serde_json::from_str::<String>(line.trim_end_matches(','))
                .expect("valid string literal")
        });
    DeployImagePin {
        repository: quoted.next().expect("repository literal in deploy.rs"),
        default_digest: quoted.next().expect("default digest literal in deploy.rs"),
        nightly_digest: quoted.next().expect("nightly digest literal in deploy.rs"),
    }
}

/// Changing the single template source changes `xtask codegen`'s embedded
/// `init --deploy` payload, `xtask docs-site render`'s guide block, and
/// `xtask deploy-templates smoke`'s rendered files together.
///
/// Covers: story:13-53
#[test]
fn template_change_updates_codegen_docs_site_and_smoke_together() {
    let workspace = Workspace::from_fixture("codegen");
    copy_deploy_sources(&workspace);

    // Mutate the single template source.
    let compose_rel = "packaging/deploy/compose/compose.yaml.tmpl";
    let mutated = format!(
        "{}# template-marker-change\n",
        workspace.read_text(compose_rel)
    );
    workspace.write(compose_rel, mutated.as_bytes());

    // 1. Embedded `init --deploy` pipeline via `xtask codegen`:
    let codegen = workspace.xtask(&[
        "codegen",
        "--json2ts",
        JSON2TS,
        "--datamodel-codegen",
        DATAMODEL_CODEGEN,
    ]);
    assert_eq!(codegen.code, 0, "{}", codegen.stderr);

    let deploy_rs = workspace.read_text("crates/goodfetch-collie/generated/deploy.rs");
    let pin = parse_generated_pin(&deploy_rs);
    let mut decompressed: Vec<(&str, Vec<u8>)> = Vec::new();
    for name in [
        "compose.yaml",
        "operator-policy.toml",
        "seccomp-profile.json",
    ] {
        let zst_path = workspace.path().join(format!(
            "crates/goodfetch-collie/generated/deploy/compose/{name}.zst"
        ));
        let zst = std::fs::read(&zst_path).unwrap();
        let raw = zstd::stream::decode_all(zst.as_slice()).unwrap();
        decompressed.push((name, raw));
    }
    let templates: Vec<DeployTemplateFile<'_>> = decompressed
        .iter()
        .map(|(path, bytes)| DeployTemplateFile {
            path,
            contents: bytes.as_slice(),
        })
        .collect();
    let version = ProductVersion::parse("0.0.0").unwrap();
    let embedded_files = render(
        &templates,
        DeployTarget::Compose,
        None,
        Product::Collie,
        &version,
        &pin,
    )
    .unwrap();
    let embedded_compose = std::str::from_utf8(&embedded_files[0].contents).unwrap();
    let embedded_policy = std::str::from_utf8(&embedded_files[1].contents).unwrap();

    // 2. Docs generator via `xtask docs-site render`:
    let docs = workspace.xtask(&["docs-site", "render"]);
    assert_eq!(docs.code, 0, "{}", docs.stderr);
    let docs_collie = workspace.read_text("target/docs-site/source/docs/collie/operations.md");
    let docs_foxhound = workspace.read_text("target/docs-site/source/docs/foxhound/operations.md");

    // 3. Smoke-job renderer via `xtask deploy-templates smoke`:
    let smoke = workspace.xtask(&[
        "deploy-templates",
        "smoke",
        "--product",
        "collie",
        "--target",
        "compose",
        "--out",
        "smoke-out",
    ]);
    assert_eq!(smoke.code, 0, "{}", smoke.stderr);
    let smoke_compose = workspace.read_text("smoke-out/compose.yaml");
    let smoke_policy = workspace.read_text("smoke-out/operator-policy.toml");

    // All three readers match byte-for-byte and carry the template mutation.
    assert_eq!(embedded_compose, smoke_compose);
    assert_eq!(embedded_policy, smoke_policy);
    assert!(docs_collie.contains(embedded_compose), "{docs_collie}");
    assert!(docs_collie.contains(embedded_policy), "{docs_collie}");
    assert!(docs_foxhound.contains("ghcr.io/goodfetch/foxhound:0.0.0@sha256:"));
    assert!(docs_foxhound.contains("# template-marker-change"));
    assert!(embedded_compose.contains("# template-marker-change"));

    // Redacted snapshot of all three outputs together (ADR 0062).
    let combined = format!(
        "=== init (codegen embedded) ===\n{embedded_compose}\n=== smoke ===\n{smoke_compose}\n=== docs-site (collie) ===\n{docs_collie}"
    );
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(r"sha256:[0-9a-f]{64}", "sha256:[IMAGE_DIGEST]");
    settings.add_filter(
        r"(Goodfetch Collie |ghcr\.io/goodfetch/collie:)0\.0\.0",
        "${1}[PRODUCT_VERSION]",
    );
    settings.bind(|| {
        insta::assert_snapshot!("three_readers_share_single_template_source", combined);
    });
}

#[test]
fn docs_site_render_rejects_unknown_deploy_directive_without_writing() {
    let workspace = Workspace::from_fixture("docs-site");
    workspace.write(
        "docs/operations.md",
        b"---\ntitle: Operations\n---\n\n# Operations\n\n{/* deploy: unknown */}\n",
    );

    let outcome = workspace.xtask(&["docs-site", "render"]);
    assert_ne!(outcome.code, 0, "expected failure on unknown deploy target");
    assert_eq!(
        outcome.findings(),
        vec![finding("docs-site", "docs/operations.md:7")]
    );
    assert!(!workspace.path().join("target/docs-site/source").exists());
}
