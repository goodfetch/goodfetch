//! Integration tests for `goodfetch-filter-broker` (ADRs 0033, 0039, 0059,
//! 0067, 0072; issue 879):
//!
//! Covers every broker lifecycle invariant and event-log requirement:
//! - Caller verification against Goodfetch release attestation under a test
//!   trust root (refusing forged/unattested/wrong-signer/wrong-digest callers).
//! - Persistent filters surviving a broker crash (simulated by dropping the
//!   broker while the OS persistent WFP store retains every filter).
//! - Refusing (`Block`) filters installed before permitting (`Permit`) filters
//!   and before the instance starts.
//! - Instance filters removed ONLY once its Job Object reports zero active
//!   processes (refusing removal while any process remains active).
//! - Exiting engine cleanup: broker terminates ONLY that exiting engine's
//!   instances' Job Objects and removes their filters, leaving other engines
//!   (Daemon vs Attached) untouched.
//! - Coordinated service stop: broker notifies every connected engine so each
//!   stops dependent instances under the crash rules before the service stops.
//! - Startup orphan reconciliation: removes every persistent filter whose
//!   `AppContainer` has no live instance while preserving filters for still-
//!   live `AppContainer`s.
//! - Uninstall: removes EVERY persistent WFP filter plus sublayer & provider.
//! - Every filter change is recorded in the Windows Event Log.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap"
)]
#![expect(
    clippy::disallowed_types,
    reason = "integration tests use std::sync::Mutex for single-process in-memory test state outside loom"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use goodfetch_confinement::attestation::{
    AttestationSigner, AttestationTrustRoot, release_signer, sha256_digest,
};
use goodfetch_confinement::windows::broker::{
    AllowedDestination, BrokerRequest, BrokerResponse, FilterAction, WfpRule, compile_rules,
};
use goodfetch_filter_broker::broker::{
    BackendError, CallerVerifier, EventLogSink, FilterBroker, InstalledFilter, JobController,
    WfpStore,
};
use goodfetch_filter_broker::events::{BrokerEvent, RemovalReason};
use goodfetch_protocol::plugin::NetworkLevel;
use goodfetch_protocol::plugin::index::{
    ProvenanceAttestation, ProvenanceStatement, SignerIdentity,
};

/// Persistent OS WFP store shared across broker instances so dropping one
/// `FilterBroker` (a broker crash) leaves every persistent filter intact.
#[derive(Clone, Default)]
struct SharedPersistentWfp {
    inner: Arc<Mutex<PersistentWfpState>>,
}

#[derive(Default)]
struct PersistentWfpState {
    provider_registered: bool,
    next_id: u64,
    filters: BTreeMap<u64, (String, WfpRule)>,
    add_order: Vec<FilterAction>,
    loopback_exemptions: BTreeSet<String>,
}

impl SharedPersistentWfp {
    fn filter_count(&self) -> usize {
        self.inner.lock().unwrap().filters.len()
    }

    fn contains_filter(&self, id: u64) -> bool {
        self.inner.lock().unwrap().filters.contains_key(&id)
    }

    fn add_order(&self) -> Vec<FilterAction> {
        self.inner.lock().unwrap().add_order.clone()
    }

    fn has_loopback_exemption(&self, app_container: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .loopback_exemptions
            .contains(app_container)
    }

    fn provider_registered(&self) -> bool {
        self.inner.lock().unwrap().provider_registered
    }
}

impl WfpStore for SharedPersistentWfp {
    fn ensure_provider_and_sublayer(&mut self) -> Result<(), BackendError> {
        self.inner.lock().unwrap().provider_registered = true;
        Ok(())
    }

    fn add_persistent_filter(
        &mut self,
        app_container_name: &str,
        rule: &WfpRule,
    ) -> Result<u64, BackendError> {
        let mut state = self.inner.lock().unwrap();
        state.next_id += 1;
        let id = state.next_id;
        state.add_order.push(rule.action);
        state
            .filters
            .insert(id, (app_container_name.to_owned(), rule.clone()));
        Ok(id)
    }

    fn delete_filter(&mut self, filter_id: u64) -> Result<(), BackendError> {
        self.inner.lock().unwrap().filters.remove(&filter_id);
        Ok(())
    }

    fn list_owned_filters(&self) -> Result<Vec<InstalledFilter>, BackendError> {
        let state = self.inner.lock().unwrap();
        Ok(state
            .filters
            .iter()
            .map(|(&filter_id, (_, rule))| InstalledFilter {
                filter_id,
                tag: rule.tag.clone(),
            })
            .collect())
    }

    fn set_loopback_exemption(
        &mut self,
        app_container_name: &str,
        enabled: bool,
    ) -> Result<(), BackendError> {
        let mut state = self.inner.lock().unwrap();
        if enabled {
            state
                .loopback_exemptions
                .insert(app_container_name.to_owned());
        } else {
            state.loopback_exemptions.remove(app_container_name);
        }
        Ok(())
    }

    fn remove_provider_and_sublayer(&mut self) -> Result<(), BackendError> {
        self.inner.lock().unwrap().provider_registered = false;
        Ok(())
    }
}

#[derive(Clone, Default)]
struct SharedJobController {
    inner: Arc<Mutex<JobState>>,
}

#[derive(Default)]
struct JobState {
    next_token: u64,
    /// Active process count per broker job token; when `stuck` is true,
    /// `terminate_job` leaves processes active to prove filters are never
    /// removed until `active_processes == 0`.
    active: BTreeMap<u64, u32>,
    stuck: BTreeSet<u64>,
    terminated: Vec<u64>,
    closed: Vec<u64>,
    by_caller: BTreeMap<(u32, u64), u64>,
    live_appcontainers: BTreeSet<String>,
}

impl SharedJobController {
    fn set_active_for_caller(&self, caller_pid: u32, job_handle: u64, count: u32) {
        let mut state = self.inner.lock().unwrap();
        if let Some(&token) = state.by_caller.get(&(caller_pid, job_handle)) {
            state.active.insert(token, count);
        }
    }

    fn set_stuck_for_caller(&self, caller_pid: u32, job_handle: u64, stuck: bool) {
        let mut state = self.inner.lock().unwrap();
        if let Some(&token) = state.by_caller.get(&(caller_pid, job_handle)) {
            if stuck {
                state.stuck.insert(token);
            } else {
                state.stuck.remove(&token);
            }
        }
    }

    fn was_terminated(&self, caller_pid: u32, job_handle: u64) -> bool {
        let state = self.inner.lock().unwrap();
        state
            .by_caller
            .get(&(caller_pid, job_handle))
            .is_some_and(|token| state.terminated.contains(token))
    }

    fn set_live_appcontainer(&self, name: &str, live: bool) {
        let mut state = self.inner.lock().unwrap();
        if live {
            state.live_appcontainers.insert(name.to_owned());
        } else {
            state.live_appcontainers.remove(name);
        }
    }
}

impl JobController for SharedJobController {
    fn attach_job(&mut self, caller_pid: u32, job_handle: u64) -> Result<u64, BackendError> {
        let mut state = self.inner.lock().unwrap();
        state.next_token += 1;
        let token = state.next_token;
        state.active.insert(token, 1);
        state.by_caller.insert((caller_pid, job_handle), token);
        Ok(token)
    }

    fn terminate_job(&mut self, broker_job: u64) -> Result<(), BackendError> {
        let mut state = self.inner.lock().unwrap();
        state.terminated.push(broker_job);
        if !state.stuck.contains(&broker_job) {
            state.active.insert(broker_job, 0);
        }
        Ok(())
    }

    fn active_processes(&self, broker_job: u64) -> Result<u32, BackendError> {
        Ok(*self
            .inner
            .lock()
            .unwrap()
            .active
            .get(&broker_job)
            .unwrap_or(&0))
    }

    fn close_job(&mut self, broker_job: u64) {
        self.inner.lock().unwrap().closed.push(broker_job);
    }

    fn has_live_appcontainer(&self, app_container_name: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .live_appcontainers
            .contains(app_container_name)
    }
}

#[derive(Clone, Default)]
struct MemoryEventLog {
    entries: Arc<Mutex<Vec<BrokerEvent>>>,
}

impl MemoryEventLog {
    fn snapshot(&self) -> Vec<BrokerEvent> {
        self.entries.lock().unwrap().clone()
    }
}

impl EventLogSink for MemoryEventLog {
    fn write(&mut self, event: BrokerEvent) {
        self.entries.lock().unwrap().push(event);
    }
}

#[derive(Clone, Default)]
struct PidDigestMap {
    digests: BTreeMap<u32, String>,
}

impl CallerVerifier for PidDigestMap {
    fn executable_digest(&self, caller_pid: u32) -> Result<String, BackendError> {
        self.digests
            .get(&caller_pid)
            .cloned()
            .ok_or_else(|| BackendError::Caller(format!("unknown pid {caller_pid}")))
    }
}

type TestBroker =
    FilterBroker<SharedPersistentWfp, SharedJobController, MemoryEventLog, PidDigestMap>;

struct Harness {
    wfp: SharedPersistentWfp,
    jobs: SharedJobController,
    events: MemoryEventLog,
    pids: PidDigestMap,
    trust: AttestationTrustRoot,
    att: ProvenanceAttestation,
}

impl Harness {
    fn new(seed: [u8; 32]) -> Self {
        let (signer, trust) = AttestationSigner::for_root("lifecycle-key", seed);
        let digest = sha256_digest(b"engine-binary");
        let att = signer
            .sign(ProvenanceStatement {
                subject_digest: digest.clone(),
                signer: release_signer(),
            })
            .unwrap();
        let mut pids = PidDigestMap::default();
        pids.digests.insert(2001, digest.clone());
        pids.digests.insert(2002, digest);
        Self {
            wfp: SharedPersistentWfp::default(),
            jobs: SharedJobController::default(),
            events: MemoryEventLog::default(),
            pids,
            trust,
            att,
        }
    }

    fn spawn_broker(&self) -> TestBroker {
        FilterBroker::new(
            self.wfp.clone(),
            self.jobs.clone(),
            self.events.clone(),
            self.pids.clone(),
            self.trust.clone(),
            release_signer(),
        )
    }

    fn auth(&self, broker: &mut TestBroker, pid: u32, engine_id: &str) {
        let resp = broker.handle_request(
            pid,
            None,
            BrokerRequest::Authenticate {
                engine_id: engine_id.to_owned(),
                attestation: self.att.clone(),
            },
        );
        assert_eq!(
            resp,
            BrokerResponse::Authenticated {
                engine_id: engine_id.to_owned()
            }
        );
    }
}

fn prepare_proxy(
    broker: &mut TestBroker,
    pid: u32,
    engine_id: &str,
    instance_id: &str,
    container: &str,
    job_handle: u64,
    port: u16,
) -> Vec<u64> {
    match broker.handle_request(
        pid,
        Some(engine_id),
        BrokerRequest::PrepareInstance {
            instance_id: instance_id.to_owned(),
            app_container_name: container.to_owned(),
            job_handle,
            network_level: NetworkLevel::ProxyTcp,
            proxy_port: Some(port),
            allowed_destinations: Vec::new(),
        },
    ) {
        BrokerResponse::Prepared { filter_ids, .. } => filter_ids,
        other => panic!("expected Prepared, got {other:?}"),
    }
}

/// Caller authentication verifies the peer executable's digest against a
/// Goodfetch release attestation under the bundled trust root, refusing
/// unauthenticated, forged, wrong-signer, and wrong-digest callers and
/// recording rejections in the event log.
///
/// Covers: story:7-24
#[test]
fn broker_authenticates_attested_goodfetch_caller_and_refuses_others() {
    let (signer, trust) = AttestationSigner::for_root("broker-test-key", [0x55; 32]);
    let (rogue, _) = AttestationSigner::for_root("broker-test-key", [0x99; 32]);
    let engine_digest = sha256_digest(b"genuine-goodfetch-collie-exe");
    let mut pids = PidDigestMap::default();
    pids.digests.insert(1001, engine_digest.clone());
    pids.digests
        .insert(1002, sha256_digest(b"tampered-caller-exe"));

    let events = MemoryEventLog::default();
    let mut broker = FilterBroker::new(
        SharedPersistentWfp::default(),
        SharedJobController::default(),
        events.clone(),
        pids,
        trust,
        release_signer(),
    );
    broker.start_and_reconcile_orphans().unwrap();

    let unauth = broker.handle_request(
        1001,
        None,
        BrokerRequest::PrepareInstance {
            instance_id: "i1".to_owned(),
            app_container_name: "goodfetch.plugin.1".to_owned(),
            job_handle: 10,
            network_level: NetworkLevel::ProxyTcp,
            proxy_port: Some(41234),
            allowed_destinations: Vec::new(),
        },
    );
    assert!(matches!(unauth, BrokerResponse::Error { ref code, .. } if code == "unauthenticated"));

    let forged = rogue
        .sign(ProvenanceStatement {
            subject_digest: engine_digest.clone(),
            signer: release_signer(),
        })
        .unwrap();
    let wrong_signer = signer
        .sign(ProvenanceStatement {
            subject_digest: engine_digest.clone(),
            signer: SignerIdentity {
                repository: "attacker/goodfetch".to_owned(),
                workflow: ".github/workflows/release.yml".to_owned(),
                issuer: release_signer().issuer,
            },
        })
        .unwrap();
    let valid_att = signer
        .sign(ProvenanceStatement {
            subject_digest: engine_digest,
            signer: release_signer(),
        })
        .unwrap();

    for (pid, att) in [
        (1001, forged),
        (1001, wrong_signer),
        (1002, valid_att.clone()),
    ] {
        let resp = broker.handle_request(
            pid,
            None,
            BrokerRequest::Authenticate {
                engine_id: "eng-bad".to_owned(),
                attestation: att,
            },
        );
        assert!(
            matches!(resp, BrokerResponse::Error { ref code, .. } if code == "attestation_invalid")
        );
    }

    let ok = broker.handle_request(
        1001,
        None,
        BrokerRequest::Authenticate {
            engine_id: "eng-ok".to_owned(),
            attestation: valid_att,
        },
    );
    assert_eq!(
        ok,
        BrokerResponse::Authenticated {
            engine_id: "eng-ok".to_owned()
        }
    );
    let rejected = events
        .snapshot()
        .iter()
        .filter(|e| matches!(e, BrokerEvent::CallerRejected { .. }))
        .count();
    assert_eq!(rejected, 3);
}

/// Refusing (`Block`) filters are added before permitting (`Permit`) filters
/// and before an instance starts, and are removed ONLY once the instance's Job
/// Object reports zero active processes, with every change written to the
/// Windows Event Log.
///
/// Covers: story:7-24, story:7-112
#[test]
fn broker_adds_refusing_filters_first_and_holds_until_job_object_is_empty() {
    let h = Harness::new([0x71; 32]);
    let mut broker = h.spawn_broker();
    broker.start_and_reconcile_orphans().unwrap();
    h.auth(&mut broker, 2001, "daemon-1");

    let filters = prepare_proxy(
        &mut broker,
        2001,
        "daemon-1",
        "inst-proxy",
        "goodfetch.plugin.proxy.1",
        101,
        45001,
    );
    assert_ne!(filters, [] as [u64; 0]);
    assert!(h.wfp.has_loopback_exemption("goodfetch.plugin.proxy.1"));
    let order = h.wfp.add_order();
    let first_permit = order
        .iter()
        .position(|&a| a == FilterAction::Permit)
        .unwrap();
    assert!(
        order[..first_permit]
            .iter()
            .all(|&a| a == FilterAction::Block)
    );
    assert!(
        order[first_permit..]
            .iter()
            .all(|&a| a == FilterAction::Permit)
    );

    // Non-empty Job Object refuses filter removal and leaves every filter active.
    h.jobs.set_active_for_caller(2001, 101, 2);
    h.jobs.set_stuck_for_caller(2001, 101, true);
    let stuck = broker.handle_request(
        2001,
        Some("daemon-1"),
        BrokerRequest::ReleaseInstance {
            instance_id: "inst-proxy".to_owned(),
        },
    );
    assert!(matches!(stuck, BrokerResponse::Error { ref code, .. } if code == "release_failed"));
    for &id in &filters {
        assert!(h.wfp.contains_filter(id));
    }

    // Once Job Object empties, all filters are removed and logged.
    h.jobs.set_stuck_for_caller(2001, 101, false);
    let released = broker.handle_request(
        2001,
        Some("daemon-1"),
        BrokerRequest::ReleaseInstance {
            instance_id: "inst-proxy".to_owned(),
        },
    );
    assert_eq!(
        released,
        BrokerResponse::Released {
            instance_id: "inst-proxy".to_owned(),
            removed_filters: filters,
        }
    );
    assert_eq!(h.wfp.filter_count(), 0);
    assert!(!h.wfp.has_loopback_exemption("goodfetch.plugin.proxy.1"));
    let log = h.events.snapshot();
    assert!(
        log.iter()
            .any(|e| matches!(e, BrokerEvent::FilterAdded { .. }))
    );
    assert!(log.iter().any(|e| matches!(
        e,
        BrokerEvent::FilterRemoved {
            reason: RemovalReason::InstanceReleased,
            ..
        }
    )));
}

/// When an engine exits, the broker terminates THAT engine's instances' Job
/// Objects itself and removes only their filters, leaving separate Daemon and
/// Attached engines' instances untouched.
///
/// Covers: story:7-112
#[test]
fn broker_terminates_exiting_engine_instances_without_affecting_other_engines() {
    let h = Harness::new([0x72; 32]);
    let mut broker = h.spawn_broker();
    broker.start_and_reconcile_orphans().unwrap();
    h.auth(&mut broker, 2001, "daemon-1");
    h.auth(&mut broker, 2002, "attached-1");

    let daemon_filters = match broker.handle_request(
        2001,
        Some("daemon-1"),
        BrokerRequest::PrepareInstance {
            instance_id: "inst-direct-a".to_owned(),
            app_container_name: "goodfetch.plugin.direct.a".to_owned(),
            job_handle: 201,
            network_level: NetworkLevel::Direct,
            proxy_port: None,
            allowed_destinations: vec![AllowedDestination {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: Some(5432),
            }],
        },
    ) {
        BrokerResponse::Prepared { filter_ids, .. } => filter_ids,
        other => panic!("expected Prepared, got {other:?}"),
    };
    let attached_filters = prepare_proxy(
        &mut broker,
        2002,
        "attached-1",
        "inst-proxy-b",
        "goodfetch.plugin.proxy.b",
        202,
        45002,
    );

    let report = broker.on_engine_exit("attached-1").unwrap();
    assert_eq!(report.instances_terminated, 1);
    assert_eq!(report.removed_filter_ids, attached_filters);
    assert!(h.jobs.was_terminated(2002, 202));
    assert!(!h.jobs.was_terminated(2001, 201));
    for &id in &daemon_filters {
        assert!(h.wfp.contains_filter(id));
    }

    let log = h.events.snapshot();
    assert!(log.iter().any(|e| matches!(
        e,
        BrokerEvent::FilterRemoved {
            reason: RemovalReason::EngineExited,
            ..
        }
    )));
    assert!(
        log.iter()
            .any(|e| matches!(e, BrokerEvent::EngineExitCleanup { .. }))
    );
}

/// Persistent filters survive a broker crash; startup orphan reconciliation
/// removes filters with no live `AppContainer` while keeping live ones;
/// uninstall removes every persistent filter and provider/sublayer; every
/// change is logged.
///
/// Covers: story:7-24, story:7-112
#[test]
fn broker_filters_survive_crash_reconcile_orphans_on_startup_and_clear_on_uninstall() {
    let h = Harness::new([0x73; 32]);
    let mut broker = h.spawn_broker();
    broker.start_and_reconcile_orphans().unwrap();
    h.auth(&mut broker, 2001, "daemon-1");

    let orphan_filters = prepare_proxy(
        &mut broker,
        2001,
        "daemon-1",
        "inst-orphan",
        "goodfetch.plugin.orphan.1",
        201,
        45010,
    );

    // Simulate broker crash: drop abruptly without releasing filters.
    drop(broker);
    for &id in &orphan_filters {
        assert!(h.wfp.contains_filter(id));
    }

    // Simulate a still-live AppContainer whose persistent filters survived crash.
    h.jobs
        .set_live_appcontainer("goodfetch.plugin.live.1", true);
    let live_ids: Vec<u64> = compile_rules(
        "goodfetch.plugin.live.1",
        "inst-live",
        NetworkLevel::ProxyTcp,
        Some(46000),
        &[],
    )
    .unwrap()
    .iter()
    .map(|rule| {
        h.wfp
            .clone()
            .add_persistent_filter("goodfetch.plugin.live.1", rule)
            .unwrap()
    })
    .collect();

    // Restart broker: orphan filters removed, live filters preserved.
    let mut restarted = h.spawn_broker();
    let orphans = restarted.start_and_reconcile_orphans().unwrap();
    assert_eq!(orphans.removed_filter_ids, orphan_filters);
    assert_eq!(orphans.preserved_filter_ids, live_ids);

    // Uninstall removes every remaining filter and unregisters provider/sublayer.
    assert_eq!(
        restarted
            .uninstall_all_filters()
            .unwrap()
            .removed_filter_ids,
        live_ids
    );
    assert_eq!(h.wfp.filter_count(), 0);
    assert!(!h.wfp.provider_registered());

    let log = h.events.snapshot();
    assert!(
        log.iter()
            .any(|e| matches!(e, BrokerEvent::OrphanFilterRemoved { .. }))
    );
    assert!(
        log.iter()
            .any(|e| matches!(e, BrokerEvent::UninstallFilterRemoved { .. }))
    );
}

/// Before the broker service stops, it notifies connected engines, refuses new
/// instance preparations with `service_stopping`, waits for drain, and logs
/// the stop drain in the Windows Event Log.
///
/// Covers: story:7-112
#[test]
fn broker_service_stop_notifies_engines_refuses_new_instances_and_drains_cleanly() {
    let h = Harness::new([0x74; 32]);
    let mut broker = h.spawn_broker();
    broker.start_and_reconcile_orphans().unwrap();
    h.auth(&mut broker, 2001, "daemon-2");
    let _ = prepare_proxy(
        &mut broker,
        2001,
        "daemon-2",
        "inst-stop",
        "goodfetch.plugin.stop.1",
        301,
        46001,
    );

    assert_eq!(
        broker.begin_service_stop().engines_to_notify,
        vec!["daemon-2".to_owned()]
    );
    let after = broker.handle_request(
        2001,
        Some("daemon-2"),
        BrokerRequest::PrepareInstance {
            instance_id: "inst-after".to_owned(),
            app_container_name: "goodfetch.plugin.stop.2".to_owned(),
            job_handle: 302,
            network_level: NetworkLevel::ProxyTcp,
            proxy_port: Some(46002),
            allowed_destinations: Vec::new(),
        },
    );
    assert!(matches!(after, BrokerResponse::Error { ref code, .. } if code == "service_stopping"));
    assert!(!broker.all_engines_drained());

    let released = broker.handle_request(
        2001,
        Some("daemon-2"),
        BrokerRequest::ReleaseInstance {
            instance_id: "inst-stop".to_owned(),
        },
    );
    assert!(matches!(released, BrokerResponse::Released { .. }));
    assert_eq!(
        broker.handle_request(2001, Some("daemon-2"), BrokerRequest::DrainAck),
        BrokerResponse::Drained
    );
    assert!(broker.all_engines_drained());
    assert_eq!(broker.finish_service_stop().unwrap(), [] as [u64; 0]);

    let log = h.events.snapshot();
    assert!(
        log.iter()
            .any(|e| matches!(e, BrokerEvent::ServiceStopDrain { .. }))
    );
}

#[cfg(windows)]
mod windows_release {
    use goodfetch_confinement::attestation::{AttestationSigner, release_signer, sha256_digest};
    use goodfetch_confinement::windows::broker::{BrokerRequest, BrokerResponse, compile_rules};
    use goodfetch_filter_broker::broker::{FilterBroker, WfpStore as _};
    use goodfetch_filter_broker::win32::{
        Win32CallerVerifier, Win32EventLog, Win32JobController, Win32WfpStore,
    };
    use goodfetch_protocol::plugin::NetworkLevel;
    use goodfetch_protocol::plugin::index::ProvenanceStatement;

    /// Real Windows WFP + Event Log + Job Object lifecycle check (Windows
    /// Release run, ADR 0033, issue 879).
    ///
    /// Covers: story:7-24, story:7-112
    #[test]
    fn windows_wfp_persistent_filters_survive_broker_crash_reconcile_orphans_and_uninstall_cleanly()
    {
        let Ok(wfp) = Win32WfpStore::open() else {
            return;
        };
        let (signer, trust) = AttestationSigner::for_root("win-release-test", [0x33; 32]);
        let current_exe = std::env::current_exe().unwrap();
        let digest = sha256_digest(&std::fs::read(&current_exe).unwrap());
        let att = signer
            .sign(ProvenanceStatement {
                subject_digest: digest,
                signer: release_signer(),
            })
            .unwrap();

        let mut broker = FilterBroker::new(
            wfp,
            Win32JobController::default(),
            Win32EventLog::open(),
            Win32CallerVerifier,
            trust.clone(),
            release_signer(),
        );
        if broker.start_and_reconcile_orphans().is_err() {
            return;
        }
        let pid = std::process::id();
        assert!(matches!(
            broker.handle_request(
                pid,
                None,
                BrokerRequest::Authenticate {
                    engine_id: "win-release-engine".to_owned(),
                    attestation: att,
                }
            ),
            BrokerResponse::Authenticated { .. }
        ));

        let rules = compile_rules(
            "goodfetch.plugin.win.crash.test",
            "crash-1",
            NetworkLevel::ProxyTcp,
            Some(49199),
            &[],
        )
        .unwrap();
        let mut wfp_check = Win32WfpStore::open().unwrap();
        let added = wfp_check
            .add_persistent_filter("goodfetch.plugin.win.crash.test", &rules[0])
            .unwrap();
        drop(broker);

        let listed = wfp_check.list_owned_filters().unwrap();
        assert!(listed.iter().any(|f| f.filter_id == added));

        let mut restarted = FilterBroker::new(
            Win32WfpStore::open().unwrap(),
            Win32JobController::default(),
            Win32EventLog::open(),
            Win32CallerVerifier,
            trust,
            release_signer(),
        );
        let orphans = restarted.start_and_reconcile_orphans().unwrap();
        assert!(orphans.removed_filter_ids.contains(&added));

        let _ = restarted.uninstall_all_filters().unwrap();
        assert_eq!(wfp_check.list_owned_filters().unwrap(), Vec::new());
    }
}
