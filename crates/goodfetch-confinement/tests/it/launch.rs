//! A Plugin started through its launcher (ADR 0033): the probe executable
//! run as `plugin-shim` confines itself to the policy, then replaces
//! itself with a copy of the probe in the Plugin's bundle, which makes its
//! probe confined, from its private temporary folder.
#![expect(
    clippy::disallowed_methods,
    reason = "these tests start processes and make files outside any async worker"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use goodfetch_confinement::launch::{LAUNCH_VARIABLE, Launch, SHIM_COMMAND};
use goodfetch_confinement::linux::MechanismKind;
use goodfetch_confinement::linux::limits::Resource;
use goodfetch_confinement::linux::probe::{Probe, Report, Request};
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::{Access, Errno, Layout, Outcome};
use goodfetch_protocol::plugin::PluginManifest;
use rstest::rstest;
use serde_json::json;
use tempfile::TempDir;

const PROBE: &str = env!("CARGO_BIN_EXE_goodfetch-confinement-probe");

/// A Plugin whose entrypoint is a copy of the probe executable, with its
/// bundle, runtime and private temporary folder under a fresh root.
struct Plugin {
    root: TempDir,
    layout: Layout,
    policy: Policy,
}

impl Plugin {
    fn new() -> Self {
        Self::limited(&json!({}))
    }

    /// A Plugin whose manifest sets `limits`.
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

    /// What the probe reports when its launcher starts it confined to the
    /// policy to make `probe`.
    fn launched(&self, probe: Probe) -> Report {
        let request = Request {
            probe,
            policy: None,
            mechanism: None,
            sandbox_baseline: false,
        };
        let launch = Launch {
            policy: self.policy.clone(),
            program: self.layout.bundle.join("probe"),
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
        let code = u8::try_from(status.code().unwrap()).unwrap();
        Report::of_status(code)
            .unwrap_or_else(|| panic!("the launcher started no probe: status {code}"))
    }

    /// What the probe reports when it makes `probe` unconfined, with no
    /// launcher: the positive control.
    fn unconfined(probe: Probe) -> Report {
        let request = Request {
            probe,
            policy: None,
            mechanism: None,
            sandbox_baseline: false,
        };
        let status = Command::new(PROBE)
            .arg(serde_json::to_string(&request).unwrap())
            .stdin(Stdio::null())
            .status()
            .unwrap();
        Report::of_status(u8::try_from(status.code().unwrap()).unwrap()).unwrap()
    }

    /// A file beside the Plugin's folders, which its policy grants nothing
    /// of.
    fn elsewhere(&self) -> PathBuf {
        fs::canonicalize(self.root.path())
            .unwrap()
            .join("elsewhere")
    }
}

/// The probe reads a file `path` names, relative to its working folder or
/// not.
fn read(path: impl AsRef<Path>) -> Probe {
    Probe::Path {
        access: Access::Read,
        path: path.as_ref().to_path_buf(),
    }
}

/// Covers: story:7-30
#[rstest]
#[case::its_bundle(|plugin: &Plugin| read(plugin.layout.bundle.join("index.js")), Outcome::Ok)]
#[case::its_temporary_folder_as_its_working_folder(|_: &Plugin| Probe::Path { access: Access::Write, path: PathBuf::from("written") }, Outcome::Ok)]
#[case::a_file_outside_its_policy(|plugin: &Plugin| read(plugin.elsewhere()), Outcome::Errno(Errno::Eacces))]
fn a_launched_plugin_runs_confined_from_its_temporary_folder(
    #[case] probe: fn(&Plugin) -> Probe,
    #[case] expected: Outcome,
) {
    let plugin = Plugin::new();
    // Without Landlock, a path outside the mount namespace's root is absent.
    let expected = match (expected, MechanismKind::of_kernel()) {
        (Outcome::Errno(Errno::Eacces), MechanismKind::MountNamespace) => {
            Outcome::Errno(Errno::Enoent)
        }
        (expected, _) => expected,
    };

    let report = plugin.launched(probe(&plugin));

    assert_eq!(report, Report::Outcome(expected));
}

/// Each limit its manifest sets holds a launched Plugin: it may set the
/// limit to its hard value, and not one past it, which the probe
/// unconfined may.
#[rstest]
#[case::memory(Resource::Memory, 64 * 1024 * 1024)]
// One second past its `cpu_time`, so that `SIGXCPU` comes first.
#[case::cpu_time(Resource::CpuTime, 61)]
#[case::open_files(Resource::OpenFiles, 32)]
fn a_launched_plugin_runs_under_each_limit_its_manifest_sets(
    #[case] resource: Resource,
    #[case] hard: u64,
) {
    let plugin = Plugin::limited(&json!({
        "memory": 64 * 1024 * 1024,
        "cpu_time": 60,
        "open_files": 32,
    }));
    let raise = |to| Probe::Raise { resource, to };

    let unconfined = Plugin::unconfined(raise(hard + 1));
    let up_to = plugin.launched(raise(hard));
    let past = plugin.launched(raise(hard + 1));

    let ok = Report::Outcome(Outcome::Ok);
    assert_eq!(
        (unconfined, up_to, past),
        (ok, ok, Report::Outcome(Outcome::Errno(Errno::Eperm)))
    );
}
