//! `xtask wording`: ADR 0035's wording policy over every text published about
//! either Product (ADR 0097). Each term of `wording.toml` ("bypass" in any
//! form, "undetectable", "undetected", "solve any CAPTCHA") fails in the
//! prose of each kind of text, whatever its case; code passes, and
//! `docs/adr/`, `docs/agents/`, `docs/testing/` and `rnd/` are not read.
//! Covers: story:1-122

use rstest::rstest;

use crate::workspace::{Finding, Workspace, finding};

fn wording(findings: &[&str]) -> Vec<Finding> {
    findings
        .iter()
        .map(|subject| finding("wording", subject))
        .collect()
}

#[test]
fn published_texts_without_a_term_pass() {
    let workspace = Workspace::from_fixture("wording");

    let outcome = workspace.xtask(&["wording"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

/// The Markdown fixture holds each term on its own line, from line 3, the
/// last one's words across a line break.
#[rstest]
#[case::docs_page("docs/collie/stealth.md")]
#[case::docs_site_page("website/release-notes/0.1.md")]
#[case::readme("sdks/typescript/collie/README.md")]
#[case::changelog("CHANGELOG.md")]
#[case::skill("skills/collie/SKILL.md")]
#[case::skill_reference("skills/collie/references/stealth.md")]
fn term_in_markdown_prose_fails(#[case] relative: &str) {
    let workspace = Workspace::from_fixture("wording");
    workspace.add_fixture_file("wording-cases/terms.md", relative);

    let outcome = workspace.xtask(&["wording"]);

    let lines = [3, 4, 5, 6].map(|line| format!("{relative}:{line}"));
    assert_eq!(
        outcome.findings(),
        wording(&lines.each_ref().map(String::as_str)),
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// Each fixture holds all four terms in the one text the subject names, or
/// one term in each of four texts.
#[rstest]
#[case::catalog_description(
    "catalog-operation.json",
    "crates/goodfetch-protocol/generated/catalog.json",
    &["crates/goodfetch-protocol/generated/catalog.json#/operations/0/description"; 4]
)]
#[case::error_code_message(
    "catalog-error-code.json",
    "crates/goodfetch-protocol/generated/catalog.json",
    &["crates/goodfetch-protocol/generated/catalog.json#/error_codes/0/description"; 4]
)]
#[case::settings_schema_description(
    "settings-schema.json",
    "crates/goodfetch-protocol/generated/bundle.json",
    &["crates/goodfetch-protocol/generated/bundle.json#/$defs/ProxySettings/properties/bypass/description"; 4]
)]
#[case::npm_package_keywords(
    "package-keywords.json",
    "sdks/typescript/collie/package.json",
    &[
        "sdks/typescript/collie/package.json#/keywords/1",
        "sdks/typescript/collie/package.json#/keywords/2",
        "sdks/typescript/collie/package.json#/keywords/3",
        "sdks/typescript/collie/package.json#/keywords/4",
    ]
)]
#[case::crate_description(
    "cargo-description.toml",
    "crates/goodfetch-collie/Cargo.toml",
    &["crates/goodfetch-collie/Cargo.toml#/package/description"; 4]
)]
#[case::pypi_package_keywords(
    "pyproject-keywords.toml",
    "sdks/python/collie/pyproject.toml",
    &[
        "sdks/python/collie/pyproject.toml#/project/keywords/1",
        "sdks/python/collie/pyproject.toml#/project/keywords/2",
        "sdks/python/collie/pyproject.toml#/project/keywords/3",
        "sdks/python/collie/pyproject.toml#/project/keywords/4",
    ]
)]
fn term_in_a_published_field_fails(
    #[case] fixture: &str,
    #[case] relative: &str,
    #[case] subjects: &[&str],
) {
    let workspace = Workspace::from_fixture("wording");
    workspace.add_fixture_file(&format!("wording-cases/{fixture}"), relative);

    let outcome = workspace.xtask(&["wording"]);

    assert_eq!(outcome.findings(), wording(subjects), "{}", outcome.stderr);
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::fenced_block("code-fenced.md")]
#[case::inline_code("code-inline.md")]
#[case::link_destination("code-link.md")]
fn term_in_code_passes(#[case] fixture: &str) {
    let workspace = Workspace::from_fixture("wording");
    workspace.add_fixture_file(&format!("wording-cases/{fixture}"), "docs/collie/proxy.md");

    let outcome = workspace.xtask(&["wording"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::decision_records("docs/adr/0035-stealth.md")]
#[case::agent_configuration("docs/agents/domain.md")]
#[case::frozen_archive("rnd/README.md")]
#[case::testing_sections("docs/testing/security.md")]
fn unread_folders_pass(#[case] relative: &str) {
    let workspace = Workspace::from_fixture("wording");
    workspace.add_fixture_file("wording-cases/terms.md", relative);

    let outcome = workspace.xtask(&["wording"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}
