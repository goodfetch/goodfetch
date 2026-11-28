//! The image digest-pin check: every container image a workflow runs is
//! pinned by digest, as every Action is pinned by SHA (ADR 0067).
//! Covers: story:1-112

use rstest::rstest;

use crate::workspace::{Workspace, finding};

#[test]
fn images_pinned_by_digest_pass() {
    let workspace = Workspace::from_fixture("image-pins");

    let outcome = workspace.xtask(&["image-pins"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
}

#[test]
fn multi_image_pin_file_without_digest_fails() {
    let workspace = Workspace::from_fixture("image-pins");
    workspace.write(
        "ci/integrations/images.toml",
        b"[n8n]\nrepository = \"docker.io/n8nio/n8n\"\ntag = \"2.50.3\"\ndigest = \"sha256:6dc48a8dbdfe070b3babacd128995635e61cd19da9172b9aa0d5546792bd4a23\"\n\n[activepieces]\nrepository = \"docker.io/activepieces/activepieces\"\ntag = \"0.94.0\"\n",
    );

    let outcome = workspace.xtask(&["image-pins"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unpinned-image", "ci/integrations/images.toml")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::job_container("container-by-tag.yml")]
#[case::job_container_object("container-image-by-tag.yml")]
#[case::service_container("service-by-tag.yml")]
#[case::docker_step("docker-step-by-tag.yml")]
#[case::run_script("run-by-tag.yml")]
#[case::run_script_docker_hub_short_name("run-docker-hub-by-tag.yml")]
#[case::digest_too_short("short-digest.yml")]
fn workflow_image_referenced_by_tag_fails(#[case] workflow: &str) {
    let workspace = Workspace::from_fixture("image-pins");
    workspace.add_fixture_file(
        &format!("image-pins-cases/{workflow}"),
        ".github/workflows/extra.yml",
    );

    let outcome = workspace.xtask(&["image-pins"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unpinned-image", "extra.yml#build")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::docker_action("action-by-tag.yml", ".github/actions/scanner/action.yml")]
#[case::containerfile_base("Containerfile-by-tag", "ci/image/Containerfile")]
#[case::pin_file_without_digest("image-toml-without-digest.toml", "ci/zap/image.toml")]
#[case::script("script-by-tag.sh", "ci/scan.sh")]
#[case::script_docker_hub_short_name("script-docker-hub-by-tag.sh", "ci/scan.sh")]
fn image_a_workflow_runs_through_its_files_by_tag_fails(#[case] case: &str, #[case] file: &str) {
    let workspace = Workspace::from_fixture("image-pins");
    workspace.add_fixture_file(&format!("image-pins-cases/{case}"), file);

    let outcome = workspace.xtask(&["image-pins"]);

    assert_eq!(outcome.findings(), vec![finding("unpinned-image", file)]);
    assert_eq!(outcome.code, 1);
}
