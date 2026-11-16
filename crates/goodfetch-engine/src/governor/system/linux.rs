//! Linux's readings, from a folder laid out as `/proc` (ADR 0040): the
//! memory limit and CPU quota of the process's own cgroup, v2 or v1, the
//! tightest along its path to the mount's root, since a parent's limit holds
//! its children too, read at each sample from the folders its `self/cgroup`
//! and the mounts its `self/mountinfo` name, which the governor finds once
//! as it starts; physical memory, `MemAvailable` and PSI's memory stalls;
//! the PSS of the engine and every descendant, so pages its browsers share
//! count once, each process read once a sample; and its CPU time.
//!
//! The opens a sample's Cost contract counts (ADR 0140's E4) go through
//! [`CountedWork::open_file`]: `meminfo`, `self/cgroup` and `self/mountinfo`,
//! and each process's `smaps_rollup`.
#![expect(
    clippy::disallowed_methods,
    reason = "every read here runs under the governor's blocking policy (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::governor::GovernorLevel;

/// A cgroup v1 memory limit at or past this many bytes is no limit: the
/// kernel reports "none" as the largest page-aligned `i64`.
const V1_UNLIMITED: u64 = 1 << 62;

/// PSI's share of the last ten seconds, in percent, from which tasks
/// stalled on memory put the host under pressure (`some`) or make it
/// critical (`full`).
const PSI_STALLED: f64 = 10.0;

/// The clock ticks per second of `/proc/<pid>/stat`, `USER_HZ`, fixed at
/// 100 for every user-space interface of the kernel.
const USER_HZ: u64 = 100;

/// A folder laid out as `/proc`, and the Counted work its counted opens
/// count on.
pub(super) struct Proc<'a> {
    pub(super) folder: &'a Path,
    pub(super) work: &'a CountedWork,
}

impl Proc<'_> {
    fn read(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.folder.join(relative)).ok()
    }

    /// [`Self::read`], its open counted on `row`.
    fn counted_read(&self, row: Row, relative: &str) -> Option<String> {
        let mut file = self
            .work
            .open_file(
                row,
                OpenOptions::new().read(true),
                &self.folder.join(relative),
            )
            .ok()?;
        let mut text = String::new();
        file.read_to_string(&mut text).ok()?;
        Some(text)
    }

    /// The process's own cgroup: where its limits are, read once, as the
    /// governor starts; none found, it has no limit.
    pub(super) fn cgroup(&self) -> Cgroup {
        let Some(cgroups) = self.counted_read(Row::GovernorStaticOpens, "self/cgroup") else {
            return Cgroup::default();
        };
        let Some(mounts) = self.counted_read(Row::GovernorStaticOpens, "self/mountinfo") else {
            return Cgroup::default();
        };
        let mounts: Vec<Mount> = mounts.lines().filter_map(Mount::parse).collect();
        let v2 = cgroups
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .and_then(|path| {
                let mount = mounts.iter().find(|mount| mount.fstype == "cgroup2")?;
                Some(mount.path_of(path))
            })
            .unwrap_or_default();
        let v1 = |controller: &str| {
            cgroups
                .lines()
                .find_map(|line| {
                    let mut fields = line.splitn(3, ':');
                    let (_, controllers, path) = (fields.next()?, fields.next()?, fields.next()?);
                    controllers
                        .split(',')
                        .any(|named| named == controller)
                        .then_some(path)
                })
                .and_then(|path| {
                    let mount = mounts.iter().find(|mount| {
                        mount.fstype == "cgroup"
                            && mount.options.split(',').any(|option| option == controller)
                    })?;
                    Some(mount.path_of(path))
                })
                .unwrap_or_default()
        };
        Cgroup {
            v2,
            v1_memory: v1("memory"),
            v1_cpu: v1("cpu"),
        }
    }

    /// `meminfo`, read once.
    pub(super) fn meminfo(&self) -> Meminfo {
        let text = self
            .counted_read(Row::GovernorMeminfoOpens, "meminfo")
            .unwrap_or_default();
        let field = |name: &str| {
            text.lines().find_map(|line| {
                let rest = line.strip_prefix(name)?.strip_prefix(':')?;
                let kilobytes: u64 = rest.trim().strip_suffix("kB")?.trim().parse().ok()?;
                Some(kilobytes * 1024)
            })
        };
        Meminfo {
            total: field("MemTotal"),
            available: field("MemAvailable"),
        }
    }

    /// The host's own memory pressure: memory in use (all but
    /// `MemAvailable`) past `pressure` or `critical` percent of physical
    /// memory, as `meminfo` read, or PSI's stalls past [`PSI_STALLED`],
    /// whichever is worse.
    pub(super) fn host(&self, meminfo: Meminfo, pressure: u64, critical: u64) -> GovernorLevel {
        let in_use = match (meminfo.total, meminfo.available) {
            (Some(total), Some(available)) if total > 0 => {
                let percent = u128::from(total.saturating_sub(available)) * 100 / u128::from(total);
                if percent >= u128::from(critical) {
                    GovernorLevel::Critical
                } else if percent >= u128::from(pressure) {
                    GovernorLevel::Pressure
                } else {
                    GovernorLevel::Normal
                }
            }
            _ => GovernorLevel::Normal,
        };
        let stalled = self
            .read("pressure/memory")
            .map_or(GovernorLevel::Normal, |psi| {
                let average = |kind: &str| {
                    psi.lines()
                        .find_map(|line| line.strip_prefix(kind))
                        .and_then(|rest| {
                            rest.split_whitespace()
                                .find_map(|field| field.strip_prefix("avg10="))
                        })
                        .and_then(|value| value.parse::<f64>().ok())
                        .unwrap_or(0.0)
                };
                if average("full ") >= PSI_STALLED {
                    GovernorLevel::Critical
                } else if average("some ") >= PSI_STALLED {
                    GovernorLevel::Pressure
                } else {
                    GovernorLevel::Normal
                }
            });
        in_use.max(stalled)
    }

    /// The PSS of the engine, `self`, and of each of `processes`, each with
    /// every descendant, in bytes; a process that ended meanwhile counts
    /// nothing. One walk from the engine reads each process's memory once,
    /// so a process of `processes` in it is not read again; one outside it
    /// is walked on its own.
    pub(super) fn footprints(&self, processes: &[u32]) -> (u64, Vec<u64>) {
        let walked = self.walk("self");
        let processes = processes
            .iter()
            .map(|process| {
                let process = process.to_string();
                if walked.0.contains_key(&process) {
                    walked.total(&process)
                } else {
                    self.footprint_from(&process)
                }
            })
            .collect();
        (walked.total("self"), processes)
    }

    /// The PSS of `process`, as `/proc` names it, and every descendant, in
    /// bytes.
    pub(super) fn footprint_from(&self, process: &str) -> u64 {
        self.walk(process).total(process)
    }

    /// `process` and every descendant, each read once.
    fn walk(&self, process: &str) -> Walked {
        let mut walked = BTreeMap::new();
        let mut found = vec![process.to_owned()];
        while let Some(process) = found.pop() {
            if walked.contains_key(&process) {
                continue;
            }
            let children = self.children(&process);
            found.extend(children.iter().cloned());
            walked.insert(process.clone(), (self.pss(&process), children));
        }
        Walked(walked)
    }

    fn pss(&self, process: &str) -> u64 {
        self.counted_read(
            Row::GovernorProcessReads,
            &format!("{process}/smaps_rollup"),
        )
        .and_then(|rollup| {
            rollup.lines().find_map(|line| {
                let kilobytes = line.strip_prefix("Pss:")?.trim().strip_suffix("kB")?.trim();
                kilobytes.parse::<u64>().ok()
            })
        })
        .map_or(0, |kilobytes| kilobytes * 1024)
    }

    fn children(&self, process: &str) -> Vec<String> {
        let Ok(threads) = self.folder.join(process).join("task").read_dir() else {
            return Vec::new();
        };
        let mut children: Vec<String> = threads
            .filter_map(Result::ok)
            .filter_map(|thread| std::fs::read_to_string(thread.path().join("children")).ok())
            .flat_map(|listed| {
                listed
                    .split_whitespace()
                    .filter(|pid| pid.bytes().all(|byte| byte.is_ascii_digit()))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect();
        children.sort();
        children.dedup();
        children
    }

    /// The engine's user and system CPU time.
    pub(super) fn cpu_time(&self) -> Duration {
        let ticks = self
            .read("self/stat")
            .and_then(|stat| {
                // The fields after the name, which may hold spaces, start
                // with its state, the third field.
                let (_, rest) = stat.rsplit_once(") ")?;
                let fields: Vec<&str> = rest.split_whitespace().collect();
                let user: u64 = fields.get(11)?.parse().ok()?;
                let system: u64 = fields.get(12)?.parse().ok()?;
                Some(user + system)
            })
            .unwrap_or(0);
        Duration::from_millis(ticks * 1000 / USER_HZ)
    }
}

/// One line of `mountinfo`.
struct Mount {
    /// The path in its file system that is the mount's root.
    root: String,
    point: PathBuf,
    fstype: String,
    options: String,
}

impl Mount {
    fn parse(line: &str) -> Option<Self> {
        let (before, after) = line.split_once(" - ")?;
        let fields: Vec<&str> = before.split(' ').collect();
        let mut after = after.split(' ');
        Some(Self {
            root: unescape(fields.get(3)?),
            point: PathBuf::from(unescape(fields.get(4)?)),
            fstype: after.next()?.to_owned(),
            options: after.nth(1).unwrap_or_default().to_owned(),
        })
    }

    /// The folders from the cgroup `path`'s own up to this mount's root,
    /// the root included.
    fn path_of(&self, path: &str) -> Vec<PathBuf> {
        self.folder_of(path)
            .ancestors()
            .take_while(|ancestor| ancestor.starts_with(&self.point))
            .map(Path::to_owned)
            .collect()
    }

    /// The folder of the cgroup `path` under this mount, when the mount
    /// shows it.
    fn folder_of(&self, path: &str) -> PathBuf {
        let relative = if self.root == "/" {
            path
        } else {
            match path.strip_prefix(self.root.as_str()) {
                Some(rest) if rest.is_empty() || rest.starts_with('/') => rest,
                // A cgroup namespace shows the process its own cgroup as
                // the mount's root.
                _ => "/",
            }
        };
        let relative = relative.trim_start_matches('/');
        if relative.is_empty() {
            self.point.clone()
        } else {
            self.point.join(relative)
        }
    }
}

/// `mountinfo`'s octal escapes (`\040` for a space) undone.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let octal = bytes.get(index + 1..index + 4).and_then(|digits| {
            let text = std::str::from_utf8(digits).ok()?;
            u8::from_str_radix(text, 8).ok()
        });
        match (bytes[index], octal) {
            (b'\\', Some(byte)) => {
                out.push(byte);
                index += 4;
            }
            (byte, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What one read of `meminfo` found, in bytes.
#[derive(Clone, Copy, Debug)]
pub(super) struct Meminfo {
    /// Physical memory, `MemTotal`.
    pub(super) total: Option<u64>,
    /// `MemAvailable`.
    pub(super) available: Option<u64>,
}

/// Where the process's own cgroup keeps its limits: for v2, and for v1's
/// `memory` and `cpu` controllers, the folders from its own cgroup's up to
/// its mount's root. Found once, as the governor starts; each limit in them
/// is read at each sample, so one changed since is seen.
#[derive(Clone, Debug, Default)]
pub(crate) struct Cgroup {
    v2: Vec<PathBuf>,
    v1_memory: Vec<PathBuf>,
    v1_cpu: Vec<PathBuf>,
}

impl Cgroup {
    /// The tightest memory limit along its path, v2 or v1, if any.
    pub(super) fn memory_limit(&self) -> Option<u64> {
        let v2 = tightest(&self.v2, "memory.max", |text| text.trim().parse().ok());
        let v1 = tightest(&self.v1_memory, "memory.limit_in_bytes", |text| {
            text.trim().parse().ok()
        })
        .filter(|limit| *limit < V1_UNLIMITED);
        [v2, v1].into_iter().flatten().min()
    }

    /// The CPUs the tightest CPU quota along its path allows, v2's
    /// `cpu.max` or v1's `cpu.cfs_quota_us` over `cpu.cfs_period_us`,
    /// rounded down and at least one, as `available_parallelism` counts
    /// them; none without a quota.
    pub(super) fn cpu_quota(&self) -> Option<u64> {
        let v2 = tightest(&self.v2, "cpu.max", |text| {
            let mut fields = text.split_whitespace();
            let quota: u64 = fields.next()?.parse().ok()?;
            let period: u64 = fields.next()?.parse().ok()?;
            (period > 0).then(|| quota / period)
        });
        let v1 = self
            .v1_cpu
            .iter()
            .filter_map(|folder| {
                let read = |file: &str| -> Option<u64> {
                    std::fs::read_to_string(folder.join(file))
                        .ok()?
                        .trim()
                        .parse()
                        .ok()
                };
                let (quota, period) = (read("cpu.cfs_quota_us")?, read("cpu.cfs_period_us")?);
                (period > 0).then(|| quota / period)
            })
            .min();
        [v2, v1]
            .into_iter()
            .flatten()
            .min()
            .map(|quota| quota.max(1))
    }
}

/// The tightest of `file` in `folders`, each read by `parse`; `max`, `-1`
/// and missing files are none.
fn tightest(folders: &[PathBuf], file: &str, parse: impl Fn(&str) -> Option<u64>) -> Option<u64> {
    folders
        .iter()
        .filter_map(|folder| std::fs::read_to_string(folder.join(file)).ok())
        .filter_map(|text| parse(&text))
        .min()
}

/// Each process a walk read, by its name in `/proc`: its PSS, in bytes,
/// and its children.
struct Walked(BTreeMap<String, (u64, Vec<String>)>);

impl Walked {
    /// The PSS of `process` and every descendant the walk read.
    fn total(&self, process: &str) -> u64 {
        let mut found = vec![process];
        let mut total = 0;
        while let Some(process) = found.pop() {
            if let Some((pss, children)) = self.0.get(process) {
                total += pss;
                found.extend(children.iter().map(String::as_str));
            }
        }
        total
    }
}
