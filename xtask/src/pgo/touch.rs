//! The pages of Foxhound's image a crawl touches, in the order it first
//! touches them: what widens the order file past the profile's functions to
//! the code it does not count (libc, the allocator) and the constants the
//! crawl reads (ADR 0057's note of 2026-11-14).
//!
//! The executable runs from a copy on tmpfs, whose `shmem_enabled` of
//! `never` gives it single pages, with the kernel's fault-around set to one
//! page for the run (`sudo -n`), so a page is in the process's page table
//! once it touched it, and only then. A thread of its own reads, every
//! [`INTERVAL`], each such process's `/proc/<pid>/pagemap` over its image's
//! mappings: the present bit, which needs no privilege.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: the sampler reads /proc with blocking calls on a thread of its own, which stops when the workload ends (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::os::unix::fs::FileExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

use crate::compare::training::{self, Executables, Workload};
use crate::output;

/// The page the map is kept in: the kernel's on x86-64.
pub(crate) const PAGE: u64 = 4096;

/// How often the sampler reads the processes' page tables.
const INTERVAL: Duration = Duration::from_millis(20);

/// The kernel's fault-around window, in bytes.
const FAULT_AROUND: &str = "/sys/kernel/debug/fault_around_bytes";

/// The crawls the map is made of: the resident-memory check's, whose peak
/// `just targets` reads, then scenario 1's.
const TOUCHING: [Workload; 2] = [Workload::Defaults, training::SCENARIO_1];

/// The pages of Foxhound's image, by their address in the linked image over
/// [`PAGE`], that [`TOUCHING`]'s crawls touched, each with the sample it was
/// first seen in.
pub(crate) fn touched_pages(executables: &Executables<'_>) -> Result<BTreeMap<u64, u32>> {
    let folder = tempfile::Builder::new()
        .prefix("goodfetch-pgo-")
        .tempdir_in("/dev/shm")
        .context("creating a folder on /dev/shm")?;
    let copy = folder.path().join("foxhound");
    std::fs::copy(executables.foxhound, &copy)
        .with_context(|| format!("copying {}", executables.foxhound.display()))?;
    let _fault_around = FaultAround::one_page()?;
    let mut touched = BTreeMap::new();
    let mut next = 0;
    for workload in TOUCHING {
        let (stop, stopped) = mpsc::channel::<()>();
        let sampler = {
            let copy = copy.clone();
            std::thread::spawn(move || sample_until(&copy, next, &stopped))
        };
        let ran = workload.run(&Executables {
            foxhound: &copy,
            ..*executables
        });
        drop(stop);
        let (found, after) = sampler
            .join()
            .map_err(|_| anyhow::anyhow!("the page sampler panicked"))?;
        ran.with_context(|| format!("running {}", workload.name()))?;
        for (page, sample) in found {
            touched.entry(page).or_insert(sample);
        }
        next = after;
    }
    if touched.is_empty() {
        bail!("no sample found Foxhound's image in a process's page table");
    }
    Ok(touched)
}

/// Samples every [`INTERVAL`] from sample `first` until `stopped` closes:
/// the pages found, each with its first sample, and the next sample's
/// number.
fn sample_until(
    executable: &Path,
    first: u32,
    stopped: &mpsc::Receiver<()>,
) -> (BTreeMap<u64, u32>, u32) {
    let mut found = BTreeMap::new();
    let mut sample = first;
    loop {
        for pid in processes_of(executable) {
            for page in present_pages(pid, executable) {
                found.entry(page).or_insert(sample);
            }
        }
        sample += 1;
        if !matches!(
            stopped.recv_timeout(INTERVAL),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            return (found, sample);
        }
    }
}

/// The processes running `executable` now.
fn processes_of(executable: &Path) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|path| path == executable)
        })
        .collect()
}

/// The pages of `executable`'s image the process `pid` has in its page
/// table, by their address in the linked image over [`PAGE`]: none once
/// it is gone.
fn present_pages(pid: u32, executable: &Path) -> Vec<u64> {
    let Ok(maps) = std::fs::read_to_string(format!("/proc/{pid}/maps")) else {
        return Vec::new();
    };
    let mappings = image_mappings(&maps, executable);
    // A static PIE is mapped whole from file offset 0 at its load address,
    // where its link address 0 is.
    let Some(base) = mappings
        .iter()
        .find(|mapping| mapping.offset == 0)
        .map(|mapping| mapping.start)
    else {
        return Vec::new();
    };
    let Ok(pagemap) = std::fs::File::open(format!("/proc/{pid}/pagemap")) else {
        return Vec::new();
    };
    let mut pages = Vec::new();
    for mapping in mappings {
        let count = usize::try_from((mapping.end - mapping.start) / PAGE).unwrap_or_default();
        let mut entries = vec![0; count * 8];
        if pagemap
            .read_exact_at(&mut entries, mapping.start / PAGE * 8)
            .is_err()
        {
            continue;
        }
        let first = (mapping.start - base) / PAGE;
        pages.extend(
            entries
                .as_chunks::<8>()
                .0
                .iter()
                .zip(first..)
                .filter(|(entry, _)| u64::from_le_bytes(**entry) >> 63 == 1)
                .map(|(_, page)| page),
        );
    }
    pages
}

/// A mapping of the executable's file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Mapping {
    pub(crate) start: u64,
    pub(crate) end: u64,
    /// Its offset in the file.
    pub(crate) offset: u64,
}

/// The mappings of `executable` in `maps`, a `/proc/<pid>/maps`: each line
/// `<start>-<end> <perms> <offset> <dev> <inode> <path>`.
pub(crate) fn image_mappings(maps: &str, executable: &Path) -> Vec<Mapping> {
    maps.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let (start, end) = fields.next()?.split_once('-')?;
            let offset = fields.nth(1)?;
            let path = fields.nth(2)?;
            (Path::new(path) == executable).then_some(())?;
            Some(Mapping {
                start: u64::from_str_radix(start, 16).ok()?,
                end: u64::from_str_radix(end, 16).ok()?,
                offset: u64::from_str_radix(offset, 16).ok()?,
            })
        })
        .collect()
}

/// The kernel's fault-around window set to one page while it lives, so a
/// fault maps the touched page alone; the window it had is set back when
/// it is dropped.
struct FaultAround {
    had: String,
}

impl FaultAround {
    fn one_page() -> Result<Self> {
        let read = Command::new("sudo")
            .args(["-n", "cat", FAULT_AROUND])
            .stderr(Stdio::inherit())
            .output()
            .context("running sudo -n cat")?;
        if !read.status.success() {
            bail!("reading {FAULT_AROUND} needs `sudo -n`, for the touch map");
        }
        let had = String::from_utf8_lossy(&read.stdout).trim().to_owned();
        write_fault_around(&PAGE.to_string())?;
        Ok(Self { had })
    }
}

impl Drop for FaultAround {
    fn drop(&mut self) {
        if let Err(error) = write_fault_around(&self.had) {
            output::error(&format!(
                "warning: {FAULT_AROUND} stays {PAGE}, not {}: {error:#}",
                self.had
            ));
        }
    }
}

fn write_fault_around(bytes: &str) -> Result<()> {
    let mut tee = Command::new("sudo")
        .args(["-n", "tee", FAULT_AROUND])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .context("running sudo -n tee")?;
    tee.stdin
        .take()
        .context("opening tee's input")?
        .write_all(bytes.as_bytes())
        .context("writing to tee")?;
    if !tee.wait().context("waiting for tee")?.success() {
        bail!("writing {FAULT_AROUND} failed");
    }
    Ok(())
}

/// The touched pages' size in MiB, which `refresh` prints.
#[expect(
    clippy::cast_precision_loss,
    reason = "an image's bytes are far below 2^52"
)]
pub(crate) fn mebibytes(touched: &BTreeMap<u64, u32>) -> f64 {
    (touched.len() as u64 * PAGE) as f64 / f64::from(1 << 20)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_mappings_are_the_executables_own() {
        let maps = "\
7f0000000000-7f0000002000 r--p 00000000 00:1a 42 /dev/shm/x/foxhound
7f0000002000-7f0000005000 r-xp 00002000 00:1a 42 /dev/shm/x/foxhound
7f0000005000-7f0000006000 rw-p 00000000 00:00 0
7f0000007000-7f0000008000 r--p 00000000 00:1a 43 /dev/shm/x/other
";

        assert_eq!(
            image_mappings(maps, Path::new("/dev/shm/x/foxhound")),
            [
                Mapping {
                    start: 0x7f00_0000_0000,
                    end: 0x7f00_0000_2000,
                    offset: 0
                },
                Mapping {
                    start: 0x7f00_0000_2000,
                    end: 0x7f00_0000_5000,
                    offset: 0x2000
                },
            ]
        );
    }
}
