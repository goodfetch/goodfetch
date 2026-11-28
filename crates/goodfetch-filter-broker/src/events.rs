//! Structured Windows Event Log records for every filter change made by
//! `goodfetch-filter-broker` (ADR 0033, issue 879).

use std::fmt;

use goodfetch_confinement::windows::broker::FilterAction;

/// Event IDs written under source [`goodfetch_confinement::windows::broker::BROKER_EVENT_SOURCE`].
pub const EVENT_FILTER_ADDED: u32 = 1001;
pub const EVENT_FILTER_REMOVED: u32 = 1002;
pub const EVENT_ORPHAN_FILTER_REMOVED: u32 = 1003;
pub const EVENT_UNINSTALL_FILTER_REMOVED: u32 = 1004;
pub const EVENT_ENGINE_EXIT_CLEANUP: u32 = 1005;
pub const EVENT_SERVICE_STOP_DRAIN: u32 = 1006;
pub const EVENT_CALLER_REJECTED: u32 = 2001;

/// Severity level mapped to Windows Event Log `EVENTLOG_*_TYPE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventSeverity {
    Information,
    Warning,
}

/// One structured filter-broker audit event written to the Windows Event Log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrokerEvent {
    /// One persistent WFP filter was added for an `AppContainer` instance.
    FilterAdded {
        filter_id: u64,
        app_container_name: String,
        instance_id: String,
        rule_name: String,
        action: FilterAction,
    },
    /// One persistent WFP filter was removed once the instance's Job Object
    /// reached zero active processes.
    FilterRemoved {
        filter_id: u64,
        app_container_name: String,
        instance_id: String,
        reason: RemovalReason,
    },
    /// One orphaned persistent WFP filter whose `AppContainer` had no live
    /// instance was removed at service startup.
    OrphanFilterRemoved {
        filter_id: u64,
        app_container_name: String,
        tag: String,
    },
    /// One persistent WFP filter was removed while uninstalling the broker.
    UninstallFilterRemoved { filter_id: u64, tag: String },
    /// An engine exited; the broker terminated its tracked instances' Job
    /// Objects and removed their filters.
    EngineExitCleanup {
        engine_id: String,
        instances_terminated: usize,
        filters_removed: usize,
    },
    /// Service stop initiated: engines were notified to stop dependent
    /// instances before shutdown.
    ServiceStopDrain {
        engines_notified: usize,
        remaining_instances_terminated: usize,
        filters_removed: usize,
    },
    /// A named-pipe caller failed Goodfetch release-attestation verification.
    CallerRejected { caller_pid: u32, reason: String },
}

/// Why an instance's persistent WFP filter was removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemovalReason {
    InstanceReleased,
    EngineExited,
    ServiceStopping,
    PrepareRollback,
}

impl BrokerEvent {
    /// Stable Windows Event Log numeric event ID.
    #[must_use]
    pub const fn event_id(&self) -> u32 {
        match self {
            Self::FilterAdded { .. } => EVENT_FILTER_ADDED,
            Self::FilterRemoved { .. } => EVENT_FILTER_REMOVED,
            Self::OrphanFilterRemoved { .. } => EVENT_ORPHAN_FILTER_REMOVED,
            Self::UninstallFilterRemoved { .. } => EVENT_UNINSTALL_FILTER_REMOVED,
            Self::EngineExitCleanup { .. } => EVENT_ENGINE_EXIT_CLEANUP,
            Self::ServiceStopDrain { .. } => EVENT_SERVICE_STOP_DRAIN,
            Self::CallerRejected { .. } => EVENT_CALLER_REJECTED,
        }
    }

    /// Windows Event Log severity level.
    #[must_use]
    pub const fn severity(&self) -> EventSeverity {
        match self {
            Self::CallerRejected { .. } => EventSeverity::Warning,
            _ => EventSeverity::Information,
        }
    }
}

impl fmt::Display for BrokerEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FilterAdded {
                filter_id,
                app_container_name,
                instance_id,
                rule_name,
                action,
            } => write!(
                formatter,
                "WFP persistent filter added: id={filter_id} app_container={app_container_name} instance={instance_id} rule={rule_name} action={action:?}"
            ),
            Self::FilterRemoved {
                filter_id,
                app_container_name,
                instance_id,
                reason,
            } => write!(
                formatter,
                "WFP persistent filter removed: id={filter_id} app_container={app_container_name} instance={instance_id} reason={reason:?}"
            ),
            Self::OrphanFilterRemoved {
                filter_id,
                app_container_name,
                tag,
            } => write!(
                formatter,
                "WFP orphan filter removed at startup: id={filter_id} app_container={app_container_name} tag={tag}"
            ),
            Self::UninstallFilterRemoved { filter_id, tag } => write!(
                formatter,
                "WFP filter removed at uninstall: id={filter_id} tag={tag}"
            ),
            Self::EngineExitCleanup {
                engine_id,
                instances_terminated,
                filters_removed,
            } => write!(
                formatter,
                "Engine exited cleanup: engine={engine_id} instances_terminated={instances_terminated} filters_removed={filters_removed}"
            ),
            Self::ServiceStopDrain {
                engines_notified,
                remaining_instances_terminated,
                filters_removed,
            } => write!(
                formatter,
                "Service stop drain completed: engines_notified={engines_notified} remaining_terminated={remaining_instances_terminated} filters_removed={filters_removed}"
            ),
            Self::CallerRejected { caller_pid, reason } => write!(
                formatter,
                "Broker caller rejected: pid={caller_pid} reason={reason}"
            ),
        }
    }
}
