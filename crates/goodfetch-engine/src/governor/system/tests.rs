//! What the governor reads from a folder laid out as Linux's `/proc`, with
//! the cgroup file system its `mountinfo` names (spec #213's Seam 4): the
//! Memory allowance, the host's pressure, the footprint of the engine and
//! its children, and its CPU time.
#![expect(
    clippy::disallowed_methods,
    reason = "the test writes the folder the governor reads, outside any runtime"
)]

use std::fs;
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::governor::{AllowanceSource, GovernorLevel};
use rstest::rstest;
use tempfile::TempDir;

use super::{Inputs, System};

const GIB: u64 = 1 << 30;
const MIB: u64 = 1 << 20;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A `/proc` of an 8 GiB machine with 6 GiB available, no memory pressure,
/// and the engine alone using 100 MiB, in no cgroup with a memory limit.
struct Proc {
    folder: TempDir,
}

impl Proc {
    fn new() -> Self {
        let folder = TempDir::new().unwrap();
        let proc = Self { folder };
        proc.meminfo(8 * GIB, 6 * GIB);
        proc.file("pressure/memory", &psi(0.0, 0.0));
        proc.file("self/smaps_rollup", &rollup(100 * MIB));
        proc.file("self/stat", &stat(0, 0));
        proc.file("self/cgroup", "0::/\n");
        proc.file("self/mountinfo", "");
        proc
    }

    fn root(&self) -> &Path {
        self.folder.path()
    }

    fn file(&self, relative: &str, text: &str) {
        write(&self.root().join("proc").join(relative), text);
    }

    fn meminfo(&self, total: u64, available: u64) {
        self.file(
            "meminfo",
            &format!(
                "MemTotal:       {} kB\nMemFree:         1024 kB\nMemAvailable:   {} kB\n",
                total / 1024,
                available / 1024
            ),
        );
    }

    /// Mounts the cgroup v2 file system at `cgroup/`.
    fn cgroup2(&self) -> std::path::PathBuf {
        let mount = self.root().join("cgroup");
        self.file(
            "self/mountinfo",
            &format!(
                "22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw\n\
                 48 22 0:30 / {} rw,nosuid,nodev,noexec,relatime shared:7 - cgroup2 cgroup2 rw,nsdelegate\n",
                mount.display()
            ),
        );
        mount
    }

    fn system(&self) -> System {
        System::linux_files(self.root().join("proc"))
    }

    fn read(&self, setting: Option<u64>) -> super::SystemReading {
        self.system().read(&Inputs {
            memory_allowance: setting,
            pressure: 85,
            critical: 95,
        })
    }
}

fn psi(some: f64, full: f64) -> String {
    format!(
        "some avg10={some:.2} avg60=0.00 avg300=0.00 total=0\nfull avg10={full:.2} avg60=0.00 avg300=0.00 total=0\n"
    )
}

fn rollup(pss: u64) -> String {
    format!(
        "55d0c0a00000-7ffd5e5f2000 ---p 00000000 00:00 0                          [rollup]\n\
         Rss:              {} kB\nPss:              {} kB\nPss_Anon:          512 kB\n",
        pss / 1024 * 2,
        pss / 1024
    )
}

/// `/proc/<pid>/stat` with `user` and `system` clock ticks (`USER_HZ`, 100).
fn stat(user: u64, system: u64) -> String {
    format!(
        "4242 (goodfetch collie) S 1 4242 4242 0 -1 4194560 100 0 0 0 {user} {system} 0 0 20 0 8 0 100 0 0\n"
    )
}

#[test]
fn with_no_limit_the_allowance_is_half_of_physical_memory() {
    let proc = Proc::new();

    let reading = proc.read(None);

    assert_eq!(
        (reading.allowance, reading.allowance_source),
        (4 * GIB, AllowanceSource::Physical)
    );
}

#[test]
fn a_cgroup_v2_limit_is_read_from_the_engines_own_cgroup() {
    let proc = Proc::new();
    let mount = proc.cgroup2();
    proc.file("self/cgroup", "0::/user.slice/run-1.scope\n");
    write(
        &mount.join("user.slice/run-1.scope/memory.max"),
        "268435456\n",
    );
    write(&mount.join("user.slice/memory.max"), "max\n");

    let reading = proc.read(None);

    assert_eq!(
        (reading.allowance, reading.allowance_source),
        (256 * MIB, AllowanceSource::Cgroup)
    );
}

#[test]
fn a_parent_cgroups_tighter_limit_holds_its_children() {
    let proc = Proc::new();
    let mount = proc.cgroup2();
    proc.file(
        "self/cgroup",
        "0::/kubepods.slice/pod.slice/container.scope\n",
    );
    write(
        &mount.join("kubepods.slice/pod.slice/container.scope/memory.max"),
        "max\n",
    );
    write(
        &mount.join("kubepods.slice/pod.slice/memory.max"),
        "536870912\n",
    );
    write(&mount.join("kubepods.slice/memory.max"), "1073741824\n");

    let reading = proc.read(None);

    assert_eq!(reading.allowance, 512 * MIB);
}

#[test]
fn a_limit_changed_since_the_last_sample_is_seen() {
    let proc = Proc::new();
    let mount = proc.cgroup2();
    proc.file("self/cgroup", "0::/pod.scope\n");
    write(&mount.join("pod.scope/memory.max"), "268435456\n");
    let system = proc.system();
    let inputs = Inputs {
        memory_allowance: None,
        pressure: 85,
        critical: 95,
    };
    let before = system.read(&inputs).allowance;

    write(&mount.join("pod.scope/memory.max"), "536870912\n");
    let after = system.read(&inputs).allowance;

    assert_eq!((before, after), (256 * MIB, 512 * MIB));
}

#[rstest]
#[case::mounted_at_the_hierarchys_root("/", "docker/abc/memory.limit_in_bytes")]
#[case::mounted_at_the_containers_own_cgroup("/docker/abc", "memory.limit_in_bytes")]
fn a_cgroup_v1_memory_limit_is_read_from_the_engines_own_cgroup(
    #[case] mount_root: &str,
    #[case] file: &str,
) {
    let proc = Proc::new();
    let mount = proc.root().join("memory");
    proc.file(
        "self/mountinfo",
        &format!(
            "30 22 0:26 / /sys/fs/cgroup/cpu rw,nosuid - cgroup cgroup rw,cpu,cpuacct\n\
             31 22 0:27 {mount_root} {} rw,nosuid - cgroup cgroup rw,memory\n",
            mount.display()
        ),
    );
    proc.file(
        "self/cgroup",
        "5:cpu,cpuacct:/docker/abc\n4:memory:/docker/abc\n0::/\n",
    );
    write(&mount.join(file), "805306368\n");

    let reading = proc.read(None);

    assert_eq!(
        (reading.allowance, reading.allowance_source),
        (768 * MIB, AllowanceSource::Cgroup)
    );
}

#[test]
fn a_cgroup_v1_without_a_limit_leaves_half_of_physical_memory() {
    let proc = Proc::new();
    let mount = proc.root().join("memory");
    proc.file(
        "self/mountinfo",
        &format!(
            "31 22 0:27 / {} rw,nosuid - cgroup cgroup rw,memory\n",
            mount.display()
        ),
    );
    proc.file("self/cgroup", "4:memory:/user.slice\n");
    write(
        &mount.join("user.slice/memory.limit_in_bytes"),
        "9223372036854771712\n",
    );

    let reading = proc.read(None);

    assert_eq!(
        (reading.allowance, reading.allowance_source),
        (4 * GIB, AllowanceSource::Physical)
    );
}

#[rstest]
#[case::tighter_than_the_cgroup(Some(128 * MIB), 128 * MIB, AllowanceSource::Setting)]
#[case::looser_than_the_cgroup(Some(GIB), 256 * MIB, AllowanceSource::Cgroup)]
#[case::unset(None, 256 * MIB, AllowanceSource::Cgroup)]
fn the_allowance_is_the_tightest_of_the_cgroup_the_setting_and_physical_memory(
    #[case] setting: Option<u64>,
    #[case] allowance: u64,
    #[case] source: AllowanceSource,
) {
    let proc = Proc::new();
    let mount = proc.cgroup2();
    proc.file("self/cgroup", "0::/run.scope\n");
    write(&mount.join("run.scope/memory.max"), "268435456\n");

    let reading = proc.read(setting);

    assert_eq!(
        (reading.allowance, reading.allowance_source),
        (allowance, source)
    );
}

#[rstest]
#[case::plenty_available(6 * GIB, psi(0.0, 0.0), GovernorLevel::Normal)]
#[case::available_under_the_pressure_share(GIB, psi(0.0, 0.0), GovernorLevel::Pressure)]
#[case::available_under_the_critical_share(256 * MIB, psi(0.0, 0.0), GovernorLevel::Critical)]
#[case::some_tasks_stalled_on_memory(6 * GIB, psi(12.5, 0.0), GovernorLevel::Pressure)]
#[case::every_task_stalled_on_memory(6 * GIB, psi(12.5, 10.0), GovernorLevel::Critical)]
fn the_hosts_own_memory_pressure_comes_from_psi_and_memavailable(
    #[case] available: u64,
    #[case] pressure: String,
    #[case] host: GovernorLevel,
) {
    let proc = Proc::new();
    proc.meminfo(8 * GIB, available);
    proc.file("pressure/memory", &pressure);

    let reading = proc.read(None);

    assert_eq!(reading.host, host);
}

#[test]
fn a_kernel_without_psi_is_judged_by_memavailable_alone() {
    let proc = Proc::new();
    fs::remove_file(proc.root().join("proc/pressure/memory")).unwrap();
    proc.meminfo(8 * GIB, GIB);

    let reading = proc.read(None);

    assert_eq!(reading.host, GovernorLevel::Pressure);
}

#[test]
fn the_footprint_is_the_pss_of_the_engine_and_every_descendant() {
    let proc = Proc::new();
    proc.file("self/task/4242/children", "5000 5001 ");
    proc.file("self/task/4243/children", "");
    proc.file("5000/smaps_rollup", &rollup(300 * MIB));
    proc.file("5000/task/5000/children", "6000");
    proc.file("6000/smaps_rollup", &rollup(50 * MIB));
    proc.file("6000/task/6000/children", "");
    // A child that exited between the listing and its reading counts nothing.
    proc.file("5001/task/5001/children", "");

    let reading = proc.read(None);

    assert_eq!(reading.used, (100 + 300 + 50) * MIB);
}

#[test]
fn the_cpu_time_is_the_engines_user_and_system_time() {
    let proc = Proc::new();
    proc.file("self/stat", &stat(150, 50));

    let reading = proc.read(None);

    assert_eq!(reading.cpu_time, Duration::from_secs(2));
}

/// The browser pool's reading of one browser: the process it names and its
/// descendants, from the OS itself (ADR 0040's per-browser cap).
#[test]
fn the_footprint_of_a_running_process_counts_its_memory_and_of_a_gone_one_nothing() {
    let own = super::footprint_of(std::process::id());
    let gone = super::footprint_of(u32::MAX);

    assert_eq!((own > MIB, gone), (true, 0));
}
