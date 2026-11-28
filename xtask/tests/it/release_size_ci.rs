//! Tests for the pull-request size estimate, merge-queue size increase gate,
//! crate tarball ceiling, both Products' five OCI tags +3 MB check, and
//! tree-keyed size reports (ADRs 0057, 0061, 0087 and 0094; issue 204).

use std::path::Path;
use std::process::Command;

use crate::github::GitHub;
use crate::workspace::{Workspace, finding};

const MB: u64 = 1_000_000;
const MIB: u64 = 1_048_576;
const TARGET: &str = "x86_64-unknown-linux-musl";

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}

fn oci_manifest(bytes: u64) -> String {
    format!(r#"{{"schemaVersion":2,"config":{{"size":1024}},"layers":[{{"size":{bytes}}}]}}"#)
}

/// Covers: story:1-56
/// The pull-request workflow runs a non-blocking `size-estimate` job that
/// updates one sticky comment and is never in `all-green`'s `needs:`, and
/// `xtask size estimate` never fails even on over-threshold growth.
#[test]
fn pull_request_size_estimate_is_non_blocking_and_updates_one_sticky_comment() {
    let pr_workflow =
        std::fs::read_to_string(repo_root().join(".github/workflows/pull-request.yml"))
            .expect("read pull-request.yml");
    assert!(
        pr_workflow.contains("size-estimate:"),
        "pull-request.yml must declare size-estimate job"
    );
    assert!(
        pr_workflow.contains("ci/release/size-comment.sh"),
        "size-estimate must invoke ci/release/size-comment.sh"
    );

    // `all-green` must never depend on `size-estimate`.
    let all_green_pos = pr_workflow.find("\n  all-green:").expect("all-green job");
    let all_green_block = &pr_workflow[all_green_pos..];
    assert!(
        !all_green_block.contains("size-estimate"),
        "all-green must never depend on size-estimate:\n{all_green_block}"
    );

    // Sticky comment script uses one stable HTML marker and PATCHes existing comments.
    let comment_script = std::fs::read_to_string(repo_root().join("ci/release/size-comment.sh"))
        .expect("read size-comment.sh");
    assert!(
        comment_script.contains("<!-- goodfetch-size-estimate -->")
            && comment_script.contains("--method PATCH")
            && comment_script.contains("--method POST"),
        "size-comment.sh must create or update one sticky comment"
    );

    // `xtask size estimate` never fails even when growth exceeds +1 MB.
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    workspace.write_sized("dist/collie.tar.xz", 25 * MB);
    workspace.write(
        "dist/files.toml",
        format!(
            "profile = \"ci\"\n\n[[file]]\nproduct = \"collie\"\nkind = \"release-archive\"\ntarget = \"{TARGET}\"\nname = \"collie.tar.xz\"\npath = \"dist/collie.tar.xz\"\n"
        )
        .as_bytes(),
    );
    let saved = workspace.xtask(&[
        "size",
        "estimate",
        "--files",
        "dist/files.toml",
        "--save",
        "main-report.json",
    ]);
    assert_eq!(saved.code, 0, "{}", saved.stderr);

    // Grow by +5 MB (well over +1 MB): estimate still exits 0.
    workspace.write_sized("dist/collie.tar.xz", 30 * MB);
    let estimate = workspace.xtask(&[
        "size",
        "estimate",
        "--files",
        "dist/files.toml",
        "--main-report",
        "main-report.json",
        "--main",
        "HEAD",
    ]);
    assert_eq!(
        estimate.code, 0,
        "estimate must never fail: {}",
        estimate.stderr
    );
    assert!(
        estimate.stdout.contains("### Size estimate"),
        "estimate must print Size estimate header:\n{}",
        estimate.stdout
    );
    let report = workspace.read_text("main-report.json");
    assert!(report.contains(&tree), "saved report must be keyed by tree");
}

fn seed_increase_workspace() -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write_sized("dist/collie.tar.xz", 20 * MB);
    workspace.write_sized("dist/foxhound.tar.xz", 25 * MB);
    workspace.write(
        "dist/collie-slim.manifest.json",
        oci_manifest(40 * MB).as_bytes(),
    );
    workspace.write(
        "dist/foxhound-full.manifest.json",
        oci_manifest(90 * MB).as_bytes(),
    );
    workspace.write_sized("dist/goodfetch-collie-0.1.0.crate", 2 * MIB);
    workspace.write(
        "dist/files.toml",
        format!(
            "profile = \"ci\"\n\
             \n[[file]]\nproduct = \"collie\"\nkind = \"release-archive\"\ntarget = \"{TARGET}\"\nname = \"goodfetch-collie-{TARGET}.tar.xz\"\npath = \"dist/collie.tar.xz\"\n\
             \n[[file]]\nproduct = \"foxhound\"\nkind = \"release-archive\"\ntarget = \"{TARGET}\"\nname = \"goodfetch-foxhound-{TARGET}.tar.xz\"\npath = \"dist/foxhound.tar.xz\"\n\
             \n[[file]]\nproduct = \"collie\"\nkind = \"oci-image\"\ntarget = \"{TARGET}\"\nname = \"collie-slim\"\npath = \"dist/collie-slim.manifest.json\"\n\
             \n[[file]]\nproduct = \"foxhound\"\nkind = \"oci-image\"\ntarget = \"{TARGET}\"\nname = \"foxhound-full\"\npath = \"dist/foxhound-full.manifest.json\"\n\
             \n[[file]]\nproduct = \"collie\"\nkind = \"crate\"\nname = \"goodfetch-collie-0.1.0.crate\"\npath = \"dist/goodfetch-collie-0.1.0.crate\"\n"
        )
        .as_bytes(),
    );
    workspace.commit();
    let base = workspace.xtask(&[
        "size",
        "estimate",
        "--files",
        "dist/files.toml",
        "--save",
        "main-report.json",
    ]);
    assert_eq!(base.code, 0, "{}", base.stderr);
    workspace
}

/// Covers: story:1-16
/// The merge-queue `increase` check fails when either Product's archive grows
/// past +1 MB or any of its five OCI tags grows past +3 MB, and passes once the
/// pull request's "Size increase" section and an owner acceptance comment
/// naming the exact head SHA exist.
#[test]
fn merge_queue_increase_fails_over_threshold_and_passes_with_head_acceptance() {
    let workspace = seed_increase_workspace();

    // Grow Collie archive by +1.5 MB (> +1 MB) and Foxhound OCI full tag by +4 MB (> +3 MB).
    workspace.write_sized("dist/collie.tar.xz", 21_500_000);
    workspace.write(
        "dist/foxhound-full.manifest.json",
        oci_manifest(94 * MB).as_bytes(),
    );

    let failed = workspace.xtask(&[
        "size",
        "increase",
        "--files",
        "dist/files.toml",
        "--main-report",
        "main-report.json",
        "--main",
        "HEAD",
    ]);
    assert_eq!(
        failed.code, 1,
        "expected size-increase failure:\n{}",
        failed.stdout
    );
    assert_eq!(
        failed.findings(),
        vec![
            finding(
                "size-increase",
                &format!("collie/{TARGET}/goodfetch-collie-{TARGET}.tar.xz")
            ),
            finding("size-increase", &format!("foxhound/{TARGET}/foxhound-full")),
        ]
    );

    // With PR "Size increase" section and owner acceptance naming exact head SHA, it passes.
    let head_sha = "1111111111111111111111111111111111111111";
    let pr_body = format!(
        "## Size increase\n\n\
         - collie/{TARGET}/goodfetch-collie-{TARGET}.tar.xz: +1.50 MB, new protocol codec\n\
         - foxhound/{TARGET}/foxhound-full: +4.00 MB, bundled parser update\n"
    );
    let pr_json = serde_json::json!({
        "number": 77,
        "body": pr_body,
        "head": { "sha": head_sha }
    })
    .to_string();
    let comments_json = serde_json::json!([
        {
            "user": { "login": "Jbmanllr" },
            "author_association": "OWNER",
            "body": format!("Size increase accepted for {head_sha}")
        }
    ])
    .to_string();
    let github = GitHub::serving(&[
        ("/repos/goodfetch/goodfetch/pulls/77", pr_json.as_str()),
        (
            "/repos/goodfetch/goodfetch/issues/77/comments?per_page=100&page=1",
            comments_json.as_str(),
        ),
    ]);

    let accepted = workspace.xtask(&[
        "size",
        "increase",
        "--files",
        "dist/files.toml",
        "--main-report",
        "main-report.json",
        "--main",
        "HEAD",
        "--pull-request",
        "77",
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
    ]);
    assert_eq!(
        accepted.code, 0,
        "accepted increase should pass:\nstdout:\n{}\nstderr:\n{}",
        accepted.stdout, accepted.stderr
    );
    assert_eq!(accepted.findings(), vec![]);

    // A crate tarball over 8 MiB fails even with PR acceptance (ceilings cannot be accepted).
    workspace.write_sized("dist/goodfetch-collie-0.1.0.crate", 8 * MIB + 1);
    let crate_over = workspace.xtask(&[
        "size",
        "increase",
        "--files",
        "dist/files.toml",
        "--main-report",
        "main-report.json",
        "--main",
        "HEAD",
    ]);
    assert_eq!(crate_over.code, 1);
    assert!(
        crate_over.findings().contains(&finding(
            "over-ceiling",
            "collie/goodfetch-collie-0.1.0.crate"
        )),
        "{:?}",
        crate_over.findings()
    );
}

/// Covers: story:1-55
/// `ci/release/size-manifest.sh` covers both Products' executables, compressed
/// archives, crate tarballs, and all five OCI image tags (`default`, `slim`,
/// `full`, `node`, `python`), and `release-suite.yml` saves tree-keyed size
/// reports on `main` and `merge_group`.
#[test]
fn size_manifest_covers_crates_and_five_oci_tags_and_release_suite_saves_reports() {
    let workspace = Workspace::from_fixture("empty");
    for product in ["collie", "foxhound"] {
        workspace.write_sized(&format!("target/{TARGET}/ci/goodfetch-{product}"), 10 * MB);
        workspace.write_sized(
            &format!("target/release-suite/archives/goodfetch-{product}-{TARGET}.tar.xz"),
            4 * MB,
        );
        for tag in ["default", "slim", "full", "node", "python"] {
            workspace.write(
                &format!("target/release-suite/oci/{product}-{tag}.manifest.json"),
                oci_manifest(20 * MB).as_bytes(),
            );
        }
    }
    workspace.write_sized(
        "target/release-suite/crates/goodfetch-collie-0.1.0.crate",
        MIB,
    );
    workspace.write_sized(
        "target/release-suite/crates/goodfetch-foxhound-0.1.0.crate",
        2 * MIB,
    );

    let output = Command::new("bash")
        .arg(repo_root().join("ci/release/size-manifest.sh"))
        .arg(TARGET)
        .current_dir(workspace.path())
        .output()
        .expect("run size-manifest.sh");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = String::from_utf8(output.stdout).expect("utf-8 manifest");
    workspace.write("size-files.toml", manifest.as_bytes());
    workspace.commit();

    // Every Product x 5 OCI tags + archives + executables + crates must parse and save.
    for product in ["collie", "foxhound"] {
        for tag in ["default", "slim", "full", "node", "python"] {
            assert!(
                manifest.contains(&format!("name = \"{product}-{tag}\"")),
                "manifest missing OCI tag {product}-{tag}:\n{manifest}"
            );
        }
    }
    assert!(manifest.contains("kind = \"crate\""));
    assert!(manifest.contains("kind = \"release-archive\""));

    let saved = workspace.xtask(&[
        "size",
        "increase",
        "--files",
        "size-files.toml",
        "--save",
        "report.json",
    ]);
    assert_eq!(saved.code, 0, "{}", saved.stderr);
    let report = workspace.read_text("report.json");
    assert!(report.contains("collie-default") && report.contains("foxhound-python"));

    let release_suite =
        std::fs::read_to_string(repo_root().join(".github/workflows/release-suite.yml"))
            .expect("read release-suite.yml");
    assert!(
        release_suite.contains("GOODFETCH_SIZE_MODE")
            && release_suite.contains("github.event_name == 'merge_group'")
            && release_suite
                .contains("size-report-${{ steps.size.outputs.tree }}-${{ matrix.target }}"),
        "release-suite.yml must run increase mode on merge_group and save tree-keyed size reports"
    );
}
