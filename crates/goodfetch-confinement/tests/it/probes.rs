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
use goodfetch_confinement::host::SIGNAL_SCOPE_ABI;
use goodfetch_confinement::linux::probe::{Fault, Probe, Report, Request};
use goodfetch_confinement::linux::{Mechanism, MechanismKind};
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::{self, Access, Layout, Outcome, ProbeTable};
use goodfetch_protocol::plugin::{NetworkLevel, PluginManifest};
use serde_json::json;
use tempfile::TempDir;

const PROBE: &str = env!("CARGO_BIN_EXE_goodfetch-confinement-probe");

/// The mechanism a runner declares it must confine with by default, so that
/// a host that refuses it fails rather than passing on the refusal (ADR
/// 0067): `landlock`, `pid_namespace` or `mount_namespace`.
const DECLARED_MECHANISM: &str = "GOODFETCH_TEST_CONFINEMENT";

/// The policy to confine a probe to, with the mechanism to use, the
/// kernel's when none.
#[derive(Clone, Copy)]
struct Confined<'a> {
    policy: &'a Policy,
    kind: Option<MechanismKind>,
}

/// What `probe` reports, run confined when asked; an error for a probe
/// that ended without a report, killed by a signal.
fn run(probe: Probe, confined: Option<Confined<'_>>) -> Result<Report, String> {
    let request = Request {
        probe,
        policy: confined.map(|confined| confined.policy.clone()),
        mechanism: confined.and_then(|confined| confined.kind),
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
    assert_every_probe_at_none(Some(MechanismKind::PidNamespace));
}

/// The mount-namespace mechanism, which a kernel without Landlock gets,
/// made on any kernel.
///
/// Covers: story:7-54
#[test]
fn every_probe_at_the_none_level_gives_its_refusal_under_the_mount_namespace_mechanism_too() {
    assert_every_probe_at_none(Some(MechanismKind::MountNamespace));
}

/// The engine's startup probe reads this host as the launcher finds it: it
/// says a Plugin can be confined exactly where the kernel's mechanism
/// confines one, and refuses it where the launcher would answer
/// `Unavailable` (ADR 0049).
/// Covers: story:7-27
#[test]
fn the_startup_probe_says_a_plugin_is_confined_exactly_where_the_launcher_confines_one() {
    let table =
        ProbeTable::read(probes::committed(Host::Linux, NetworkLevel::None).unwrap()).unwrap();
    let plugin = Plugin::new(&table, NetworkLevel::None);
    let host = goodfetch_confinement::linux::host::read();

    let mechanism = run(
        Probe::Mechanism,
        Some(Confined {
            policy: &plugin.policy,
            kind: None,
        }),
    )
    .unwrap();

    assert_eq!(
        host.confines(),
        matches!(mechanism, Report::Mechanism(_)),
        "the probe read {host:?}, and the launcher answered {mechanism:?}"
    );
}

/// Makes every case of the `none` table unconfined, then confined with
/// `kind` (the kernel's when none), after asserting the mechanism against
/// the kernel's own answers to the raw calls: Landlock where it scopes
/// signals, a namespace where user namespaces are allowed, a refusal to
/// confine elsewhere.
fn assert_every_probe_at_none(kind: Option<MechanismKind>) {
    let table =
        ProbeTable::read(probes::committed(Host::Linux, NetworkLevel::None).unwrap()).unwrap();
    let plugin = Plugin::new(&table, NetworkLevel::None);
    let confined = Confined {
        policy: &plugin.policy,
        kind,
    };
    let (used, enforced) = assert_mechanism(confined);
    let refused = Report::Fault(Fault::Unavailable);

    let mut failures = Vec::new();
    for case in &table.call {
        let probe = || Probe::Call {
            call: case.call,
            args: case.args.clone(),
        };
        let expected_confined = match (enforced, used) {
            (false, _) => refused,
            (true, MechanismKind::Landlock) => Report::Outcome(case.confined),
            (true, MechanismKind::PidNamespace | MechanismKind::MountNamespace) => {
                Report::Outcome(case.in_pid_namespace())
            }
        };
        if let Err(failure) = run_probe_case(
            &case.rule,
            Report::Outcome(case.unconfined),
            expected_confined,
            || run(probe(), None),
            || run(probe(), Some(confined)),
        ) {
            failures.push(failure);
        }
    }
    for case in &table.path {
        let probe = || Probe::Path {
            access: case.access,
            path: plugin.layout.resolve(&case.path),
        };
        let expected_confined = match (enforced, used) {
            (false, _) => refused,
            (true, MechanismKind::MountNamespace) => Report::Outcome(case.fallback),
            (true, _) => Report::Outcome(case.landlock),
        };
        if let Err(failure) = run_probe_case(
            &case.rule,
            Report::Outcome(case.unconfined),
            expected_confined,
            || run(probe(), None),
            || run(probe(), Some(confined)),
        ) {
            failures.push(failure);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Runs a probe's unconfined positive control first and rejects the probe
/// immediately on a positive-control mismatch before running the confined
/// phase, so a confined refusal never masks a broken control (ADR 0067).
fn run_probe_case(
    rule: &str,
    expected_unconfined: Report,
    expected_confined: Report,
    mut run_unconfined: impl FnMut() -> Result<Report, String>,
    mut run_confined: impl FnMut() -> Result<Report, String>,
) -> Result<(), String> {
    match run_unconfined() {
        Ok(found) if found == expected_unconfined => {}
        Ok(found) => {
            return Err(format!(
                "{rule}, positive-control unconfined: {found:?}, not {expected_unconfined:?}"
            ));
        }
        Err(error) => return Err(format!("{rule}, positive-control unconfined: {error}")),
    }
    match run_confined() {
        Ok(found) if found == expected_confined => Ok(()),
        Ok(found) => Err(format!(
            "{rule}, confined: {found:?}, not {expected_confined:?}"
        )),
        Err(error) => Err(format!("{rule}, confined: {error}")),
    }
}

#[test]
fn a_probe_with_a_failing_positive_control_is_rejected_even_when_confined_refuses() {
    let table =
        ProbeTable::read(probes::committed(Host::Linux, NetworkLevel::None).unwrap()).unwrap();
    let ptrace = table
        .call
        .iter()
        .find(|case| case.rule == "ptrace")
        .unwrap()
        .clone();
    let broken_control = goodfetch_confinement::probes::CallCase {
        // Real unconfined `ptrace` returns ESRCH; expecting `ok` makes the positive control fail.
        unconfined: Outcome::Ok,
        ..ptrace
    };
    let plugin = Plugin::new(&table, NetworkLevel::None);
    let confined = Confined {
        policy: &plugin.policy,
        kind: None,
    };
    let probe = || Probe::Call {
        call: broken_control.call,
        args: broken_control.args.clone(),
    };
    let mut confined_ran = false;
    let result = run_probe_case(
        &broken_control.rule,
        Report::Outcome(broken_control.unconfined),
        Report::Outcome(broken_control.confined),
        || run(probe(), None),
        || {
            confined_ran = true;
            run(probe(), Some(confined))
        },
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("positive-control unconfined"),
        "expected positive-control failure, got: {err}"
    );
    assert!(
        !confined_ran,
        "confined phase must not run after a failing positive control"
    );
}

/// Asserts the mechanism `confined` gets against the kernel's own answers,
/// and the one this runner declares; gives the mechanism used and whether
/// it is enforced here.
fn assert_mechanism(confined: Confined<'_>) -> (MechanismKind, bool) {
    let Ok(Report::KernelAbi(kernel)) = run(Probe::KernelAbi, None) else {
        panic!("the probe did not report the kernel's Landlock ABI");
    };
    let user_namespace = run(Probe::UserNamespace, None);
    let namespaces = user_namespace == Ok(Report::Outcome(Outcome::Ok));
    let used = confined.kind.unwrap_or(match kernel {
        0 => MechanismKind::MountNamespace,
        abi if abi < SIGNAL_SCOPE_ABI => MechanismKind::PidNamespace,
        _ => MechanismKind::Landlock,
    });
    let enforced = match used {
        MechanismKind::Landlock => kernel >= SIGNAL_SCOPE_ABI,
        MechanismKind::PidNamespace => kernel > 0 && namespaces,
        MechanismKind::MountNamespace => namespaces,
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
        (true, MechanismKind::Landlock, Report::Mechanism(Mechanism::Landlock { abi }))
            if (SIGNAL_SCOPE_ABI..=kernel).contains(&abi) => {}
        (true, MechanismKind::PidNamespace, Report::Mechanism(Mechanism::PidNamespace { abi }))
            if abi == kernel.min(SIGNAL_SCOPE_ABI - 1) => {}
        (true, MechanismKind::MountNamespace, Report::Mechanism(Mechanism::MountNamespace)) => {}
        (true, _, found) => panic!("{host}, and the probe was confined by {found:?} with {used:?}"),
    }
    if confined.kind.is_none()
        && let Ok(declared) = std::env::var(DECLARED_MECHANISM)
    {
        let kind = match mechanism {
            Report::Mechanism(Mechanism::Landlock { .. }) => "landlock",
            Report::Mechanism(Mechanism::PidNamespace { .. }) => "pid_namespace",
            Report::Mechanism(Mechanism::MountNamespace) => "mount_namespace",
            _ => "none",
        };
        assert_eq!(
            kind, declared,
            "this runner declares {DECLARED_MECHANISM}={declared}, and {host}"
        );
    }
    (used, enforced)
}
