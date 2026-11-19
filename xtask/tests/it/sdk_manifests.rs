//! `xtask sdk-manifests`: every member an SDK's manifest lists serves a
//! catalog operation or none, and never one only a person may call, such as
//! `usage.raise`, so no SDK member raises a Limit (ADRs 0041, 0080, 0099).
//! A receiver lists one member per entry, a member with no entry is spelled
//! from its words and form, no line names a member the generator writes, and
//! a member's words are on the same receiver in every other SDK unless its
//! line says why (ADR 0099, "Generated and hand-written").
//! Covers: story:11-51

use rstest::rstest;

use crate::workspace::{Finding, Workspace, finding};

const TYPESCRIPT: &str = "sdks/typescript/manifest.toml";
const FOXHOUND: &str = "crates/foxhound/goodfetch-foxhound/manifest.toml";
/// The line of a member its other SDKs lack, so that a case shows one
/// finding alone.
const ONLY: &str = "only = \"Its other SDK has no such member.\"\n";

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
    "@goodfetch/collie",
    "Collie",
    vec![finding("person-only", "sdks/typescript/manifest.toml Collie.raise")],
)]
#[case::rust_member_raising_a_limit(
    FOXHOUND,
    "goodfetch-foxhound",
    "Foxhound",
    vec![finding(
        "person-only",
        "crates/foxhound/goodfetch-foxhound/manifest.toml Foxhound.raise",
    )],
)]
fn check_refuses_a_member_serving_a_person_only_operation(
    #[case] manifest: &str,
    #[case] package: &str,
    #[case] receiver: &str,
    #[case] expected: Vec<Finding>,
) {
    let workspace = Workspace::from_fixture("sdk-manifests");
    workspace.append(
        manifest,
        &line(
            package,
            receiver,
            "raise",
            "usage.raise",
            &["raise"],
            "round_trip",
            ONLY,
        ),
    );

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(outcome.findings(), expected);
    assert_eq!(outcome.code, 1);
}

#[test]
fn check_refuses_a_member_serving_no_catalog_operation() {
    let workspace = Workspace::from_fixture("sdk-manifests");
    workspace.append(
        TYPESCRIPT,
        &line(
            "@goodfetch/collie",
            "Collie",
            "lift",
            "usage.lift",
            &["lift"],
            "round_trip",
            ONLY,
        ),
    );

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

/// Covers: story:11-186
#[rstest]
#[case::typescript(TYPESCRIPT, "@goodfetch/collie", "Collie", "begin")]
#[case::rust(FOXHOUND, "goodfetch-foxhound", "Foxhound", "begin")]
fn check_refuses_two_members_of_one_receiver_for_one_entry(
    #[case] manifest: &str,
    #[case] package: &str,
    #[case] receiver: &str,
    #[case] name: &str,
) {
    let workspace = Workspace::from_fixture("sdk-manifests");
    workspace.append(
        manifest,
        &line(
            package,
            receiver,
            name,
            "run.start",
            &["begin"],
            "opener",
            ONLY,
        ),
    );

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "duplicate-entry",
            &format!("{manifest} {receiver}.{name}")
        )]
    );
    assert_eq!(outcome.code, 1);
}

/// Covers: story:11-186
#[rstest]
#[case::typescript_method_in_snake_case(TYPESCRIPT, "@goodfetch/collie", "Collie", "connect_over", &["connect", "over"], "opener")]
#[case::typescript_type_in_camel_case(TYPESCRIPT, "@goodfetch/collie", "(module)", "callOptions", &["call", "options"], "type")]
#[case::typescript_constructor_of_another_receiver(TYPESCRIPT, "@goodfetch/collie", "Collie", "constructor", &["crawl"], "read")]
#[case::typescript_disposal_of_another_act(TYPESCRIPT, "@goodfetch/collie", "Run", "[Symbol.asyncDispose]", &["abort"], "closer")]
#[case::rust_method_in_camel_case(FOXHOUND, "goodfetch-foxhound", "Foxhound", "connectOver", &["connect", "over"], "opener")]
#[case::rust_constant_in_lowercase(FOXHOUND, "goodfetch-foxhound", "(crate)", "free_fetches", &["free", "fetches"], "constant")]
#[case::rust_member_of_other_words(FOXHOUND, "goodfetch-foxhound", "Foxhound", "open", &["connect"], "opener")]
fn check_refuses_a_member_with_no_entry_not_spelled_from_its_words(
    #[case] manifest: &str,
    #[case] package: &str,
    #[case] receiver: &str,
    #[case] name: &str,
    #[case] words: &[&str],
    #[case] form: &str,
) {
    let workspace = Workspace::from_fixture("sdk-manifests");
    workspace.append(
        manifest,
        &line(package, receiver, name, "none", words, form, ONLY),
    );

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "misspelled",
            &format!("{manifest} {receiver}.{name}")
        )]
    );
    assert_eq!(outcome.code, 1);
}

/// Covers: story:11-186
#[test]
fn check_refuses_a_form_no_manifest_declares() {
    let workspace = Workspace::from_fixture("sdk-manifests");
    workspace.append(
        TYPESCRIPT,
        &line(
            "@goodfetch/collie",
            "Collie",
            "engine",
            "none",
            &["engine"],
            "getter",
            "",
        ),
    );

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unknown-form",
            "sdks/typescript/manifest.toml Collie.engine"
        )]
    );
    assert_eq!(outcome.code, 1);
}

/// Covers: story:11-186
#[rstest]
#[case::typescript(TYPESCRIPT, "@goodfetch/collie-client", "Run", "abort")]
#[case::rust_blocking_twin(FOXHOUND, "goodfetch-foxhound", "blocking::Run", "abort")]
fn check_refuses_a_line_naming_a_member_the_generator_writes(
    #[case] manifest: &str,
    #[case] package: &str,
    #[case] receiver: &str,
    #[case] name: &str,
) {
    let workspace = Workspace::from_fixture("sdk-manifests");
    workspace.append(
        manifest,
        &line(
            package,
            receiver,
            name,
            "run.abort",
            &["abort"],
            "round_trip",
            ONLY,
        ),
    );

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "generated",
            &format!("{manifest} {receiver}.{name}")
        )]
    );
    assert_eq!(outcome.code, 1);
}

/// Covers: story:11-186
#[rstest]
#[case::typescript_member_rust_lacks(TYPESCRIPT, "@goodfetch/foxhound", "Foxhound", "engine", &["engine"], "read")]
#[case::rust_blocking_member_typescript_lacks(FOXHOUND, "goodfetch-foxhound", "blocking::Foxhound", "scope", &["scope"], "closer")]
fn check_refuses_a_member_missing_from_the_same_receiver_in_another_sdk(
    #[case] manifest: &str,
    #[case] package: &str,
    #[case] receiver: &str,
    #[case] name: &str,
    #[case] words: &[&str],
    #[case] form: &str,
) {
    let workspace = Workspace::from_fixture("sdk-manifests");
    workspace.append(
        manifest,
        &line(package, receiver, name, "none", words, form, ""),
    );

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "missing-in-other-sdk",
            &format!("{manifest} {receiver}.{name}")
        )]
    );
    assert_eq!(outcome.code, 1);
}

/// Covers: story:11-186
#[test]
fn check_passes_a_member_one_sdk_alone_has_when_its_line_says_why() {
    let workspace = Workspace::from_fixture("sdk-manifests");
    let only = "only = \"Rust's Connection does not read the engine yet.\"\n";
    workspace.append(
        TYPESCRIPT,
        &line(
            "@goodfetch/foxhound",
            "Foxhound",
            "engine",
            "none",
            &["engine"],
            "read",
            only,
        ),
    );

    let outcome = workspace.xtask(&["sdk-manifests"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// One manifest line; `extra` holds further `key = value` lines.
fn line(
    package: &str,
    receiver: &str,
    member: &str,
    entry: &str,
    words: &[&str],
    form: &str,
    extra: &str,
) -> String {
    let key = if package.starts_with('@') {
        "package"
    } else {
        "crate"
    };
    let words = words
        .iter()
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let reason = if entry == "none" {
        "reason = \"A member of the test.\"\n"
    } else {
        ""
    };
    format!(
        "\n[[member]]\n{key} = \"{package}\"\nreceiver = \"{receiver}\"\nmember = \"{member}\"\nentry = \"{entry}\"\n{reason}{extra}words = [{words}]\nform = \"{form}\"\n"
    )
}
