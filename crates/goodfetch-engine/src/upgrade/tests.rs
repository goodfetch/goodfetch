//! `upgrade-files` from fixtures (ADR 0066's Quick part): its refusals,
//! derived from the committed `compat/upgrade-paths.toml`, which lists no
//! source before the first release, and the copy itself from a table that
//! lists one. Executables' versions are fabricated: no build carries a
//! version override (ADR 0059).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_test_support::TestEnv;

use super::{UpgradeError, UpgradePaths, Upgraded, upgrade_files};
use crate::home::Home;
use crate::ledger::Ledger;

fn version(text: &str) -> ProductVersion {
    ProductVersion::parse(text).unwrap()
}

/// Every file under `root`, by its path from `root`, with its bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut folders = vec![root.to_owned()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.map(Result::unwrap) {
            if entry.file_type().unwrap().is_dir() {
                folders.push(entry.path());
            } else {
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    files
}

/// A Goodfetch home holding major 2's files: its Operator policy, user
/// configuration, Run index and a run folder, and its Daemon's runtime
/// file.
fn home_of_major_2(env: &TestEnv) -> Home {
    let root = env.folder().join("upgraded home");
    let older = root.join("2");
    std::fs::create_dir_all(older.join("runs/crawl-1")).unwrap();
    for (path, text) in [
        (
            "operator-policy.toml",
            "version = \"1.4\"\n[lock.timeout]\nnavigation = 1000\n",
        ),
        (
            "configuration.toml",
            "version = \"1.4\"\n[settings.timeout]\nrun = 60000\n",
        ),
        ("run-index.sqlite", "the Run index"),
        ("runs/crawl-1/run.sqlite", "a run file"),
        ("daemon.json", "{\"pid\": 1}"),
    ] {
        std::fs::write(older.join(path), text).unwrap();
    }
    Home::new(root)
}

#[test]
fn the_committed_upgrade_paths_read() {
    let paths = UpgradePaths::committed();

    assert_eq!(paths.sources(), &[] as &[String]);
}

/// A source the committed table does not list is refused before any file
/// is written, naming the release to upgrade through first, as Cassandra's
/// upgrade graph does (ADR 0066); before the first release the table lists
/// none.
///
/// Covers: story:3-96, error:upgrade_source_unsupported
#[test]
fn a_source_the_table_does_not_list_is_refused_naming_the_release_to_upgrade_through() {
    let env = TestEnv::new();
    let home = home_of_major_2(&env);
    std::fs::rename(home.root().join("2"), home.root().join("1")).unwrap();
    let paths = UpgradePaths::committed();
    assert!(!paths.sources().contains(&"1".to_owned()));
    let before = tree(home.root());

    let refused = upgrade_files(&home, &version("3.0.0"), None, &paths);

    assert_eq!(
        (refused, tree(home.root()) == before),
        (
            Err(UpgradeError::NotListed {
                major: "1".to_owned(),
                current: "3".to_owned(),
                through: Some("2".to_owned()),
            }),
            true
        )
    );
}

/// A newer major than the executable's is refused with a message to
/// upgrade (ADR 0066).
///
/// Covers: story:3-96, error:upgrade_source_unsupported
#[test]
fn a_newer_major_is_refused_with_a_message_to_upgrade() {
    let env = TestEnv::new();
    let home = home_of_major_2(&env);
    let before = tree(home.root());

    let refused = upgrade_files(
        &home,
        &version("1.7.0"),
        Some("2"),
        &UpgradePaths::committed(),
    );

    assert_eq!(
        (
            refused.as_ref().map_err(ToString::to_string),
            tree(home.root()) == before
        ),
        (
            Err("the files of major 2 are newer than this Goodfetch's major 1; upgrade Goodfetch to major 2 or newer to use them".to_owned()),
            true
        )
    );
}

/// From a listed source, the new major gets its copy of every per-user
/// and per-install file but its Daemon's, the older major's files keep
/// their bytes, and a second run changes nothing (ADRs 0030 and 0066).
///
/// Covers: story:3-96
#[test]
fn a_listed_source_gives_the_new_major_its_copy_and_leaves_the_older_majors_files() {
    let env = TestEnv::new();
    let home = home_of_major_2(&env);
    let paths = UpgradePaths::parse("[[source]]\nmajor = \"2\"\n").unwrap();
    let older_before = tree(&home.root().join("2"));

    let first = upgrade_files(&home, &version("3.1.0"), None, &paths).unwrap();
    let second = upgrade_files(&home, &version("3.1.0"), None, &paths).unwrap();

    let mut copied = older_before.clone();
    copied.remove(Path::new("daemon.json"));
    let shown = |names: &[&str]| -> Vec<PathBuf> {
        names.iter().map(|name| Path::new("3").join(name)).collect()
    };
    let files = [
        "configuration.toml",
        "operator-policy.toml",
        "run-index.sqlite",
        "runs/crawl-1/run.sqlite",
    ];
    assert_eq!(
        (
            first,
            second,
            tree(&home.root().join("2")) == older_before,
            tree(&home.root().join("3")) == copied,
        ),
        (
            Upgraded {
                from: Some("2".to_owned()),
                to: "3".to_owned(),
                copied: shown(&files),
                unchanged: vec![],
                kept: vec![],
            },
            Upgraded {
                from: Some("2".to_owned()),
                to: "3".to_owned(),
                copied: vec![],
                unchanged: shown(&files),
                kept: vec![],
            },
            true,
            true,
        )
    );
}

/// None of the older major's Daemon's own files reaches the new major: its
/// runtime file, socket, locks, service record, the page that opens its
/// Console, and its tokens, which grant access to that Daemon alone.
///
/// Covers: story:3-96
#[test]
fn no_file_of_the_older_majors_daemon_reaches_the_new_major() {
    let env = TestEnv::new();
    let home = home_of_major_2(&env);
    let daemon_files = [
        "console.html",
        "daemon-service.json",
        "daemon-start.lock",
        "daemon.json",
        "daemon.lock",
        "daemon.sock",
        "daemon.token",
        "http-listener.token",
        "ops.token",
    ];
    for name in daemon_files {
        std::fs::write(home.root().join("2").join(name), name).unwrap();
    }
    let paths = UpgradePaths::parse("[[source]]\nmajor = \"2\"\n").unwrap();

    upgrade_files(&home, &version("3.1.0"), None, &paths).unwrap();

    let reached: Vec<PathBuf> = tree(&home.root().join("3"))
        .into_keys()
        .filter(|path| daemon_files.iter().any(|name| path == Path::new(name)))
        .collect();
    assert_eq!(reached, Vec::<PathBuf>::new());
}

/// A Goodfetch home that cannot be read is an error, never "nothing to
/// upgrade", which would leave the older major's files behind unsaid.
///
/// Covers: story:3-96
#[test]
fn a_goodfetch_home_that_cannot_be_read_is_an_error_not_nothing_to_upgrade() {
    let env = TestEnv::new();
    let root = env.folder().join("a file, not a home");
    std::fs::write(&root, "").unwrap();
    let paths = UpgradePaths::parse("[[source]]\nmajor = \"2\"\n").unwrap();

    let upgraded = upgrade_files(&Home::new(root), &version("3.1.0"), None, &paths);

    assert!(
        matches!(upgraded, Err(UpgradeError::Io { .. })),
        "{upgraded:?}"
    );
}

/// A file the new major already holds, other than the older major's, is
/// kept as it is (ADR 0030: never lose what the new major wrote).
///
/// Covers: story:3-96
#[test]
fn a_file_the_new_major_already_holds_is_kept() {
    let env = TestEnv::new();
    let home = home_of_major_2(&env);
    let newer = home.root().join("3");
    std::fs::create_dir_all(&newer).unwrap();
    std::fs::write(newer.join("configuration.toml"), "version = \"2.0\"\n").unwrap();
    let paths = UpgradePaths::parse("[[source]]\nmajor = \"2\"\n").unwrap();

    let upgraded = upgrade_files(&home, &version("3.0.0"), Some("2"), &paths).unwrap();

    assert_eq!(
        (
            upgraded.kept,
            std::fs::read_to_string(newer.join("configuration.toml")).unwrap()
        ),
        (
            vec![Path::new("3").join("configuration.toml")],
            "version = \"2.0\"\n".to_owned()
        )
    );
}

/// During `0.y` previews each `0.y` minor counts as a major (ADR 0030): the
/// previous preview's files are an older major's, and a home with no older
/// major's files has nothing to upgrade.
///
/// Covers: story:3-96
#[test]
fn each_preview_minor_upgrades_from_the_previous_preview() {
    let env = TestEnv::new();
    let home = home_of_major_2(&env);
    std::fs::rename(home.root().join("2"), home.root().join("0.4")).unwrap();
    let paths = UpgradePaths::parse("[[source]]\nmajor = \"0.4\"\n").unwrap();

    let upgraded = upgrade_files(&home, &version("0.5.2"), None, &paths).unwrap();
    let nothing = upgrade_files(&home, &version("0.4.0"), None, &paths).unwrap();

    assert_eq!(
        (upgraded.from.as_deref(), upgraded.copied.len(), nothing),
        (
            Some("0.4"),
            4,
            Upgraded {
                to: "0.4".to_owned(),
                ..Upgraded::default()
            }
        )
    );
}

/// `upgrade-files` replaces older majors' managed rows of the Usage ledger
/// with the new major's, and drops those of majors it no longer supports,
/// so that an uninstalled major's cap does not linger (ADR 0041).
///
/// Covers: story:3-77
#[test]
fn the_new_major_takes_over_the_older_majors_managed_limits() {
    let env = TestEnv::new();
    let home = home_of_major_2(&env);
    let ledger = Ledger::at(home.usage_ledger(), None);
    let now = jiff::Timestamp::UNIX_EPOCH;
    let install_limit = |amount| goodfetch_protocol::usage::ManagedLimit {
        usage_kind: "requests".to_owned(),
        currency: None,
        period: None,
        amount,
    };
    ledger
        .write_managed("1", &[install_limit(10).limit()], now)
        .unwrap();
    ledger
        .write_managed("2", &[install_limit(100).limit()], now)
        .unwrap();
    let paths = UpgradePaths::parse("[[source]]\nmajor = \"2\"\n").unwrap();

    upgrade_files(&home, &version("3.0.0"), None, &paths).unwrap();

    let rows: Vec<(Option<String>, u64)> = ledger
        .usage(goodfetch_protocol::catalog::Product::Collie, now)
        .unwrap()
        .into_iter()
        .map(|usage| (usage.managed_major, usage.limit.amount))
        .collect();
    assert_eq!(rows, [(Some("3".to_owned()), 100)]);
}
