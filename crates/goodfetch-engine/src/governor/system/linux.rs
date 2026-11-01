//! Linux's readings, from a folder laid out as `/proc` (ADR 0040): the
//! memory limit of the process's own cgroup, v2 or v1, found through its
//! `self/cgroup` and the mount its `self/mountinfo` names, the tightest
//! along its path to the mount's root, since a parent's limit holds its
//! children too; physical memory, `MemAvailable` and PSI's memory stalls;
//! the PSS of the engine and every descendant, so pages its browsers share
//! count once; and its CPU time.
#![expect(
    clippy::disallowed_methods,
    reason = "every read here runs under the governor's blocking policy (STYLE.md)"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// A folder laid out as `/proc`.
pub(super) struct Proc<'a>(pub(super) &'a Path);

impl Proc<'_> {
    fn read(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(relative)).ok()
    }

    /// The memory limit of the process's own cgroup, if any.
    pub(super) fn cgroup_limit(&self) -> Option<u64> {
        let cgroups = self.read("self/cgroup")?;
        let mounts = self.read("self/mountinfo")?;
        let v2 = cgroups
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .and_then(|path| {
                let mount = mounts
                    .lines()
                    .filter_map(Mount::parse)
                    .find(|mount| mount.fstype == "cgroup2")?;
                tightest(&mount.folder_of(path), &mount.point, "memory.max")
            });
        let v1 = cgroups
            .lines()
            .find_map(|line| {
                let mut fields = line.splitn(3, ':');
                let (_, controllers, path) = (fields.next()?, fields.next()?, fields.next()?);
                controllers
                    .split(',')
                    .any(|controller| controller == "memory")
                    .then_some(path)
            })
            .and_then(|path| {
                let mount = mounts.lines().filter_map(Mount::parse).find(|mount| {
                    mount.fstype == "cgroup"
                        && mount.options.split(',').any(|option| option == "memory")
                })?;
                tightest(
                    &mount.folder_of(path),
                    &mount.point,
                    "memory.limit_in_bytes",
                )
            })
            .filter(|limit| *limit < V1_UNLIMITED);
        [v2, v1].into_iter().flatten().min()
    }

    /// `meminfo`'s field `name`, in bytes.
    fn meminfo(&self, name: &str) -> Option<u64> {
        self.read("meminfo")?.lines().find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(':')?;
            let kilobytes: u64 = rest.trim().strip_suffix("kB")?.trim().parse().ok()?;
            Some(kilobytes * 1024)
        })
    }

    /// Physical memory, in bytes.
    pub(super) fn physical(&self) -> Option<u64> {
        self.meminfo("MemTotal")
    }

    /// The host's own memory pressure: memory in use (all but
    /// `MemAvailable`) past `pressure` or `critical` percent of physical
    /// memory, or PSI's stalls past [`PSI_STALLED`], whichever is worse.
    pub(super) fn host(&self, pressure: u64, critical: u64) -> GovernorLevel {
        let in_use = match (self.meminfo("MemTotal"), self.meminfo("MemAvailable")) {
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

    /// The PSS of the engine, `self`, and every descendant, in bytes; a
    /// process that ended meanwhile counts nothing.
    pub(super) fn footprint(&self) -> u64 {
        self.footprint_from("self")
    }

    /// The PSS of `process`, as `/proc` names it, and every descendant, in
    /// bytes.
    pub(super) fn footprint_from(&self, process: &str) -> u64 {
        let mut found = vec![process.to_owned()];
        let mut total = 0;
        while let Some(process) = found.pop() {
            total += self.pss(&process);
            found.extend(self.children(&process));
        }
        total
    }

    fn pss(&self, process: &str) -> u64 {
        self.read(&format!("{process}/smaps_rollup"))
            .and_then(|rollup| {
                rollup.lines().find_map(|line| {
                    let kilobytes = line.strip_prefix("Pss:")?.trim().strip_suffix("kB")?.trim();
                    kilobytes.parse::<u64>().ok()
                })
            })
            .map_or(0, |kilobytes| kilobytes * 1024)
    }

    fn children(&self, process: &str) -> Vec<String> {
        let Ok(threads) = self.0.join(process).join("task").read_dir() else {
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

/// The tightest limit in `file` from `folder` up to `mount`, the mount's
/// root included; `max` and missing files are none.
fn tightest(folder: &Path, mount: &Path, file: &str) -> Option<u64> {
    folder
        .ancestors()
        .take_while(|ancestor| ancestor.starts_with(mount))
        .filter_map(|ancestor| std::fs::read_to_string(ancestor.join(file)).ok())
        .filter_map(|text| text.trim().parse::<u64>().ok())
        .min()
}
