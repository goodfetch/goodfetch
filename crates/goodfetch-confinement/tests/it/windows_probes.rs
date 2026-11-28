//! Windows AppContainer + filter broker probes (Release, Windows; ADR 0067,
//! issues 873 and 879): every case of `probes/windows/none.toml`,
//! `probes/windows/proxy_tcp.toml`, and `probes/windows/direct.toml`, run
//! through `goodfetch-confinement-probe` unconfined (positive control), then
//! confined via `plugin-shim` under AppContainer + restricted token + Job
//! Object + alternate desktop (+ persistent WFP filters for `proxy_tcp` and
//! `direct`), asserting both the exact error (`EACCES`) and the active
//! mechanism (`Mechanism::AppContainer`, verified by
//! `GetTokenInformation(..., TokenIsAppContainer, ...) != 0`).
#![expect(
    clippy::disallowed_methods,
    reason = "these tests start processes and make files outside any async worker"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

use std::fs;
use std::net::TcpListener;
use std::process::{Command, Stdio};

use goodfetch_confinement::launch::{LAUNCH_VARIABLE, Launch, SHIM_COMMAND};
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::{Access, Layout};
use goodfetch_confinement::windows::probe::{Probe, Report, Request};
use goodfetch_confinement::windows::{
    Mechanism, WindowsOperation, WindowsProbeTable, committed_probes,
};
use goodfetch_protocol::plugin::{NetworkLevel, PluginManifest};
use serde_json::json;
use tempfile::TempDir;

const PROBE: &str = env!("CARGO_BIN_EXE_goodfetch-confinement-probe");

struct Fixture {
    _root: TempDir,
    layout: Layout,
    policy: Policy,
    other_listener: TcpListener,
}

impl Fixture {
    fn new(table: &WindowsProbeTable) -> Self {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let layout = Layout::under(&canonical_root);
        for folder in [&layout.bundle, &layout.runtime, &layout.instance] {
            fs::create_dir_all(folder).unwrap();
        }
        fs::copy(PROBE, layout.bundle.join("probe.exe")).unwrap();
        for case in &table.path {
            let path = layout.resolve(&case.path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            if case.access == Access::Read {
                fs::write(&path, "probe").unwrap();
            }
        }
        let manifest = json!({
            "manifest_version": "0.1",
            "name": "probe",
            "version": "1.0.0",
            "kinds": ["notification"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "runtime": { "kind": "node", "version": "22.11.0" },
            "entrypoint": "probe.exe",
            "network_level": "none",
        });
        let manifest = PluginManifest::read(manifest.to_string().as_bytes()).unwrap();
        let instance = Instance {
            bundle: layout.bundle.clone(),
            runtime: Some(layout.runtime.clone()),
            folders: vec![layout.instance.clone()],
            ..Instance::default()
        };
        let policy = Policy::of(&manifest, &instance, &layout.places).unwrap();
        let other_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        Self {
            _root: root,
            layout,
            policy,
            other_listener,
        }
    }

    fn other_port(&self) -> u16 {
        self.other_listener.local_addr().unwrap().port()
    }

    fn run_unconfined(&self, probe: Probe) -> Result<Report, String> {
        let request = Request {
            probe,
            parent_pid: Some(std::process::id()),
        };
        let status = Command::new(PROBE)
            .arg(serde_json::to_string(&request).unwrap())
            .stdin(Stdio::null())
            .status()
            .unwrap();
        decode_status(status)
    }

    fn run_confined(&self, probe: Probe) -> Result<Report, String> {
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
        decode_status(status)
    }
}

fn decode_status(status: std::process::ExitStatus) -> Result<Report, String> {
    let code = status
        .code()
        .ok_or_else(|| format!("ended without a report: {status}"))?;
    u8::try_from(code)
        .ok()
        .and_then(Report::of_status)
        .ok_or_else(|| format!("status {code} is no report"))
}

/// Covers: story:7-105
#[test]
fn every_windows_probe_at_none_passes_unconfined_then_refuses_under_appcontainer() {
    let table = WindowsProbeTable::read(committed_probes(NetworkLevel::None).unwrap()).unwrap();
    let fixture = Fixture::new(&table);

    let mechanism = fixture.run_confined(Probe::Mechanism).unwrap();
    assert_eq!(
        mechanism,
        Report::Mechanism(Mechanism::AppContainer),
        "expected AppContainer mechanism at none"
    );

    let mut failures = Vec::new();
    for case in &table.operation {
        let port =
            (case.operation == WindowsOperation::NetworkOutboundIp).then(|| fixture.other_port());
        let make = || Probe::Operation {
            operation: case.operation,
            port,
        };
        let unconfined = fixture.run_unconfined(make());
        if unconfined != Ok(Report::Outcome(case.unconfined)) {
            failures.push(format!(
                "{}: unconfined expected {:?}, got {:?}",
                case.rule,
                Report::Outcome(case.unconfined),
                unconfined
            ));
            continue;
        }
        let confined = fixture.run_confined(make());
        if confined != Ok(Report::Outcome(case.confined)) {
            failures.push(format!(
                "{}: confined expected {:?}, got {:?}",
                case.rule,
                Report::Outcome(case.confined),
                confined
            ));
        }
    }

    for case in &table.path {
        let path = fixture.layout.resolve(&case.path);
        let make = || Probe::Path {
            access: case.access,
            path: path.clone(),
        };
        let unconfined = fixture.run_unconfined(make());
        if unconfined != Ok(Report::Outcome(case.unconfined)) {
            failures.push(format!(
                "{}: unconfined expected {:?}, got {:?}",
                case.rule,
                Report::Outcome(case.unconfined),
                unconfined
            ));
            continue;
        }
        let confined = fixture.run_confined(make());
        if confined != Ok(Report::Outcome(case.appcontainer)) {
            failures.push(format!(
                "{}: confined expected {:?}, got {:?}",
                case.rule,
                Report::Outcome(case.appcontainer),
                confined
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Windows AppContainer probes at `none` failed:\n{}",
        failures.join("\n")
    );
}

/// Every Windows probe table (`none`, `proxy_tcp`, `direct`) loads cleanly and
/// specifies exact `EACCES` outcomes for every refused operation and path
/// (ADR 0067, issue 879).
///
/// Covers: story:7-24
#[test]
fn windows_proxy_tcp_and_direct_probe_tables_specify_broker_filter_outcomes() {
    for level in [NetworkLevel::ProxyTcp, NetworkLevel::Direct] {
        let table = WindowsProbeTable::read(committed_probes(level).unwrap()).unwrap();
        let expected_rules = goodfetch_confinement::windows::rules(level).unwrap();
        let denied: Vec<_> =
            goodfetch_confinement::deny_list::on(goodfetch_confinement::deny_list::Host::Windows)
                .collect();
        let mismatches =
            goodfetch_confinement::windows::check_probes(&table, &expected_rules, &denied);
        assert!(
            mismatches.is_empty(),
            "probe table mismatches at {level:?}: {mismatches:?}"
        );
    }
}
