//! `xtask perf build`, before the deny (ADR 0071): it checks the base out
//! and builds the bench targets to measure, writing the plan `xtask perf run`
//! measures under the deny. The base measures every bench target of each
//! selected crate, so a ceiling whose bench target the change renamed or
//! removed still finds its figure missing in the head (ADR 0134).

use serde_json::Value;

use crate::workspace::Workspace;

const MANIFEST: &str = "crates/foxhound/goodfetch-frontier/Cargo.toml";

/// A workspace of one crate whose one bench target is `bench`.
fn crate_with_bench(workspace: &Workspace, bench: &str) {
    workspace.write(
        "Cargo.toml",
        b"[workspace]\nresolver = \"3\"\nmembers = [\"crates/foxhound/goodfetch-frontier\"]\n",
    );
    workspace.write(
        "Cargo.lock",
        b"version = 4\n\n[[package]]\nname = \"goodfetch-frontier\"\nversion = \"0.0.0\"\n",
    );
    workspace.write(
        MANIFEST,
        format!(
            "[package]\nname = \"goodfetch-frontier\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[[bench]]\nname = \"{bench}\"\nharness = false\n"
        )
        .as_bytes(),
    );
    workspace.write(
        "crates/foxhound/goodfetch-frontier/src/lib.rs",
        b"//! The Frontier.\n",
    );
    workspace.write(
        &format!("crates/foxhound/goodfetch-frontier/benches/{bench}.rs"),
        b"//! A benchmark.\n\nfn main() {}\n",
    );
}

#[test]
fn base_measures_a_bench_target_the_head_renamed_in_a_selected_crate() {
    let workspace = Workspace::from_fixture("empty");
    crate_with_bench(&workspace, "instructions");
    workspace.commit_all();
    workspace.remove("crates/foxhound/goodfetch-frontier/benches/instructions.rs");
    crate_with_bench(&workspace, "counts");
    workspace.commit_all();

    let outcome = workspace.xtask(&[
        "perf",
        "build",
        "--base",
        "HEAD^1",
        "--bench-targets",
        "goodfetch-frontier/counts",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let plan: Value =
        serde_json::from_slice(&workspace.read("target/perf/plan.json").unwrap()).unwrap();
    assert_eq!(
        plan["base_targets"],
        serde_json::json!(["goodfetch-frontier/instructions"])
    );
    assert_eq!(
        plan["head_targets"],
        serde_json::json!(["goodfetch-frontier/counts"])
    );
}
