//! B3's governor scenario (ADR 0140's E4), shared by its counts cell
//! (`examples/counts.rs`) and its test: the engine's Resource governor taking
//! [`SAMPLES`] samples on the paused `Clock`, reading a folder laid out as
//! Linux's `/proc`, as the governor's own tests write one, and the user
//! configuration of a home of its own. The engine runs a browser, a child of
//! its process, which a Run's pool joined to the governor: so each sample
//! reads the engine's memory and the browser's, as a crawl's sample does.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::governor::{Governor, System};
use goodfetch_engine::home::Home;
use goodfetch_engine::settings::{Settings, SettingsFiles};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;

/// The samples the scenario takes.
pub(crate) const SAMPLES: u64 = 4;

/// The processes each sample reads the memory of: the engine and its
/// browser.
pub(crate) const PROCESSES: u64 = 2;

/// The browser's process ID in the stand-in `/proc`.
const BROWSER: u32 = 4242;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// One extra unit of an E4 row's work, done once per unit of its row through
/// the primitive that counts it.
pub(crate) type Plant = Row;

/// What one run of the scenario counted.
pub(crate) struct Sampled {
    /// The samples the governor sent.
    pub(crate) samples: u64,
    /// The processes whose memory those samples read, each once a sample.
    pub(crate) process_samples: u64,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the scenario writes the stand-in `/proc` the governor reads, as the governor's tests do"
)]
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("invariant: a file in a folder"))
        .expect("invariant: the temporary folder is writable");
    std::fs::write(path, text).expect("invariant: the temporary folder is writable");
}

/// A `/proc` under `root` of an 8 GiB machine with 6 GiB available and no
/// memory stalls, whose engine uses 100 MiB and has the browser [`BROWSER`]
/// of 200 MiB for its one child, in the cgroup `/pod.scope` with a 4 GiB
/// limit: the governor's own tests' folder, with the browser.
fn stand_in_proc(root: &Path) -> PathBuf {
    let proc = root.join("proc");
    let cgroup = root.join("cgroup");
    let file = |relative: &str, text: &str| write(&proc.join(relative), text);
    file(
        "meminfo",
        &format!(
            "MemTotal: {} kB\nMemAvailable: {} kB\n",
            8 * GIB / 1024,
            6 * GIB / 1024
        ),
    );
    file(
        "pressure/memory",
        "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
    );
    file("self/smaps_rollup", &rollup(100 * MIB));
    file("self/stat", "1 (engine) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0\n");
    file("self/task/1/children", &format!("{BROWSER}\n"));
    file(&format!("{BROWSER}/smaps_rollup"), &rollup(200 * MIB));
    file("self/cgroup", "0::/pod.scope\n");
    file(
        "self/mountinfo",
        &format!(
            "48 22 0:30 / {} rw,relatime - cgroup2 cgroup2 rw\n",
            cgroup.display()
        ),
    );
    write(&cgroup.join("memory.max"), "max\n");
    write(
        &cgroup.join("pod.scope/memory.max"),
        &format!("{}\n", 4 * GIB),
    );
    proc
}

fn rollup(pss: u64) -> String {
    format!("Rss: {} kB\nPss: {} kB\n", pss / 1024 * 2, pss / 1024)
}

/// One extra unit of `row`'s work in `proc`, through its counted function:
/// a settings file's read has none, so it is counted where the governor
/// counts it.
fn planted(row: Row, proc: &Path, work: &CountedWork) {
    let open = |relative: String| {
        drop(work.open_file(row, OpenOptions::new().read(true), &proc.join(relative)));
    };
    match row {
        Row::GovernorConfigurationReads => work.count(row),
        Row::GovernorMeminfoOpens => open("meminfo".to_owned()),
        Row::GovernorStaticOpens => open("self/cgroup".to_owned()),
        Row::GovernorProcessReads => {
            open("self/smaps_rollup".to_owned());
            open(format!("{BROWSER}/smaps_rollup"));
        }
        other => panic!("no governor plant for the row {}", other.name()),
    }
}

/// Runs the scenario in `home`, its work counted on `work`, doing `plant`
/// once per unit of its row, and answers what it sampled.
#[expect(
    clippy::disallowed_methods,
    reason = "the scenario samples on the paused Clock, the Clock's test version (ADR 0059)"
)]
pub(crate) fn run(home: &Path, work: &CountedWork, plant: Option<Plant>) -> Sampled {
    let proc = stand_in_proc(home);
    let version = ProductVersion::parse("0.0.0").expect("invariant: a literal version");
    let files = SettingsFiles::of(&Home::new(home.join("home")), &version);
    write(
        files.user_configuration(),
        "version = \"0.1\"\n\n[settings]\ngovernor.sample_interval = 250\n",
    );
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let settings = Settings::new(files, Product::Collie, owner.child());
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let governor = Governor::start(
            System::linux_files(proc.clone()),
            settings,
            clock,
            &owner,
            work.clone(),
        );
        let _browser = governor.join(100).browser(BROWSER);
        let mut info = governor.system_info();
        let mut samples = 0;
        while samples < SAMPLES {
            info.changed()
                .await
                .expect("invariant: the governor samples until its owner stops");
            if info.borrow_and_update().is_some() {
                samples += 1;
                if let Some(row) = plant {
                    planted(row, &proc, work);
                }
            }
        }
        // Stopped before its next sample: the governor sleeps on the paused
        // Clock until this task waits on nothing else.
        owner.stop().await;
        Sampled {
            samples,
            process_samples: samples * PROCESSES,
        }
    })
}
