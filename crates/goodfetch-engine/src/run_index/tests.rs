//! The Run index's lowest-minor marker (ADR 0098), from fabricated
//! versions: no build carries a version override (ADR 0059), so each engine
//! here is an [`Opener`] of a fabricated format and Product version.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::point_in_time;
use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::formats::{Compatibility, Format, RUN_INDEX};
use goodfetch_protocol::run::{Lifecycle, Presence, RunId, RunIndexRow, RunKind, Tags};
use goodfetch_test_support::{TestEnv, wal_of};
use rusqlite::Connection;

use super::{IndexFile, RunIndex};
use crate::lowest_minor::{self, Marker, Opener};
use crate::run_state::RunStateError;
use crate::sqlite_store::StoreFile as _;
use crate::tasks::{BlockingPolicy, TaskOwner};

/// Writes `row` as the engine `opener` stands for, on a connection of its own.
fn write_as(path: &Path, opener: &Opener, row: &RunIndexRow) -> Result<(), RunStateError> {
    let file = IndexFile::new(path.to_owned(), *opener, None);
    file.write(&file.open()?, row)
}

/// The rows the engine `opener` stands for reads, on a connection of its own.
fn rows_as(path: &Path, opener: &Opener) -> Result<Vec<RunIndexRow>, RunStateError> {
    let file = IndexFile::new(path.to_owned(), *opener, None);
    file.rows(&file.open()?)
}

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
        tags: Tags::default(),
        lifecycle: Lifecycle::Running,
        verdict: None,
        completeness: None,
        started_at: point_in_time::from_rfc3339("2026-10-21T08:00:00Z").unwrap(),
        ended_at: None,
        owner: None,
        presence: Presence::Present,
        progress_message: None,
        stop_reason: None,
        replayed: false,
    }
}

/// A crawl's row, which needs Foxhound.
fn crawl_row(run_id: &str) -> RunIndexRow {
    RunIndexRow {
        kind: RunKind::Crawl,
        needs: Product::Foxhound,
        ..row(run_id)
    }
}

fn marker(path: &PathBuf) -> Option<Marker> {
    lowest_minor::read_sqlite(&Connection::open(path).unwrap(), RUN_INDEX.name).unwrap()
}

/// A newer minor, a Foxhound engine's, adds a crawl's row an older engine
/// reads: the marker records the newer format and keeps its lowest minor,
/// and the older engine reads the Foxhound row as it was written (ADRs
/// 0094 and 0098).
///
/// Covers: story:3-144, format:run-index
#[test]
fn a_newer_minor_that_adds_a_row_an_older_engine_reads_leaves_the_marker_unchanged() {
    let (_folder, path) = index();
    write_as(&path, &older(), &row("older")).unwrap();

    write_as(&path, &newer(), &crawl_row("newer")).unwrap();

    assert_eq!(
        (marker(&path), rows_as(&path, &older()).unwrap()),
        (
            Some(Marker {
                version: version(1, 3),
                lowest_minor: 0,
            }),
            vec![crawl_row("newer"), row("older")]
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
        Marker {
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

/// Each generated refusal case of the Run index (ADR 0066), an index whose
/// marker holds the fabricated version and lowest minor, opened by the
/// engine the case names, or this one: the rule's verdict, each refusal's
/// Error code and message snapshotted.
///
/// Covers: story:3-95, story:3-144, format:run-index, error:format_version_unsupported
#[rstest::rstest]
fn each_generated_refusal_of_the_run_index_gets_its_verdict(
    #[files("../goodfetch-protocol/generated/refusals/run-index/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: goodfetch_protocol::refusals::RefusalCase = serde_json::from_str(case).unwrap();
    let engine = case.engine.unwrap_or_else(|| {
        goodfetch_protocol::connection::ProductVersion::parse(env!("CARGO_PKG_VERSION"))
            .unwrap()
            .major_minor()
    });
    let opener = Opener::new(&RUN_INDEX, engine);
    let (_folder, path) = index();
    write_as(&path, &opener, &row("before")).unwrap();
    lowest_minor::write_sqlite(
        &Connection::open(&path).unwrap(),
        RUN_INDEX.name,
        Marker {
            version: case.found,
            lowest_minor: case.lowest_minor.unwrap_or(0),
        },
    )
    .unwrap();

    let read = rows_as(&path, &opener);

    match (case.compatibility, read) {
        (Compatibility::Compatible, read) => {
            assert_eq!(read, Ok(vec![row("before")]), "{case:#?}");
        }
        (compatibility, Err(RunStateError::Format { found, install, .. })) => {
            let refusal = match install {
                Some(install) => crate::run::store_refusal(&RUN_INDEX, &found, &install),
                None => crate::run::format_refusal(&RUN_INDEX, &found, compatibility),
            };
            insta::assert_snapshot!(
                format!("run-index_{found}_{}", case.lowest_minor.unwrap_or(0)),
                format!("{}: {}", refusal.code(), refusal.message())
            );
        }
        (_, read) => panic!("{case:#?} was not refused: {read:?}"),
    }
}

/// Engines that open a new Run index at once, one starting a Run while
/// another reads "Needs you", each open it and read or write it, however
/// their first statements interleave: none is refused `database is locked`
/// while the busy timeout lets them take turns. Its 40 new indexes lie in
/// memory, where it was red before the fix (2367d7ee2): on a busy disk their
/// synced creations passed the Quick kill (fix-queue 123).
///
/// Covers: format:run-index
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test opens the index from threads of its own, as engines in their processes do"
)]
fn engines_that_open_a_new_index_at_once_each_open_it() {
    const ENGINES: usize = 8;
    let env = TestEnv::in_memory();
    let mut refused = Vec::new();
    for round in 0..40 {
        let path = env.folder().join(format!("run-index-{round}.sqlite"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(ENGINES));
        let engines: Vec<_> = (0..ENGINES)
            .map(|engine| {
                let path = path.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    if engine % 2 == 0 {
                        write_as(&path, &older(), &row(&format!("run-{engine}")))
                    } else {
                        rows_as(&path, &older()).map(|_| ())
                    }
                })
            })
            .collect();
        refused.extend(
            engines
                .into_iter()
                .filter_map(|engine| engine.join().unwrap().err())
                .map(|error| error.to_string()),
        );
    }
    assert_eq!(refused, Vec::<String>::new());
}

/// A read takes no write lock, so it answers while another engine holds it,
/// where it would otherwise wait out the busy timeout and fail.
///
/// Covers: format:run-index
#[test]
fn a_read_answers_while_another_engine_holds_the_write_lock() {
    let (_folder, path) = index();
    write_as(&path, &older(), &row("written")).unwrap();
    let other_engine = Connection::open(&path).unwrap();
    other_engine.execute_batch("BEGIN IMMEDIATE").unwrap();

    assert_eq!(rows_as(&path, &older()).unwrap(), vec![row("written")]);
}

/// A first open records the marker, whether a read or a write makes it.
///
/// Covers: format:run-index
#[test]
fn a_read_that_opens_a_new_index_records_its_marker() {
    let (_folder, path) = index();

    assert_eq!(
        (rows_as(&path, &older()).unwrap(), marker(&path)),
        (
            Vec::new(),
            Some(Marker {
                version: version(1, 2),
                lowest_minor: 0,
            })
        )
    );
}

/// The index holds to the synced per-user store's durability: its WAL stays
/// beside it as a write's connection closes (#2110).
#[test]
fn the_wal_stays_beside_the_index_once_a_write_closed_its_connection() {
    let env = TestEnv::in_memory();
    let path = env.folder().join("run-index.sqlite");

    write_as(&path, &older(), &row("first")).unwrap();

    assert!(wal_of(&path).exists());
}

/// An engine's Run index, through `vfs`, as tasks of the test's runtime.
fn engines_index(path: &Path, vfs: &str) -> RunIndex {
    RunIndex::of(
        path.to_owned(),
        &goodfetch_protocol::connection::ProductVersion::parse("1.0.0").unwrap(),
        Some(vfs.to_owned()),
        std::sync::Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    )
}

/// An engine's Run index opens its file once for its writes and once for
/// its reads, however many of each it makes (#1886).
#[test]
fn an_engines_index_opens_its_file_once_for_its_writes_and_once_for_its_reads() {
    let (_folder, path) = index();
    let vfs = goodfetch_test_support::CountingVfs::register();
    let index = engines_index(&path, vfs.name());

    let (listed, erased) = goodfetch_test_support::runtime().block_on(async {
        for run_id in ["first", "second", "third"] {
            index.write(row(run_id)).await.unwrap();
            index.read(RunId::parse(run_id).unwrap()).await.unwrap();
        }
        index.check().await.unwrap();
        let listed = index.rows().await.unwrap();
        let erased = index.erase(RunId::parse("second").unwrap()).await.unwrap();
        (listed, erased)
    });

    assert_eq!((listed.len(), erased, vfs.opens()), (3, true, 2));
}

/// Rows written are in the index after a power cut once their calls
/// answered, while the engine still holds the index open for its writes and
/// reads: each commit synced them (ADR 0060, #1886).
#[test]
fn rows_written_survive_a_power_cut_while_the_engine_holds_the_index() {
    let env = TestEnv::new();
    let vfs = goodfetch_test_support::CrashVfs::register(
        &env.unique_name("run-index-power-cut"),
        env.seed(),
    )
    .unwrap();
    let path = env.home().join("run-index.sqlite");
    let index = engines_index(&path, vfs.name());
    goodfetch_test_support::runtime().block_on(async {
        for run_id in ["first", "second"] {
            index.write(row(run_id)).await.unwrap();
            index.read(RunId::parse(run_id).unwrap()).await.unwrap();
        }
    });

    vfs.crash(goodfetch_test_support::CrashKind::PowerCut);
    let connection = goodfetch_test_support::after_crash(&vfs, &path.to_string_lossy()).unwrap();

    let runs: Vec<String> = connection
        .prepare("SELECT run_id FROM runs ORDER BY run_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(runs, ["first", "second"]);
}
