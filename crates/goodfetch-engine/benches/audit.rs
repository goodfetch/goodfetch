//! B3's Audit entry scenario (ADR 0140's E2), shared by its counts cell
//! (`examples/counts.rs`) and its test: an Audit log in a home of its own,
//! opened as the engine opens it (its retention read from the machine's
//! settings files), records [`ENTRIES`] entries of one Run, one second
//! apart. Each outcome the path has is in it: an entry chained, an entry of
//! a Run whose chain is off, and an append that prunes under an
//! `audit.retention`. The store opens through a counting VFS when one is
//! named, which sees its syncs. The log's writer is an actor of an owner of
//! the scenario's, on a runtime of one thread that blocks on each entry.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::audit::{AuditLog, Origin};
use goodfetch_engine::home::Home;
use goodfetch_engine::settings::SettingsFiles;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::audit::{Actor, SECRET_REFERENCE_USED};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::RunId;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use tokio::runtime::Runtime;

/// The entries each run records.
pub(crate) const ENTRIES: u64 = 20;

/// How the scenario's Audit log runs: as an engine opens it, with its Run's
/// chain off, or under an `audit.retention` that each append prunes by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Audit {
    Chained,
    Unchained,
    Retained,
}

/// One extra unit of a row's work, called after each entry with the log and
/// the runtime its writer runs on.
pub(crate) type Plant = Arc<dyn Fn(&AuditLog, &Runtime) + Send + Sync>;

/// The Product version whose settings files the log reads.
fn version() -> ProductVersion {
    ProductVersion::parse("1.4.0").expect("invariant: a literal version")
}

/// The Run every entry is recorded in.
fn run_id() -> RunId {
    RunId::parse("audited").expect("invariant: a literal Run id")
}

/// The scenario's runtime, of one thread.
pub(crate) fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("invariant: the OS gives a runtime of one thread")
}

/// The Audit log of `home`, as the engine's Runs open it, counting on
/// `work`, its store opened through the VFS named `vfs`.
pub(crate) fn log(home: &Path, vfs: Option<&str>, work: &CountedWork) -> AuditLog {
    let home = Home::new(home.to_owned());
    AuditLog::of(&home, vfs.map(str::to_owned), owner())
        .counting_on(work.clone())
        .retained_under(SettingsFiles::of(&home, &version()), Product::Collie)
}

/// Records [`ENTRIES`] entries in `home` as `audit` says, each followed by
/// `plant`; answers the entries recorded.
pub(crate) fn run(
    home: &Path,
    audit: Audit,
    vfs: Option<&str>,
    work: &CountedWork,
    plant: Option<&Plant>,
) -> u64 {
    if audit == Audit::Retained {
        let files = SettingsFiles::of(&Home::new(home.to_owned()), &version());
        let file = files.user_configuration();
        std::fs::create_dir_all(file.parent().expect("invariant: a file in a folder"))
            .expect("invariant: a temporary folder takes a folder");
        // Each append prunes the entry recorded a second before it.
        std::fs::write(
            file,
            "version = \"0.1\"\n\n[settings]\naudit.retention = 500\n",
        )
        .expect("invariant: a temporary folder takes a file");
    }
    let runtime = runtime();
    let log = log(home, vfs, work);
    if audit == Audit::Unchained {
        log.unchain(run_id());
    }
    for index in 0..ENTRIES {
        run_one(&log, &runtime, index);
        if let Some(plant) = plant {
            plant(&log, &runtime);
        }
    }
    if audit == Audit::Retained {
        // Read on a log that counts nowhere, so the check adds no open.
        let exported = AuditLog::of(&Home::new(home.to_owned()), vfs.map(str::to_owned), owner())
            .export()
            .expect("invariant: the log reads back");
        assert!(
            exported.len() < usize::try_from(ENTRIES).unwrap_or(usize::MAX),
            "the retention pruned the log"
        );
    }
    ENTRIES
}

/// The owner of a log's writer, which runs it as a task.
fn owner() -> Arc<TaskOwner> {
    Arc::new(TaskOwner::root(BlockingPolicy::Inline))
}

/// Records the scenario's entry numbered `index`, `index` seconds after its
/// start, blocking on `runtime`.
pub(crate) fn run_one(log: &AuditLog, runtime: &Runtime, index: u64) {
    let origin = Origin {
        actor: Actor::Engine,
        surface: None,
        layer: None,
        run_id: Some(run_id()),
    };
    let start: Timestamp = "2026-01-05T09:00:00Z"
        .parse()
        .expect("invariant: a literal time");
    let now = start + SignedDuration::from_secs(i64::try_from(index).unwrap_or(0));
    runtime
        .block_on(log.record(
            &origin,
            &SECRET_REFERENCE_USED,
            json!({ "reference": format!("env:SECRET_{index}"), "domain": "example.com" }),
            now,
        ))
        .expect("invariant: a temporary folder takes the Audit log");
}

/// The retention a planted prune pass prunes by.
pub(crate) const PLANTED_RETENTION: Duration = Duration::from_secs(1);
