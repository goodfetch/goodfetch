//! Distinguishing a Group member's restart from its removal on drain (ADR 0055,
//! issue 245):
//! - On VMs, `group leave` is a removal (`DrainMode::Removal`), while a bare
//!   `SIGTERM` drain is a restart (`DrainMode::Restart`).
//! - In the Helm chart's Kubernetes `StatefulSet`, a pod checks its ordinal
//!   against the `StatefulSet`'s desired replicas: an ordinal at or above
//!   `replicas` is a scale-down removal (`DrainMode::Removal`), while a lower
//!   ordinal (a rolling restart or reschedule) is a restart
//!   (`DrainMode::Restart`).

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};

/// Environment variable carrying the Kubernetes `StatefulSet` pod name (e.g.
/// `goodfetch-2`), injected by the chart's Downward API.
pub const POD_NAME_ENV: &str = "GOODFETCH_K8S_POD_NAME";

/// Environment variable carrying the Kubernetes `StatefulSet`'s desired replica
/// count when observed by the chart's preStop / controller hook or test
/// harness.
pub const STATEFULSET_REPLICAS_ENV: &str = "GOODFETCH_K8S_STATEFULSET_REPLICAS";

/// Whether a draining Group member is restarting or being removed from the
/// Daemon group (ADR 0055).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrainMode {
    /// The Group member is restarting (e.g. rolling upgrade or VM reboot): it
    /// checkpoints its live Runs and leaves them assigned to itself so it
    /// resumes them when it starts again.
    Restart,
    /// The Group member is being removed (e.g. `group leave` on VMs or
    /// `StatefulSet` scale-down where `pod_ordinal >= desired_replicas`): it
    /// checkpoints its live Runs and hands each eligible Run's Checkpoint over
    /// the peer link to the Group member the Lead Daemon places it on.
    Removal,
}

/// Extracts the trailing numeric pod ordinal from a Kubernetes `StatefulSet`
/// pod name such as `goodfetch-0` or `goodfetch-daemon-2`.
#[must_use]
pub fn pod_ordinal(pod_name: &str) -> Option<u32> {
    let (_, suffix) = pod_name.rsplit_once('-')?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    suffix.parse::<u32>().ok()
}

/// Classifies a Group member's drain as a [`DrainMode::Restart`] or
/// [`DrainMode::Removal`] (ADR 0055):
/// - `is_group_leave == true` always means [`DrainMode::Removal`] (`group leave`
///   on VMs).
/// - Otherwise, when both a `StatefulSet` `pod_ordinal` and `desired_replicas`
///   are known, `pod_ordinal >= desired_replicas` means [`DrainMode::Removal`].
/// - Otherwise, it is [`DrainMode::Restart`].
#[must_use]
pub fn classify_drain(
    is_group_leave: bool,
    pod_ordinal: Option<u32>,
    desired_replicas: Option<u32>,
) -> DrainMode {
    if is_group_leave {
        return DrainMode::Removal;
    }
    if let (Some(ordinal), Some(replicas)) = (pod_ordinal, desired_replicas)
        && ordinal >= replicas
    {
        return DrainMode::Removal;
    }
    DrainMode::Restart
}

/// Classifies a `SIGTERM` drain from the Daemon's process environment map.
#[must_use]
pub fn classify_sigterm_drain(environment: &BTreeMap<OsString, OsString>) -> DrainMode {
    let ordinal = environment
        .get(OsStr::new(POD_NAME_ENV))
        .and_then(|name| name.to_str())
        .and_then(pod_ordinal);
    let replicas = environment
        .get(OsStr::new(STATEFULSET_REPLICAS_ENV))
        .and_then(|raw| raw.to_str())
        .and_then(|raw| raw.trim().parse::<u32>().ok());
    classify_drain(false, ordinal, replicas)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    use super::{
        DrainMode, POD_NAME_ENV, STATEFULSET_REPLICAS_ENV, classify_drain, classify_sigterm_drain,
        pod_ordinal,
    };

    /// `group leave` always classifies as a removal; a `SIGTERM` on a VM
    /// without `StatefulSet` metadata classifies as a restart; in a `StatefulSet`,
    /// `pod_ordinal >= desired_replicas` classifies as removal and
    /// `pod_ordinal < desired_replicas` as restart.
    ///
    /// Covers: story:3-111
    #[test]
    fn drain_distinguishes_restart_from_removal() {
        assert_eq!(classify_drain(true, None, None), DrainMode::Removal);
        assert_eq!(classify_drain(true, Some(0), Some(3)), DrainMode::Removal);
        assert_eq!(classify_drain(false, None, None), DrainMode::Restart);
        assert_eq!(classify_drain(false, Some(1), Some(3)), DrainMode::Restart);
        assert_eq!(classify_drain(false, Some(2), Some(3)), DrainMode::Restart);
        assert_eq!(classify_drain(false, Some(2), Some(2)), DrainMode::Removal);
        assert_eq!(classify_drain(false, Some(3), Some(2)), DrainMode::Removal);

        assert_eq!(pod_ordinal("goodfetch-0"), Some(0));
        assert_eq!(pod_ordinal("goodfetch-daemon-12"), Some(12));
        assert_eq!(pod_ordinal("goodfetch"), None);
        assert_eq!(pod_ordinal("goodfetch-abc"), None);

        let vm_env = BTreeMap::new();
        assert_eq!(classify_sigterm_drain(&vm_env), DrainMode::Restart);

        let mut k8s_restart = BTreeMap::new();
        k8s_restart.insert(OsString::from(POD_NAME_ENV), OsString::from("goodfetch-1"));
        k8s_restart.insert(
            OsString::from(STATEFULSET_REPLICAS_ENV),
            OsString::from("3"),
        );
        assert_eq!(classify_sigterm_drain(&k8s_restart), DrainMode::Restart);

        let mut k8s_removal = BTreeMap::new();
        k8s_removal.insert(OsString::from(POD_NAME_ENV), OsString::from("goodfetch-2"));
        k8s_removal.insert(
            OsString::from(STATEFULSET_REPLICAS_ENV),
            OsString::from("2"),
        );
        assert_eq!(classify_sigterm_drain(&k8s_removal), DrainMode::Removal);
    }
}
