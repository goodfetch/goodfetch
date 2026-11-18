//! `xtask sdk-manifests`: every member an SDK's manifest lists serves a
//! catalog operation or none, and never one only a person may call, such as
//! `usage.raise`, so no SDK member raises a Limit (ADRs 0041, 0080, 0099).
//! Covers: story:11-51

use rstest::rstest;

use crate::workspace::{Finding, Workspace, finding};

const TYPESCRIPT: &str = "sdks/typescript/manifest.toml";
const FOXHOUND: &str = "crates/foxhound/goodfetch-foxhound/manifest.toml";

#[test]
fn check_passes_on_manifests_serving_sdk_operations() {
    let workspace = Workspace::from_fixture("sdk-manifests");

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::typescript_member_raising_a_limit(
    TYPESCRIPT,
    "Collie",
    vec![finding("person-only", "sdks/typescript/manifest.toml Collie.raise")],
)]
#[case::rust_member_raising_a_limit(
    FOXHOUND,
    "Foxhound",
    vec![finding(
        "person-only",
        "crates/foxhound/goodfetch-foxhound/manifest.toml Foxhound.raise",
    )],
)]
fn check_refuses_a_member_serving_a_person_only_operation(
    #[case] manifest: &str,
    #[case] receiver: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("sdk-manifests");
    workspace.append(manifest, &member(receiver, "raise", "usage.raise"));

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(outcome.findings(), expected);
    assert_eq!(outcome.code, 1);
}

#[test]
fn check_refuses_a_member_serving_no_catalog_operation() {
    let workspace = Workspace::from_fixture("sdk-manifests");
    workspace.append(TYPESCRIPT, &member("Collie", "lift", "usage.lift"));

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unknown-entry",
            "sdks/typescript/manifest.toml Collie.lift"
        )]
    );
    assert_eq!(outcome.code, 1);
}

fn member(receiver: &str, member: &str, entry: &str) -> String {
    format!(
        "\n[[member]]\nreceiver = \"{receiver}\"\nmember = \"{member}\"\nentry = \"{entry}\"\nwords = [\"{member}\"]\nform = \"round_trip\"\n"
    )
}
