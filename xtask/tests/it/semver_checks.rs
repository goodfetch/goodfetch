//! The API breakage check of the Rust crates (ADRs 0030, 0069, 0094 and
//! 0095): cargo-semver-checks on `goodfetch-collie-client`,
//! `goodfetch-collie` and `goodfetch-foxhound`, their testing kits
//! (`test-util`) included, against the commit `compat/api.toml` names as the
//! previous release. A break fails unless `compat/api.toml` announces it for
//! the next major as `rust <crate> <lint>`; an addition and a deprecation
//! pass. The tests run the pinned cargo-semver-checks on a workspace of
//! three small crates under the Products' crates' names.
//! Covers: story:1-30

use crate::workspace::{Workspace, finding};

const API: &str = "compat/api.toml";

const CLIENT: &str = "crates/goodfetch-collie-client/src/lib.rs";
const COLLIE: &str = "crates/goodfetch-collie/src/lib.rs";
const FOXHOUND: &str = "crates/foxhound/goodfetch-foxhound/src/lib.rs";

/// A crate's manifest, with the features it declares.
fn manifest(name: &str, features: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[features]\n{features}"
    )
}

/// The three crates, Collie's with its testing kit behind `test-util`,
/// committed as the previous release `compat/api.toml` names, with
/// `announced` as its announced breaks.
fn released(announced: &str) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "Cargo.toml",
        b"[workspace]\nresolver = \"3\"\nmembers = [\n  \"crates/goodfetch-collie-client\",\n  \"crates/goodfetch-collie\",\n  \"crates/foxhound/goodfetch-foxhound\",\n]\n",
    );
    for (folder, name, features) in [
        (
            "crates/goodfetch-collie-client",
            "goodfetch-collie-client",
            "",
        ),
        (
            "crates/goodfetch-collie",
            "goodfetch-collie",
            "test-util = []\n",
        ),
        (
            "crates/foxhound/goodfetch-foxhound",
            "goodfetch-foxhound",
            "",
        ),
    ] {
        workspace.write(
            &format!("{folder}/Cargo.toml"),
            manifest(name, features).as_bytes(),
        );
    }
    workspace.write(
        CLIENT,
        b"//! The client.\n\n/// Connects.\npub fn connect() {}\n\n/// Closes.\npub fn close() {}\n",
    );
    workspace.write(
        COLLIE,
        b"//! Collie.\n\n/// Opens a page.\npub fn open() {}\n\n/// The testing kit.\n#[cfg(feature = \"test-util\")]\npub mod testing {\n    /// A test engine.\n    pub fn engine() {}\n}\n",
    );
    workspace.write(
        FOXHOUND,
        b"//! Foxhound.\n\n/// Starts a crawl.\npub fn crawl() {}\n",
    );
    let lock = workspace.cargo(&["generate-lockfile", "--offline"]);
    assert_eq!(lock.code, 0, "{}", lock.stderr);
    let previous = workspace.commit_all();
    workspace.write(
        API,
        format!("previous = \"{previous}\"\n{announced}").as_bytes(),
    );
    workspace
}

#[test]
fn an_item_removed_from_a_testing_kit_is_an_unannounced_break_of_its_crate() {
    let workspace = released("");
    workspace.edit(
        COLLIE,
        "    /// A test engine.\n    pub fn engine() {}\n",
        "",
    );
    // Committed: the comparison reads the previous release, not the head.
    workspace.commit_all();

    let outcome = workspace.xtask(&["semver-checks"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unannounced-break",
            "rust goodfetch-collie function_missing"
        )],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn an_added_item_and_a_deprecation_pass() {
    let workspace = released("");
    workspace.append(FOXHOUND, "\n/// Stops a crawl.\npub fn stop() {}\n");
    workspace.edit(
        CLIENT,
        "/// Closes.\n",
        "/// Closes.\n#[deprecated = \"dropping the connection closes it\"]\n",
    );

    let outcome = workspace.xtask(&["semver-checks"]);

    assert_eq!(
        outcome.findings(),
        vec![],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn a_copy_of_a_crate_elsewhere_in_the_repository_is_not_compared() {
    // The repository holds such copies in test fixtures and proofs.
    let workspace = released("");
    workspace.write(
        "fixtures/goodfetch-collie/Cargo.toml",
        manifest("goodfetch-collie", "").as_bytes(),
    );
    workspace.write("fixtures/goodfetch-collie/src/lib.rs", b"");
    workspace.commit_all();

    let outcome = workspace.xtask(&["semver-checks"]);

    assert_eq!(
        outcome.findings(),
        vec![],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn an_announced_break_passes_as_a_note() {
    let workspace = released(
        "\n[[announced]]\nentry = \"rust goodfetch-collie-client function_missing\"\nreason = \"`close` goes: dropping the connection closes it.\"\n",
    );
    workspace.edit(CLIENT, "/// Closes.\npub fn close() {}\n", "");

    let outcome = workspace.xtask(&["semver-checks"]);

    assert_eq!(
        outcome.findings(),
        vec![],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome.notes().contains(&finding(
            "announced-break",
            "rust goodfetch-collie-client function_missing"
        )),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn an_announcement_of_a_crates_break_that_no_longer_breaks_fails() {
    let workspace = released(
        "\n[[announced]]\nentry = \"rust goodfetch-foxhound function_missing\"\nreason = \"`crawl` goes.\"\n",
    );

    let outcome = workspace.xtask(&["semver-checks"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "announced-without-break",
            "rust goodfetch-foxhound function_missing"
        )],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}
