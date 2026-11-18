//! Seam A of #412: the runtime matrix `ci/runtimes.toml` held to ADR 0069's
//! placement. A runtime the ADR lists that is missing, or a runtime in a
//! suite the ADR does not run it in, fails the check; so does a pin that
//! does not match its line, a set the bash reader cannot read, and a job
//! step that names no set or installs no runtime of one.

use rstest::rstest;

use crate::workspace::{Workspace, finding};

const FILE: &str = "ci/runtimes.toml";
const KIND: &str = "runtime-matrix";

/// The fixture's matrix with `id` gone: its runtime's line, and its place in
/// every set.
fn without(workspace: &Workspace, id: &str) {
    let text = workspace.read_text(FILE);
    let kept: Vec<String> = text
        .lines()
        .filter(|line| {
            !line.starts_with(&format!("{id} = {{")) && !line.starts_with(&format!("\"{id}\" = {{"))
        })
        .map(|line| {
            line.replace(&format!(", \"{id}\""), "")
                .replace(&format!("\"{id}\", "), "")
        })
        .collect();
    workspace.write(FILE, format!("{}\n", kept.join("\n")).as_bytes());
}

/// The fixture's matrix with the set `name` holding `ids`.
fn with_set(workspace: &Workspace, name: &str, ids: &[&str]) {
    let text = workspace.read_text(FILE);
    let line = text
        .lines()
        .find(|line| line.starts_with(&format!("{name} = [")))
        .unwrap();
    let quoted: Vec<String> = ids.iter().map(|id| format!("\"{id}\"")).collect();
    workspace.edit(FILE, line, &format!("{name} = [{}]", quoted.join(", ")));
}

#[test]
fn the_matrix_adr_0069_places_passes() {
    let workspace = Workspace::from_fixture("runtimes");

    let outcome = workspace.xtask(&["runtimes"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

/// Each case removes a runtime and names the one the check says is missing:
/// any Bun line, or the very line ADR 0069 and the manifests require.
#[rstest]
#[case::the_oldest_supported_node_line("node-24", "node-24")]
#[case::bun("bun-1", "bun")]
#[case::deno_2("deno-2", "deno-2")]
#[case::workerd("workerd", "workerd")]
#[case::vercels_edge_runtime("vercel-edge", "vercel-edge")]
#[case::a_cpython_line_between_the_ends("cpython-3.12", "cpython-3.12")]
#[case::the_oldest_supported_cpython_line("cpython-3.11", "cpython-3.11")]
#[case::the_free_threaded_build("cpython-3.14t", "cpython-3.14t")]
fn a_missing_runtime_fails(#[case] id: &str, #[case] missing: &str) {
    let workspace = Workspace::from_fixture("runtimes");
    without(&workspace, id);

    let outcome = workspace.xtask(&["runtimes"]);

    assert!(
        outcome
            .findings()
            .contains(&finding(KIND, &format!("{FILE}#{missing}"))),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::pre_merge_on_the_oldest_node_line("pre-merge", &["node-24", "cpython-3.14"])]
#[case::pre_merge_on_the_oldest_cpython("pre-merge", &["node-26", "cpython-3.11"])]
#[case::pre_merge_with_bun("pre-merge", &["node-26", "bun-1", "cpython-3.14"])]
#[case::linux_x64_without_a_middle_line("release-linux-x64-gnu", &["node-24", "node-26", "bun-1", "deno-2", "workerd", "vercel-edge", "cpython-3.11", "cpython-3.14", "cpython-3.14t"])]
#[case::linux_without_free_threading("release-linux", &["node-24", "node-26", "bun-1", "deno-2", "cpython-3.11", "cpython-3.14"])]
#[case::linux_without_deno("release-linux", &["node-24", "node-26", "bun-1", "cpython-3.11", "cpython-3.14", "cpython-3.14t"])]
#[case::other_platforms_with_free_threading("release-other", &["node-24", "node-26", "bun-1", "deno-2", "cpython-3.11", "cpython-3.14", "cpython-3.14t"])]
#[case::other_platforms_with_a_middle_line("release-other", &["node-24", "node-26", "bun-1", "deno-2", "cpython-3.11", "cpython-3.13", "cpython-3.14"])]
#[case::other_platforms_with_an_edge_runtime("release-other", &["node-24", "node-26", "bun-1", "deno-2", "workerd", "cpython-3.11", "cpython-3.14"])]
#[case::nightly_with_a_gil_build("nightly", &["cpython-3.14", "cpython-3.14t"])]
#[case::a_runtime_that_is_not_one("nightly", &["cpython-3.14t", "cpython-3.15t"])]
fn a_runtime_in_the_wrong_suite_fails(#[case] set: &str, #[case] ids: &[&str]) {
    let workspace = Workspace::from_fixture("runtimes");
    with_set(&workspace, set, ids);

    let outcome = workspace.xtask(&["runtimes"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(KIND, &format!("{FILE}#{set}"))]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_missing_set_fails() {
    let workspace = Workspace::from_fixture("runtimes");
    let text = workspace.read_text(FILE);
    let line = text
        .lines()
        .find(|line| line.starts_with("nightly = ["))
        .unwrap();
    workspace.edit(FILE, line, "");

    let outcome = workspace.xtask(&["runtimes"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(KIND, &format!("{FILE}#nightly"))]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::a_pin_of_another_line(
    "node-26 = { version = \"26.13.0\", lts = true }",
    "node-26 = { version = \"24.19.0\", lts = true }",
    "node-26"
)]
#[case::no_pin("deno-2 = { version = \"2.9.7\" }", "deno-2 = {}", "deno-2")]
#[case::an_unknown_field(
    "bun-1 = { version = \"1.4.2\" }",
    "bun-1 = { version = \"1.4.2\", canary = true }",
    "bun-1"
)]
#[case::a_node_line_below_package_jsons_engines(
    "node-24 = { version = \"24.19.0\", lts = true }",
    "node-24 = { version = \"24.19.0\", lts = true }\nnode-22 = { version = \"22.22.0\", lts = true }",
    "node-22"
)]
#[case::an_id_of_no_runtime("workerd = {}", "workerd = {}\nminiflare-4 = {}", "miniflare-4")]
fn a_runtime_that_is_not_pinned_to_its_line_fails(
    #[case] from: &str,
    #[case] to: &str,
    #[case] id: &str,
) {
    let workspace = Workspace::from_fixture("runtimes");
    workspace.edit(FILE, from, to);

    let outcome = workspace.xtask(&["runtimes"]);

    assert!(
        outcome
            .findings()
            .contains(&finding(KIND, &format!("{FILE}#{id}"))),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_set_over_several_lines_fails() {
    let workspace = Workspace::from_fixture("runtimes");
    workspace.edit(
        FILE,
        "nightly = [\"cpython-3.14t\"]",
        "nightly = [\n  \"cpython-3.14t\",\n]",
    );

    let outcome = workspace.xtask(&["runtimes"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(KIND, &format!("{FILE}#nightly"))]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_job_that_installs_no_set_fails() {
    let workspace = Workspace::from_fixture("runtimes");
    workspace.edit(
        ".github/workflows/release-suite.yml",
        "runtimes: release-other",
        "runtimes: ends",
    );

    let outcome = workspace.xtask(&["runtimes"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(KIND, "release-suite.yml#windows")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_runtime_the_job_steps_never_install_fails() {
    let workspace = Workspace::from_fixture("runtimes");
    workspace.edit(
        ".github/actions/host-tools/action.yml",
        "bun-version: ${{ steps.runtimes.outputs.bun-1 }}",
        "bun-version: latest",
    );
    workspace.edit(
        ".github/actions/host-tools/action.yml",
        "if: steps.runtimes.outputs.bun-1 != ''",
        "if: true",
    );

    let outcome = workspace.xtask(&["runtimes"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(KIND, ".github/actions/host-tools/action.yml#bun-1")]
    );
    assert_eq!(outcome.code, 1);
}
