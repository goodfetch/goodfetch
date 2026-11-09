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

use goodfetch_protocol::home::USER_CONFIGURATION_FILE;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{TestEnv, assert_cli_snapshot, cli};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// A home that holds no older major's files has nothing to upgrade, and
/// `--json` prints so as one document of its type.
///
/// Covers: story:3-96, story:11-214, command:upgrade-files
#[test]
fn upgrade_files_json_with_no_older_major_has_nothing_to_upgrade() {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args(["upgrade-files", "--json"])
        .json("FilesUpgraded")
        .unwrap();

    assert_cli_snapshot!(checked.output, @r#"
    exit: 0
    ----- stdout -----
    {
      "refused": [],
      "unchanged": [],
      "upgraded": []
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
    std::fs::write(newer.join(USER_CONFIGURATION_FILE), "version = \"0.1\"\n").unwrap();
    let own = HomeLayout::of(env.home()).user_configuration();
    let before = std::fs::read(&own).ok();

    let output = cli(&env, COLLIE)
        .args(["upgrade-files", "--from", "0.1"])
        .run()
        .unwrap();

    assert_eq!(std::fs::read(&own).ok(), before);
    assert_cli_snapshot!(output);
}

/// A `--from` that names no major is a usage error, `2`, as clap's own
/// refusal of a flag's value is; under `--json`, a major it does not
/// upgrade from prints `{ "error" }` with `upgrade_source_unsupported` and
/// exits its row, `4`, as every command's failure does (ADR 0069).
///
/// Covers: story:3-96, command:upgrade-files, error:upgrade_source_unsupported, exit:usage, exit:refused
#[test]
fn upgrade_files_fails_a_bad_from_by_its_row() {
    let env = TestEnv::new();
    let newer = env.home().join("0.1");
    std::fs::create_dir_all(&newer).unwrap();
    std::fs::write(newer.join(USER_CONFIGURATION_FILE), "version = \"0.1\"\n").unwrap();

    let no_major = cli(&env, COLLIE)
        .args(["upgrade-files", "--from", "abc", "--json"])
        .run()
        .unwrap();
    let refused = cli(&env, COLLIE)
        .args(["upgrade-files", "--from", "0.1", "--json"])
        .run()
        .unwrap();
    let printed: serde_json::Value =
        serde_json::from_str(&refused.stdout).unwrap_or(serde_json::Value::Null);

    assert_eq!(
        (
            no_major.exit,
            no_major.stdout.as_str(),
            refused.exit,
            printed["error"]["code"].as_str(),
        ),
        (Some(2), "", Some(4), Some("upgrade_source_unsupported")),
        "{no_major}\n{refused}"
    );
}
