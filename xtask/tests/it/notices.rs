//! `xtask notices`: the third-party notice inventory (ADRs 0018 and 0047).
//! It lists every dependency of Cargo, npm and Python and every declared
//! input (the Console's bundled assets, the Goodfetch Chromium's inputs, the
//! Clean-up rules' inputs and the block-detection rules' sources) with its
//! license, and fails on a license outside ADR 0018's allowlist, naming the
//! dependency and its ecosystem.
//! Covers: story:1-62, story:1-101

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// Where `pnpm install` leaves each npm package's manifest.
const PERMISSIVE_PACKAGE: &str = "node_modules/.pnpm/permissive-package@1.0.0_peer-package@4.0.0/node_modules/permissive-package/package.json";

/// Where `uv sync` leaves each Python distribution's metadata.
const PERMISSIVE_DIST: &str =
    ".venv/lib/python3.14/site-packages/permissive_dist-1.0.0.dist-info/METADATA";

/// The fixture's installed packages, as `pnpm install` and `uv sync` leave
/// them, each `(fixture file, path in the workspace)`. The platform package
/// is for AIX, and the Windows-only distribution is not installed either.
const INSTALLED: [(&str, &str); 8] = [
    (
        "notices-installed/permissive-package.json",
        PERMISSIVE_PACKAGE,
    ),
    (
        "notices-installed/scoped-transitive-package.json",
        "node_modules/.pnpm/@scoped+transitive-package@2.0.0/node_modules/@scoped/transitive-package/package.json",
    ),
    (
        "notices-installed/peer-package.json",
        "node_modules/.pnpm/peer-package@4.0.0/node_modules/peer-package/package.json",
    ),
    (
        "notices-installed/dev-only-tool.json",
        "node_modules/.pnpm/dev-only-tool@1.0.0/node_modules/dev-only-tool/package.json",
    ),
    (
        "notices-installed/permissive_dist-1.0.0.METADATA",
        PERMISSIVE_DIST,
    ),
    (
        "notices-installed/speed_dist-2.3.0.METADATA",
        ".venv/lib/python3.14/site-packages/speed_dist-2.3.0.dist-info/METADATA",
    ),
    (
        "notices-installed/dev_only_dist-5.0.0.METADATA",
        ".venv/lib/python3.14/site-packages/dev_only_dist-5.0.0.dist-info/METADATA",
    ),
    (
        "notices-installed/typing_extensions-4.16.0.METADATA",
        ".venv/lib/python3.14/site-packages/typing_extensions-4.16.0.dist-info/METADATA",
    ),
];

/// The `notices` fixture with its packages installed.
fn installed_workspace() -> Workspace {
    let workspace = Workspace::from_fixture("notices");
    for (fixture, relative) in INSTALLED {
        workspace.add_fixture_file(fixture, relative);
    }
    workspace
}

#[test]
fn allowed_dependencies_and_inputs_pass() {
    let workspace = installed_workspace();

    let outcome = workspace.xtask(&["notices"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// Each case gives one dependency or input a license outside the allowlist.
#[rstest]
#[case::cargo(
    "vendor/permissive-crate/Cargo.toml",
    r#"license = "MIT OR Apache-2.0""#,
    r#"license = "SSPL-1.0""#,
    "cargo permissive-crate@1.0.0"
)]
#[case::npm(
    PERMISSIVE_PACKAGE,
    r#""license": "MIT""#,
    r#""license": "SSPL-1.0""#,
    "npm permissive-package@1.0.0"
)]
#[case::python(
    PERMISSIVE_DIST,
    "License-Expression: BSD-3-Clause",
    "License-Expression: SSPL-1.0",
    "python permissive-dist@1.0.0"
)]
#[case::console_asset(
    "console/third-party.toml",
    r#"license = "ISC""#,
    r#"license = "CC-BY-NC-4.0""#,
    "console-assets Lucide icons@0.900.0"
)]
#[case::chromium_input(
    "chromium/third-party.toml",
    r#"license = "Unicode-3.0""#,
    r#"license = "SSPL-1.0""#,
    "chromium ICU@77.1"
)]
#[case::clean_up_rules_input(
    "clean-up-rules/third-party.toml",
    r#"license = "Apache-2.0 AND MPL-2.0""#,
    r#"license = "Apache-2.0 AND CC-BY-NC-SA-4.0""#,
    "clean-up-rules DuckDuckGo's tracker surrogates@1d2e3f4"
)]
#[case::cc_by_sa_code(
    "clean-up-rules/third-party.toml",
    r#"license = "Apache-2.0 AND MPL-2.0""#,
    r#"license = "CC-BY-SA-4.0""#,
    "clean-up-rules DuckDuckGo's tracker surrogates@1d2e3f4"
)]
#[case::block_detection_rule_source(
    "block-detection/third-party.toml",
    r#"license = "BSD-3-Clause""#,
    r#"license = "SSPL-1.0""#,
    "block-detection-rules wafw00f@69fbe39"
)]
fn license_outside_the_allowlist_fails_with_the_dependency_and_its_ecosystem(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] subject: &str,
) {
    let workspace = installed_workspace();
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["notices"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("license-not-allowed", subject)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// The inventory `notices --out` wrote, as JSON.
fn inventory(workspace: &Workspace) -> serde_json::Value {
    serde_json::from_str(&workspace.read_text("target/notices.json")).unwrap()
}

/// The one notice of `name` in `ecosystem`.
fn notice<'a>(
    inventory: &'a serde_json::Value,
    ecosystem: &str,
    name: &str,
) -> &'a serde_json::Value {
    let matching: Vec<_> = inventory["notices"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|notice| notice["ecosystem"] == ecosystem && notice["name"] == name)
        .collect();
    assert_eq!(matching.len(), 1, "{ecosystem} {name} in {inventory:#}");
    matching[0]
}

#[test]
fn inventory_lists_every_dependency_and_input_with_its_license() {
    let workspace = installed_workspace();

    let outcome = workspace.xtask(&["notices", "--out", "target/notices.json"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    insta::assert_snapshot!(workspace.read_text("target/notices.json"));
}

/// An MPL-2.0 entry's notice says where its source is (ADR 0047).
#[rstest]
#[case::crate_from_its_registry("cargo", "mpl-crate", "https://crates.io/crates/mpl-crate/2.1.0")]
#[case::declared_input(
    "clean-up-rules",
    "autoconsent",
    "https://github.com/duckduckgo/autoconsent/tree/v14.20.0"
)]
fn mpl_entry_carries_its_source_location(
    #[case] ecosystem: &str,
    #[case] name: &str,
    #[case] source: &str,
) {
    let workspace = installed_workspace();

    let outcome = workspace.xtask(&["notices", "--out", "target/notices.json"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let inventory = inventory(&workspace);
    let notice = notice(&inventory, ecosystem, name);
    assert_eq!(notice["license"], "MPL-2.0");
    assert_eq!(notice["source"], source);
}

/// A crate from outside a registry, as a fork Goodfetch carries in its
/// repository, is listed as modified, with its folder as its source; a
/// crate from its registry is listed as published.
#[rstest]
#[case::fork_in_the_repository("patched-crate", "modified", "forks/patched-crate")]
#[case::crate_from_its_registry(
    "permissive-crate",
    "unmodified",
    "https://crates.io/crates/permissive-crate/1.0.0"
)]
fn crate_outside_a_registry_is_listed_as_modified(
    #[case] name: &str,
    #[case] usage: &str,
    #[case] source: &str,
) {
    let workspace = installed_workspace();

    let outcome = workspace.xtask(&["notices", "--out", "target/notices.json"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let inventory = inventory(&workspace);
    let notice = notice(&inventory, "cargo", name);
    assert_eq!(
        (notice["use"].as_str(), notice["source"].as_str()),
        (Some(usage), Some(source))
    );
}

/// A compiled Filter list is recorded under CC BY-SA, whether its authors
/// offer CC BY-SA beside the GPL or, as fanboy.co.nz's copies say, CC BY
/// (ADRs 0018 and 0047).
#[rstest]
#[case::cc_by_sa_or_gpl("EasyList")]
#[case::cc_by("Fanboy Annoyance (fanboy.co.nz)")]
fn compiled_filter_list_is_recorded_as_cc_by_sa(#[case] name: &str) {
    let workspace = installed_workspace();

    let outcome = workspace.xtask(&["notices", "--out", "target/notices.json"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let inventory = inventory(&workspace);
    let notice = notice(&inventory, "clean-up-rules", name);
    assert_eq!(notice["license"], "CC-BY-SA-4.0");
}

/// A GPL list may only be downloaded, never embedded (ADRs 0018 and 0047).
#[rstest]
#[case::compiled_into_the_embedded_part("compiled-filter-list")]
#[case::embedded_as_published("unmodified")]
fn gpl_list_marked_for_the_embedded_part_fails(#[case] usage: &str) {
    let workspace = installed_workspace();
    workspace.edit(
        "clean-up-rules/third-party.toml",
        "filters\"\nuse = \"download-only\"",
        &format!("filters\"\nuse = \"{usage}\""),
    );

    let outcome = workspace.xtask(&["notices"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "gpl-embedded",
            "clean-up-rules uBlock filters@5e6f7a8"
        )],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// MPL-2.0 is allowed for unmodified files only (ADR 0018).
#[test]
fn modified_mpl_file_fails() {
    let workspace = installed_workspace();
    workspace.edit(
        "clean-up-rules/third-party.toml",
        "shims\"\nuse = \"unmodified\"",
        "shims\"\nuse = \"modified\"",
    );

    let outcome = workspace.xtask(&["notices"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "license-modified-file",
            "clean-up-rules Mozilla's web-compatibility shims@150.0"
        )],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// A license may be allowed for several packages, one entry each, as
/// OFL-1.1 is for the Console's two fonts.
#[test]
fn license_allowed_for_two_packages_passes_on_each() {
    let workspace = installed_workspace();
    workspace.edit(
        PERMISSIVE_DIST,
        "License-Expression: BSD-3-Clause",
        "License-Expression: PSF-2.0",
    );
    workspace.append(
        "license-allowlist.toml",
        "\n[[license]]\nid = \"PSF-2.0\"\nonly = { ecosystem = \"python\", package = \"permissive-dist\" }\n",
    );

    let outcome = workspace.xtask(&["notices"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn dependency_without_a_license_fails() {
    let workspace = installed_workspace();
    workspace.edit(PERMISSIVE_PACKAGE, "\"license\": \"MIT\",\n", "");

    let outcome = workspace.xtask(&["notices"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("license-unknown", "npm permissive-package@1.0.0")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// A block-detection rule's source that states no license, or no place its
/// source is, or an empty one, fails the check, so the inventory names
/// every rule's source and license (story 60).
///
/// Covers: story:8-60
#[rstest]
#[case::no_license("license = \"BSD-3-Clause\"\n", "", "license-unknown")]
#[case::no_source(
    "source = \"https://github.com/EnableSecurity/wafw00f/tree/69fbe39/wafw00f/plugins\"\n",
    "",
    "source-unknown"
)]
#[case::empty_source(
    "source = \"https://github.com/EnableSecurity/wafw00f/tree/69fbe39/wafw00f/plugins\"\n",
    "source = \" \"\n",
    "source-unknown"
)]
fn block_detection_rule_source_without_a_license_or_a_source_fails(
    #[case] line: &str,
    #[case] replacement: &str,
    #[case] kind: &str,
) {
    let workspace = installed_workspace();
    workspace.edit("block-detection/third-party.toml", line, replacement);

    let outcome = workspace.xtask(&["notices"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(kind, "block-detection-rules wafw00f@69fbe39")],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// A block-detection rule's source that is a watchlist, by its name or by
/// where its source is, fails the check whatever license it states: the
/// watchlists give vendor names only, and a rule is written from the
/// vendor's documentation and Goodfetch's captures (story 61).
///
/// Covers: story:8-61
#[rstest]
#[case::by_name(
    "Scrapfly's Antibot-Detector",
    "https://example.invalid/detectors",
    "block-detection-rules Scrapfly's Antibot-Detector@1"
)]
#[case::by_source(
    "vendor markers",
    "https://github.com/enthec/webappanalyzer/blob/1/src/technologies/d.json",
    "block-detection-rules vendor markers@1"
)]
#[case::by_name_spelled_without_its_hyphen(
    "Scrapfly Antibot Detector",
    "https://example.invalid/detectors",
    "block-detection-rules Scrapfly Antibot Detector@1"
)]
#[case::by_name_spelled_with_spaces(
    "Web App Analyzer",
    "https://example.invalid/technologies",
    "block-detection-rules Web App Analyzer@1"
)]
fn block_detection_rule_source_that_is_a_watchlist_fails(
    #[case] name: &str,
    #[case] source: &str,
    #[case] subject: &str,
) {
    let workspace = installed_workspace();
    workspace.append(
        "block-detection/third-party.toml",
        &format!(
            "\n[[input]]\nname = \"{name}\"\nversion = \"1\"\nlicense = \"MIT\"\nsource = \"{source}\"\nuse = \"modified\"\n"
        ),
    );

    let outcome = workspace.xtask(&["notices"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("watchlist-source", subject)],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

/// A further file of inputs, such as the list a Goodfetch Chromium build
/// writes with the variations seed (ADR 0043), joins the inventory and its
/// check; the seed's license is on no allowlist yet.
#[test]
fn further_inputs_join_the_inventory_and_its_check() {
    let workspace = installed_workspace();
    workspace.add_fixture_file(
        "notices-cases/chromium-build.toml",
        "target/chromium-build.toml",
    );

    let outcome = workspace.xtask(&[
        "notices",
        "--inputs",
        "target/chromium-build.toml",
        "--out",
        "target/notices.json",
    ]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "license-not-allowed",
            "chromium Variations seed@150.0.7800.0"
        )],
        "{}",
        outcome.stderr
    );
    let inventory = inventory(&workspace);
    let notice = notice(&inventory, "chromium", "Variations seed");
    assert_eq!(notice["license"], "LicenseRef-Google-Variations-Seed");
}

/// A shipped package missing from `node_modules` leaves its license unknown,
/// so the check reaches no verdict rather than passing.
#[test]
fn uninstalled_package_reaches_no_verdict() {
    let workspace = installed_workspace();
    workspace.remove(PERMISSIVE_PACKAGE);

    let outcome = workspace.xtask(&["notices"]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
}

/// A comparison side (`just compare`'s Crawlee and Scrapy, ADR 0138): the
/// `notices` fixture's lockfiles and installed packages in `side/`, a folder
/// of its own outside the workspace's, which is left uninstalled.
fn side_workspace() -> Workspace {
    let workspace = Workspace::from_fixture("notices");
    for lockfile in ["pnpm-lock.yaml", "uv.lock"] {
        workspace.add_fixture_file(&format!("notices/{lockfile}"), &format!("side/{lockfile}"));
    }
    for (fixture, relative) in INSTALLED {
        workspace.add_fixture_file(fixture, &format!("side/{relative}"));
    }
    workspace
}

/// `--side` checks a side's own npm and Python packages against the same
/// allowlist, from its lockfiles and what its install left, and reads none
/// of the workspace's: the workspace here is not installed, which its own
/// check refuses.
#[test]
fn a_sides_packages_are_checked_against_the_allowlist() {
    let workspace = side_workspace();

    let passing = workspace.xtask(&["notices", "--side", "side"]);
    workspace.edit(
        &format!("side/{PERMISSIVE_DIST}"),
        "License-Expression: BSD-3-Clause",
        "License-Expression: SSPL-1.0",
    );
    let failing = workspace.xtask(&["notices", "--side", "side"]);

    assert_eq!(
        (passing.findings(), passing.code),
        (vec![], 0),
        "{}",
        passing.stderr
    );
    assert_eq!(
        (failing.findings(), failing.code),
        (
            vec![finding(
                "license-not-allowed",
                "python permissive-dist@1.0.0"
            )],
            1
        ),
        "{}",
        failing.stderr
    );
}

/// An npm manifest may list its licenses as an array of names, an older
/// form npm still installs (pause-stream 0.0.11's `["MIT", "Apache2"]`):
/// each is offered, as with `OR`.
#[test]
fn an_npm_license_array_offers_each_license() {
    let workspace = installed_workspace();
    workspace.edit(
        PERMISSIVE_PACKAGE,
        r#""license": "MIT""#,
        r#""license": ["SSPL-1.0", "MIT"]"#,
    );

    let outcome = workspace.xtask(&["notices", "--out", "target/notices.json"]);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (vec![], 0),
        "{}",
        outcome.stderr
    );
    assert_eq!(
        notice(&inventory(&workspace), "npm", "permissive-package")["license"],
        "SSPL-1.0 OR MIT"
    );
}

/// A side's `licenses.toml` accepts, for that side alone, the license it
/// states for one package at one version, read from the package's own
/// license file: one the package's metadata states in no SPDX expression,
/// or one off the allowlist, since a side never ships. An exception no
/// locked package uses is a finding, so none outlives its package.
#[test]
fn a_sides_license_exceptions_accept_their_package_and_no_other() {
    let workspace = side_workspace();
    workspace.edit(
        &format!("side/{PERMISSIVE_DIST}"),
        "License-Expression: BSD-3-Clause",
        "License-Expression: SSPL-1.0",
    );
    workspace.write(
        "side/licenses.toml",
        br#"
[[package]]
ecosystem = "python"
name = "permissive-dist"
version = "1.0.0"
license = "SSPL-1.0"
reason = "a test"

[[package]]
ecosystem = "npm"
name = "unlocked-package"
version = "1.0.0"
license = "MIT"
reason = "no longer locked"
"#,
    );

    let outcome = workspace.xtask(&["notices", "--side", "side"]);

    assert_eq!(
        (outcome.findings(), outcome.code),
        (
            vec![finding(
                "license-exception-unused",
                "npm unlocked-package@1.0.0"
            )],
            1
        ),
        "{}",
        outcome.stderr
    );
}
