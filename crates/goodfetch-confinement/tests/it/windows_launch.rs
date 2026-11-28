//! Windows launcher and Job Object limit tests (Release, Windows; ADR 0033,
//! issue 873): verifies that `plugin-shim` enforces Job Object limits for
//! memory, CPU time, active processes, and open handles, reporting the
//! corresponding `limit_exceeded_of_status` code that the engine translates to
//! `plugin_limit_exceeded`.
#![expect(
    clippy::disallowed_methods,
    reason = "these tests start processes and make files outside any async worker"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

use std::fs;
use std::process::{Command, Stdio};

use goodfetch_confinement::launch::{
    LAUNCH_VARIABLE, Launch, SHIM_COMMAND, limit_exceeded_of_status,
};
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::Layout;
use goodfetch_confinement::windows::probe::{Probe, Request};
use goodfetch_protocol::plugin::{PluginManifest, PluginResourceLimit};
use serde_json::json;
use tempfile::TempDir;

const PROBE: &str = env!("CARGO_BIN_EXE_goodfetch-confinement-probe");

struct Plugin {
    _root: TempDir,
    layout: Layout,
    policy: Policy,
}

impl Plugin {
    fn limited(limits: &serde_json::Value) -> Self {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::under(&fs::canonicalize(root.path()).unwrap());
        for folder in [&layout.bundle, &layout.runtime, &layout.instance] {
            fs::create_dir_all(folder).unwrap();
        }
        fs::copy(PROBE, layout.bundle.join("probe.exe")).unwrap();
        let manifest = json!({
            "manifest_version": "0.1",
            "name": "probe",
            "version": "1.0.0",
            "kinds": ["proxy"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "probe.exe",
            "network_level": "none",
            "limits": limits,
        });
        let manifest = PluginManifest::read(manifest.to_string().as_bytes()).unwrap();
        let instance = Instance {
            bundle: layout.bundle.clone(),
            runtime: None,
            folders: vec![layout.instance.clone()],
            ..Instance::default()
        };
        let policy = Policy::of(&manifest, &instance, &layout.places).unwrap();
        Self {
            _root: root,
            layout,
            policy,
        }
    }

    fn run_raw(&self, probe: Probe) -> u8 {
        let request = Request {
            probe,
            parent_pid: Some(std::process::id()),
        };
        let launch = Launch {
            policy: self.policy.clone(),
            program: self.layout.bundle.join("probe.exe"),
            args: vec![serde_json::to_string(&request).unwrap()],
            folder: self.layout.instance.clone(),
            proxy_relay: None,
            proxy_loopback: None,
            windows_broker: None,
            reduced_confinement: false,
        };
        let status = Command::new(PROBE)
            .arg(SHIM_COMMAND)
            .env_clear()
            .env(LAUNCH_VARIABLE, serde_json::to_string(&launch).unwrap())
            .stdin(Stdio::null())
            .status()
            .unwrap();
        u8::try_from(status.code().unwrap()).unwrap()
    }
}

/// Covers: story:7-105, error:plugin_limit_exceeded
#[test]
fn a_windows_job_object_cpu_limit_ends_with_cpu_time_limit_exceeded() {
    let plugin = Plugin::limited(&json!({ "cpu_time": 1 }));
    let code = plugin.run_raw(Probe::BurnCpu);
    assert_eq!(
        limit_exceeded_of_status(code),
        Some(PluginResourceLimit::CpuTime),
        "expected CpuTime limit exit code, got {code}"
    );
}

/// Covers: story:7-105, error:plugin_limit_exceeded
#[test]
fn a_windows_job_object_memory_limit_ends_with_memory_limit_exceeded() {
    let plugin = Plugin::limited(&json!({ "memory": 32 * 1024 * 1024 }));
    let code = plugin.run_raw(Probe::AllocateMemory {
        bytes: 128 * 1024 * 1024,
    });
    assert_eq!(
        limit_exceeded_of_status(code),
        Some(PluginResourceLimit::Memory),
        "expected Memory limit exit code, got {code}"
    );
}

/// Covers: story:7-105, error:plugin_limit_exceeded
#[test]
fn a_windows_job_object_process_limit_ends_with_processes_limit_exceeded() {
    let plugin = Plugin::limited(&json!({ "processes": 1 }));
    let code = plugin.run_raw(Probe::SpawnChild);
    assert_eq!(
        limit_exceeded_of_status(code),
        Some(PluginResourceLimit::Processes),
        "expected Processes limit exit code, got {code}"
    );
}

/// Covers: story:7-105, error:plugin_limit_exceeded
#[test]
fn a_windows_job_object_open_files_limit_ends_with_open_files_limit_exceeded() {
    let plugin = Plugin::limited(&json!({ "open_files": 48 }));
    let code = plugin.run_raw(Probe::HoldHandles {
        dir: plugin.layout.instance.clone(),
        count: 64,
    });
    assert_eq!(
        limit_exceeded_of_status(code),
        Some(PluginResourceLimit::OpenFiles),
        "expected OpenFiles limit exit code, got {code}"
    );
}
