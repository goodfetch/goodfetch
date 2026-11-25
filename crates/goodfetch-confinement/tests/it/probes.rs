//! Linux's probes at the `none` network level: every case of the committed
//! table, made by `goodfetch-confinement-probe` unconfined (the positive
//! control), then confined to a Plugin's policy, after the mechanism is
//! asserted against the kernel's own answer (`docs/testing/security.md`,
//! Confinement).
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

use goodfetch_confinement::deny_list::Host;
use goodfetch_confinement::linux::Mechanism;
use goodfetch_confinement::linux::probe::{Fault, Probe, Report, Request};
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::{self, Access, Layout, ProbeTable};
use goodfetch_protocol::plugin::{NetworkLevel, PluginManifest};
use serde_json::json;
use tempfile::TempDir;

const PROBE: &str = env!("CARGO_BIN_EXE_goodfetch-confinement-probe");

/// The Landlock ABI the applier needs, from which it scopes signals: below
/// it, it refuses to confine.
const SIGNAL_SCOPE_ABI: u8 = 6;

/// What `probe` reports, run confined to `policy` when there is one; an
/// error for a probe that ended without a report, killed by a signal.
fn run(probe: Probe, policy: Option<&Policy>) -> Result<Report, String> {
    let request = Request {
        probe,
        policy: policy.cloned(),
    };
    let status = Command::new(PROBE)
        .arg(serde_json::to_string(&request).unwrap())
        .stdin(Stdio::null())
        .status()
        .unwrap();
    let code = status
        .code()
        .ok_or_else(|| format!("it ended without a report: {status}"))?;
    u8::try_from(code)
        .ok()
        .and_then(Report::of_status)
        .ok_or_else(|| format!("its status {code} is no report"))
}

/// A Plugin's folders under a fresh root, with every file the table's
/// probes read and the folders its probes write in, and the policy of an
/// instance of the Plugin at `level`.
struct Plugin {
    _root: TempDir,
    layout: Layout,
    policy: Policy,
}

impl Plugin {
    fn new(table: &ProbeTable, level: NetworkLevel) -> Self {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::under(&fs::canonicalize(root.path()).unwrap());
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
            "kinds": ["proxy"],
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
        Self {
            _root: root,
            layout,
            policy,
        }
    }
}

/// Covers: story:7-54
#[test]
fn every_probe_at_the_none_level_gives_its_outcome_unconfined_then_exactly_its_refusal_confined() {
    let table =
        ProbeTable::read(probes::committed(Host::Linux, NetworkLevel::None).unwrap()).unwrap();
    let plugin = Plugin::new(&table, NetworkLevel::None);

    // The mechanism, against the kernel's answer to the raw call: Landlock
    // where it scopes signals, a refusal to confine elsewhere.
    let Ok(Report::KernelAbi(kernel)) = run(Probe::KernelAbi, None) else {
        panic!("the probe did not report the kernel's Landlock ABI");
    };
    let mechanism = run(Probe::Mechanism, Some(&plugin.policy)).unwrap();
    let confines = kernel >= SIGNAL_SCOPE_ABI;
    if confines {
        assert!(
            matches!(mechanism, Report::Mechanism(Mechanism::Landlock { abi }) if (SIGNAL_SCOPE_ABI..=kernel).contains(&abi)),
            "the kernel gives Landlock ABI {kernel}, and the probe was confined by {mechanism:?}"
        );
    } else {
        assert_eq!(
            mechanism,
            Report::Fault(Fault::Unavailable),
            "the kernel gives Landlock ABI {kernel}, so confining must be refused"
        );
    }
    let refused = Report::Fault(Fault::Unavailable);

    let mut failures = Vec::new();
    let mut check =
        |rule: &str, phase: &str, found: Result<Report, String>, expected: Report| match found {
            Ok(found) if found == expected => {}
            Ok(found) => failures.push(format!("{rule}, {phase}: {found:?}, not {expected:?}")),
            Err(error) => failures.push(format!("{rule}, {phase}: {error}")),
        };
    for case in &table.call {
        let probe = || Probe::Call {
            call: case.call,
            args: case.args.clone(),
        };
        check(
            &case.rule,
            "unconfined",
            run(probe(), None),
            Report::Outcome(case.unconfined),
        );
        check(
            &case.rule,
            "confined",
            run(probe(), Some(&plugin.policy)),
            if confines {
                Report::Outcome(case.confined)
            } else {
                refused
            },
        );
    }
    for case in &table.path {
        let probe = || Probe::Path {
            access: case.access,
            path: plugin.layout.resolve(&case.path),
        };
        check(
            &case.rule,
            "unconfined",
            run(probe(), None),
            Report::Outcome(case.unconfined),
        );
        check(
            &case.rule,
            "confined",
            run(probe(), Some(&plugin.policy)),
            if confines {
                Report::Outcome(case.landlock)
            } else {
                refused
            },
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
