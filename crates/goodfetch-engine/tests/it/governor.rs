//! The Resource governor through `goodfetch-engine`'s public interface
//! (ADR 0040; spec #213's Seams 3 and 4): its sampling loop on the paused
//! `Clock`, reading a folder laid out as Linux's `/proc` that the test
//! writes, and the user configuration of the test's Goodfetch home, each
//! read again at every sample; and Runs sharing its admissions.
#![expect(
    clippy::disallowed_methods,
    reason = "the test writes the files the governor reads, and runs it on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::CountedWork;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::governor::{
    BrowserUse, GovernedBrowser, Governor, HostGovernor, PageHold, RunShare, System,
    last_resort_run,
};
use goodfetch_engine::home::Home;
use goodfetch_engine::settings::{Settings, SettingsFiles};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::governor::{
    AllowanceSource, GovernorLevel, PressureSource, SystemInfo, WorkKind,
};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use tempfile::TempDir;
use tokio::sync::watch;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// The settings every test starts from: a sample every 250 ms.
const SAMPLED: &str = "governor.sample_interval = 250\n";

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A `/proc` of an 8 GiB machine with 6 GiB available and no memory
/// stalls, whose engine uses 100 MiB, in the cgroup `/pod.scope` with a
/// 256 MiB limit and its parents' none.
struct Proc {
    folder: TempDir,
}

impl Proc {
    fn new() -> Self {
        let proc = Self {
            folder: TempDir::new().unwrap(),
        };
        proc.meminfo(6 * GIB);
        proc.file("pressure/memory", &psi(0.0));
        proc.file("self/smaps_rollup", &rollup(100 * MIB));
        proc.file("self/stat", "1 (engine) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0\n");
        proc.file("self/cgroup", "0::/pod.scope\n");
        proc.file(
            "self/mountinfo",
            &format!(
                "48 22 0:30 / {} rw,relatime - cgroup2 cgroup2 rw\n",
                proc.cgroup().display()
            ),
        );
        proc.limit("max", 256 * MIB);
        proc
    }

    fn cgroup(&self) -> PathBuf {
        self.folder.path().join("cgroup")
    }

    fn proc(&self) -> PathBuf {
        self.folder.path().join("proc")
    }

    fn file(&self, relative: &str, text: &str) {
        write(&self.proc().join(relative), text);
    }

    fn limit(&self, parent: &str, limit: u64) {
        write(&self.cgroup().join("memory.max"), &format!("{parent}\n"));
        write(
            &self.cgroup().join("pod.scope/memory.max"),
            &format!("{limit}\n"),
        );
    }

    fn meminfo(&self, available: u64) {
        self.file(
            "meminfo",
            &format!(
                "MemTotal: {} kB\nMemAvailable: {} kB\n",
                8 * GIB / 1024,
                available / 1024
            ),
        );
    }
}

fn psi(some: f64) -> String {
    format!(
        "some avg10={some:.2} avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n"
    )
}

fn rollup(pss: u64) -> String {
    format!("Rss: {} kB\nPss: {} kB\n", pss / 1024 * 2, pss / 1024)
}

/// A governor sampling `proc` under the user configuration of `env`, with
/// `run` given it, on the paused Clock.
fn governed<T>(env: &TestEnv, proc: &Proc, run: impl AsyncFnOnce(Governor) -> T) -> T {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let version = ProductVersion::parse("0.0.0").unwrap();
        let settings = Settings::new(
            SettingsFiles::of(&Home::new(env.home().to_owned()), &version),
            Product::Collie,
            owner.child(),
        );
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let governor = Governor::start(
            System::linux_files(proc.proc()),
            settings,
            clock,
            &owner,
            CountedWork::default(),
        );
        let answer = run(governor).await;
        owner.stop().await;
        answer
    })
}

/// The governor's samples, in turn, from its first.
struct Samples(watch::Receiver<Option<SystemInfo>>);

impl Samples {
    fn of(governor: &Governor) -> Self {
        Self(governor.system_info())
    }

    /// The next sample after the last one this gave.
    async fn next(&mut self) -> SystemInfo {
        self.0.changed().await.unwrap();
        self.0
            .borrow_and_update()
            .clone()
            .expect("invariant: the governor sends only samples")
    }
}

/// The process's own cgroup limit is the Memory allowance, read again at
/// every sample, so a limit changed in place, as an in-place Pod resize
/// does, is seen at the next one; and the next sample comes only once
/// `governor.sample_interval` passed on the Clock.
///
/// Covers: story:3-56, setting:governor.sample_interval
#[test]
fn the_allowance_is_the_own_cgroups_limit_read_again_at_every_sample() {
    let env = TestEnv::new();
    env.user_configuration(SAMPLED);
    let proc = Proc::new();

    let (first, early, second) = governed(&env, &proc, async |governor| {
        let mut samples = Samples::of(&governor);
        let first = samples.next().await;
        proc.limit("max", 512 * MIB);
        let early = tokio::time::timeout(Duration::from_millis(249), samples.next())
            .await
            .is_ok();
        let second = samples.next().await;
        (first, early, second)
    });

    assert_eq!(
        (
            (first.memory.allowance, first.memory.allowance_source),
            early,
            (second.memory.allowance, second.memory.allowance_source),
        ),
        (
            (256 * MIB, AllowanceSource::Cgroup),
            false,
            (512 * MIB, AllowanceSource::Cgroup),
        )
    );
}

/// `governor.memory_allowance` tighter than the cgroup's limit is the
/// allowance, read again at every sample from the user configuration.
///
/// Covers: story:3-56, setting:governor.memory_allowance
#[test]
fn a_tighter_memory_allowance_setting_takes_over_at_the_next_sample() {
    let env = TestEnv::new();
    env.user_configuration(SAMPLED);
    let proc = Proc::new();

    let (before, after) = governed(&env, &proc, async |governor| {
        let mut samples = Samples::of(&governor);
        let before = samples.next().await;
        env.user_configuration(&format!(
            "{SAMPLED}governor.memory_allowance = {}\n",
            128 * MIB
        ));
        let after = samples.next().await;
        (before, after)
    });

    assert_eq!(
        (
            (before.memory.allowance, before.memory.allowance_source),
            (after.memory.allowance, after.memory.allowance_source),
        ),
        (
            (256 * MIB, AllowanceSource::Cgroup),
            (128 * MIB, AllowanceSource::Setting),
        )
    );
}

/// The engine's memory is that of the engine and every descendant, PSS on
/// Linux, so a browser's shared pages count once; past `governor.pressure`
/// of the allowance, every kind's target halves at each sample.
///
/// Covers: story:3-53, story:3-58, setting:governor.pressure
#[test]
fn the_engine_and_its_children_past_the_pressure_share_halve_every_target() {
    let env = TestEnv::new();
    env.user_configuration(SAMPLED);
    let proc = Proc::new();
    // A browser and its renderer: 100 + 80 + 40 MiB is 85.9 % of 256 MiB.
    proc.file("self/task/1/children", "70");
    proc.file("70/smaps_rollup", &rollup(80 * MIB));
    proc.file("70/task/70/children", "71");
    proc.file("71/smaps_rollup", &rollup(40 * MIB));

    let (first, second) = governed(&env, &proc, async |governor| {
        let mut samples = Samples::of(&governor);
        (samples.next().await, samples.next().await)
    });

    assert_eq!(
        (
            first.memory.used,
            first.level,
            first.pressure_source,
            [
                first.concurrency.http.target,
                first.concurrency.browser_page.target,
                first.concurrency.model_call.target,
            ],
            [
                second.concurrency.http.target,
                second.concurrency.browser_page.target,
                second.concurrency.model_call.target,
            ],
        ),
        (
            220 * MIB,
            GovernorLevel::Pressure,
            Some(PressureSource::Memory),
            [100, 16, 8],
            [50, 8, 4],
        )
    );
}

/// At pressure nothing new is admitted (ADR 0040's shedding ladder): from
/// the first sample under pressure, a Run takes no admission of a kind
/// past its floor, though each target only halved.
///
/// Covers: story:3-59, setting:governor.pressure
#[test]
fn at_pressure_nothing_new_is_admitted_past_each_floor() {
    let env = TestEnv::new();
    env.user_configuration(SAMPLED);
    let proc = Proc::new();
    // 220 MiB of a 256 MiB allowance is 85.9 %, past `governor.pressure`.
    proc.file("self/smaps_rollup", &rollup(220 * MIB));

    let (target, admitted) = governed(&env, &proc, async |governor| {
        let info = Samples::of(&governor).next().await;
        let run = governor.join(100);
        let admitted = (0..8)
            .filter_map(|_| run.try_admit(WorkKind::BrowserPage).map(std::mem::forget))
            .count();
        (info.concurrency.browser_page.target, admitted)
    });

    assert_eq!((target, admitted), (16, 1));
}

/// At critical the governor sheds browsers (ADR 0040's shedding ladder):
/// at each sample it closes every idle browser, then recycles the largest
/// that has open pages, one at a time, measured again in between, never a
/// held one; a browser it shed is not shed again.
///
/// Covers: story:3-59, story:3-60, setting:governor.critical
#[test]
fn at_critical_idle_browsers_close_then_the_largest_is_recycled_skipping_held_ones() {
    let env = TestEnv::new();
    env.user_configuration(SAMPLED);
    let proc = Proc::new();
    proc.file("self/smaps_rollup", &rollup(250 * MIB));
    for (process, footprint) in [(70, 10), (71, 50), (72, 30), (73, 80)] {
        proc.file(&format!("{process}/smaps_rollup"), &rollup(footprint * MIB));
    }

    let shed = governed(&env, &proc, async |governor| {
        let run = governor.join(100);
        let browsers: Vec<(u32, GovernedBrowser)> = [
            (70, 0, PageHold::None),
            (71, 1, PageHold::None),
            (72, 2, PageHold::None),
            (73, 1, PageHold::Held),
        ]
        .into_iter()
        .map(|(process, open_pages, held)| {
            let browser = run.browser(process);
            browser.report(BrowserUse { open_pages, held });
            (process, browser)
        })
        .collect();
        let mut samples = Samples::of(&governor);
        let mut shed = Vec::new();
        for _ in 0..3 {
            let _ = samples.next().await;
            let mut now = Vec::new();
            for (process, browser) in &browsers {
                if tokio::time::timeout(Duration::ZERO, browser.shed())
                    .await
                    .is_ok()
                {
                    now.push(*process);
                }
            }
            shed.push(now);
        }
        shed
    });

    assert_eq!(shed, [vec![70, 71], vec![70, 71, 72], vec![70, 71, 72]]);
}

/// Each sample says what the browsers it counts use, each measured with
/// its descendants and summed across Runs, which
/// `goodfetch.browser.memory` records; a browser that drops is no longer
/// counted at the next sample.
///
/// Covers: story:3-103
#[test]
fn each_sample_sums_what_the_browsers_it_counts_use() {
    let env = TestEnv::new();
    env.user_configuration(SAMPLED);
    let proc = Proc::new();
    proc.file("self/smaps_rollup", &rollup(100 * MIB));
    for (process, footprint) in [(70, 10), (71, 30)] {
        proc.file(&format!("{process}/smaps_rollup"), &rollup(footprint * MIB));
    }

    let used = governed(&env, &proc, async |governor| {
        let (first, second) = (governor.join(100), governor.join(100));
        let kept = first.browser(70);
        let dropped = second.browser(71);
        let mut samples = Samples::of(&governor);
        let both = samples.next().await.memory.browsers;
        drop(dropped);
        let one = samples.next().await.memory.browsers;
        drop(kept);
        (both, one)
    });

    assert_eq!(used, (40 * MIB, 10 * MIB));
}

/// The Run, of `runs` by their names, the last resort stops.
fn pick(runs: Vec<(&'static str, &RunShare)>) -> Option<&'static str> {
    last_resort_run(runs.into_iter().map(|(name, run)| (name, run.weight())))
}

/// The last resort stops the Run with the largest footprint, its browsers'
/// memory as the governor last measured it; a Run whose page is under a
/// person's control in the Live view is stopped last, and only once that
/// person hands back.
///
/// Covers: story:3-59, story:3-60
#[test]
fn the_last_resort_picks_the_largest_run_and_a_persons_run_only_once_handed_back() {
    let env = TestEnv::new();
    env.user_configuration(SAMPLED);
    let proc = Proc::new();
    for (process, footprint) in [(70, 10), (71, 50), (72, 300)] {
        proc.file(&format!("{process}/smaps_rollup"), &rollup(footprint * MIB));
    }

    let picks = governed(&env, &proc, async |governor| {
        let (small, large, persons) = (governor.join(100), governor.join(100), governor.join(100));
        let _small = small.browser(70);
        let _large = large.browser(71);
        let held = persons.browser(72);
        held.report(BrowserUse {
            open_pages: 1,
            held: PageHold::Person,
        });
        let _ = Samples::of(&governor).next().await;
        let first = pick(vec![
            ("small", &small),
            ("large", &large),
            ("persons", &persons),
        ]);
        let alone = pick(vec![("persons", &persons)]);
        held.report(BrowserUse {
            open_pages: 1,
            held: PageHold::None,
        });
        let handed_back = pick(vec![("small", &small), ("persons", &persons)]);
        (
            (first, alone, handed_back),
            [
                small.weight().footprint,
                large.weight().footprint,
                persons.weight().footprint,
            ],
        )
    });

    assert_eq!(
        picks,
        (
            (Some("large"), None, Some("persons")),
            [10 * MIB, 50 * MIB, 300 * MIB]
        )
    );
}

/// The host's own memory pressure counts as the engine's, though the
/// engine's own memory has headroom: three Attached engines never each
/// claim half the machine. `governor.host_memory` is on by default; the
/// tests' configuration turns it off, so this one turns it on again.
///
/// Covers: story:3-57, setting:governor.host_memory
#[test]
fn the_hosts_own_memory_pressure_counts_as_the_engines() {
    let env = TestEnv::new();
    env.user_configuration(&format!("{SAMPLED}governor.host_memory = true\n"));
    let proc = Proc::new();
    proc.meminfo(GIB / 4);

    let info = governed(&env, &proc, async |governor| {
        Samples::of(&governor).next().await
    });

    assert_eq!(
        (
            info.memory.host,
            info.level,
            info.pressure_source,
            info.concurrency.browser_page.target,
        ),
        (
            GovernorLevel::Critical,
            GovernorLevel::Critical,
            Some(PressureSource::HostMemory),
            16,
        )
    );
}

/// With `governor.host_memory` off, the host's critical pressure, by
/// `MemAvailable` or by PSI, holds the engine at no level, its own memory
/// having headroom.
///
/// Covers: story:3-57, setting:governor.host_memory
#[rstest]
#[case::available_under_the_critical_share(GIB / 4, psi(0.0))]
#[case::every_task_stalled_on_memory(6 * GIB, "some avg10=50.00 avg60=0.00 avg300=0.00 total=0\nfull avg10=50.00 avg60=0.00 avg300=0.00 total=0\n".to_owned())]
fn with_the_host_memory_off_the_hosts_pressure_is_not_the_engines(
    #[case] available: u64,
    #[case] pressure: String,
) {
    let env = TestEnv::new();
    env.user_configuration(&format!("{SAMPLED}governor.host_memory = false\n"));
    let proc = Proc::new();
    proc.meminfo(available);
    proc.file("pressure/memory", &pressure);

    let info = governed(&env, &proc, async |governor| {
        Samples::of(&governor).next().await
    });

    assert_eq!(
        (info.memory.host, info.level, info.pressure_source),
        (GovernorLevel::Normal, GovernorLevel::Normal, None)
    );
}

/// Memory critical with every kind at its floor past
/// `governor.last_resort_after` is the governor's last resort; a floor
/// equal to the ceiling fixes each kind's concurrency there.
///
/// Covers: story:3-53, setting:governor.last_resort_after, setting:governor.critical, setting:governor.http.floor, setting:governor.http.ceiling, setting:governor.browser_page.floor, setting:governor.browser_page.ceiling, setting:governor.model_call.floor, setting:governor.model_call.ceiling
#[test]
fn memory_critical_at_a_fixed_floor_reaches_the_last_resort_after_its_time() {
    let env = TestEnv::new();
    env.user_configuration(&format!(
        "{SAMPLED}governor.last_resort_after = 1000\n\
         governor.http.floor = 4\ngovernor.http.ceiling = 4\n\
         governor.browser_page.floor = 2\ngovernor.browser_page.ceiling = 2\n\
         governor.model_call.floor = 1\ngovernor.model_call.ceiling = 1\n"
    ));
    let proc = Proc::new();
    proc.file("self/smaps_rollup", &rollup(250 * MIB));

    let samples = governed(&env, &proc, async |governor| {
        let mut samples = Samples::of(&governor);
        let mut seen = Vec::new();
        for _ in 0..6 {
            let info = samples.next().await;
            seen.push((
                info.level,
                info.concurrency.http.target,
                info.concurrency.browser_page.target,
                governor.at_last_resort(),
            ));
        }
        seen
    });

    // Critical from the first sample at 0 s; the fifth, at 1 s, has been
    // critical at the floor for `governor.last_resort_after`.
    let critical = |last_resort| (GovernorLevel::Critical, 4, 2, last_resort);
    assert_eq!(
        samples,
        [
            critical(false),
            critical(false),
            critical(false),
            critical(false),
            critical(true),
            critical(true),
        ]
    );
}

/// Each kind is admitted on its own up to its target, and while Runs wait,
/// what one gives back goes round-robin to the others, never back to the
/// first Run to grow; `governor.run_share` caps what one Run may hold.
///
/// Covers: story:3-54, story:3-55, setting:governor.run_share
#[test]
fn runs_share_each_kinds_admissions_in_turn_within_their_share() {
    let env = TestEnv::new();
    env.user_configuration(&format!(
        "{SAMPLED}governor.http.floor = 2\ngovernor.http.ceiling = 2\n\
         governor.browser_page.floor = 4\ngovernor.browser_page.ceiling = 4\n"
    ));
    let proc = Proc::new();

    let (pages, turns, capped) = governed(&env, &proc, async |governor| {
        let _ = Samples::of(&governor).next().await;
        let (first, second) = (governor.join(100), governor.join(100));
        // The first Run grows first and holds every HTTP admission; the
        // browser pages are another kind.
        let held = [
            first.try_admit(WorkKind::Http),
            first.try_admit(WorkKind::Http),
        ];
        let pages = (0..4)
            .filter_map(|_| {
                second
                    .try_admit(WorkKind::BrowserPage)
                    .map(std::mem::forget)
            })
            .count();
        let waiting = (
            second.try_admit(WorkKind::Http).is_none(),
            first.try_admit(WorkKind::Http).is_none(),
        );
        drop(held);
        let turns = (
            waiting,
            second.try_admit(WorkKind::Http).is_some(),
            first.try_admit(WorkKind::Http).is_some(),
        );
        let halved = governor.join(50);
        let capped = (0..10)
            .filter_map(|_| halved.try_admit(WorkKind::ModelCall).map(std::mem::forget))
            .count();
        (pages, turns, capped)
    });

    // Of `model_call`'s default ceiling of 16, a 50 % share holds 8.
    assert_eq!((pages, turns, capped), (4, ((true, true), true, true), 8));
}

/// A target starts at its ceiling as the machine's settings set it, not at
/// the built-in one (ADR 0040): before its first sample read them, the
/// governor admits nothing, and a Run that asked waits for its turn, which
/// the first sample gives it, up to the user configuration's ceiling. A
/// Run that started before the first sample once took three of a fixed
/// cap of two, and a crawl test waiting for that cap never saw it (#1792).
///
/// Covers: story:3-55, setting:governor.http.ceiling
#[test]
fn before_its_first_sample_the_governor_admits_nothing_past_the_configured_ceiling() {
    let env = TestEnv::new();
    env.user_configuration(&format!(
        "{SAMPLED}governor.http.floor = 2\ngovernor.http.ceiling = 2\n"
    ));
    let proc = Proc::new();

    let (before, after) = governed(&env, &proc, async |governor| {
        let run = governor.join(100);
        let before: Vec<_> = (0..3)
            .filter_map(|_| run.try_admit(WorkKind::Http))
            .collect();
        let turn = run.admit(WorkKind::Http).await;
        let after: Vec<_> = std::iter::once(turn)
            .chain((0..3).filter_map(|_| run.try_admit(WorkKind::Http)))
            .collect();
        (before.len(), after.len())
    });

    assert_eq!((before, after), (0, 2));
}

/// A Browser host's Session's pages are admitted by the targets the host
/// reports from its own governor, not by the engine's own: here the engine
/// is under pressure and the host has room, then the other way round.
///
/// Covers: story:3-65
#[test]
fn a_browser_hosts_pages_are_admitted_by_its_reported_pressure() {
    let env = TestEnv::new();
    env.user_configuration(SAMPLED);
    let proc = Proc::new();
    proc.file("self/smaps_rollup", &rollup(250 * MIB));

    let (own, hosted, after_report) = governed(&env, &proc, async |governor| {
        let mut samples = Samples::of(&governor);
        let mut last = None;
        for _ in 0..6 {
            last = Some(samples.next().await);
        }
        let own = governor.join(100);
        let own_pages = (0..8)
            .filter_map(|_| own.try_admit(WorkKind::BrowserPage).map(std::mem::forget))
            .count();
        let host = HostGovernor::new();
        let mut report = last.unwrap();
        report.level = GovernorLevel::Normal;
        report.concurrency.browser_page.target = 6;
        host.report(&report);
        let session = host.join(100);
        let hosted = (0..8)
            .filter_map(|_| {
                session
                    .try_admit(WorkKind::BrowserPage)
                    .map(std::mem::forget)
            })
            .count();
        report.concurrency.browser_page.target = 7;
        host.report(&report);
        let after_report = (0..8)
            .filter_map(|_| {
                session
                    .try_admit(WorkKind::BrowserPage)
                    .map(std::mem::forget)
            })
            .count();
        (own_pages, hosted, after_report)
    });

    assert_eq!((own, hosted, after_report), (1, 6, 1));
}
