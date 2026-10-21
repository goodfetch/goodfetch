use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::formats::{Compatibility, Format, RUN_INDEX};
use rstest::rstest;

use super::{Marker, Opener, Refusal};

const fn version(major: u16, minor: u16) -> ProtocolVersion {
    ProtocolVersion { major, minor }
}

/// The Run index as an engine whose format is 1.3 knows it.
const RUN_INDEX_1_3: Format = Format {
    version: version(1, 3),
    ..RUN_INDEX
};

fn marker(found: ProtocolVersion, lowest_minor: u16) -> Marker {
    Marker {
        version: found,
        lowest_minor,
    }
}

/// Covers: story:3-144
#[test]
fn a_store_an_engine_creates_can_be_used_by_every_minor_of_its_major() {
    let opener = Opener::new(&RUN_INDEX_1_3, version(2, 5));

    assert_eq!(opener.created(), marker(version(1, 3), 0));
}

/// An older engine opens a store whose lowest minor it meets, its format's
/// minor newer included, and refuses one marked newer than itself, naming
/// the version to install (ADR 0098).
///
/// Covers: story:3-144
#[rstest]
#[case::a_newer_formats_store_it_meets(marker(version(1, 5), 4), None)]
#[case::its_own_minor(marker(version(1, 3), 4), None)]
#[case::a_store_marked_newer(
    marker(version(1, 5), 5),
    Some(Refusal {
        format: "run-index",
        found: version(1, 5),
        compatibility: Compatibility::NewerMinor,
        install: Some("2.5".to_owned()),
    })
)]
#[case::a_newer_major(
    marker(version(2, 0), 0),
    Some(Refusal {
        format: "run-index",
        found: version(2, 0),
        compatibility: Compatibility::NewerMajor,
        install: None,
    })
)]
#[case::an_older_major(
    marker(version(0, 9), 0),
    Some(Refusal {
        format: "run-index",
        found: version(0, 9),
        compatibility: Compatibility::OlderMajor,
        install: None,
    })
)]
fn an_engine_opens_a_store_whose_lowest_minor_it_meets(
    #[case] found: Marker,
    #[case] refusal: Option<Refusal>,
) {
    let opener = Opener::new(&RUN_INDEX_1_3, version(2, 4));

    assert_eq!(opener.check(&found).err(), refusal);
}

/// A newer engine raises the lowest minor only when it writes what an older
/// engine would misread, to the minor that added it, and records its newer
/// format; it never lowers either (ADR 0098).
///
/// Covers: story:3-144
#[rstest]
#[case::a_write_any_minor_reads(marker(version(1, 2), 1), None, marker(version(1, 3), 1))]
#[case::a_write_its_minor_added(
    marker(version(1, 2), 1),
    Some(version(2, 4)),
    marker(version(1, 3), 4)
)]
#[case::a_write_an_older_minor_added(
    marker(version(1, 2), 3),
    Some(version(2, 2)),
    marker(version(1, 3), 3)
)]
#[case::a_write_an_older_major_added(
    marker(version(1, 2), 1),
    Some(version(1, 7)),
    marker(version(1, 3), 1)
)]
#[case::an_older_engines_write(marker(version(1, 5), 1), None, marker(version(1, 5), 1))]
fn a_newer_engine_raises_the_lowest_minor_only_for_what_an_older_one_would_misread(
    #[case] found: Marker,
    #[case] needs: Option<ProtocolVersion>,
    #[case] after: Marker,
) {
    let opener = Opener::new(&RUN_INDEX_1_3, version(2, 4));

    assert_eq!(opener.written(found, needs), after);
}

/// During `0.y` previews each `0.y` minor counts as a major (ADR 0030): a
/// preview engine's line has the minor 0 alone, so nothing it writes raises
/// the lowest minor, and the version to install names the preview.
///
/// Covers: story:3-144
#[test]
fn a_preview_engine_never_raises_the_lowest_minor() {
    let opener = Opener::new(&RUN_INDEX_1_3, version(0, 4));

    let after = opener.written(marker(version(1, 3), 0), Some(version(0, 4)));
    let refused = opener.check(&marker(version(1, 3), 1)).err();

    assert_eq!(after, marker(version(1, 3), 0));
    assert_eq!(
        refused.and_then(|refusal| refusal.install),
        Some("0.4.1".to_owned())
    );
}

/// The lowest Product version the Daemon's pick must meet is the highest
/// lowest minor the major's shared stores record (ADR 0098).
///
/// Covers: story:3-144
#[rstest]
#[case::no_store_raised(None, None, None)]
#[case::the_run_index(Some(3), None, Some(version(2, 3)))]
#[case::user_configuration_above_the_run_index(Some(3), Some(5), Some(version(2, 5)))]
fn the_daemons_pick_meets_the_highest_lowest_minor_of_the_majors_stores(
    #[case] run_index: Option<u16>,
    #[case] user_configuration: Option<u16>,
    #[case] floor: Option<ProtocolVersion>,
) {
    let folder = tempfile::tempdir().unwrap();
    let home = crate::home::Home::new(folder.path().to_owned());
    let engine = goodfetch_protocol::connection::ProductVersion::parse("2.6.0").unwrap();
    let major = home.per_major(&engine);
    std::fs::create_dir_all(&major).unwrap();
    if let Some(lowest_minor) = run_index {
        let connection = rusqlite::Connection::open(home.run_index(&engine)).unwrap();
        connection.execute(super::FORMAT_TABLE, []).unwrap();
        super::write_sqlite(
            &connection,
            RUN_INDEX.name,
            &marker(RUN_INDEX.version, lowest_minor),
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

    assert_eq!(super::floor(&home, &engine), floor);
}
