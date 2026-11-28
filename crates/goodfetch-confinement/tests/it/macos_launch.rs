//! macOS launcher tests (Release, macOS; ADR 0033, issue 867): verifies that
//! `plugin-shim` confines via Seatbelt, sets macOS rlimits (`RLIMIT_CPU`,
//! `RLIMIT_NOFILE`), and runs from the instance's private temporary folder.
#![expect(
    clippy::disallowed_methods,
    reason = "these tests start processes and make files outside any async worker"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use goodfetch_confinement::launch::{LAUNCH_VARIABLE, Launch, SHIM_COMMAND};
use goodfetch_confinement::macos::limits::Resource;
use goodfetch_confinement::macos::probe::{Probe, Report, Request};
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::{Access, Errno, Layout, Outcome};
use goodfetch_protocol::plugin::PluginManifest;
use rstest::rstest;
use serde_json::json;
use tempfile::TempDir;

const PROBE: &str = env!("CARGO_BIN_EXE_goodfetch-confinement-probe");

struct Plugin {
    root: TempDir,
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
        fs::copy(PROBE, layout.bundle.join("probe")).unwrap();
        fs::write(layout.bundle.join("index.js"), "probe").unwrap();
        fs::write(root.path().join("elsewhere"), "probe").unwrap();
        let manifest = json!({
            "manifest_version": "0.1",
            "name": "probe",
            "version": "1.0.0",
            "kinds": ["proxy"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "probe",
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
            root,
            layout,
            policy,
        }
    }

    fn launched(&self, probe: Probe) -> Report {
        let request = Request {
            probe,
            policy: None,
            proxy_port: None,
        };
        let launch = Launch {
            policy: self.policy.clone(),
            program: self.layout.bundle.join("probe"),
            args: vec![serde_json::to_string(&request).unwrap()],
            folder: self.layout.instance.clone(),
            proxy_relay: None,
            proxy_loopback: None,
            reduced_confinement: false,
        };
        let status = Command::new(PROBE)
            .arg(SHIM_COMMAND)
            .env_clear()
            .env(LAUNCH_VARIABLE, serde_json::to_string(&launch).unwrap())
            .stdin(Stdio::null())
            .status()
            .unwrap();
        let code = u8::try_from(status.code().unwrap()).unwrap();
        Report::of_status(code)
            .unwrap_or_else(|| panic!("the launcher started no probe: status {code}"))
    }

    fn unconfined(probe: Probe) -> Report {
        let request = Request {
            probe,
            policy: None,
            proxy_port: None,
        };
        let status = Command::new(PROBE)
            .arg(serde_json::to_string(&request).unwrap())
            .stdin(Stdio::null())
            .status()
            .unwrap();
        Report::of_status(u8::try_from(status.code().unwrap()).unwrap()).unwrap()
    }

    fn elsewhere(&self) -> PathBuf {
        fs::canonicalize(self.root.path())
            .unwrap()
            .join("elsewhere")
    }
}

/// Covers: story:7-22, story:7-54
#[test]
fn a_launched_macos_plugin_runs_under_seatbelt_from_its_temporary_folder() {
    let plugin = Plugin::limited(&json!({}));
    assert_eq!(
        plugin.launched(Probe::Path {
            access: Access::Read,
            path: plugin.layout.bundle.join("index.js"),
        }),
        Report::Outcome(Outcome::Ok)
    );
    assert_eq!(
        plugin.launched(Probe::Path {
            access: Access::Write,
            path: PathBuf::from("written"),
        }),
        Report::Outcome(Outcome::Ok)
    );
    assert_eq!(
        plugin.launched(Probe::Path {
            access: Access::Read,
            path: plugin.elsewhere(),
        }),
        Report::Outcome(Outcome::Errno(Errno::Eperm))
    );
}

/// Covers: story:7-22, story:7-54
#[rstest]
#[case::cpu_time(Resource::CpuTime, 61)]
#[case::open_files(Resource::OpenFiles, 32)]
fn a_launched_macos_plugin_enforces_cpu_time_and_open_files_rlimits(
    #[case] resource: Resource,
    #[case] hard: u64,
) {
    let plugin = Plugin::limited(&json!({
        "cpu_time": 60,
        "open_files": 32,
    }));
    let raise = |to| Probe::Raise { resource, to };

    let ok = Report::Outcome(Outcome::Ok);
    assert_eq!(
        (
            Plugin::unconfined(raise(hard + 1)),
            plugin.launched(raise(hard)),
            plugin.launched(raise(hard + 1)),
        ),
        (ok, ok, Report::Outcome(Outcome::Errno(Errno::Eperm)))
    );
}
