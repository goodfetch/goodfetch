//! The Run index's lowest-minor marker (ADR 0098), from fabricated
//! versions: no build carries a version override (ADR 0059), so each engine
//! here is an [`Opener`] of a fabricated format and Product version.

use std::path::PathBuf;

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::formats::{Compatibility, Format, RUN_INDEX};
use goodfetch_protocol::run::{Lifecycle, PointInTime, Presence, RunId, RunIndexRow, RunKind};
use rusqlite::Connection;

use super::{rows_as, write_as};
use crate::lowest_minor::{self, Marker, Opener};
use crate::run_state::RunStateError;

const fn version(major: u16, minor: u16) -> ProtocolVersion {
    ProtocolVersion { major, minor }
}

/// The Run index as Goodfetch 2.2 knows it.
const RUN_INDEX_1_2: Format = Format {
    version: version(1, 2),
    ..RUN_INDEX
};

/// The Run index as Goodfetch 2.3, a newer minor, knows it.
const RUN_INDEX_1_3: Format = Format {
    version: version(1, 3),
    ..RUN_INDEX
};

fn older() -> Opener {
    Opener::new(&RUN_INDEX_1_2, version(2, 2))
}

fn newer() -> Opener {
    Opener::new(&RUN_INDEX_1_3, version(2, 3))
}

fn index() -> (tempfile::TempDir, PathBuf) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("run-index.sqlite");
    (folder, path)
}

fn row(run_id: &str) -> RunIndexRow {
    RunIndexRow {
        run_id: RunId::parse(run_id).unwrap(),
        location: format!("runs/{run_id}"),
        kind: RunKind::Collie,
        needs: Product::Collie,
        tags: Default::default(),
        lifecycle: Lifecycle::Running,
        verdict: None,
        completeness: None,
        started_at: PointInTime::from_rfc3339("2026-10-21T08:00:00Z".to_owned()),
        ended_at: None,
        owner: None,
        presence: Presence::Present,
    }
}

fn marker(path: &PathBuf) -> Option<Marker> {
    lowest_minor::read_sqlite(&Connection::open(path).unwrap(), RUN_INDEX.name).unwrap()
}

/// Covers: story:3-144, format:run-index
#[test]
fn a_newer_minor_that_adds_a_row_an_older_engine_reads_leaves_the_marker_unchanged() {
    let (_folder, path) = index();
    write_as(&path, &older(), &row("older")).unwrap();

    write_as(&path, &newer(), &row("newer")).unwrap();

    assert_eq!(
        (marker(&path), rows_as(&path, &older()).unwrap()),
        (
            Some(Marker {
                version: version(1, 3),
                lowest_minor: 0,
            }),
            vec![row("newer"), row("older")]
        )
    );
}

/// An engine reads the marker in each transaction, so one that finds it
/// raised past itself while it runs stops using the index with the version
/// refusal, naming the version to install (ADR 0098).
///
/// Covers: story:3-144, format:run-index
#[test]
fn an_engine_that_finds_the_lowest_minor_raised_stops_using_the_index() {
    let (_folder, path) = index();
    write_as(&path, &older(), &row("older")).unwrap();
    lowest_minor::write_sqlite(
        &Connection::open(&path).unwrap(),
        RUN_INDEX.name,
        &Marker {
            version: version(1, 3),
            lowest_minor: 3,
        },
    )
    .unwrap();

    let refused = write_as(&path, &older(), &row("later"));

    assert_eq!(
        (refused, rows_as(&path, &newer()).unwrap()),
        (
            Err(RunStateError::Format {
                format: "run-index",
                found: "1.3".to_owned(),
                compatibility: Compatibility::NewerMinor,
                install: Some("2.3".to_owned()),
            }),
            vec![row("older")]
        )
    );
}
