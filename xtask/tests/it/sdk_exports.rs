//! `xtask sdk-exports`: an SDK's manifest lists every public hand-written
//! member its packages export, once however many entries export it, and no
//! member they do not export or the generator writes (ADRs 0069, 0099). The
//! fixture's TypeScript package is read by the repository's TypeScript.
//! Covers: story:11-186

use rstest::rstest;

use crate::workspace::{Workspace, finding};

const MANIFEST: &str = "sdks/typescript/manifest.toml";
const COLLIE: &str = "sdks/typescript/collie/src/collie.ts";

fn workspace() -> Workspace {
    let workspace = Workspace::from_fixture("sdk-exports");
    workspace.link_installed_runners();
    workspace
}

#[test]
fn check_passes_when_the_manifest_lists_each_hand_written_export() {
    let workspace = workspace();

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// `Subscription`'s `[Symbol.dispose]` is `Disposable`'s, which no line has
/// to list and one may.
#[test]
fn check_passes_a_line_naming_a_member_the_language_library_declares() {
    let workspace = workspace();
    workspace.append(
        MANIFEST,
        "\n[[member]]\npackage = \"@goodfetch/collie\"\nreceiver = \"Subscription\"\nmember = \"[Symbol.dispose]\"\nentry = \"none\"\nreason = \"The scoped form of `close()`.\"\nwords = [\"close\"]\nform = \"closer\"\n",
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::function_of_an_entry(
    "sdks/typescript/collie/src/index.ts",
    "export { Collie }",
    "export function openOver(): void {}\nexport { Collie }",
    "(module)",
    "openOver"
)]
#[case::method_of_an_exported_class(
    COLLIE,
    "  close(): void {",
    "  abort(): void {}\n\n  close(): void {",
    "Collie",
    "abort"
)]
#[case::public_constructor(
    COLLIE,
    "private constructor() {}",
    "constructor() {}",
    "Collie",
    "constructor"
)]
fn check_refuses_a_hand_written_export_no_line_lists(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] receiver: &str,
    #[case] member: &str,
) {
    let workspace = workspace();
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unlisted",
            &format!("{MANIFEST} {receiver}.{member}")
        )],
        "{}",
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains(&format!(
            "package = \"@goodfetch/collie\", receiver = \"{receiver}\", member = \"{member}\""
        )),
        "the finding names the line to add: {}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::member_no_entry_exports("Collie", "begin", "[\"begin\"]", "opener")]
#[case::member_the_generator_writes("(module)", "Page", "[\"page\"]", "type")]
fn check_refuses_a_line_naming_no_hand_written_export(
    #[case] receiver: &str,
    #[case] member: &str,
    #[case] words: &str,
    #[case] form: &str,
) {
    let workspace = workspace();
    workspace.append(
        MANIFEST,
        &format!(
            "\n[[member]]\npackage = \"@goodfetch/collie\"\nreceiver = \"{receiver}\"\nmember = \"{member}\"\nentry = \"none\"\nreason = \"A member of the test.\"\nwords = {words}\nform = \"{form}\"\n"
        ),
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unused-line",
            &format!("{MANIFEST} {receiver}.{member}")
        )]
    );
    assert_eq!(outcome.code, 1);
}
