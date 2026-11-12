//! The process tree's sampler: every [`INTERVAL_MILLISECONDS`], the resident
//! memory, private memory, proportional set size (PSS), CPU time, threads
//! and open files of the tool's process and every process under it, read
//! from `/proc` (Linux), so a browser's processes count with the tool's;
//! and, when asked, the followed process's executable's own file-backed
//! pages, as `xtask rss` reads them (#2114). Private memory is what each
//! process allocated, its resident pages less those backed by a file or
//! shared memory (`RssAnon`, spec #2165). Summed resident memory counts a
//! page two processes share twice; summed PSS counts it once, each process
//! taking its share. The OS's own accounting of the
//! waited-for children gives the totals; the samples give the peaks over
//! the tree, the time-weighted mean and the cores used over time. An OS
//! without `/proc` samples nothing.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: the sampler reads /proc with blocking calls on a thread of its own (STYLE.md)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "xtask is no engine crate loom models (ADR 0060): the sampler's thread reads its stop flag and the followed process from std's atomics"
)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::trials::Sampled;

pub(crate) const INTERVAL_MILLISECONDS: u64 = 50;
/// The samples a peak of cores spans: 250 ms, so the clock ticks CPU time
/// is counted in (10 ms) blur it by at most a few percent of a core.
pub(crate) const CORE_WINDOW_SAMPLES: usize = 5;

/// One sample of the tree.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Sample {
    /// Seconds from the trial's start.
    pub(crate) at: f64,
    pub(crate) resident_bytes: u64,
    /// The tree's private memory: each process's resident pages less its
    /// file-backed and shared ones (`RssAnon`).
    pub(crate) private_bytes: u64,
    /// The followed process's executable's own file-backed resident pages,
    /// when the sampler reads them and could.
    pub(crate) executable_bytes: Option<u64>,
    /// The tree's PSS: each resident page's bytes divided among the
    /// processes that map it, so the sum over the tree counts a shared page
    /// once. None when a live process of the tree kept its own from the
    /// sampler, so the sum would leave it out.
    pub(crate) pss_bytes: Option<u64>,
    /// CPU seconds of the tree's live processes so far.
    pub(crate) cpu_seconds: f64,
    pub(crate) threads: u64,
    pub(crate) open_files: u64,
}

/// A running sampler of the tree under the process [`Sampler::follow`]
/// names.
pub(crate) struct Sampler {
    root: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Vec<Sample>>>,
}

/// The OS's clock ticks per second and page size.
#[cfg(not(unix))]
fn system() -> (f64, u64) {
    (100.0, 4096)
}

/// The OS's clock ticks per second and page size.
#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "the clock's ticks and the page size are read through the OS's C interface: sysconf"
)]
fn system() -> (f64, u64) {
    // SAFETY: `sysconf` reads a constant of the OS.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    // SAFETY: as above.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    #[expect(
        clippy::cast_precision_loss,
        reason = "the clock's ticks per second, about 100"
    )]
    (
        if ticks > 0 { ticks as f64 } else { 100.0 },
        u64::try_from(page).unwrap_or(4096),
    )
}

/// The process `pid`'s children, from every thread's `children` file.
fn children(pid: u32) -> Vec<u32> {
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return Vec::new();
    };
    tasks
        .flatten()
        .filter_map(|task| std::fs::read_to_string(task.path().join("children")).ok())
        .flat_map(|text| {
            text.split_whitespace()
                .filter_map(|pid| pid.parse().ok())
                .collect::<Vec<u32>>()
        })
        .collect()
}

/// `pid` and every process under it.
pub(crate) fn tree(pid: u32) -> Vec<u32> {
    let mut tree = vec![pid];
    let mut next = 0;
    while next < tree.len() {
        let found = children(tree[next]);
        tree.extend(found);
        next += 1;
    }
    tree
}

/// What one sample reads of one process.
struct Process {
    resident_bytes: u64,
    /// `RssAnon`: resident less `statm`'s shared pages, its `RssFile` and
    /// `RssShmem`.
    private_bytes: u64,
    /// None when the OS refuses the sampler its memory map's totals.
    pss_bytes: Option<u64>,
    cpu_ticks: u64,
    threads: u64,
    open_files: u64,
}

/// The `Pss:` line of a `smaps_rollup` file, in bytes: the kernel writes it
/// in kibibytes.
fn pss_of(rollup: &str) -> Option<u64> {
    let kibibytes: u64 = rollup
        .lines()
        .find_map(|line| line.strip_prefix("Pss:"))?
        .trim()
        .strip_suffix("kB")?
        .trim()
        .parse()
        .ok()?;
    Some(kibibytes * 1024)
}

/// The process `pid`'s PSS, from the totals of its memory map
/// (`smaps_rollup`): none where the OS refuses them to the sampler, as for
/// a process that made itself undumpable, and zero for one that is exiting
/// and has no memory map left.
fn pss(pid: u32) -> Option<u64> {
    match std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
        Ok(rollup) => pss_of(&rollup),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => None,
        Err(_) => Some(0),
    }
}

/// One process's figures; none once it is gone, or exited and not yet
/// waited for (a zombie, which `/proc` lists with nothing resident).
fn process(pid: u32, page: u64) -> Option<Process> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the command's closing parenthesis, from the state (3).
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    if matches!(fields.first(), Some(&("Z" | "X"))) {
        return None;
    }
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let mut pages = statm.split_whitespace().skip(1);
    let resident: u64 = pages.next()?.parse().ok()?;
    let shared: u64 = pages.next()?.parse().ok()?;
    let ticks: u64 = fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?;
    let threads: u64 = fields.get(17)?.parse().ok()?;
    let open_files =
        std::fs::read_dir(format!("/proc/{pid}/fd")).map_or(0, |entries| entries.count() as u64);
    Some(Process {
        resident_bytes: resident * page,
        private_bytes: resident.saturating_sub(shared) * page,
        pss_bytes: pss(pid),
        cpu_ticks: ticks,
        threads,
        open_files,
    })
}

/// The process `pid`'s executable's own file-backed resident pages; none
/// once it is gone, or on an OS that does not give them.
#[cfg(target_os = "linux")]
fn executable_bytes(pid: u32) -> Option<u64> {
    let executable = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    crate::rss::executable::resident(pid, &executable)
}

/// The process `pid`'s executable's own file-backed resident pages; none
/// once it is gone, or on an OS that does not give them.
#[cfg(not(target_os = "linux"))]
fn executable_bytes(_pid: u32) -> Option<u64> {
    None
}

impl Sampler {
    /// Starts sampling, timed from `start`, with the followed process's
    /// executable's own pages when `executable_pages` asks for them; a
    /// sampler on an OS without `/proc` takes no sample.
    pub(crate) fn start(start: Instant, executable_pages: bool) -> Self {
        let root = Arc::new(AtomicU32::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = Path::new("/proc/self/stat").exists().then(|| {
            let (root, stop) = (Arc::clone(&root), Arc::clone(&stop));
            std::thread::spawn(move || {
                let (ticks_per_second, page) = system();
                let mut samples = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    let pid = root.load(Ordering::Relaxed);
                    if pid != 0 {
                        let mut sample = Sample {
                            at: start.elapsed().as_secs_f64(),
                            pss_bytes: Some(0),
                            ..Sample::default()
                        };
                        let mut ticks = 0;
                        let mut live = false;
                        for process in tree(pid).into_iter().filter_map(|pid| process(pid, page)) {
                            live = true;
                            sample.resident_bytes += process.resident_bytes;
                            sample.private_bytes += process.private_bytes;
                            sample.pss_bytes = sample
                                .pss_bytes
                                .zip(process.pss_bytes)
                                .map(|(tree, own)| tree + own);
                            ticks += process.cpu_ticks;
                            sample.threads += process.threads;
                            sample.open_files += process.open_files;
                        }
                        #[expect(
                            clippy::cast_precision_loss,
                            reason = "clock ticks stay far below 2^52"
                        )]
                        let seconds = ticks as f64 / ticks_per_second;
                        sample.cpu_seconds = seconds;
                        if executable_pages {
                            sample.executable_bytes = executable_bytes(pid);
                        }
                        // A tree with no live process, the tool exited or
                        // killed before its resume, has nothing resident to
                        // read: its sample would read as none.
                        if live {
                            samples.push(sample);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(INTERVAL_MILLISECONDS));
                }
                samples
            })
        });
        Self { root, stop, thread }
    }

    /// Samples the tree under `pid` from now on: the tool's process, or
    /// the one that resumes it.
    pub(crate) fn follow(&self, pid: u32) {
        self.root.store(pid, Ordering::Relaxed);
    }

    /// Stops sampling and returns the samples, none on an OS without
    /// `/proc`.
    pub(crate) fn finish(mut self) -> Option<Vec<Sample>> {
        self.stop.store(true, Ordering::Relaxed);
        let thread = self.thread.take()?;
        thread.join().ok()
    }
}

/// The figures of `samples`, with the tree's resident memory at each
/// milestone of `milestones` the crawl reached: `(pages, seconds)`, when
/// that many pages had been answered.
pub(crate) fn figures(samples: &[Sample], milestones: &[(u64, f64)]) -> Sampled {
    let mut figures = Sampled {
        samples: samples.len() as u64,
        ..Sampled::default()
    };
    let mut weighted = 0.0;
    for pair in samples.windows(2) {
        weighted += (pair[1].at - pair[0].at)
            * f64::midpoint(float(pair[0].resident_bytes), float(pair[1].resident_bytes));
    }
    let span = match (samples.first(), samples.last()) {
        (Some(first), Some(last)) if last.at > first.at => last.at - first.at,
        _ => 0.0,
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a mean of resident sizes, which fits a u64"
    )]
    {
        figures.average_bytes = if span > 0.0 {
            (weighted / span) as u64
        } else {
            samples.first().map_or(0, |sample| sample.resident_bytes)
        };
    }
    // No sample read no PSS: none, where the report has the OS's own peak
    // to fall back on for the RSS.
    figures.peak_pss_bytes = samples
        .iter()
        .map(|sample| sample.pss_bytes)
        .reduce(|peak, pss| Some(peak?.max(pss?)))
        .flatten();
    for sample in samples {
        figures.peak_bytes = figures.peak_bytes.max(sample.resident_bytes);
        figures.peak_private_bytes = figures.peak_private_bytes.max(sample.private_bytes);
        figures.peak_executable_bytes = figures.peak_executable_bytes.max(sample.executable_bytes);
        figures.peak_threads = figures.peak_threads.max(sample.threads);
        figures.peak_open_files = figures.peak_open_files.max(sample.open_files);
    }
    // A crawl shorter than one window has no peak: none, not 0 cores.
    for window in samples.windows(CORE_WINDOW_SAMPLES) {
        let (first, last) = (window[0], window[window.len() - 1]);
        let elapsed = last.at - first.at;
        // A resumed tool's new process starts its CPU time from zero.
        if elapsed > 0.0 && last.cpu_seconds >= first.cpu_seconds {
            let cores = (last.cpu_seconds - first.cpu_seconds) / elapsed;
            figures.peak_cores = Some(figures.peak_cores.map_or(cores, |peak| peak.max(cores)));
        }
    }
    figures.at_pages = milestones
        .iter()
        .filter_map(|&(pages, at)| {
            samples
                .iter()
                .find(|sample| sample.at >= at)
                .or(samples.last())
                .map(|sample| (pages, sample.resident_bytes))
        })
        .collect();
    figures
}

#[expect(
    clippy::cast_precision_loss,
    reason = "resident sizes stay far below 2^52"
)]
fn float(count: u64) -> f64 {
    count as f64
}

#[cfg(test)]
mod tests;
