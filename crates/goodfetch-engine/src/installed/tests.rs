//! The list of installed executables and the pick (spec #213's Seam 4):
//! what counts as installed on the test `Clock`'s readings, which
//! executable of a major runs its Daemon, and a rewrite of the list that
//! keeps what a newer engine wrote (ADR 0098).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests lay out and read the list's file themselves; its SQL statements are its own, on no Shared path (ADR 0140)"
)]

use std::time::{Duration, SystemTime};

use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind};
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::executables::{ExecutableKind, InstalledExecutable};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::{InstalledList, Seen, counts_as_installed, pick};
use crate::browser_cache::MaxUnusedAge;
use crate::home::Home;

const DAY: Duration = Duration::from_hours(24);

fn now() -> jiff::Timestamp {
    "2026-10-20T12:00:00Z".parse().unwrap()
}

fn days_ago(days: u64) -> jiff::Timestamp {
    now() - DAY * u32::try_from(days).unwrap()
}

fn build(name: &str) -> ArtifactId {
    ArtifactId {
        kind: ArtifactKind::Browser,
        name: name.to_owned(),
        build: "1".to_owned(),
    }
}

fn entry(product: Product, version: &str, kind: ExecutableKind) -> InstalledExecutable {
    InstalledExecutable {
        kind,
        product,
        product_version: ProductVersion::parse(version).unwrap(),
        path: format!(
            "/opt/{}/{version}/goodfetch-{}",
            product.as_str(),
            product.as_str()
        ),
        size: 1,
        modified_at: SystemTime::from(days_ago(100)),
        pins: vec![build("chrome-for-testing")],
        copy: None,
        last_seen: SystemTime::from(now()),
    }
}

fn executable(product: Product, version: &str) -> InstalledExecutable {
    entry(product, version, ExecutableKind::ProductExecutable)
}

/// `entry`, last seen `days` ago.
fn last_seen(entry: InstalledExecutable, days: u64) -> InstalledExecutable {
    InstalledExecutable {
        last_seen: SystemTime::from(days_ago(days)),
        ..entry
    }
}

/// `entry`, at its path or not, with its versioned copy in the Browser
/// cache or not.
fn seen(entry: &InstalledExecutable, at_path: bool, copy: bool) -> Seen<'_> {
    Seen {
        entry,
        still_at_path: at_path,
        copy_present: copy,
    }
}

/// Covers: story:3-139, setting:browser_cache.max_unused_age
#[rstest]
#[case::at_its_path_unseen_for_a_year(true, 365, MaxUnusedAge::default(), true)]
#[case::gone_but_seen_within_the_age(false, 29, MaxUnusedAge::default(), true)]
#[case::gone_and_unseen_for_the_age(false, 30, MaxUnusedAge::default(), false)]
#[case::gone_within_a_shorter_age(false, 2, MaxUnusedAge::new(Some(DAY * 3)), true)]
#[case::gone_past_a_shorter_age(false, 3, MaxUnusedAge::new(Some(DAY * 3)), false)]
#[case::gone_while_the_age_is_null(false, 29, MaxUnusedAge::new(None), true)]
fn an_entry_counts_as_installed_at_its_path_or_within_the_unused_age(
    #[case] still_at_path: bool,
    #[case] days: u64,
    #[case] age: MaxUnusedAge,
    #[case] counts: bool,
) {
    assert_eq!(
        counts_as_installed(still_at_path, SystemTime::from(days_ago(days)), now(), age),
        counts
    );
}

fn version_of(picked: Option<&InstalledExecutable>) -> Option<(Product, String)> {
    picked.map(|entry| (entry.product, entry.product_version.to_string()))
}

/// Covers: story:3-137
#[test]
fn the_pick_is_foxhound_whenever_one_of_the_major_counts_and_can_start() {
    let (collie, foxhound) = (
        executable(Product::Collie, "1.5.0"),
        last_seen(executable(Product::Foxhound, "1.4.0"), 5),
    );
    let list = [seen(&collie, true, false), seen(&foxhound, true, false)];

    let picked = pick(
        &list,
        &ProductVersion::parse("1.5.0").unwrap(),
        None,
        now(),
        MaxUnusedAge::default(),
    );

    assert_eq!(
        version_of(picked),
        Some((Product::Foxhound, "1.4.0".to_owned()))
    );
}

/// Covers: story:3-137
#[test]
fn the_pick_is_the_newest_version_of_the_fullest_product() {
    let versions = [
        executable(Product::Collie, "1.10.0"),
        executable(Product::Collie, "1.9.3"),
        executable(Product::Collie, "1.10.0-rc.1"),
    ];
    let list: Vec<Seen<'_>> = versions
        .iter()
        .map(|entry| seen(entry, true, false))
        .collect();

    let picked = pick(
        &list,
        &ProductVersion::parse("1.0.0").unwrap(),
        None,
        now(),
        MaxUnusedAge::default(),
    );

    assert_eq!(
        version_of(picked),
        Some((Product::Collie, "1.10.0".to_owned()))
    );
}

/// Covers: story:3-137
#[rstest]
#[case::a_linked_program_is_never_the_pick(
    entry(Product::Foxhound, "1.6.0", ExecutableKind::LinkedProgram),
    0,
    true,
    true
)]
#[case::gone_from_its_path_without_a_copy(executable(Product::Foxhound, "1.6.0"), 1, false, false)]
#[case::no_longer_counting(executable(Product::Foxhound, "1.6.0"), 31, false, true)]
#[case::of_another_major(executable(Product::Foxhound, "2.0.0"), 0, true, true)]
fn the_pick_passes_over_what_cannot_run_the_daemon(
    #[case] passed_over: InstalledExecutable,
    #[case] days: u64,
    #[case] at_path: bool,
    #[case] copy: bool,
) {
    let collie = executable(Product::Collie, "1.5.0");
    let passed_over = last_seen(passed_over, days);
    let list = [
        seen(&collie, true, false),
        seen(&passed_over, at_path, copy),
    ];

    let picked = pick(
        &list,
        &ProductVersion::parse("1.5.0").unwrap(),
        None,
        now(),
        MaxUnusedAge::default(),
    );

    assert_eq!(
        version_of(picked),
        Some((Product::Collie, "1.5.0".to_owned()))
    );
}

/// Covers: story:3-137
#[test]
fn an_executable_gone_from_its_path_runs_the_daemon_from_its_versioned_copy() {
    let foxhound = last_seen(executable(Product::Foxhound, "1.4.0"), 3);
    let list = [seen(&foxhound, false, true)];

    let picked = pick(
        &list,
        &ProductVersion::parse("1.4.0").unwrap(),
        None,
        now(),
        MaxUnusedAge::default(),
    );

    assert_eq!(
        version_of(picked),
        Some((Product::Foxhound, "1.4.0".to_owned()))
    );
}

/// A store whose lowest minor a newer engine raised passes over every
/// older executable: an older Foxhound then gives way to a newer Collie,
/// and nothing older than the floor can be the pick (ADR 0098).
///
/// Covers: story:3-137
#[rstest]
#[case::below_the_floor_foxhound_gives_way("1.5", Some((Product::Collie, "1.5.0".to_owned())))]
#[case::below_every_executable("1.6", None)]
fn the_pick_runs_no_executable_older_than_the_stores_lowest_minor(
    #[case] lowest_minor: &str,
    #[case] expected: Option<(Product, String)>,
) {
    let (collie, foxhound) = (
        executable(Product::Collie, "1.5.0"),
        executable(Product::Foxhound, "1.4.0"),
    );
    let list = [seen(&collie, true, false), seen(&foxhound, true, false)];
    let floor = lowest_minor.parse().unwrap();

    let picked = pick(
        &list,
        &ProductVersion::parse("1.5.0").unwrap(),
        Some(floor),
        now(),
        MaxUnusedAge::default(),
    );

    assert_eq!(version_of(picked), expected);
}

/// Covers: story:3-139, format:installed-executables
#[test]
fn recording_an_executable_keeps_every_entry_and_field_a_newer_engine_wrote() {
    let env = TestEnv::new();
    let list = InstalledList::of(&Home::new(env.home().to_owned()));
    let mine = executable(Product::Collie, "1.5.0");
    let newer = json!({
        "format_version": "0.7",
        "written_by": "a newer engine",
        "executables": [
            { "kind": "a_kind_of_tomorrow", "path": "/opt/tomorrow" },
            {
                "kind": "product_executable", "product": "collie", "product_version": "1.5.0",
                "path": mine.path, "size": 7, "modified_at": "2026-01-01T00:00:00Z",
                "pins": [], "last_seen": "2026-01-01T00:00:00Z", "sandbox": "kept",
            },
        ],
    });
    std::fs::create_dir_all(env.home()).unwrap();
    std::fs::write(list.path(), serde_json::to_vec(&newer).unwrap()).unwrap();

    list.record(&mine).unwrap();

    let written: Value = serde_json::from_slice(&std::fs::read(list.path()).unwrap()).unwrap();
    assert_eq!(
        written,
        json!({
            "format_version": "0.7",
            "written_by": "a newer engine",
            "executables": [
                { "kind": "a_kind_of_tomorrow", "path": "/opt/tomorrow" },
                {
                    "kind": "product_executable", "product": "collie", "product_version": "1.5.0",
                    "path": mine.path, "size": 1, "modified_at": "2026-07-12T12:00:00Z",
                    "pins": [{ "kind": "browser", "name": "chrome-for-testing", "build": "1" }],
                    "last_seen": "2026-10-20T12:00:00Z", "sandbox": "kept",
                },
            ],
        })
    );
}

/// Covers: format:installed-executables
#[test]
fn a_list_of_a_newer_major_is_read_as_empty_and_never_rewritten() {
    let env = TestEnv::new();
    let list = InstalledList::of(&Home::new(env.home().to_owned()));
    let newer = br#"{"format_version": "1.0", "executables": []}"#;
    std::fs::create_dir_all(env.home()).unwrap();
    std::fs::write(list.path(), newer).unwrap();

    let read = list.read().unwrap();
    let recorded = list.record(&executable(Product::Collie, "1.5.0"));

    assert_eq!(
        (read, recorded.is_err(), std::fs::read(list.path()).unwrap()),
        (Vec::new(), true, newer.to_vec())
    );
}

/// Covers: story:3-139
#[test]
fn recording_the_same_executable_again_updates_its_one_entry() {
    let env = TestEnv::new();
    let list = InstalledList::of(&Home::new(env.home().to_owned()));
    let first = last_seen(executable(Product::Foxhound, "1.4.0"), 10);
    let again = executable(Product::Foxhound, "1.4.0");

    list.record(&first).unwrap();
    list.record(&executable(Product::Foxhound, "1.5.0"))
        .unwrap();
    list.record(&again).unwrap();

    assert_eq!(
        list.read().unwrap(),
        [again, executable(Product::Foxhound, "1.5.0")]
    );
}

/// `daemon uninstall` forgets the copy its service ran, and only that
/// entry, keeping what a newer engine wrote beside it.
///
/// Covers: story:3-21, story:3-139
#[test]
fn forgetting_an_executable_removes_its_one_entry_and_keeps_the_rest() {
    let env = TestEnv::new();
    let list = InstalledList::of(&Home::new(env.home().to_owned()));
    let kept = executable(Product::Collie, "1.4.0");
    let forgotten = executable(Product::Foxhound, "1.4.0");
    list.record(&kept).unwrap();
    list.record(&forgotten).unwrap();
    let mut document: Value = serde_json::from_slice(&std::fs::read(list.path()).unwrap()).unwrap();
    document["executables"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "kind": "a_kind_of_a_newer_engine" }));
    std::fs::write(list.path(), serde_json::to_vec(&document).unwrap()).unwrap();

    list.forget(&forgotten).unwrap();
    list.forget(&forgotten).unwrap();

    let document: Value = serde_json::from_slice(&std::fs::read(list.path()).unwrap()).unwrap();
    assert_eq!(
        (
            list.read().unwrap(),
            document["executables"].as_array().unwrap().len()
        ),
        (vec![kept], 2)
    );
}

/// The lowest Product version the Daemon's pick must meet is the highest
/// lowest minor the major's shared stores record, and the store that
/// records it is named (ADR 0098).
///
/// Covers: story:3-144
#[rstest]
#[case::no_store_raised(None, None, None)]
#[case::the_run_index(
    Some(3),
    None,
    Some((ProtocolVersion { major: 2, minor: 3 }, "run-index"))
)]
#[case::user_configuration_above_the_run_index(
    Some(3),
    Some(5),
    Some((ProtocolVersion { major: 2, minor: 5 }, "user-configuration"))
)]
fn the_daemons_pick_meets_the_highest_lowest_minor_of_the_majors_stores(
    #[case] run_index: Option<u16>,
    #[case] user_configuration: Option<u16>,
    #[case] floor: Option<(ProtocolVersion, &str)>,
) {
    let folder = tempfile::tempdir().unwrap();
    let home = crate::home::Home::new(folder.path().to_owned());
    let engine = goodfetch_protocol::connection::ProductVersion::parse("2.6.0").unwrap();
    let major = home.per_major(&engine);
    std::fs::create_dir_all(&major).unwrap();
    if let Some(lowest_minor) = run_index {
        let connection = rusqlite::Connection::open(home.run_index(&engine)).unwrap();
        connection
            .execute(crate::lowest_minor::FORMAT_TABLE, [])
            .unwrap();
        crate::lowest_minor::write_sqlite(
            &connection,
            goodfetch_protocol::formats::RUN_INDEX.name,
            crate::lowest_minor::Marker {
                version: goodfetch_protocol::formats::RUN_INDEX.version,
                lowest_minor,
            },
        )
        .unwrap();
    }
    if let Some(lowest_minor) = user_configuration {
        std::fs::write(
            major.join("configuration.toml"),
            format!("version = \"0.1\"\nlowest_minor = {lowest_minor}\n"),
        )
        .unwrap();
    }

    let found = super::stores_floor(&home, &engine);

    assert_eq!(found.map(|found| (found.version, found.format.name)), floor);
}
