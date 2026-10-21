//! `upgrade-files` through Collie's executable (spec #213's Seam 1, ADR
//! 0066's Pre-merge part): the refusals its committed upgrade paths give,
//! which list no source before the first release, and its `--json` output.
//! This executable's own version is the only one it can have (ADR 0059),
//! so the copy from a listed source is tested in process, from fixtures.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's files themselves"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{TestEnv, assert_cli_snapshot, cli};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// A home that holds no older major's files has nothing to upgrade, and
/// `--json` prints so as one document of its type.
///
/// Covers: story:3-96, command:upgrade-files
#[test]
fn upgrade_files_json_with_no_older_major_has_nothing_to_upgrade() {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["upgrade-files", "--json"])
        .json("UpgradeFilesOutput")
        .unwrap();

    assert_cli_snapshot!(checked.output, @r#"
    exit: 0
    ----- stdout -----
    {
      "copied": [],
      "from": null,
      "kept": [],
      "to": "0.0",
      "unchanged": []
    }
    ----- stderr -----
    "#);
}

/// A newer major than the executable's is refused with a message to
/// upgrade, and no file is written; during `0.y` previews the next preview
/// is a newer major (ADRs 0030 and 0066).
///
/// Covers: story:3-96, command:upgrade-files, error:upgrade_source_unsupported
#[test]
fn upgrade_files_refuses_a_newer_major() {
    let env = TestEnv::new();
    let newer = env.home().join("0.1");
    std::fs::create_dir_all(&newer).unwrap();
    std::fs::write(newer.join("configuration.toml"), "version = \"0.1\"\n").unwrap();
    let own = env.home().join("0.0").join("configuration.toml");
    let before = std::fs::read(&own).ok();

    let output = cli(&env, COLLIE)
        .args(["upgrade-files", "--from", "0.1"])
        .run()
        .unwrap();

    assert_eq!(std::fs::read(&own).ok(), before);
    assert_cli_snapshot!(output);
}
