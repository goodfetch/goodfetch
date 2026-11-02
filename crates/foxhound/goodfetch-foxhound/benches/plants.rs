//! The run-state rows' plants (ADR 0140): one extra unit of a row's work per
//! page served, done for real through the code that counts it, so a counter
//! that stops seeing the work fails. A plant's run file, settings and Audit
//! log are its own, beside the crawl's, and run on a runtime of their own: the
//! crawl's stays where it is while its site serves a page. The runtime lives
//! as long as the benchmark's process.
#![expect(
    clippy::disallowed_methods,
    reason = "benchmark harness: a plant blocks the crawl's one thread on its own work, on purpose"
)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::home::Home;
use goodfetch_engine::run_state::{RunState, RunStateOptions, RunStateTables};
use goodfetch_engine::settings::{HostDefaults, Settings, SettingsFiles};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use tokio::runtime::Runtime;

/// One extra unit of a row's work, as the scenario's site calls it per page.
type Plant = Arc<dyn Fn(&str) + Send + Sync>;

/// What a plant counts on: the crawl's own counts and its run file's VFS.
pub(crate) struct Counted<'a> {
    pub(crate) work: &'a CountedWork,
    pub(crate) vfs: &'a str,
}

/// Runs `future` to its end on `runtime`, from a thread outside the crawl's.
fn beside<T: Send>(runtime: &Runtime, future: impl Future<Output = T> + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(|| runtime.block_on(future))
            .join()
            .expect("invariant: a plant's work does not panic")
    })
}

/// A runtime for one plant, for the process's life, and the folder its files
/// are in.
fn beside_the_crawl() -> (&'static Runtime, Arc<tempfile::TempDir>) {
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    (
        Box::leak(Box::new(goodfetch_test_support::runtime())),
        Arc::new(folder),
    )
}

/// The table a plant writes, as a commit with nothing to write syncs nothing.
const PLANTED: RunStateTables = RunStateTables {
    name: "planted",
    statements: &["CREATE TABLE IF NOT EXISTS planted (value INTEGER NOT NULL) STRICT"],
};

/// A writer of a run file of its own through `counted`'s VFS, counting on its
/// counts.
fn writer(counted: &Counted<'_>) -> (&'static Runtime, Arc<tempfile::TempDir>, RunState) {
    let (runtime, folder) = beside_the_crawl();
    let options = RunStateOptions {
        vfs: Some(counted.vfs.to_owned()),
        work: counted.work.clone(),
        ..RunStateOptions::default()
    };
    let path = folder.path().join("planted.sqlite");
    let state = beside(runtime, async {
        let owner: &'static TaskOwner =
            Box::leak(Box::new(TaskOwner::root(BlockingPolicy::Inline)));
        RunState::open(path, &options, vec![PLANTED], 1, owner)
            .await
            .expect("invariant: a temporary folder takes a run file")
    });
    (runtime, folder, state)
}

/// The plant of the row named `name`, one of the run-state writer's.
pub(crate) fn run_state_plant(name: &str, counted: &Counted<'_>) -> Plant {
    match name {
        // One job more on a writer: a commit, which at `full` syncs.
        "F2.transactions" | "F2.syncs" => {
            let (runtime, folder, state) = writer(counted);
            Arc::new(move |_: &str| {
                let _kept = &folder;
                let insert = |connection: &rusqlite::Connection| {
                    connection.execute("INSERT INTO planted VALUES (1)", [])
                };
                beside(runtime, state.write(insert))
                    .expect("invariant: the planted file takes a write");
            })
        }
        "F2.writer_reads" => {
            let (runtime, folder, state) = writer(counted);
            Arc::new(move |_: &str| {
                let _kept = &folder;
                beside(runtime, state.read(|_| Ok(())))
                    .expect("invariant: the planted file takes a read");
            })
        }
        "F2.configuration_reads" => {
            let (runtime, folder) = beside_the_crawl();
            let version = ProductVersion::parse("1.4.0").expect("invariant: a literal version");
            let settings = Settings::with_host(
                SettingsFiles::of(&Home::new(folder.path().to_owned()), &version),
                Product::Foxhound,
                TaskOwner::root(BlockingPolicy::Inline),
                counted.work.clone(),
                || Box::pin(std::future::ready(HostDefaults::default())),
            );
            Arc::new(move |_: &str| {
                let _kept = &folder;
                beside(runtime, settings.machine())
                    .expect("invariant: a home with no settings file reads as empty");
            })
        }
        "F2.prunes" => {
            let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
            let log = AuditLog::of(&Home::new(folder.path().to_owned()), None)
                .counting_on(counted.work.clone());
            Arc::new(move |_: &str| {
                let _kept = &folder;
                log.prune(Duration::from_secs(1), None, jiff::Timestamp::UNIX_EPOCH)
                    .expect("invariant: an Audit log with nothing to prune");
            })
        }
        // The journal's synced commit is not built until #1550: its counted
        // seam is the count alone.
        "F2.effect_syncs" => {
            let work = counted.work.clone();
            Arc::new(move |_: &str| work.count(Row::EffectSyncs))
        }
        // Callgrind counts the process's calls of SQLite's own function, on
        // any connection.
        "F2.prepares" => Arc::new(|_: &str| {
            let connection = rusqlite::Connection::open_in_memory()
                .expect("invariant: SQLite opens an in-memory database");
            let _ = connection
                .prepare("SELECT 1")
                .expect("invariant: a literal statement");
        }),
        other => panic!("no plant for the row {other}"),
    }
}
