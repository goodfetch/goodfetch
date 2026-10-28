//! The Resource governor (ADR 0040): one per engine process, under its
//! ownership tree, which admits three kinds of work separately, HTTP and
//! discovered-API fetches, open browser pages and model calls, each between
//! its floor and its ceiling, and shares each fairly between the live Runs.
//! At every `governor.sample_interval`, read on the `Clock`, it reads the
//! machine (the `system` module), its settings from the machine's layers,
//! which may change between samples, the engine's CPU time against its
//! allowance from `available_parallelism`, and the Tokio workers' busy time
//! (`worker_total_busy_duration`); its controller sizes each kind's target
//! from them, the `shares` module admits up to it, and what it read and
//! decided is its [`SystemInfo`], which the engine sends as
//! `engine.system_info` and records as metrics.
//!
//! At each sample it also reads, once per filesystem, every filesystem
//! holding a folder it is asked to sample (ADR 0122): the run folders, the
//! Goodfetch home and the engine's temporary folder, and any folder a Run
//! writes to that its holder adds. Each is at a storage level, `normal`,
//! `low` or `critical`, set by `storage.low_free` and
//! `storage.critical_free`, read from the machine's layers at each sample;
//! a level change is logged at warning, or at info on the way back. What
//! it found is its [`StorageSample`], which the Runs act on.
//!
//! An engine samples only when its starter gives it the machine to read, as
//! each Product's executable does; otherwise its governor holds each kind
//! at its default ceiling, ADR 0004's fixed cap, and reads no filesystem.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::governor::{
    CRITICAL, Concurrency, CpuReading, GovernorLevel, KindConcurrency, LAST_RESORT_AFTER,
    MEMORY_ALLOWANCE, MemoryReading, PRESSURE, SAMPLE_INTERVAL, SystemInfo, WorkKind,
};
use goodfetch_protocol::storage::StorageLevel;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::clock::Clock;
use crate::settings::{Layers, Resolved, Settings, resolve};
use crate::tasks::TaskOwner;

mod browsers;
mod controller;
mod ladder;
mod shares;
mod storage;
mod system;

pub use browsers::{BrowserUse, GovernedBrowser, PageHold};
pub use ladder::{Weight, last_resort_run};
pub use shares::{Permit, RunShare};
pub use storage::{Filesystem, Held, StorageSample};
pub use system::{System, footprint_of};

#[cfg(test)]
pub(crate) use storage::DiskView;
use storage::{Disks, Folders, Settled};

use controller::{Bounds, Controller, Decision, Limits, Reading};
use shares::Shares;
use system::{Inputs, SystemReading};

/// The engine's Resource governor: a handle any task may clone.
#[derive(Clone, Debug)]
pub struct Governor {
    shares: Shares,
    info: watch::Receiver<Option<SystemInfo>>,
    /// Where it reads filesystems; none when it never samples.
    disks: Option<Disks>,
    folders: Folders,
    storage: watch::Receiver<Arc<StorageSample>>,
    settled: watch::Receiver<Settled>,
    last_resort: watch::Receiver<LastResort>,
}

/// Whether a sample found the governor at its last resort (ADR 0040).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LastResort {
    /// The sample's number, from 1 for the first.
    pub sample: u64,
    /// Whether memory stayed critical at the floor past
    /// `governor.last_resort_after`.
    pub reached: bool,
}

impl Governor {
    /// A governor that samples `system` every `governor.sample_interval`
    /// on `clock`, reading its settings from the machine's layers of
    /// `settings` at each sample, as a task of `owner`, until `owner`
    /// stops.
    pub fn start(system: System, settings: Settings, clock: Clock, owner: &TaskOwner) -> Self {
        let defaults = Limits::of(&defaults(&settings));
        let shares = Shares::new(defaults.bounds.map(|bounds| bounds.ceiling));
        let (sender, info) = watch::channel(None);
        let (stored, storage) = watch::channel(Arc::new(StorageSample::default()));
        let (settling, settled) = watch::channel(Settled::built_in());
        let (resorting, last_resort) = watch::channel(LastResort::default());
        let folders = Folders::default();
        let disks = system.disks.clone();
        let sampler = Sampler {
            system,
            settings,
            clock,
            shares: shares.clone(),
            sender,
            blocking: owner.child(),
            folders: folders.clone(),
            stored,
            settling,
            last_resort: resorting,
        };
        let token = owner.token().clone();
        owner.spawn(async move {
            tokio::select! {
                () = token.cancelled() => {}
                () = sampler.run() => {}
            }
        });
        Self {
            shares,
            info,
            disks: Some(disks),
            folders,
            storage,
            settled,
            last_resort,
        }
    }

    /// A governor that never samples: each kind stays at its default
    /// ceiling.
    pub fn unsampled() -> Self {
        let ceilings = WorkKind::ALL.map(|kind| {
            u32::try_from((kind.ceiling().default)().as_u64().unwrap_or(1)).unwrap_or(u32::MAX)
        });
        Self {
            shares: Shares::new(ceilings),
            info: watch::channel(None).1,
            disks: None,
            folders: Folders::default(),
            storage: watch::channel(Arc::new(StorageSample::default())).1,
            settled: watch::channel(Settled::built_in()).1,
            last_resort: watch::channel(LastResort::default()).1,
        }
    }

    /// Samples the filesystem holding `folder` from the next sample on,
    /// until the answer drops (ADR 0122).
    pub fn sample(&self, folder: PathBuf) -> Held {
        self.folders.hold(folder)
    }

    /// Samples the filesystem holding each of `folders`, all from the same
    /// sample on, so that no sample finds some of them without the others,
    /// until each answer drops (ADR 0122).
    pub fn sample_together(&self, folders: impl IntoIterator<Item = PathBuf>) -> Vec<Held> {
        self.folders.hold_together(folders)
    }

    /// What it found of the filesystems at its last storage sample, which
    /// changes at each sample; empty before the first, or ever when it
    /// never samples.
    pub fn storage(&self) -> watch::Receiver<Arc<StorageSample>> {
        self.storage.clone()
    }

    /// The level now of the filesystem holding `path`, a folder or a file
    /// to write, under the settings of its last sample; none when it never
    /// samples or the filesystem cannot be read. Blocking: run it under a
    /// blocking policy.
    pub fn storage_at(&self, path: &Path) -> Option<Filesystem> {
        let settled = *self.settled.borrow();
        self.disks.as_ref()?.at(path, &settled)
    }

    /// A Run joins with `share`, its `governor.run_share` in percent of
    /// each kind's target.
    pub fn join(&self, share: u64) -> RunShare {
        self.shares.join(share)
    }

    /// What it read and decided at its last sample, which changes at each
    /// sample; none before the first, or ever when it never samples.
    pub fn system_info(&self) -> watch::Receiver<Option<SystemInfo>> {
        self.info.clone()
    }

    /// Whether each sample found it at its last resort, which changes at
    /// each sample; the Runs' registry stops one Run there, one at a time
    /// (ADR 0040).
    pub fn last_resort(&self) -> watch::Receiver<LastResort> {
        self.last_resort.clone()
    }

    /// Whether it is at its last resort: memory stayed critical at the
    /// floor past `governor.last_resort_after`.
    pub fn at_last_resort(&self) -> bool {
        self.info
            .borrow()
            .as_ref()
            .is_some_and(|info| info.last_resort)
    }
}

/// A Browser host's governor, as the engine sees it (ADRs 0022 and 0040):
/// the host runs a governor of its own where its browser runs and reports
/// what it decided, and the engine admits that Session's work by the
/// targets of its last report, not by its own. Until its first report,
/// each kind stays at one.
#[derive(Clone, Debug)]
pub struct HostGovernor {
    shares: Shares,
}

impl HostGovernor {
    /// A Browser host that has reported nothing yet.
    pub fn new() -> Self {
        Self {
            shares: Shares::new([1; 3]),
        }
    }

    /// The host reported `info`, what its governor read and decided.
    pub fn report(&self, info: &SystemInfo) {
        self.shares
            .set_targets(WorkKind::ALL.map(|kind| info.concurrency.of(kind).target));
    }

    /// A Session on the host joins with `share`, its Run's
    /// `governor.run_share`.
    pub fn join(&self, share: u64) -> RunShare {
        self.shares.join(share)
    }
}

impl Default for HostGovernor {
    fn default() -> Self {
        Self::new()
    }
}

impl Limits {
    fn of(settings: &Resolved) -> Self {
        let count = |key: &str, default: u64| settings.count(key).unwrap_or(default);
        let bounds = WorkKind::ALL.map(|kind| Bounds {
            floor: u32::try_from(count(kind.floor().name, 1)).unwrap_or(u32::MAX),
            ceiling: u32::try_from(count(kind.ceiling().name, 1)).unwrap_or(u32::MAX),
        });
        Self {
            bounds,
            pressure: count(PRESSURE.name, 85),
            critical: count(CRITICAL.name, 95),
            last_resort_after: settings
                .duration(LAST_RESORT_AFTER.name)
                .unwrap_or(Duration::from_secs(60)),
        }
    }
}

/// The built-in values of `settings`' Product, without any layer.
fn defaults(settings: &Settings) -> Resolved {
    resolve(&Layers::default(), settings.product(), settings.product())
        .expect("invariant: the built-in defaults resolve")
}

/// The sampling loop's state.
struct Sampler {
    system: System,
    settings: Settings,
    clock: Clock,
    shares: Shares,
    sender: watch::Sender<Option<SystemInfo>>,
    /// Where the readings, which block, run.
    blocking: TaskOwner,
    /// The folders whose filesystems it samples.
    folders: Folders,
    /// Where each storage sample goes.
    stored: watch::Sender<Arc<StorageSample>>,
    /// Where the storage settings of each sample go.
    settling: watch::Sender<Settled>,
    /// Where each sample says whether it is at its last resort.
    last_resort: watch::Sender<LastResort>,
}

/// What the previous sample read, which the next one's CPU and Tokio
/// shares are measured from.
struct Previous {
    at: Instant,
    cpu_time: Duration,
    tokio_busy: Duration,
}

impl Sampler {
    async fn run(self) {
        let started = self.clock.now();
        let mut controller = Controller::new();
        let mut previous: Option<Previous> = None;
        let mut levels: Vec<(Vec<PathBuf>, StorageLevel)> = Vec::new();
        loop {
            let settings = self.machine().await;
            levels = self.sample_storage(&settings, &levels).await;
            let limits = Limits::of(&settings);
            let inputs = Inputs {
                memory_allowance: settings.count(MEMORY_ALLOWANCE.name),
                pressure: limits.pressure,
                critical: limits.critical,
            };
            let system = self.system.clone();
            let processes = self.shares.browsers().processes();
            let (read, footprints) = self
                .blocking
                .blocking(move || {
                    let footprints: Vec<(u64, u64)> = processes
                        .into_iter()
                        .map(|(id, process)| (id, system.footprint(process)))
                        .collect();
                    (system.read(&inputs), footprints)
                })
                .await;
            self.shares.browsers().measured(&footprints);
            let browsers = footprints.iter().map(|(_, footprint)| footprint).sum();
            let now = self.clock.now();
            let tokio_busy = tokio_busy();
            let parallelism = std::thread::available_parallelism().map_or(1, usize::from);
            let workers = tokio::runtime::Handle::current()
                .metrics()
                .num_workers()
                .max(1);
            let (cpu_busy, tokio_share) = previous.as_ref().map_or((0, 0), |previous| {
                let elapsed = now.duration_since(previous.at);
                (
                    percent(
                        read.cpu_time.saturating_sub(previous.cpu_time),
                        elapsed,
                        parallelism,
                    ),
                    percent(
                        tokio_busy.saturating_sub(previous.tokio_busy),
                        elapsed,
                        workers,
                    ),
                )
            });
            previous = Some(Previous {
                at: now,
                cpu_time: read.cpu_time,
                tokio_busy,
            });
            let decision = controller.step(
                &limits,
                &Reading {
                    at: now.duration_since(started),
                    allowance: read.allowance,
                    used: read.used,
                    host: read.host,
                    cpu_busy,
                    tokio_busy: tokio_share,
                    in_use: self.shares.in_use(),
                },
            );
            self.shares.set_targets(decision.admits);
            self.shares.sampled(decision.level);
            if decision.level == GovernorLevel::Critical {
                for process in self.shares.browsers().shed_at_critical() {
                    tracing::info!(process, "memory is critical: the governor sheds a browser");
                }
            }
            let cpu = CpuReading {
                parallelism: u32::try_from(parallelism).unwrap_or(u32::MAX),
                busy: u32::try_from(cpu_busy).unwrap_or(u32::MAX),
                tokio_busy: u32::try_from(tokio_share).unwrap_or(u32::MAX),
            };
            self.last_resort.send_modify(|last_resort| {
                last_resort.sample += 1;
                last_resort.reached = decision.last_resort;
            });
            self.sender.send_replace(Some(info(
                &limits,
                (&read, browsers),
                &decision,
                cpu,
                self.shares.in_use(),
            )));
            let interval = settings
                .duration(SAMPLE_INTERVAL.name)
                .unwrap_or(Duration::from_secs(1))
                .max(Duration::from_millis(1));
            self.clock.sleep(interval).await;
        }
    }

    /// Samples the filesystems of the folders held now under `settings`,
    /// sends what it found, and logs each change of level from `before`,
    /// the levels of the previous sample's filesystems by their folders;
    /// answers this sample's.
    async fn sample_storage(
        &self,
        settings: &Resolved,
        before: &[(Vec<PathBuf>, StorageLevel)],
    ) -> Vec<(Vec<PathBuf>, StorageLevel)> {
        let settled = Settled::of(settings);
        let folders = self.folders.now();
        let disks = self.system.disks.clone();
        let sample = self
            .blocking
            .blocking(move || disks.sample(&folders, &settled))
            .await;
        for filesystem in &sample.filesystems {
            let was = before
                .iter()
                .find(|(folders, _)| {
                    folders
                        .iter()
                        .any(|folder| filesystem.folders.contains(folder))
                })
                .map_or(StorageLevel::Normal, |(_, level)| *level);
            log_change(filesystem, was);
        }
        let levels = sample
            .filesystems
            .iter()
            .map(|filesystem| (filesystem.folders.clone(), filesystem.level))
            .collect();
        self.settling.send_replace(settled);
        self.stored.send_replace(Arc::new(sample));
        levels
    }

    /// The governor's settings from the machine's layers now; the built-in
    /// values when they cannot be read, as the warning logged says.
    async fn machine(&self) -> Resolved {
        let product = self.settings.product();
        let read = self.settings.machine().await.and_then(|machine| {
            resolve(
                &Layers {
                    operator_policy: machine.operator_policy,
                    user_configuration: machine.user_configuration,
                    ..Layers::default()
                },
                product,
                product,
            )
        });
        match read {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(%error, "the governor's settings cannot be read: it samples on their built-in values");
                defaults(&self.settings)
            }
        }
    }
}

/// Logs `filesystem`'s change of level from `was`: at warning when it got
/// worse, at info on the way back.
fn log_change(filesystem: &Filesystem, was: StorageLevel) {
    let folder = filesystem
        .folders
        .first()
        .map(|folder| folder.display().to_string())
        .unwrap_or_default();
    let (level, free) = (filesystem.level.as_str(), filesystem.free);
    if filesystem.level > was {
        tracing::warn!(
            folder,
            level,
            free_bytes = free,
            was = was.as_str(),
            "a filesystem the Runs write to changed storage level"
        );
    } else if filesystem.level < was {
        tracing::info!(
            folder,
            level,
            free_bytes = free,
            was = was.as_str(),
            "a filesystem the Runs write to changed storage level"
        );
    }
}

/// `busy` in percent of `elapsed` on each of `threads`.
fn percent(busy: Duration, elapsed: Duration, threads: usize) -> u64 {
    let available = elapsed.as_nanos() * threads as u128;
    if available == 0 {
        return 0;
    }
    u64::try_from(busy.as_nanos() * 100 / available).unwrap_or(u64::MAX)
}

/// The Tokio workers' busy time since the runtime started, all added up.
fn tokio_busy() -> Duration {
    let metrics = tokio::runtime::Handle::current().metrics();
    (0..metrics.num_workers())
        .map(|worker| metrics.worker_total_busy_duration(worker))
        .sum()
}

/// The sample of `read`, with `browsers`, what the browsers counted used.
fn info(
    limits: &Limits,
    (read, browsers): (&SystemReading, u64),
    decision: &Decision,
    cpu: CpuReading,
    in_use: [u32; 3],
) -> SystemInfo {
    let kind = |index: usize| KindConcurrency {
        floor: limits.bounds[index].floor,
        ceiling: limits.bounds[index].ceiling.max(limits.bounds[index].floor),
        target: decision.targets[index],
        in_use: in_use[index],
    };
    SystemInfo {
        level: decision.level,
        pressure_source: decision.source,
        last_resort: decision.last_resort,
        memory: MemoryReading {
            allowance: read.allowance,
            allowance_source: read.allowance_source,
            used: read.used,
            browsers,
            host: read.host,
        },
        cpu,
        concurrency: Concurrency {
            http: kind(0),
            browser_page: kind(1),
            model_call: kind(2),
        },
    }
}
