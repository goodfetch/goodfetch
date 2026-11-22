//! The run-state and Politeness rows' plants (ADR 0140): one extra unit of a row's work per
//! page served, done for real through the code that counts it, so a counter
//! that stops seeing the work fails. A plant's run file, settings and Audit
//! log are its own, beside the crawl's, and run on a runtime of their own: the
//! crawl's stays where it is while its site serves a page. The runtime lives
//! as long as the benchmark's process.
#![expect(
    clippy::disallowed_methods,
    reason = "benchmark harness: a plant blocks the crawl's one thread on its own work, on purpose; its SQL statements are its own, on no Shared path (ADR 0140)"
)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::politeness::{
    KeyState, MemoryTables, Politeness, PolitenessOptions, PolitenessTables,
};
use goodfetch_engine::robots_txt::{Answer, Copies as _};
use goodfetch_engine::run_state::{RunState, RunStateOptions, RunStateTables};
use goodfetch_engine::settings::{HostDefaults, HostSearch, Settings, SettingsFiles};
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
        // One job more on a writer: a commit, which at `full` syncs. The
        // planted writer batches, as a Run that sets nothing does (ADR
        // 0142), and the job is a barrier, so it also seals a batch early.
        "F2.transactions" | "F2.syncs" | "F2.batches" => {
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
        // A read no longer reaches the writer (#1923): `RunState::read` runs
        // on the file's readers, so nothing reads through the writer, and
        // the row's counted seam is the count alone.
        "F2.writer_reads" => {
            let work = counted.work.clone();
            Arc::new(move |_: &str| work.count(Row::WriterReads))
        }
        // A page waits for the writer inside the crawl's own tasks, which
        // nothing beside the crawl enters: the rows' counted seam is the
        // count alone, held at `batch` and `off`, and at `full`.
        "F2.page_waits" | "F2.page_waits.full" => {
            let work = counted.work.clone();
            Arc::new(move |_: &str| work.count(Row::RunPageWaits))
        }
        // The Frontier reads its evidence inside a page's learning, which
        // nothing beside the crawl enters: the row's counted seam is the
        // count alone.
        "F2.evidence_reads" => {
            let work = counted.work.clone();
            Arc::new(move |_: &str| work.count(Row::EvidenceReads))
        }
        // The same for the evidence an admission reads and writes, which
        // learning off must not touch either.
        "F2.admission_evidence" | "F2.learning_off_evidence" => {
            let work = counted.work.clone();
            Arc::new(move |_: &str| work.count(Row::AdmissionEvidence))
        }
        // The Frontier learns on a page's path only where its Run does not
        // learn behind, which no crawl's Run is: the row's counted seam is
        // the count alone.
        "F2.page_learning" => {
            let work = counted.work.clone();
            Arc::new(move |_: &str| work.count(Row::PageLearning))
        }
        "F2.configuration_reads" => {
            let (runtime, folder) = beside_the_crawl();
            let version = ProductVersion::parse("1.4.0").expect("invariant: a literal version");
            let settings = Settings::with_host(
                SettingsFiles::of(&Home::new(folder.path().to_owned()), &version),
                Product::Foxhound,
                TaskOwner::root(BlockingPolicy::Inline),
                counted.work.clone(),
                HostSearch::opening(|| Box::pin(std::future::ready(HostDefaults::default()))),
            );
            Arc::new(move |_: &str| {
                let _kept = &folder;
                beside(runtime, settings.machine())
                    .expect("invariant: a home with no settings file reads as empty");
            })
        }
        "F2.prunes" => {
            let (runtime, folder) = beside_the_crawl();
            let log = AuditLog::of(
                &Home::new(folder.path().to_owned()),
                None,
                Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
            )
            .counting_on(counted.work.clone());
            Arc::new(move |_: &str| {
                let _kept = &folder;
                beside(
                    runtime,
                    log.prune(Duration::from_secs(1), None, jiff::Timestamp::UNIX_EPOCH),
                )
                .expect("invariant: an Audit log with nothing to prune");
            })
        }
        // The journal's synced commit is not built until #1550: its counted
        // seam is the count alone.
        "F2.effect_syncs" => {
            let work = counted.work.clone();
            Arc::new(move |_: &str| work.count(Row::EffectSyncs))
        }
        // Callgrind counts SQLite's own function beneath the writer's: one
        // statement a write prepares again on each page.
        "F2.prepares" => {
            let (runtime, folder, state) = writer(counted);
            Arc::new(move |_: &str| {
                let _kept = &folder;
                let prepare =
                    |connection: &rusqlite::Connection| connection.prepare("SELECT 1").map(drop);
                beside(runtime, state.write(prepare))
                    .expect("invariant: the planted file takes a write");
            })
        }
        other => panic!("no plant for the row {other}"),
    }
}

/// The table a Politeness plant writes, as a commit with nothing to write is
/// not a transaction its commit hook sees.
const PLANTED_POLITENESS: PolitenessTables = PolitenessTables {
    name: "planted",
    statements: &["CREATE TABLE IF NOT EXISTS planted (value INTEGER NOT NULL) STRICT"],
    prepared: &[],
};

/// A Politeness file of its own, in its plant's folder, counting on
/// `work`, opened before the crawl so its opening is no page's. A Run's own
/// Politeness keeps no file (#2171), so a file's work is what a plant adds.
fn politeness(work: &CountedWork) -> (&'static Runtime, Politeness) {
    let (runtime, folder) = beside_the_crawl();
    let options = PolitenessOptions {
        work: work.clone(),
        ..PolitenessOptions::default()
    };
    let file = beside(runtime, async {
        let owner: &'static TaskOwner =
            Box::leak(Box::new(TaskOwner::root(BlockingPolicy::Inline)));
        Politeness::open(
            folder.path().join("politeness.sqlite"),
            &options,
            vec![PLANTED_POLITENESS, goodfetch_frontier::ROBOTS_TXT_TABLES],
            |_, _| false,
            "planted".to_owned(),
            owner,
        )
        .await
        .expect("invariant: SQLite opens a Politeness file in a temporary folder")
    });
    // The folder lasts as long as the process, as the runtime does.
    std::mem::forget(folder);
    (runtime, file)
}

/// A planted key's state on a Politeness file: loading it prepares one
/// statement more when its change asks, and saving it writes one row more
/// when its change wrote, in a transaction counted on `E6.transactions`.
struct Planted;

/// The same, its transaction counted on `E6.delegation_transactions`.
struct PlantedDelegation;

/// The same, its transaction counted on `E6.other_transactions`.
struct PlantedOther;

/// Implements [`KeyState`] for a planted state, its transactions counted
/// on `row`.
macro_rules! planted {
    ($state:ident, $row:expr) => {
        impl KeyState for $state {
            type Key = str;
            /// Whether loading it prepares a statement.
            type Reads = bool;
            /// Whether saving it writes a row.
            type Writes = bool;

            const TRANSACTIONS: Row = $row;

            fn load(
                connection: &rusqlite::Connection,
                _: &str,
                prepares: bool,
            ) -> rusqlite::Result<Self> {
                if prepares {
                    connection.prepare("SELECT 1").map(drop)?;
                }
                Ok(Self)
            }

            fn save(
                &self,
                connection: &rusqlite::Connection,
                _: &str,
                writes: &bool,
            ) -> rusqlite::Result<()> {
                if *writes {
                    connection.execute("INSERT INTO planted VALUES (1)", [])?;
                }
                Ok(())
            }

            fn load_from(_: &MemoryTables, _: &str, _: bool) -> Self {
                Self
            }

            fn save_to(&self, _: &mut MemoryTables, _: &str, _: &bool) {}
        }
    };
}

planted!(Planted, Row::PolitenessTransactions);
planted!(PlantedDelegation, Row::PolitenessDelegationTransactions);
planted!(PlantedOther, Row::PolitenessOtherTransactions);

/// The plant of the row named `name`, one of the Politeness permit's (E6).
pub(crate) fn politeness_plant(name: &str, work: &CountedWork) -> Plant {
    match name {
        // One transaction more on a Politeness file, which Politeness off
        // and a Run's own Politeness must not do either.
        "E6.transactions"
        | "E6.off_work"
        | "E6.private_transactions"
        | "E6.undelegated_transactions"
        | "E6.shared_transactions" => {
            let (runtime, file) = politeness(work);
            Arc::new(move |_: &str| {
                let insert = file.change("planted", false, |_: &mut Planted| ((), true));
                beside(runtime, insert).expect("invariant: the planted file takes a write");
            })
        }
        // One transaction more on a Politeness file, counted as a
        // Delegation's or as another.
        "E6.delegation_transactions" => {
            let (runtime, file) = politeness(work);
            Arc::new(move |_: &str| {
                let insert = file.change("planted", false, |_: &mut PlantedDelegation| ((), true));
                beside(runtime, insert).expect("invariant: the planted file takes a write");
            })
        }
        "E6.other_transactions" => {
            let (runtime, file) = politeness(work);
            Arc::new(move |_: &str| {
                let insert = file.change("planted", false, |_: &mut PlantedOther| ((), true));
                beside(runtime, insert).expect("invariant: the planted file takes a write");
            })
        }
        // A Delegation is taken by a Politeness file's actor for a Run's
        // take, which a plant has no Run for: its counted seam is the count
        // alone.
        "E6.delegations" => {
            let work = work.clone();
            Arc::new(move |_: &str| work.count(Row::PolitenessDelegations))
        }
        // One statement prepared more on a Politeness file's connection,
        // which Callgrind sees beneath the file's own functions.
        "E6.opens" => {
            let (runtime, file) = politeness(work);
            Arc::new(move |_: &str| {
                let prepare = file.change("planted", true, |_: &mut Planted| ((), false));
                beside(runtime, prepare).expect("invariant: the planted file prepares a statement");
            })
        }
        // No Politeness store sends a script until one lands: its counted
        // seam is the count alone.
        "E6.script_sends" => {
            let work = work.clone();
            Arc::new(move |_: &str| work.count(Row::PolitenessScriptSends))
        }
        // One read more of a shared robots.txt copy, which parses it again:
        // each through a copy of its own, as a Run keeps the parse of what it
        // read.
        "E6.robots_parses" | "E6.robots_reads" => {
            let (runtime, file) = politeness(work);
            // A paused Clock is made on the runtime it runs on.
            let clock = beside(runtime, async { Clock::paused(std::time::UNIX_EPOCH) });
            let location = url::Url::parse("http://planted.test/robots.txt")
                .expect("invariant: a literal URL");
            Arc::new(move |_: &str| {
                let copies = goodfetch_frontier::SharedRobotsTxt::new(file.clone(), clock);
                let fetch = Box::pin(std::future::ready(Answer::Response {
                    status: 200,
                    body: b"User-agent: *\nAllow: /\n".to_vec(),
                }));
                beside(runtime, copies.copy(&location, fetch));
            })
        }
        other => panic!("no plant for the row {other}"),
    }
}
