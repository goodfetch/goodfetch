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
use goodfetch_confinement::linux::probe::{Fault, Probe, Report, Request};
use goodfetch_confinement::linux::{Mechanism, SignalScope};
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::{self, Access, Layout, Outcome, ProbeTable};
use goodfetch_protocol::plugin::{NetworkLevel, PluginManifest};
use serde_json::json;
use tempfile::TempDir;

const PROBE: &str = env!("CARGO_BIN_EXE_goodfetch-confinement-probe");

/// The Landlock ABI from which Landlock scopes signals: below it, a PID
/// namespace does.
const SIGNAL_SCOPE_ABI: u8 = 6;

/// The mechanism a runner declares it must confine with by default, so that
/// a host that refuses it fails rather than passing on the refusal (ADR
/// 0067): `landlock` or `pid_namespace`.
const DECLARED_MECHANISM: &str = "GOODFETCH_TEST_CONFINEMENT";

/// The policy to confine a probe to, with the signal scope to use, the
/// kernel's when none.
#[derive(Clone, Copy)]
struct Confined<'a> {
    policy: &'a Policy,
    scope: Option<SignalScope>,
}

/// What `probe` reports, run confined when asked; an error for a probe
/// that ended without a report, killed by a signal.
fn run(probe: Probe, confined: Option<Confined<'_>>) -> Result<Report, String> {
    let request = Request {
        probe,
        policy: confined.map(|confined| confined.policy.clone()),
        scope: confined.and_then(|confined| confined.scope),
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
    assert_every_probe_at_none(None);
}

/// The PID-namespace mechanism, which a kernel before Landlock ABI 6 gets,
/// made on any kernel.
///
/// Covers: story:7-54
#[test]
fn every_probe_at_the_none_level_gives_its_refusal_under_the_pid_namespace_mechanism_too() {
    assert_every_probe_at_none(Some(SignalScope::PidNamespace));
}

/// Makes every case of the `none` table unconfined, then confined with
/// `scope` (the kernel's when none), after asserting the mechanism against
/// the kernel's own answers to the raw calls: Landlock where it scopes
/// signals, a PID namespace where user namespaces are allowed, a refusal to
/// confine elsewhere.
fn assert_every_probe_at_none(scope: Option<SignalScope>) {
    let table =
        ProbeTable::read(probes::committed(Host::Linux, NetworkLevel::None).unwrap()).unwrap();
    let plugin = Plugin::new(&table, NetworkLevel::None);
    let confined = Confined {
        policy: &plugin.policy,
        scope,
    };
    let (used, enforced) = assert_mechanism(confined);
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
            run(probe(), Some(confined)),
            match (enforced, used) {
                (false, _) => refused,
                (true, SignalScope::Landlock) => Report::Outcome(case.confined),
                (true, SignalScope::PidNamespace) => Report::Outcome(case.in_pid_namespace()),
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
            // Landlock keeps the files under both mechanisms.
            run(probe(), Some(confined)),
            if enforced {
                Report::Outcome(case.landlock)
            } else {
                refused
            },
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Asserts the mechanism `confined` gets against the kernel's own answers,
/// and the one this runner declares; gives the signal scope used and
/// whether it is enforced here.
fn assert_mechanism(confined: Confined<'_>) -> (SignalScope, bool) {
    let scope = confined.scope;
    let Ok(Report::KernelAbi(kernel)) = run(Probe::KernelAbi, None) else {
        panic!("the probe did not report the kernel's Landlock ABI");
    };
    let user_namespace = run(Probe::UserNamespace, None);
    let used = scope.unwrap_or(if kernel >= SIGNAL_SCOPE_ABI {
        SignalScope::Landlock
    } else {
        SignalScope::PidNamespace
    });
    let enforced = match used {
        SignalScope::Landlock => kernel >= SIGNAL_SCOPE_ABI,
        SignalScope::PidNamespace => {
            kernel > 0 && user_namespace == Ok(Report::Outcome(Outcome::Ok))
        }
    };
    let host =
        format!("the kernel gives Landlock ABI {kernel} and a user namespace {user_namespace:?}");
    let mechanism = run(Probe::Mechanism, Some(confined)).unwrap();
    match (enforced, used, mechanism) {
        (false, _, found) => assert_eq!(
            found,
            Report::Fault(Fault::Unavailable),
            "{host}, so confining with {used:?} must be refused"
        ),
        (true, SignalScope::Landlock, Report::Mechanism(Mechanism::Landlock { abi }))
            if (SIGNAL_SCOPE_ABI..=kernel).contains(&abi) => {}
        (true, SignalScope::PidNamespace, Report::Mechanism(Mechanism::PidNamespace { abi }))
            if abi == kernel.min(SIGNAL_SCOPE_ABI - 1) => {}
        (true, _, found) => panic!("{host}, and the probe was confined by {found:?} with {used:?}"),
    }
    if scope.is_none()
        && let Ok(declared) = std::env::var(DECLARED_MECHANISM)
    {
        let kind = match mechanism {
            Report::Mechanism(Mechanism::Landlock { .. }) => "landlock",
            Report::Mechanism(Mechanism::PidNamespace { .. }) => "pid_namespace",
            _ => "none",
        };
        assert_eq!(
            kind, declared,
            "this runner declares {DECLARED_MECHANISM}={declared}, and {host}"
        );
    }
    (used, enforced)
}
