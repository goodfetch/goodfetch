//! Core lifecycle state machine of `goodfetch-filter-broker` (ADRs 0033 and
//! 0039, issue 879).
//!
//! Enforces every broker invariant independently of Win32 FFI so the full
//! lifecycle runs both against in-memory backends in cross-platform tests and
//! against the real Windows Filtering Platform, Job Objects, Named Pipes, and
//! Windows Event Log in the Windows Release run:
//!
//! 1. **Persistent filters:** added via [`WfpStore::add_persistent_filter`]
//!    so they outlive a crash of the broker process.
//! 2. **Refusing filters before start:** `compile_rules` emits every `Block`
//!    filter before every `Permit` filter, and `prepare_instance` installs all
//!    filters before returning `Prepared` (before the Plugin process starts).
//! 3. **Job object empty before removal:** `release_instance` terminates the
//!    Job Object if needed and verifies `active_processes == 0` BEFORE
//!    removing any filter; if the Job Object is not empty, every filter stays
//!    in place and an error is returned.
//! 4. **Engine exit cleanup:** `on_engine_exit` terminates every Job Object
//!    owned by that engine (leaving other Daemon / Attached engines untouched),
//!    confirms each Job Object is empty, and then removes their filters.
//! 5. **Service stop drain:** `begin_service_stop` notifies every connected
//!    engine so each stops its dependent instances under the crash rules;
//!    `finish_service_stop` cleans up any remaining stragglers.
//! 6. **Orphan cleanup at service start:** `reconcile_orphans_on_startup`
//!    enumerates all persistent filters under the broker's provider and
//!    removes every filter whose `AppContainer` has no live process while
//!    preserving filters for still-running `AppContainer`s.
//! 7. **Uninstall cleanup:** `uninstall_all_filters` removes every persistent
//!    filter under the broker's provider, plus its sublayer and provider.
//! 8. **Windows Event Log:** every filter add, remove, orphan cleanup,
//!    uninstall removal, engine-exit cleanup, service-stop drain, and caller
//!    rejection is emitted to [`EventLogSink`].

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_confinement::attestation::{
    AttestationTrustRoot, AttestationVerdict, verify_attestations,
};
use goodfetch_confinement::windows::broker::{
    AllowedDestination, BrokerProtocolError, BrokerRequest, BrokerResponse, WfpRule, compile_rules,
    needs_loopback_exemption, parse_filter_tag,
};
use goodfetch_protocol::plugin::NetworkLevel;
use goodfetch_protocol::plugin::index::{ProvenanceAttestation, SignerIdentity};

use crate::events::{BrokerEvent, RemovalReason};

/// A persistent WFP filter currently present in the OS filter store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledFilter {
    pub filter_id: u64,
    pub tag: String,
}

/// Errors returned by OS backend operations.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    #[error("WFP operation failed: {0}")]
    Wfp(String),
    #[error("Job Object operation failed: {0}")]
    Job(String),
    #[error(
        "Job Object for instance `{instance_id}` still has {active_processes} active process(es); refusing to remove filters"
    )]
    JobNotEmpty {
        instance_id: String,
        active_processes: u32,
    },
    #[error("caller verification failed: {0}")]
    Caller(String),
}

/// Persistent Windows Filtering Platform store (backed by `Fwpm*` on Windows).
pub trait WfpStore {
    /// Ensures the persistent WFP provider and sublayer exist.
    fn ensure_provider_and_sublayer(&mut self) -> Result<(), BackendError>;

    /// Adds one persistent WFP filter (`FWPM_FILTER_FLAG_PERSISTENT`) scoped
    /// to `app_container_name`'s SID, returning its OS-assigned filter ID.
    fn add_persistent_filter(
        &mut self,
        app_container_name: &str,
        rule: &WfpRule,
    ) -> Result<u64, BackendError>;

    /// Removes one persistent WFP filter by its OS-assigned filter ID.
    fn delete_filter(&mut self, filter_id: u64) -> Result<(), BackendError>;

    /// Lists every persistent WFP filter currently registered under the
    /// broker's provider GUID (including filters that survived a prior broker
    /// crash).
    fn list_owned_filters(&self) -> Result<Vec<InstalledFilter>, BackendError>;

    /// Grants or revokes Windows `AppContainer` loopback exemption for
    /// `app_container_name` (needed for `proxy_tcp` and configured loopback
    /// `direct` targets).
    fn set_loopback_exemption(
        &mut self,
        app_container_name: &str,
        enabled: bool,
    ) -> Result<(), BackendError>;

    /// Removes the broker's WFP sublayer and provider during uninstall (after
    /// all filters have been deleted).
    fn remove_provider_and_sublayer(&mut self) -> Result<(), BackendError>;
}

/// Job Object & `AppContainer` liveness boundary (backed by Win32 Job Object and
/// process-token APIs on Windows).
pub trait JobController {
    /// Duplicates or attaches to `job_handle` owned by `caller_pid`, returning
    /// a broker-owned handle token.
    fn attach_job(&mut self, caller_pid: u32, job_handle: u64) -> Result<u64, BackendError>;

    /// Terminates every process in the broker-owned Job Object `broker_job`.
    fn terminate_job(&mut self, broker_job: u64) -> Result<(), BackendError>;

    /// Returns the number of currently active processes inside `broker_job`.
    fn active_processes(&self, broker_job: u64) -> Result<u32, BackendError>;

    /// Releases the broker's handle to `broker_job`.
    fn close_job(&mut self, broker_job: u64);

    /// Returns `true` if any live process on the host currently runs inside
    /// `app_container_name` (used during startup orphan reconciliation so a
    /// live instance's filters are never dropped after a broker crash/restart).
    fn has_live_appcontainer(&self, app_container_name: &str) -> bool;
}

/// Windows Event Log sink recording every filter change (backed by
/// `RegisterEventSourceW`/`ReportEventW` on Windows).
pub trait EventLogSink {
    /// Writes `event` to the Windows Event Log.
    fn write(&mut self, event: BrokerEvent);
}

/// Resolves a connected named-pipe peer's executable digest so the broker can
/// verify its Goodfetch release attestation under the bundled trust root.
pub trait CallerVerifier {
    /// Returns the `sha256:<hex>` digest of the executable running as
    /// `caller_pid`.
    fn executable_digest(&self, caller_pid: u32) -> Result<String, BackendError>;
}

/// One live confined Plugin instance tracked by the broker.
#[derive(Clone, Debug)]
struct TrackedInstance {
    app_container_name: String,
    broker_job: u64,
    filter_ids: Vec<u64>,
    loopback_exempt: bool,
}

/// One authenticated engine connection (Daemon or Attached engine).
#[derive(Clone, Debug)]
struct ConnectedEngine {
    caller_pid: u32,
    drained: bool,
    instances: BTreeMap<String, TrackedInstance>,
}

/// What `begin_service_stop` returns: the engine IDs that must be notified
/// with [`BrokerResponse::ServiceStopping`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopPlan {
    pub engines_to_notify: Vec<String>,
}

/// Summary of startup orphan reconciliation.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OrphanReport {
    pub removed_filter_ids: Vec<u64>,
    pub preserved_filter_ids: Vec<u64>,
}

/// Summary of broker uninstallation.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct UninstallReport {
    pub removed_filter_ids: Vec<u64>,
}

/// Summary of engine-exit cleanup.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EngineExitReport {
    pub instances_terminated: usize,
    pub removed_filter_ids: Vec<u64>,
}

/// Stateful core of `goodfetch-filter-broker`.
pub struct FilterBroker<W, J, E, V> {
    wfp: W,
    jobs: J,
    events: E,
    verifier: V,
    trust_root: AttestationTrustRoot,
    expected_signer: SignerIdentity,
    engines: BTreeMap<String, ConnectedEngine>,
    stopping: bool,
}

impl<W, J, E, V> FilterBroker<W, J, E, V>
where
    W: WfpStore,
    J: JobController,
    E: EventLogSink,
    V: CallerVerifier,
{
    /// Creates a new broker instance with its OS backends, bundled Sigstore
    /// `trust_root`, and expected Goodfetch release `expected_signer`.
    pub fn new(
        wfp: W,
        jobs: J,
        events: E,
        verifier: V,
        trust_root: AttestationTrustRoot,
        expected_signer: SignerIdentity,
    ) -> Self {
        Self {
            wfp,
            jobs,
            events,
            verifier,
            trust_root,
            expected_signer,
            engines: BTreeMap::new(),
            stopping: false,
        }
    }

    /// Borrows the WFP store (for inspection in tests and Windows shutdown).
    pub fn wfp(&self) -> &W {
        &self.wfp
    }

    /// Borrows the Job Object controller.
    pub fn jobs(&self) -> &J {
        &self.jobs
    }

    /// Borrows the Event Log sink.
    pub fn events(&self) -> &E {
        &self.events
    }

    /// Initializes the persistent WFP provider/sublayer and removes every
    /// persistent filter whose `AppContainer` no longer has a live instance,
    /// preserving any filter belonging to an `AppContainer` that is still
    /// running (e.g. after a broker crash and restart, ADR 0033, issue 879).
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] if WFP provider setup or filter enumeration
    /// fails.
    pub fn start_and_reconcile_orphans(&mut self) -> Result<OrphanReport, BackendError> {
        self.wfp.ensure_provider_and_sublayer()?;
        let filters = self.wfp.list_owned_filters()?;
        let mut report = OrphanReport::default();
        let mut orphan_containers = BTreeSet::new();
        for filter in filters {
            let app_container =
                parse_filter_tag(&filter.tag).map(|(container, _, _)| container.to_owned());
            let live = app_container
                .as_deref()
                .is_some_and(|container| self.jobs.has_live_appcontainer(container));
            if live {
                report.preserved_filter_ids.push(filter.filter_id);
            } else {
                self.wfp.delete_filter(filter.filter_id)?;
                let container_label = app_container.unwrap_or_else(|| "unknown".to_owned());
                orphan_containers.insert(container_label.clone());
                self.events.write(BrokerEvent::OrphanFilterRemoved {
                    filter_id: filter.filter_id,
                    app_container_name: container_label,
                    tag: filter.tag,
                });
                report.removed_filter_ids.push(filter.filter_id);
            }
        }
        for container in orphan_containers {
            if container != "unknown" {
                let _ = self.wfp.set_loopback_exemption(&container, false);
            }
        }
        Ok(report)
    }

    /// Handles one request from a connected pipe peer (`caller_pid`, optional
    /// already-authenticated `engine_id`).
    pub fn handle_request(
        &mut self,
        caller_pid: u32,
        authenticated_engine: Option<&str>,
        request: BrokerRequest,
    ) -> BrokerResponse {
        match request {
            BrokerRequest::Ping => BrokerResponse::Pong,
            BrokerRequest::Authenticate {
                engine_id,
                attestation,
            } => self.authenticate_caller(caller_pid, engine_id, &attestation),
            BrokerRequest::PrepareInstance {
                instance_id,
                app_container_name,
                job_handle,
                network_level,
                proxy_port,
                allowed_destinations,
            } => {
                let Some(engine_id) = authenticated_engine else {
                    return error_response(
                        "unauthenticated",
                        "caller must authenticate with a Goodfetch release attestation before preparing filters",
                    );
                };
                if self.stopping {
                    return error_response(
                        "service_stopping",
                        "filter broker is stopping and refuses new instances",
                    );
                }
                self.prepare_instance(
                    engine_id,
                    instance_id,
                    app_container_name,
                    job_handle,
                    network_level,
                    proxy_port,
                    &allowed_destinations,
                )
            }
            BrokerRequest::ReleaseInstance { instance_id } => {
                let Some(engine_id) = authenticated_engine else {
                    return error_response(
                        "unauthenticated",
                        "caller must authenticate before releasing filters",
                    );
                };
                self.release_instance(engine_id, &instance_id, RemovalReason::InstanceReleased)
            }
            BrokerRequest::DrainAck => {
                let Some(engine_id) = authenticated_engine else {
                    return error_response(
                        "unauthenticated",
                        "caller must authenticate before acknowledging drain",
                    );
                };
                if let Some(engine) = self.engines.get_mut(engine_id) {
                    engine.drained = true;
                }
                BrokerResponse::Drained
            }
        }
    }

    /// Verifies `caller_pid`'s executable digest against `attestation` under
    /// the broker's bundled Sigstore trust root and expected Goodfetch release
    /// signer (ADR 0033, issue 879).
    fn authenticate_caller(
        &mut self,
        caller_pid: u32,
        engine_id: String,
        attestation: &ProvenanceAttestation,
    ) -> BrokerResponse {
        if engine_id.is_empty() {
            return error_response("invalid_engine_id", "engine_id must be non-empty");
        }
        let digest = match self.verifier.executable_digest(caller_pid) {
            Ok(digest) => digest,
            Err(error) => {
                let reason = error.to_string();
                self.events.write(BrokerEvent::CallerRejected {
                    caller_pid,
                    reason: reason.clone(),
                });
                return error_response("caller_unverified", &reason);
            }
        };
        match verify_attestations(
            std::slice::from_ref(attestation),
            &digest,
            Some(&self.expected_signer),
            &self.trust_root,
        ) {
            AttestationVerdict::Verified(_) => {
                self.engines
                    .entry(engine_id.clone())
                    .or_insert_with(|| ConnectedEngine {
                        caller_pid,
                        drained: false,
                        instances: BTreeMap::new(),
                    });
                BrokerResponse::Authenticated { engine_id }
            }
            AttestationVerdict::Refused { reason, .. } => {
                let detail =
                    format!("Goodfetch release attestation failed verification ({reason:?})");
                self.events.write(BrokerEvent::CallerRejected {
                    caller_pid,
                    reason: detail.clone(),
                });
                error_response("attestation_invalid", &detail)
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "maps 1:1 from PrepareInstance IPC fields"
    )]
    fn prepare_instance(
        &mut self,
        engine_id: &str,
        instance_id: String,
        app_container_name: String,
        job_handle: u64,
        network_level: NetworkLevel,
        proxy_port: Option<u16>,
        allowed_destinations: &[AllowedDestination],
    ) -> BrokerResponse {
        let Some(caller_pid) = self.engines.get(engine_id).map(|engine| engine.caller_pid) else {
            return error_response("unknown_engine", "engine is not registered");
        };
        let rules = match compile_rules(
            &app_container_name,
            &instance_id,
            network_level,
            proxy_port,
            allowed_destinations,
        ) {
            Ok(rules) => rules,
            Err(error) => return protocol_error_response(&error),
        };
        let broker_job = match self.jobs.attach_job(caller_pid, job_handle) {
            Ok(handle) => handle,
            Err(error) => return error_response("job_attach_failed", &error.to_string()),
        };

        let loopback_exempt = needs_loopback_exemption(network_level, allowed_destinations);
        let mut installed_ids = Vec::with_capacity(rules.len());
        // `compile_rules` guarantees all Block (refusing) rules come first,
        // so refusing filters are installed before any Permit rule and before
        // returning Prepared to start the Plugin.
        for rule in &rules {
            match self.wfp.add_persistent_filter(&app_container_name, rule) {
                Ok(filter_id) => {
                    installed_ids.push(filter_id);
                    self.events.write(BrokerEvent::FilterAdded {
                        filter_id,
                        app_container_name: app_container_name.clone(),
                        instance_id: instance_id.clone(),
                        rule_name: rule.rule_name.clone(),
                        action: rule.action,
                    });
                }
                Err(error) => {
                    // Roll back partially added filters since the instance has not started.
                    for id in installed_ids.drain(..).rev() {
                        if self.wfp.delete_filter(id).is_ok() {
                            self.events.write(BrokerEvent::FilterRemoved {
                                filter_id: id,
                                app_container_name: app_container_name.clone(),
                                instance_id: instance_id.clone(),
                                reason: RemovalReason::PrepareRollback,
                            });
                        }
                    }
                    self.jobs.close_job(broker_job);
                    return error_response("wfp_add_failed", &error.to_string());
                }
            }
        }
        if loopback_exempt
            && let Err(error) = self.wfp.set_loopback_exemption(&app_container_name, true)
        {
            for id in installed_ids.drain(..).rev() {
                if self.wfp.delete_filter(id).is_ok() {
                    self.events.write(BrokerEvent::FilterRemoved {
                        filter_id: id,
                        app_container_name: app_container_name.clone(),
                        instance_id: instance_id.clone(),
                        reason: RemovalReason::PrepareRollback,
                    });
                }
            }
            self.jobs.close_job(broker_job);
            return error_response("loopback_exemption_failed", &error.to_string());
        }

        if let Some(engine) = self.engines.get_mut(engine_id) {
            engine.instances.insert(
                instance_id.clone(),
                TrackedInstance {
                    app_container_name: app_container_name.clone(),
                    broker_job,
                    filter_ids: installed_ids.clone(),
                    loopback_exempt,
                },
            );
        }
        BrokerResponse::Prepared {
            instance_id,
            app_container_name,
            filter_ids: installed_ids,
        }
    }

    fn release_instance(
        &mut self,
        engine_id: &str,
        instance_id: &str,
        reason: RemovalReason,
    ) -> BrokerResponse {
        let Some(tracked) = self
            .engines
            .get(engine_id)
            .and_then(|engine| engine.instances.get(instance_id))
            .cloned()
        else {
            return error_response(
                "unknown_instance",
                &format!("no tracked instance `{instance_id}` for engine `{engine_id}`"),
            );
        };
        match self.terminate_and_remove_instance(instance_id, &tracked, reason) {
            Ok(removed_filters) => {
                if let Some(engine) = self.engines.get_mut(engine_id) {
                    engine.instances.remove(instance_id);
                }
                BrokerResponse::Released {
                    instance_id: instance_id.to_owned(),
                    removed_filters,
                }
            }
            Err(error) => error_response("release_failed", &error.to_string()),
        }
    }

    /// Terminates `tracked`'s Job Object, verifies it has zero active
    /// processes left, and ONLY THEN deletes its persistent WFP filters and
    /// revokes its loopback exemption (ADR 0033, issue 879).
    fn terminate_and_remove_instance(
        &mut self,
        instance_id: &str,
        tracked: &TrackedInstance,
        reason: RemovalReason,
    ) -> Result<Vec<u64>, BackendError> {
        let active_before = self.jobs.active_processes(tracked.broker_job)?;
        if active_before > 0 {
            self.jobs.terminate_job(tracked.broker_job)?;
        }
        let active_after = self.jobs.active_processes(tracked.broker_job)?;
        if active_after > 0 {
            // NEVER remove filters while any process remains inside the Job Object.
            return Err(BackendError::JobNotEmpty {
                instance_id: instance_id.to_owned(),
                active_processes: active_after,
            });
        }
        let mut removed = Vec::with_capacity(tracked.filter_ids.len());
        for &filter_id in &tracked.filter_ids {
            self.wfp.delete_filter(filter_id)?;
            self.events.write(BrokerEvent::FilterRemoved {
                filter_id,
                app_container_name: tracked.app_container_name.clone(),
                instance_id: instance_id.to_owned(),
                reason,
            });
            removed.push(filter_id);
        }
        if tracked.loopback_exempt {
            let _ = self
                .wfp
                .set_loopback_exemption(&tracked.app_container_name, false);
        }
        self.jobs.close_job(tracked.broker_job);
        Ok(removed)
    }

    /// Called when the engine identified by `engine_id` exits (gracefully or
    /// by crash): terminates every Job Object owned by THAT engine (since the
    /// broker's own handle keeps kill-on-close from firing), verifies each is
    /// empty, and removes only THAT engine's filters, leaving every other
    /// Daemon or Attached engine untouched (ADR 0033, issue 879).
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] if terminating a Job Object or removing a
    /// filter fails; any instance whose Job Object did not drain keeps its
    /// filters.
    pub fn on_engine_exit(&mut self, engine_id: &str) -> Result<EngineExitReport, BackendError> {
        let Some(engine) = self.engines.remove(engine_id) else {
            return Ok(EngineExitReport::default());
        };
        let mut report = EngineExitReport::default();
        let mut failed: Option<BackendError> = None;
        for (instance_id, tracked) in engine.instances {
            match self.terminate_and_remove_instance(
                &instance_id,
                &tracked,
                RemovalReason::EngineExited,
            ) {
                Ok(removed) => {
                    report.instances_terminated += 1;
                    report.removed_filter_ids.extend(removed);
                }
                Err(error) => {
                    failed = Some(error);
                }
            }
        }
        self.events.write(BrokerEvent::EngineExitCleanup {
            engine_id: engine_id.to_owned(),
            instances_terminated: report.instances_terminated,
            filters_removed: report.removed_filter_ids.len(),
        });
        if let Some(error) = failed {
            return Err(error);
        }
        Ok(report)
    }

    /// Begins a coordinated service stop: rejects new `PrepareInstance` calls
    /// and returns every connected engine ID that must receive
    /// [`BrokerResponse::ServiceStopping`] so each engine stops its dependent
    /// instances under the crash rules before the service exits.
    pub fn begin_service_stop(&mut self) -> StopPlan {
        self.stopping = true;
        let engines_to_notify = self.engines.keys().cloned().collect();
        StopPlan { engines_to_notify }
    }

    /// Returns `true` once every connected engine has drained its instances
    /// and sent [`BrokerRequest::DrainAck`].
    #[must_use]
    pub fn all_engines_drained(&self) -> bool {
        self.engines
            .values()
            .all(|engine| engine.drained && engine.instances.is_empty())
    }

    /// Completes service shutdown after engines have drained (or after the
    /// stop grace period expires): terminates any remaining straggler Job
    /// Objects, removes their filters, and logs [`BrokerEvent::ServiceStopDrain`].
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] if any straggler cleanup fails.
    pub fn finish_service_stop(&mut self) -> Result<Vec<u64>, BackendError> {
        let engines_notified = self.engines.len();
        let remaining = std::mem::take(&mut self.engines);
        let mut remaining_terminated = 0_usize;
        let mut removed_all = Vec::new();
        for (_engine_id, engine) in remaining {
            for (instance_id, tracked) in engine.instances {
                let removed = self.terminate_and_remove_instance(
                    &instance_id,
                    &tracked,
                    RemovalReason::ServiceStopping,
                )?;
                remaining_terminated += 1;
                removed_all.extend(removed);
            }
        }
        self.events.write(BrokerEvent::ServiceStopDrain {
            engines_notified,
            remaining_instances_terminated: remaining_terminated,
            filters_removed: removed_all.len(),
        });
        Ok(removed_all)
    }

    /// Uninstalls the broker: removes EVERY persistent WFP filter registered
    /// under the broker's provider, revokes their loopback exemptions, removes
    /// the provider and sublayer, and records every filter removal in the
    /// Windows Event Log (ADR 0033, issue 879).
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] if WFP enumeration or deletion fails.
    pub fn uninstall_all_filters(&mut self) -> Result<UninstallReport, BackendError> {
        let filters = self.wfp.list_owned_filters()?;
        let mut report = UninstallReport::default();
        let mut containers = BTreeSet::new();
        for filter in filters {
            if let Some((container, _, _)) = parse_filter_tag(&filter.tag) {
                containers.insert(container.to_owned());
            }
            self.wfp.delete_filter(filter.filter_id)?;
            self.events.write(BrokerEvent::UninstallFilterRemoved {
                filter_id: filter.filter_id,
                tag: filter.tag,
            });
            report.removed_filter_ids.push(filter.filter_id);
        }
        for container in containers {
            let _ = self.wfp.set_loopback_exemption(&container, false);
        }
        self.wfp.remove_provider_and_sublayer()?;
        Ok(report)
    }
}

fn error_response(code: &str, message: &str) -> BrokerResponse {
    BrokerResponse::Error {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

fn protocol_error_response(error: &BrokerProtocolError) -> BrokerResponse {
    error_response("invalid_request", &error.to_string())
}
