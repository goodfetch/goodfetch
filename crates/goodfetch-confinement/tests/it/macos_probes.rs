//! macOS Seatbelt probes (Release, macOS; ADR 0067, issue 867): every case of
//! `probes/macos/{none,proxy_tcp,direct}.toml`, run through
//! `goodfetch-confinement-probe` unconfined (positive control), then confined
//! under Seatbelt, asserting both the exact error (`EPERM`) and the active
//! mechanism (`Mechanism::Seatbelt`, verified by
//! `sandbox_check(pid, NULL, SANDBOX_FILTER_NONE) == 1`).
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
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use goodfetch_confinement::macos::probe::{Probe, Report, Request};
use goodfetch_confinement::macos::{
    MacOsProbeTable, Mechanism, SeatbeltOperation, committed_probes,
};
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::{Access, Layout, Outcome};
use goodfetch_protocol::plugin::{NetworkLevel, PluginManifest};
use serde_json::json;
use tempfile::TempDir;

const PROBE: &str = env!("CARGO_BIN_EXE_goodfetch-confinement-probe");

struct Fixture {
    _root: TempDir,
    layout: Layout,
    policy: Policy,
    proxy_listener: TcpListener,
    other_listener: TcpListener,
    unix_path: PathBuf,
    _unix_listener: UnixListener,
}

impl Fixture {
    fn new(table: &MacOsProbeTable, level: NetworkLevel) -> Self {
        let root = tempfile::tempdir().unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let layout = Layout::under(&canonical_root);
        for folder in [&layout.bundle, &layout.runtime, &layout.instance] {
            fs::create_dir_all(folder).unwrap();
        }
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
            "entrypoint": "index.js",
            "network_level": level.as_str(),
        });
        let manifest = PluginManifest::read(manifest.to_string().as_bytes()).unwrap();
        let instance = Instance {
            bundle: layout.bundle.clone(),
            runtime: Some(layout.runtime.clone()),
            folders: vec![layout.instance.clone()],
            ..Instance::default()
        };
        let policy = Policy::of(&manifest, &instance, &layout.places).unwrap();
        let proxy_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let other_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let unix_path = canonical_root.join("probe.sock");
        let unix_listener = UnixListener::bind(&unix_path).unwrap();
        Self {
            _root: root,
            layout,
            policy,
            proxy_listener,
            other_listener,
            unix_path,
            _unix_listener: unix_listener,
        }
    }

    fn proxy_port(&self) -> u16 {
        self.proxy_listener.local_addr().unwrap().port()
    }

    fn other_port(&self) -> u16 {
        self.other_listener.local_addr().unwrap().port()
    }
}

fn run(probe: Probe, policy: Option<&Policy>, proxy_port: Option<u16>) -> Result<Report, String> {
    let request = Request {
        probe,
        policy: policy.cloned(),
        proxy_port,
    };
    let status = Command::new(PROBE)
        .arg(serde_json::to_string(&request).unwrap())
        .stdin(Stdio::null())
        .status()
        .unwrap();
    let code = status
        .code()
        .ok_or_else(|| format!("ended without a report: {status}"))?;
    u8::try_from(code)
        .ok()
        .and_then(Report::of_status)
        .ok_or_else(|| format!("status {code} is no report"))
}

fn assert_every_probe_at(level: NetworkLevel) {
    let table = MacOsProbeTable::read(committed_probes(level)).unwrap();
    let fixture = Fixture::new(&table, level);
    let proxy_port = (level == NetworkLevel::ProxyTcp).then(|| fixture.proxy_port());

    let mechanism = run(Probe::Mechanism, Some(&fixture.policy), proxy_port).unwrap();
    assert_eq!(
        mechanism,
        Report::Mechanism(Mechanism::Seatbelt),
        "expected Seatbelt mechanism for {level:?}"
    );

    let mut failures = Vec::new();
    for case in &table.operation {
        let port = match case.operation {
            SeatbeltOperation::NetworkOutboundProxy => Some(fixture.proxy_port()),
            SeatbeltOperation::NetworkOutboundIp => Some(fixture.other_port()),
            _ => None,
        };
        let unix_path = (case.operation == SeatbeltOperation::NetworkUnixSocket)
            .then(|| fixture.unix_path.clone());
        let make = || Probe::Operation {
            operation: case.operation,
            port,
            unix_path: unix_path.clone(),
        };
        let unconfined = run(make(), None, None);
        if unconfined != Ok(Report::Outcome(case.unconfined)) {
            failures.push(format!(
                "{}: unconfined expected {:?}, got {:?}",
                case.rule,
                Report::Outcome(case.unconfined),
                unconfined
            ));
            continue;
        }
        let confined = run(make(), Some(&fixture.policy), proxy_port);
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
        let unconfined = run(make(), None, None);
        if unconfined != Ok(Report::Outcome(case.unconfined)) {
            failures.push(format!(
                "{}: unconfined expected {:?}, got {:?}",
                case.rule,
                Report::Outcome(case.unconfined),
                unconfined
            ));
            continue;
        }
        let confined = run(make(), Some(&fixture.policy), proxy_port);
        if confined != Ok(Report::Outcome(case.seatbelt)) {
            failures.push(format!(
                "{}: confined expected {:?}, got {:?}",
                case.rule,
                Report::Outcome(case.seatbelt),
                confined
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "macOS Seatbelt probes at {level:?} failed:\n{}",
        failures.join("\n")
    );
}

/// Covers: story:7-22, story:7-54
#[test]
fn every_macos_probe_at_none_passes_unconfined_then_refuses_under_seatbelt() {
    assert_every_probe_at(NetworkLevel::None);
}

/// Covers: story:7-22, story:7-54
#[test]
fn every_macos_probe_at_proxy_tcp_passes_unconfined_then_enforces_seatbelt() {
    assert_every_probe_at(NetworkLevel::ProxyTcp);
}

/// Covers: story:7-22, story:7-54
#[test]
fn every_macos_probe_at_direct_passes_unconfined_then_enforces_seatbelt() {
    assert_every_probe_at(NetworkLevel::Direct);
}
