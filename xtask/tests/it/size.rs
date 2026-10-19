//! Each Product's Size budget: `xtask size` measures a set of built files and
//! checks them against `main`'s report (increase mode) or against the
//! Product's own ceilings (ceilings mode); estimate mode never fails (ADRs
//! 0057, 0061 and 0087). Sizes far over the fixture limit are written as
//! sparse files of an exact length, since only the length is measured.

use std::fmt::Write as _;

use rstest::rstest;

use crate::github::GitHub;
use crate::workspace::{Outcome, Workspace, finding};

const LINUX_X64: &str = "x86_64-unknown-linux-gnu";

/// One built file of the manifest that `xtask size` reads.
struct Built<'a> {
    product: &'a str,
    /// `None` for a crate tarball, which has no target.
    target: Option<&'a str>,
    kind: &'a str,
    name: &'a str,
    bytes: u64,
}

fn built<'a>(product: &'a str, kind: &'a str, name: &'a str, bytes: u64) -> Built<'a> {
    Built {
        product,
        target: Some(LINUX_X64),
        kind,
        name,
        bytes,
    }
}

/// Writes each file at its exact size (an OCI image as a manifest of one
/// layer of that size) and the manifest naming them.
fn write_files(workspace: &Workspace, profile: &str, files: &[Built<'_>]) {
    let mut manifest = format!("profile = \"{profile}\"\n");
    for file in files {
        let path = format!("dist/{}/{}", file.product, file.name);
        if file.kind == "oci-image" {
            workspace.write(&path, oci_manifest(0, &[file.bytes]).as_bytes());
        } else {
            workspace.write_sized(&path, file.bytes);
        }
        write!(
            manifest,
            "\n[[file]]\nproduct = \"{}\"\nkind = \"{}\"\nname = \"{}\"\npath = \"{path}\"\n",
            file.product, file.kind, file.name
        )
        .unwrap();
        if let Some(target) = file.target {
            writeln!(manifest, "target = \"{target}\"").unwrap();
        }
    }
    workspace.write("dist/files.toml", manifest.as_bytes());
}

fn size(workspace: &Workspace, mode: &str, extra: &[&str]) -> Outcome {
    let mut args = vec!["size", mode, "--files", "dist/files.toml"];
    args.extend_from_slice(extra);
    workspace.xtask(&args)
}

#[rstest]
#[case::compressed_file_of_55_mb_for_collie("collie", "release-archive", 55_000_000)]
#[case::raw_executable_of_130_mb_for_collie("collie", "executable", 130_000_000)]
#[case::compressed_file_of_61_mb_for_foxhound("foxhound", "wheel", 61_000_000)]
#[case::raw_executable_of_161_mb_for_foxhound("foxhound", "executable", 161_000_000)]
#[case::crate_tarball_over_8_mib("collie", "crate", 8_388_609)]
fn file_over_its_products_ceiling_fails(
    #[case] product: &str,
    #[case] kind: &str,
    #[case] bytes: u64,
) {
    let workspace = Workspace::from_fixture("empty");
    let target = (kind != "crate").then_some(LINUX_X64);
    write_files(
        &workspace,
        "release",
        &[Built {
            product,
            target,
            kind,
            name: "file",
            bytes,
        }],
    );

    let outcome = size(&workspace, "ceilings", &[]);

    let subject = match target {
        Some(target) => format!("{product}/{target}/file"),
        None => format!("{product}/file"),
    };
    assert_eq!(outcome.findings(), vec![finding("over-ceiling", &subject)]);
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::compressed_file_of_55_mb_for_foxhound("foxhound", "npm-package", 55_000_000)]
#[case::raw_executable_of_130_mb_for_foxhound("foxhound", "executable", 130_000_000)]
#[case::compressed_file_of_exactly_50_mb_for_collie("collie", "wheel", 50_000_000)]
#[case::raw_executable_of_exactly_120_mb_for_collie("collie", "executable", 120_000_000)]
fn file_within_its_products_ceiling_passes(
    #[case] product: &str,
    #[case] kind: &str,
    #[case] bytes: u64,
) {
    let workspace = Workspace::from_fixture("empty");
    write_files(
        &workspace,
        "release",
        &[built(product, kind, "file", bytes)],
    );

    let outcome = size(&workspace, "ceilings", &[]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

/// An OCI image is measured from its manifest: the config's and the layers'
/// compressed sizes.
fn oci_manifest(config: u64, layers: &[u64]) -> String {
    let layers: Vec<String> = layers
        .iter()
        .map(|size| {
            format!(
                r#"{{ "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip", "digest": "sha256:{}", "size": {size} }}"#,
                "0".repeat(64)
            )
        })
        .collect();
    format!(
        r#"{{ "schemaVersion": 2, "mediaType": "application/vnd.oci.image.manifest.v1+json",
  "config": {{ "mediaType": "application/vnd.oci.image.config.v1+json", "digest": "sha256:{}", "size": {config} }},
  "layers": [ {} ] }}"#,
        "1".repeat(64),
        layers.join(", ")
    )
}

#[test]
fn oci_image_has_no_ceiling() {
    let workspace = Workspace::from_fixture("empty");
    write_files(
        &workspace,
        "release",
        &[built("foxhound", "oci-image", "foxhound-latest", 0)],
    );
    workspace.write(
        "dist/foxhound/foxhound-latest",
        oci_manifest(1_500, &[400_000_000, 500_000_000]).as_bytes(),
    );

    let outcome = size(&workspace, "ceilings", &[]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn ceilings_on_files_of_another_profile_is_an_error() {
    let workspace = Workspace::from_fixture("empty");
    write_files(
        &workspace,
        "ci",
        &[built("collie", "wheel", "file", 1_000_000)],
    );

    let outcome = size(&workspace, "ceilings", &[]);

    assert_eq!(outcome.code, 2);
}

/// `main`'s report: one figure per Product, target and file, saved for the
/// tree `main`'s head has.
fn write_main_report(workspace: &Workspace, tree: &str, profile: &str, files: &[Built<'_>]) {
    let files: Vec<String> = files
        .iter()
        .map(|file| {
            let target = file
                .target
                .map(|target| format!(r#""target": "{target}", "#))
                .unwrap_or_default();
            format!(
                r#"{{ "product": "{}", {target}"kind": "{}", "name": "{}", "bytes": {} }}"#,
                file.product, file.kind, file.name, file.bytes
            )
        })
        .collect();
    workspace.write(
        "main-report.json",
        format!(
            r#"{{ "tree": "{tree}", "profile": "{profile}", "files": [ {} ] }}"#,
            files.join(", ")
        )
        .as_bytes(),
    );
}

/// The increase check against `main`'s report, with the workspace's own
/// committed head standing in for `main`'s.
fn increase(workspace: &Workspace, extra: &[&str]) -> Outcome {
    let mut args = vec!["--main-report", "main-report.json", "--main", "HEAD"];
    args.extend_from_slice(extra);
    size(workspace, "increase", &args)
}

#[rstest]
#[case::compressed_file_0_9_mb_over_main("wheel", 30_000_000, 30_900_000)]
#[case::compressed_file_smaller_than_main("npm-package", 30_000_000, 29_000_000)]
#[case::oci_image_2_9_mb_over_main("oci-image", 80_000_000, 82_900_000)]
#[case::executable_5_mb_over_main("executable", 70_000_000, 75_000_000)]
fn file_within_its_increase_threshold_passes(
    #[case] kind: &str,
    #[case] main: u64,
    #[case] head: u64,
) {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    write_main_report(
        &workspace,
        &tree,
        "ci",
        &[built("collie", kind, "file", main)],
    );
    write_files(&workspace, "ci", &[built("collie", kind, "file", head)]);

    let outcome = increase(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::release_archive_1_1_mb_over_main("release-archive", 30_000_000, 31_100_000)]
#[case::wheel_1_1_mb_over_main("wheel", 30_000_000, 31_100_000)]
#[case::npm_package_1_1_mb_over_main("npm-package", 30_000_000, 31_100_000)]
#[case::oci_image_3_1_mb_over_main("oci-image", 80_000_000, 83_100_000)]
fn file_past_its_increase_threshold_fails(
    #[case] kind: &str,
    #[case] main: u64,
    #[case] head: u64,
) {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    write_main_report(
        &workspace,
        &tree,
        "ci",
        &[built("collie", kind, "file", main)],
    );
    write_files(&workspace, "ci", &[built("collie", kind, "file", head)]);

    let outcome = increase(&workspace, &[]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "size-increase",
            "collie/x86_64-unknown-linux-gnu/file"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn oci_image_is_measured_from_its_manifests_compressed_sizes() {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    write_main_report(
        &workspace,
        &tree,
        "ci",
        &[built(
            "foxhound",
            "oci-image",
            "foxhound-latest",
            80_001_500,
        )],
    );
    write_files(
        &workspace,
        "ci",
        &[built("foxhound", "oci-image", "foxhound-latest", 0)],
    );
    workspace.write(
        "dist/foxhound/foxhound-latest",
        oci_manifest(1_500, &[50_000_000, 33_100_000]).as_bytes(),
    );

    let outcome = increase(&workspace, &[]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "size-increase",
            "foxhound/x86_64-unknown-linux-gnu/foxhound-latest"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn crate_tarball_over_8_mib_fails_the_increase_check() {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    let tarball = |bytes| Built {
        product: "foxhound",
        target: None,
        kind: "crate",
        name: "goodfetch-foxhound.crate",
        bytes,
    };
    write_main_report(&workspace, &tree, "ci", &[tarball(8_000_000)]);
    write_files(&workspace, "ci", &[tarball(8_388_609)]);

    let outcome = increase(&workspace, &[]);

    assert_eq!(
        outcome.findings(),
        vec![finding("over-ceiling", "foxhound/goodfetch-foxhound.crate")]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn each_products_file_is_compared_with_mains_file_of_the_same_product() {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    write_main_report(
        &workspace,
        &tree,
        "ci",
        &[
            built("collie", "wheel", "wheel", 30_000_000),
            built("foxhound", "wheel", "wheel", 45_000_000),
        ],
    );
    write_files(
        &workspace,
        "ci",
        &[
            built("collie", "wheel", "wheel", 31_100_000),
            built("foxhound", "wheel", "wheel", 45_500_000),
        ],
    );

    let outcome = increase(&workspace, &[]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "size-increase",
            "collie/x86_64-unknown-linux-gnu/wheel"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn file_main_has_no_figure_for_is_reported_never_gated() {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    write_main_report(&workspace, &tree, "ci", &[]);
    write_files(
        &workspace,
        "ci",
        &[built("collie", "wheel", "wheel", 40_000_000)],
    );

    let outcome = increase(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn main_report_of_another_tree_is_an_error() {
    let workspace = Workspace::from_fixture("empty");
    workspace.commit();
    write_main_report(
        &workspace,
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
        "ci",
        &[built("collie", "wheel", "wheel", 30_000_000)],
    );
    write_files(
        &workspace,
        "ci",
        &[built("collie", "wheel", "wheel", 30_000_000)],
    );

    let outcome = increase(&workspace, &[]);

    assert_eq!(outcome.code, 2);
}

/// The estimate never fails (#201): a report of `main` it cannot compare
/// with is left out, and the estimate says so.
#[rstest]
#[case::another_tree("4b825dc642cb6eb9a060e54bf8d69288fbee4904", "ci")]
#[case::another_profile("", "release")]
fn estimate_with_an_incomparable_main_report_still_passes(
    #[case] report_tree: &str,
    #[case] report_profile: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    let report_tree = if report_tree.is_empty() {
        tree.as_str()
    } else {
        report_tree
    };
    write_main_report(
        &workspace,
        report_tree,
        report_profile,
        &[built("collie", "wheel", "wheel", 30_000_000)],
    );
    write_files(
        &workspace,
        "ci",
        &[built("collie", "wheel", "wheel", 30_000_000)],
    );

    let outcome = size(
        &workspace,
        "estimate",
        &["--main-report", "main-report.json", "--main", "HEAD"],
    );

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn main_report_of_another_profile_is_an_error() {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    write_main_report(
        &workspace,
        &tree,
        "release",
        &[built("collie", "wheel", "wheel", 30_000_000)],
    );
    write_files(
        &workspace,
        "ci",
        &[built("collie", "wheel", "wheel", 30_000_000)],
    );

    let outcome = increase(&workspace, &[]);

    assert_eq!(outcome.code, 2);
}

const HEAD_SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER_SHA: &str = "fedcba9876543210fedcba9876543210fedcba98";
const PULL_REQUEST: &str = "/repos/goodfetch/goodfetch/pulls/7";
const COMMENTS: &str = "/repos/goodfetch/goodfetch/issues/7/comments?per_page=100&page=1";

/// A pull request whose head is `HEAD_SHA`, with `body` and one comment.
fn pull_request(body: &str, comment: &str, author_association: &str) -> GitHub {
    let pull_request = format!(
        r#"{{ "number": 7, "body": {}, "head": {{ "sha": "{HEAD_SHA}" }} }}"#,
        serde_json::to_string(body).unwrap()
    );
    let comments = format!(
        r#"[ {{ "body": {}, "author_association": "{author_association}" }} ]"#,
        serde_json::to_string(comment).unwrap()
    );
    GitHub::serving(&[(PULL_REQUEST, &pull_request), (COMMENTS, &comments)])
}

/// Collie's wheel, 1.10 MB over `main`'s.
fn write_wheel_grown_by_1_1_mb(workspace: &Workspace) {
    let tree = workspace.commit();
    write_main_report(
        workspace,
        &tree,
        "ci",
        &[built("collie", "wheel", "wheel", 30_000_000)],
    );
    write_files(
        workspace,
        "ci",
        &[built("collie", "wheel", "wheel", 31_100_000)],
    );
}

fn increase_for_pull_request(workspace: &Workspace, github: &GitHub) -> Outcome {
    increase(
        workspace,
        &[
            "--repository",
            "goodfetch/goodfetch",
            "--pull-request",
            "7",
            "--api-url",
            github.url(),
        ],
    )
}

const SECTION: &str = "## Summary\n\nAdds the extractor.\n\n## Size increase\n\n\
    - collie/x86_64-unknown-linux-gnu/wheel: +1.10 MB, the extractor's tables\n\n## Evidence\n";

#[rstest]
#[case::full_head_sha(&format!("Size increase accepted for {HEAD_SHA}."))]
#[case::abbreviated_head_sha("Size increase accepted at 0123456789a")]
fn tripped_file_with_its_section_and_acceptance_on_the_head_passes(#[case] comment: &str) {
    let workspace = Workspace::from_fixture("empty");
    write_wheel_grown_by_1_1_mb(&workspace);
    let github = pull_request(SECTION, comment, "OWNER");

    let outcome = increase_for_pull_request(&workspace, &github);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[rstest]
#[case::acceptance_names_another_sha(
    SECTION,
    &format!("Size increase accepted for {OTHER_SHA}."),
    "OWNER"
)]
#[case::section_delta_smaller_than_measured(
    "## Size increase\n\n- collie/x86_64-unknown-linux-gnu/wheel: +1.05 MB, the extractor's tables\n",
    &format!("Size increase accepted for {HEAD_SHA}."),
    "OWNER"
)]
#[case::section_lists_another_file(
    "## Size increase\n\n- foxhound/x86_64-unknown-linux-gnu/wheel: +1.10 MB, the extractor's tables\n",
    &format!("Size increase accepted for {HEAD_SHA}."),
    "OWNER"
)]
#[case::no_section(
    "## Summary\n\n- collie/x86_64-unknown-linux-gnu/wheel: +1.10 MB\n",
    &format!("Size increase accepted for {HEAD_SHA}."),
    "OWNER"
)]
#[case::no_acceptance(SECTION, &format!("Looks fine at {HEAD_SHA}."), "OWNER")]
#[case::acceptance_by_someone_without_write_access(
    SECTION,
    &format!("Size increase accepted for {HEAD_SHA}."),
    "CONTRIBUTOR"
)]
// The owner accepts when merging (ADR 0133); a collaborator's agent cannot.
#[case::acceptance_by_a_collaborator(
    SECTION,
    &format!("Size increase accepted for {HEAD_SHA}."),
    "COLLABORATOR"
)]
#[case::section_line_without_a_reason(
    "## Size increase\n\n- collie/x86_64-unknown-linux-gnu/wheel: +1.10 MB\n",
    &format!("Size increase accepted for {HEAD_SHA}."),
    "OWNER"
)]
fn tripped_file_without_a_matching_acceptance_fails(
    #[case] body: &str,
    #[case] comment: &str,
    #[case] author_association: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    write_wheel_grown_by_1_1_mb(&workspace);
    let github = pull_request(body, comment, author_association);

    let outcome = increase_for_pull_request(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "size-increase",
            "collie/x86_64-unknown-linux-gnu/wheel"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn unreadable_pull_request_is_an_error_never_a_pass() {
    let workspace = Workspace::from_fixture("empty");
    write_wheel_grown_by_1_1_mb(&workspace);
    let github = GitHub::serving(&[]);

    let outcome = increase_for_pull_request(&workspace, &github);

    assert_eq!(outcome.code, 2);
}

#[test]
fn estimate_never_fails_past_a_threshold_or_a_ceiling() {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    write_main_report(
        &workspace,
        &tree,
        "ci",
        &[built("collie", "release-archive", "archive", 30_000_000)],
    );
    write_files(
        &workspace,
        "ci",
        &[
            built("collie", "release-archive", "archive", 55_000_000),
            built("collie", "executable", "collie", 130_000_000),
        ],
    );

    let outcome = size(
        &workspace,
        "estimate",
        &["--main-report", "main-report.json", "--main", "HEAD"],
    );

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn saved_report_serves_as_mains_report_for_its_tree() {
    let workspace = Workspace::from_fixture("empty");
    workspace.commit();
    write_files(
        &workspace,
        "ci",
        &[built("collie", "wheel", "wheel", 30_000_000)],
    );
    size(&workspace, "estimate", &["--save", "saved.json"]);
    write_files(
        &workspace,
        "ci",
        &[built("collie", "wheel", "wheel", 31_100_000)],
    );

    let outcome = size(
        &workspace,
        "increase",
        &["--main-report", "saved.json", "--main", "HEAD"],
    );

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "size-increase",
            "collie/x86_64-unknown-linux-gnu/wheel"
        )]
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn estimate_without_mains_report_never_fails() {
    let workspace = Workspace::from_fixture("empty");
    write_files(
        &workspace,
        "ci",
        &[built("foxhound", "wheel", "wheel", 70_000_000)],
    );

    let outcome = size(&workspace, "estimate", &[]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

/// Both Products' files of one build, as each mode's table shows them.
fn both_products(main_or_head: fn(u64, u64) -> u64) -> Vec<Built<'static>> {
    let tarball = Built {
        product: "collie",
        target: None,
        kind: "crate",
        name: "goodfetch-collie.crate",
        bytes: main_or_head(2_400_000, 2_500_000),
    };
    vec![
        built(
            "collie",
            "executable",
            "collie",
            main_or_head(70_000_000, 75_000_000),
        ),
        built(
            "collie",
            "wheel",
            "collie.whl",
            main_or_head(30_000_000, 31_100_000),
        ),
        built(
            "collie",
            "npm-package",
            "collie.tgz",
            main_or_head(30_000_000, 30_900_000),
        ),
        tarball,
        built(
            "foxhound",
            "oci-image",
            "foxhound-latest",
            main_or_head(80_000_000, 83_100_000),
        ),
        built(
            "foxhound",
            "wheel",
            "foxhound.whl",
            main_or_head(40_000_000, 39_800_000),
        ),
    ]
}

#[test]
fn increase_report_lists_every_file_of_both_products() {
    let workspace = Workspace::from_fixture("empty");
    let tree = workspace.commit();
    write_main_report(&workspace, &tree, "ci", &both_products(|main, _| main));
    let mut head = both_products(|_, head| head);
    head.push(built(
        "foxhound",
        "release-archive",
        "foxhound.tar.gz",
        45_000_000,
    ));
    write_files(&workspace, "ci", &head);
    let github = pull_request(
        "## Size increase\n\n- collie/x86_64-unknown-linux-gnu/collie.whl: +1.10 MB, the extractor\n",
        &format!("Size increase accepted for {HEAD_SHA}."),
        "OWNER",
    );

    let outcome = increase_for_pull_request(&workspace, &github);

    insta::assert_snapshot!(outcome.stdout);
}

#[test]
fn ceilings_report_lists_each_products_own_ceilings() {
    let workspace = Workspace::from_fixture("empty");
    write_files(&workspace, "release", &both_products(|_, head| head));

    let outcome = size(&workspace, "ceilings", &[]);

    insta::assert_snapshot!(outcome.stdout);
}

#[test]
fn estimate_is_a_pull_request_comment() {
    let workspace = Workspace::from_fixture("empty");
    write_files(
        &workspace,
        "ci",
        &[built(
            "collie",
            "release-archive",
            "collie.tar.gz",
            31_234_567,
        )],
    );

    let outcome = size(&workspace, "estimate", &[]);

    insta::assert_snapshot!(outcome.stdout, @"
    ### Size estimate

    | Product | Target | File | Size | main | Change | Limit | Verdict |
    |---|---|---|---:|---:|---:|---:|---|
    | Collie | x86_64-unknown-linux-gnu | collie.tar.gz | 31.23 MB | — | — | +1 MB | new |

    No report of `main` to compare with; the merge queue's increase check decides.
    ");
}
