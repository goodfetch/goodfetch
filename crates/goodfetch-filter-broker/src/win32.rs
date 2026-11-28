//! The sole `unsafe` Win32 implementation module of `goodfetch-filter-broker`
//! (ADRs 0033 and 0039, issue 879):
//!
//! - Persistent Windows Filtering Platform (WFP) provider, sublayer, and
//!   per-`AppContainer` filters (`FWPM_FILTER_FLAG_PERSISTENT`).
//! - Windows `AppContainer` loopback exemption management
//!   (`NetworkIsolationGetAppContainerConfig` /
//!   `NetworkIsolationSetAppContainerConfig`).
//! - Named pipe server restricted via a protected SDDL DACL to `LocalSystem`,
//!   `Administrators`, and the installing user SID alone.
//! - Caller executable verification (`GetNamedPipeClientProcessId` ->
//!   `QueryFullProcessImageNameW` -> SHA-256 -> Sigstore release attestation).
//! - Job Object handle duplication (`DuplicateHandle`), termination
//!   (`TerminateJobObject`), zero-process verification
//!   (`QueryInformationJobObject`), and live `AppContainer` scan across
//!   running processes for startup orphan cleanup.
//! - Windows Event Log reporting (`RegisterEventSourceW` / `ReportEventW`).
//! - Windows Service Control Manager registration, dispatch, stop handling,
//!   and uninstallation.
//!
//! Size: one concern — isolating every Windows `unsafe` Win32 WFP, Named Pipe,
//! Job Object, Event Log, Network Isolation and SCM call in one auditable
//! module (ADR 0033).

#![expect(
    unsafe_code,
    reason = "Windows filter broker calls Win32 WFP, Named Pipe, Job Object, Event Log, Network Isolation and SCM APIs via windows-sys (ADR 0033)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the filter broker service runs blocking Win32 I/O outside any async engine runtime"
)]
#![expect(
    clippy::disallowed_types,
    reason = "Windows service control callbacks and pipe handler threads run outside loom-tested engine concurrency (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{self, BufRead as _, BufReader, Write as _};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::FromRawHandle as _;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use goodfetch_confinement::attestation::{AttestationTrustRoot, release_signer, sha256_digest};
use goodfetch_confinement::windows::broker::{
    BROKER_DISPLAY_NAME, BROKER_EVENT_SOURCE, BROKER_SERVICE_NAME, BrokerRequest, BrokerResponse,
    DEFAULT_PIPE_NAME, FilterAction, FilterLayer, IpRange, PROVIDER_GUID, SUBLAYER_GUID, WfpRule,
    pipe_sddl_for_user_sid,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, ERROR_BROKEN_PIPE, ERROR_PIPE_CONNECTED, FALSE, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree, MAX_PATH,
};
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::{
    FWP_ACTION_BLOCK, FWP_ACTION_PERMIT, FWP_BYTE_ARRAY16, FWP_CONDITION_VALUE0,
    FWP_CONDITION_VALUE0_0, FWP_MATCH_EQUAL, FWP_SID, FWP_UINT8, FWP_UINT16, FWP_V4_ADDR_AND_MASK,
    FWP_V4_ADDR_MASK, FWP_V6_ADDR_AND_MASK, FWP_V6_ADDR_MASK, FWP_VALUE0, FWP_VALUE0_0,
    FWPM_CONDITION_ALE_PACKAGE_ID, FWPM_CONDITION_IP_PROTOCOL, FWPM_CONDITION_IP_REMOTE_ADDRESS,
    FWPM_CONDITION_IP_REMOTE_PORT, FWPM_DISPLAY_DATA0, FWPM_FILTER_CONDITION0,
    FWPM_FILTER_ENUM_TEMPLATE0, FWPM_FILTER_FLAG_PERSISTENT, FWPM_FILTER0,
    FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6,
    FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V4, FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V6,
    FWPM_PROVIDER_FLAG_PERSISTENT, FWPM_PROVIDER0, FWPM_SUBLAYER_FLAG_PERSISTENT, FWPM_SUBLAYER0,
    FwpmEngineClose0, FwpmEngineOpen0, FwpmFilterAdd0, FwpmFilterCreateEnumHandle0,
    FwpmFilterDeleteById0, FwpmFilterDestroyEnumHandle0, FwpmFilterEnum0, FwpmFreeMemory0,
    FwpmProviderAdd0, FwpmProviderDeleteByKey0, FwpmSubLayerAdd0, FwpmSubLayerDeleteByKey0,
    FwpmTransactionAbort0, FwpmTransactionBegin0, FwpmTransactionCommit0,
};
use windows_sys::Win32::NetworkManagement::WindowsFirewall::{
    NetworkIsolationGetAppContainerConfig, NetworkIsolationSetAppContainerConfig,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::Isolation::DeriveAppContainerSidFromAppContainerName;
use windows_sys::Win32::Security::{
    EqualSid, FreeSid, GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    SID_AND_ATTRIBUTES, TOKEN_APPCONTAINER_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    TokenAppContainerSid, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, SYNCHRONIZE,
};
use windows_sys::Win32::System::EventLog::{
    DeregisterEventSource, EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE, RegisterEventSourceW,
    ReportEventW,
};
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList, QueryInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ControlService, CreateServiceW, DeleteService, OpenSCManagerW,
    OpenServiceW, RegisterServiceCtrlHandlerExW, SC_HANDLE, SC_MANAGER_ALL_ACCESS,
    SERVICE_ACCEPT_STOP, SERVICE_AUTO_START, SERVICE_CONTROL_STOP, SERVICE_ERROR_NORMAL,
    SERVICE_RUNNING, SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STATUS_HANDLE,
    SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
    SetServiceStatus, StartServiceCtrlDispatcherW, StartServiceW,
};
use windows_sys::Win32::System::SystemServices::{JOB_OBJECT_QUERY, JOB_OBJECT_TERMINATE};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_DUP_HANDLE,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::core::GUID;

use crate::broker::{
    BackendError, CallerVerifier, EventLogSink, FilterBroker, InstalledFilter, JobController,
    WfpStore,
};
use crate::events::{BrokerEvent, EventSeverity};

/// `RPC_C_AUTHN_WINNT` = 10 (`FwpmEngineOpen0`).
const RPC_C_AUTHN_WINNT: u32 = 10;
/// `FWP_E_ALREADY_EXISTS` = `0x8032_0009`.
const FWP_E_ALREADY_EXISTS: u32 = 0x8032_0009;
/// `FWP_E_FILTER_NOT_FOUND` / `FWP_E_PROVIDER_NOT_FOUND` / `FWP_E_SUBLAYER_NOT_FOUND`.
const FWP_E_PROVIDER_NOT_FOUND: u32 = 0x8032_0005;
const FWP_E_SUBLAYER_NOT_FOUND: u32 = 0x8032_0007;
const FWP_E_FILTER_NOT_FOUND: u32 = 0x8032_0003;
/// `SERVICE_ALL_ACCESS` = `0x000F_01FF`.
const SERVICE_ALL_ACCESS: u32 = 0x000F_01FF;

fn to_wide_nul(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn wide_ptr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0_usize;
    // SAFETY: NUL-terminated UTF-16 string returned by Win32.
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}

/// Real Windows Filtering Platform store using persistent provider, sublayer,
/// and filters (`FWPM_FILTER_FLAG_PERSISTENT`).
pub struct Win32WfpStore {
    engine: HANDLE,
}

// SAFETY: Win32 WFP engine handles (`FwpmEngineOpen0`) are kernel handles
// safe to move across threads when serialized by `Mutex`.
unsafe impl Send for Win32WfpStore {}

impl Win32WfpStore {
    /// Opens a WFP engine session (not dynamic: filters added with
    /// `FWPM_FILTER_FLAG_PERSISTENT` survive broker crashes).
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Wfp`] if `FwpmEngineOpen0` fails.
    pub fn open() -> Result<Self, BackendError> {
        let mut engine: HANDLE = ptr::null_mut();
        // SAFETY: opens local WFP Base Filtering Engine session with default
        // persistent (non-dynamic) session flags.
        let status = unsafe {
            FwpmEngineOpen0(
                ptr::null(),
                RPC_C_AUTHN_WINNT,
                ptr::null(),
                ptr::null(),
                &raw mut engine,
            )
        };
        if status != 0 || engine.is_null() {
            return Err(BackendError::Wfp(format!(
                "FwpmEngineOpen0 failed: 0x{status:08x}"
            )));
        }
        Ok(Self { engine })
    }
}

impl Drop for Win32WfpStore {
    fn drop(&mut self) {
        if !self.engine.is_null() {
            // SAFETY: closes the WFP engine handle opened in `open`.
            unsafe {
                let _ = FwpmEngineClose0(self.engine);
            }
        }
    }
}

struct SidGuard(PSID);

impl Drop for SidGuard {
    fn drop(&mut self) {
        // SAFETY: SID was allocated by `DeriveAppContainerSidFromAppContainerName`.
        unsafe {
            FreeSid(self.0);
        }
    }
}

struct SdGuard(PSECURITY_DESCRIPTOR);

impl Drop for SdGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: frees security descriptor allocated by Win32.
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

#[repr(C)]
struct PidList {
    header: JOBOBJECT_BASIC_PROCESS_ID_LIST,
    extra: [usize; 63],
}

fn derive_sid(app_container_name: &str) -> Result<SidGuard, BackendError> {
    let wide = to_wide_nul(app_container_name);
    let mut sid: PSID = ptr::null_mut();
    // SAFETY: derives the AppContainer SID from its profile name.
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(wide.as_ptr(), &raw mut sid) };
    if hr < 0 || sid.is_null() {
        return Err(BackendError::Wfp(format!(
            "DeriveAppContainerSidFromAppContainerName({app_container_name}) failed: 0x{:08x}",
            hr.cast_unsigned()
        )));
    }
    Ok(SidGuard(sid))
}

fn build_filter_conditions(
    sid: PSID,
    rule: &WfpRule,
    v4_mask: &mut FWP_V4_ADDR_AND_MASK,
    v6_mask: &mut FWP_V6_ADDR_AND_MASK,
) -> Vec<FWPM_FILTER_CONDITION0> {
    let mut conditions: Vec<FWPM_FILTER_CONDITION0> = Vec::with_capacity(4);
    conditions.push(FWPM_FILTER_CONDITION0 {
        fieldKey: FWPM_CONDITION_ALE_PACKAGE_ID,
        matchType: FWP_MATCH_EQUAL,
        conditionValue: FWP_CONDITION_VALUE0 {
            r#type: FWP_SID,
            Anonymous: FWP_CONDITION_VALUE0_0 { sid: sid.cast() },
        },
    });
    if let Some(ip) = rule.remote_ip {
        match ip {
            IpRange::V4 { addr, mask } => {
                v4_mask.addr = u32::from(addr);
                v4_mask.mask = u32::from(mask);
                conditions.push(FWPM_FILTER_CONDITION0 {
                    fieldKey: FWPM_CONDITION_IP_REMOTE_ADDRESS,
                    matchType: FWP_MATCH_EQUAL,
                    conditionValue: FWP_CONDITION_VALUE0 {
                        r#type: FWP_V4_ADDR_MASK,
                        Anonymous: FWP_CONDITION_VALUE0_0 {
                            v4AddrMask: ptr::from_mut(v4_mask),
                        },
                    },
                });
            }
            IpRange::V6 { addr, prefix_len } => {
                v6_mask.addr = addr.octets();
                v6_mask.prefixLength = prefix_len;
                conditions.push(FWPM_FILTER_CONDITION0 {
                    fieldKey: FWPM_CONDITION_IP_REMOTE_ADDRESS,
                    matchType: FWP_MATCH_EQUAL,
                    conditionValue: FWP_CONDITION_VALUE0 {
                        r#type: FWP_V6_ADDR_MASK,
                        Anonymous: FWP_CONDITION_VALUE0_0 {
                            v6AddrMask: ptr::from_mut(v6_mask),
                        },
                    },
                });
            }
        }
    }
    if let Some(proto) = rule.protocol {
        conditions.push(FWPM_FILTER_CONDITION0 {
            fieldKey: FWPM_CONDITION_IP_PROTOCOL,
            matchType: FWP_MATCH_EQUAL,
            conditionValue: FWP_CONDITION_VALUE0 {
                r#type: FWP_UINT8,
                Anonymous: FWP_CONDITION_VALUE0_0 { uint8: proto },
            },
        });
    }
    if let Some(port) = rule.remote_port {
        conditions.push(FWPM_FILTER_CONDITION0 {
            fieldKey: FWPM_CONDITION_IP_REMOTE_PORT,
            matchType: FWP_MATCH_EQUAL,
            conditionValue: FWP_CONDITION_VALUE0 {
                r#type: FWP_UINT16,
                Anonymous: FWP_CONDITION_VALUE0_0 { uint16: port },
            },
        });
    }
    conditions
}

impl WfpStore for Win32WfpStore {
    fn ensure_provider_and_sublayer(&mut self) -> Result<(), BackendError> {
        let mut provider_name = to_wide_nul("Goodfetch Filter Broker Provider");
        let mut provider_desc = to_wide_nul("Persistent WFP provider for Goodfetch Plugins");
        // SAFETY: zero-initialized optional blob field for `FWPM_PROVIDER0`.
        let empty_provider_data = unsafe { std::mem::zeroed() };
        let provider = FWPM_PROVIDER0 {
            providerKey: GUID::from_u128(PROVIDER_GUID),
            displayData: FWPM_DISPLAY_DATA0 {
                name: provider_name.as_mut_ptr(),
                description: provider_desc.as_mut_ptr(),
            },
            flags: FWPM_PROVIDER_FLAG_PERSISTENT,
            providerData: empty_provider_data,
            serviceName: ptr::null_mut(),
        };
        // SAFETY: registers persistent WFP provider.
        let status = unsafe { FwpmProviderAdd0(self.engine, &raw const provider, ptr::null_mut()) };
        if status != 0 && status != FWP_E_ALREADY_EXISTS {
            return Err(BackendError::Wfp(format!(
                "FwpmProviderAdd0 failed: 0x{status:08x}"
            )));
        }

        let mut sublayer_name = to_wide_nul("Goodfetch Filter Broker Sublayer");
        let mut sublayer_desc = to_wide_nul("Persistent WFP sublayer for Goodfetch Plugins");
        let provider_key = GUID::from_u128(PROVIDER_GUID);
        // SAFETY: zero-initialized optional blob field for `FWPM_SUBLAYER0`.
        let empty_sublayer_data = unsafe { std::mem::zeroed() };
        let sublayer = FWPM_SUBLAYER0 {
            subLayerKey: GUID::from_u128(SUBLAYER_GUID),
            displayData: FWPM_DISPLAY_DATA0 {
                name: sublayer_name.as_mut_ptr(),
                description: sublayer_desc.as_mut_ptr(),
            },
            flags: FWPM_SUBLAYER_FLAG_PERSISTENT,
            providerKey: (&raw const provider_key).cast_mut(),
            providerData: empty_sublayer_data,
            weight: 0x8000,
        };
        // SAFETY: registers persistent WFP sublayer.
        let status = unsafe { FwpmSubLayerAdd0(self.engine, &raw const sublayer, ptr::null_mut()) };
        if status != 0 && status != FWP_E_ALREADY_EXISTS {
            return Err(BackendError::Wfp(format!(
                "FwpmSubLayerAdd0 failed: 0x{status:08x}"
            )));
        }
        Ok(())
    }

    fn add_persistent_filter(
        &mut self,
        app_container_name: &str,
        rule: &WfpRule,
    ) -> Result<u64, BackendError> {
        let sid_guard = derive_sid(app_container_name)?;
        // SAFETY: zero-initialized IPv4/IPv6 mask structs populated by `build_filter_conditions`.
        let mut v4_mask: FWP_V4_ADDR_AND_MASK = unsafe { std::mem::zeroed() };
        // SAFETY: zero-initialized IPv6 mask struct populated by `build_filter_conditions`.
        let mut v6_mask: FWP_V6_ADDR_AND_MASK = unsafe { std::mem::zeroed() };
        let mut conditions = build_filter_conditions(sid_guard.0, rule, &mut v4_mask, &mut v6_mask);

        let mut name_wide = to_wide_nul(&rule.tag);
        let mut desc_wide = to_wide_nul(&rule.rule_name);
        let provider_key = GUID::from_u128(PROVIDER_GUID);
        let layer_key = match rule.layer {
            FilterLayer::AleAuthConnectV4 => FWPM_LAYER_ALE_AUTH_CONNECT_V4,
            FilterLayer::AleAuthConnectV6 => FWPM_LAYER_ALE_AUTH_CONNECT_V6,
            FilterLayer::AleAuthRecvAcceptV4 => FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V4,
            FilterLayer::AleAuthRecvAcceptV6 => FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V6,
        };
        let action_type = match rule.action {
            FilterAction::Block => FWP_ACTION_BLOCK,
            FilterAction::Permit => FWP_ACTION_PERMIT,
        };

        // SAFETY: zero-initialized `FWPM_FILTER0` populated with valid pointers
        // living across `FwpmFilterAdd0`.
        let mut filter: FWPM_FILTER0 = unsafe { std::mem::zeroed() };
        filter.displayData = FWPM_DISPLAY_DATA0 {
            name: name_wide.as_mut_ptr(),
            description: desc_wide.as_mut_ptr(),
        };
        filter.flags = FWPM_FILTER_FLAG_PERSISTENT;
        filter.providerKey = (&raw const provider_key).cast_mut();
        filter.layerKey = layer_key;
        filter.subLayerKey = GUID::from_u128(SUBLAYER_GUID);
        filter.weight = FWP_VALUE0 {
            r#type: FWP_UINT8,
            Anonymous: FWP_VALUE0_0 { uint8: rule.weight },
        };
        filter.numFilterConditions =
            u32::try_from(conditions.len()).expect("invariant: filter condition count fits in u32");
        filter.filterCondition = conditions.as_mut_ptr();
        filter.action.r#type = action_type;

        let mut filter_id = 0_u64;
        // SAFETY: installs persistent WFP filter in an explicit transaction.
        let begin = unsafe { FwpmTransactionBegin0(self.engine, 0) };
        if begin != 0 {
            return Err(BackendError::Wfp(format!(
                "FwpmTransactionBegin0 failed: 0x{begin:08x}"
            )));
        }
        // SAFETY: all condition pointers (`v4_mask`, `v6_mask`, `sid_guard`,
        // `conditions`, `name_wide`, `desc_wide`) live until after commit.
        let add = unsafe {
            FwpmFilterAdd0(
                self.engine,
                &raw const filter,
                ptr::null_mut(),
                &raw mut filter_id,
            )
        };
        if add != 0 {
            // SAFETY: aborts open transaction on error.
            unsafe {
                let _ = FwpmTransactionAbort0(self.engine);
            }
            return Err(BackendError::Wfp(format!(
                "FwpmFilterAdd0({}) failed: 0x{add:08x}",
                rule.tag
            )));
        }
        // SAFETY: commits open WFP transaction.
        let commit = unsafe { FwpmTransactionCommit0(self.engine) };
        if commit != 0 {
            return Err(BackendError::Wfp(format!(
                "FwpmTransactionCommit0 failed: 0x{commit:08x}"
            )));
        }
        let _ = FWP_BYTE_ARRAY16 {
            byteArray16: [0; 16],
        };
        Ok(filter_id)
    }

    fn delete_filter(&mut self, filter_id: u64) -> Result<(), BackendError> {
        // SAFETY: deletes persistent WFP filter by its OS ID.
        let status = unsafe { FwpmFilterDeleteById0(self.engine, filter_id) };
        if status != 0 && status != FWP_E_FILTER_NOT_FOUND {
            return Err(BackendError::Wfp(format!(
                "FwpmFilterDeleteById0({filter_id}) failed: 0x{status:08x}"
            )));
        }
        Ok(())
    }

    fn list_owned_filters(&self) -> Result<Vec<InstalledFilter>, BackendError> {
        let provider_key = GUID::from_u128(PROVIDER_GUID);
        // SAFETY: zero-initialized enum template filtered by `PROVIDER_GUID`.
        let mut template: FWPM_FILTER_ENUM_TEMPLATE0 = unsafe { std::mem::zeroed() };
        template.providerKey = (&raw const provider_key).cast_mut();
        let mut enum_handle: HANDLE = ptr::null_mut();
        // SAFETY: creates filter enumeration handle scoped to our provider.
        let status = unsafe {
            FwpmFilterCreateEnumHandle0(self.engine, &raw const template, &raw mut enum_handle)
        };
        if status != 0 {
            return Err(BackendError::Wfp(format!(
                "FwpmFilterCreateEnumHandle0 failed: 0x{status:08x}"
            )));
        }
        let mut out = Vec::new();
        loop {
            let mut entries: *mut *mut FWPM_FILTER0 = ptr::null_mut();
            let mut returned = 0_u32;
            // SAFETY: enumerates up to 128 filters per batch.
            let status = unsafe {
                FwpmFilterEnum0(
                    self.engine,
                    enum_handle,
                    128,
                    &raw mut entries,
                    &raw mut returned,
                )
            };
            if status != 0 {
                // SAFETY: cleans up enum handle on error.
                unsafe {
                    let _ = FwpmFilterDestroyEnumHandle0(self.engine, enum_handle);
                }
                return Err(BackendError::Wfp(format!(
                    "FwpmFilterEnum0 failed: 0x{status:08x}"
                )));
            }
            if returned == 0 || entries.is_null() {
                break;
            }
            // SAFETY: `entries` holds `returned` valid `*mut FWPM_FILTER0` pointers.
            let slice = unsafe { std::slice::from_raw_parts(entries, returned as usize) };
            for &entry in slice {
                if !entry.is_null() {
                    // SAFETY: valid `FWPM_FILTER0` returned by `FwpmFilterEnum0`.
                    let (id, name_ptr) = unsafe { ((*entry).filterId, (*entry).displayData.name) };
                    out.push(InstalledFilter {
                        filter_id: id,
                        tag: wide_ptr_to_string(name_ptr),
                    });
                }
            }
            let mut raw = entries.cast::<core::ffi::c_void>();
            // SAFETY: frees the array allocated by `FwpmFilterEnum0`.
            unsafe {
                FwpmFreeMemory0(&raw mut raw);
            }
        }
        // SAFETY: destroys enumeration handle.
        unsafe {
            let _ = FwpmFilterDestroyEnumHandle0(self.engine, enum_handle);
        }
        Ok(out)
    }

    fn set_loopback_exemption(
        &mut self,
        app_container_name: &str,
        enabled: bool,
    ) -> Result<(), BackendError> {
        let guard = derive_sid(app_container_name)?;
        let target_sid = guard.0;

        let mut count = 0_u32;
        let mut sids_ptr: *mut SID_AND_ATTRIBUTES = ptr::null_mut();
        // SAFETY: reads current system AppContainer loopback exemption list.
        let status =
            unsafe { NetworkIsolationGetAppContainerConfig(&raw mut count, &raw mut sids_ptr) };
        if status != 0 {
            return Err(BackendError::Wfp(format!(
                "NetworkIsolationGetAppContainerConfig failed: {status}"
            )));
        }
        let mut next: Vec<SID_AND_ATTRIBUTES> = if count > 0 && !sids_ptr.is_null() {
            // SAFETY: `sids_ptr` has `count` elements from `NetworkIsolationGetAppContainerConfig`.
            unsafe { std::slice::from_raw_parts(sids_ptr, count as usize) }.to_vec()
        } else {
            Vec::new()
        };
        let mut already_present = false;
        next.retain(|entry| {
            // SAFETY: compares two valid Windows SIDs.
            let eq = unsafe { EqualSid(entry.Sid, target_sid) != FALSE };
            if eq {
                already_present = true;
                enabled
            } else {
                true
            }
        });
        if enabled && !already_present {
            next.push(SID_AND_ATTRIBUTES {
                Sid: target_sid,
                Attributes: 0,
            });
        }
        // SAFETY: updates system AppContainer loopback exemption list.
        let set_status = unsafe {
            NetworkIsolationSetAppContainerConfig(
                u32::try_from(next.len()).expect("invariant: loopback SID count fits in u32"),
                next.as_ptr(),
            )
        };
        if !sids_ptr.is_null() {
            // SAFETY: frees memory returned by `NetworkIsolationGetAppContainerConfig`.
            unsafe {
                LocalFree(sids_ptr.cast());
            }
        }
        if set_status != 0 {
            return Err(BackendError::Wfp(format!(
                "NetworkIsolationSetAppContainerConfig failed: {set_status}"
            )));
        }
        Ok(())
    }

    fn remove_provider_and_sublayer(&mut self) -> Result<(), BackendError> {
        let sublayer_key = GUID::from_u128(SUBLAYER_GUID);
        // SAFETY: deletes broker's persistent WFP sublayer.
        let s1 = unsafe { FwpmSubLayerDeleteByKey0(self.engine, &raw const sublayer_key) };
        if s1 != 0 && s1 != FWP_E_SUBLAYER_NOT_FOUND {
            return Err(BackendError::Wfp(format!(
                "FwpmSubLayerDeleteByKey0 failed: 0x{s1:08x}"
            )));
        }
        let provider_key = GUID::from_u128(PROVIDER_GUID);
        // SAFETY: deletes broker's persistent WFP provider.
        let s2 = unsafe { FwpmProviderDeleteByKey0(self.engine, &raw const provider_key) };
        if s2 != 0 && s2 != FWP_E_PROVIDER_NOT_FOUND {
            return Err(BackendError::Wfp(format!(
                "FwpmProviderDeleteByKey0 failed: 0x{s2:08x}"
            )));
        }
        Ok(())
    }
}

/// Real Win32 Job Object controller and live `AppContainer` scanner.
#[derive(Default)]
pub struct Win32JobController {
    next_token: u64,
    handles: BTreeMap<u64, HANDLE>,
}

// SAFETY: Win32 Job Object handles are process-wide kernel handles safe to
// transfer across threads when serialized by `Mutex`.
unsafe impl Send for Win32JobController {}

impl Drop for Win32JobController {
    fn drop(&mut self) {
        for handle in self.handles.values() {
            if !handle.is_null() && *handle != INVALID_HANDLE_VALUE {
                // SAFETY: closes duplicated Job Object handle owned by broker.
                unsafe {
                    CloseHandle(*handle);
                }
            }
        }
    }
}

impl JobController for Win32JobController {
    fn attach_job(&mut self, caller_pid: u32, job_handle: u64) -> Result<u64, BackendError> {
        // SAFETY: opens caller engine process solely to duplicate its Job Object handle.
        let source_proc = unsafe { OpenProcess(PROCESS_DUP_HANDLE, FALSE, caller_pid) };
        if source_proc.is_null() {
            return Err(BackendError::Job(format!(
                "OpenProcess(pid={caller_pid}) failed: {}",
                io::Error::last_os_error()
            )));
        }
        let mut duplicated: HANDLE = ptr::null_mut();
        let src_handle = usize::try_from(job_handle)
            .map_err(|_| BackendError::Job("job handle exceeds pointer width".to_owned()))?
            as HANDLE;
        // SAFETY: duplicates `job_handle` from `caller_pid` into broker process.
        let ok = unsafe {
            DuplicateHandle(
                source_proc,
                src_handle,
                GetCurrentProcess(),
                &raw mut duplicated,
                JOB_OBJECT_TERMINATE | JOB_OBJECT_QUERY | SYNCHRONIZE,
                FALSE,
                0,
            )
        };
        // SAFETY: closes source process handle.
        unsafe {
            CloseHandle(source_proc);
        }
        if ok == FALSE || duplicated.is_null() {
            return Err(BackendError::Job(format!(
                "DuplicateHandle(pid={caller_pid}) failed: {}",
                io::Error::last_os_error()
            )));
        }
        self.next_token = self.next_token.wrapping_add(1);
        let token = self.next_token;
        self.handles.insert(token, duplicated);
        Ok(token)
    }

    fn terminate_job(&mut self, broker_job: u64) -> Result<(), BackendError> {
        let Some(&handle) = self.handles.get(&broker_job) else {
            return Err(BackendError::Job(format!("unknown job token {broker_job}")));
        };
        // SAFETY: terminates all processes inside the Job Object.
        if unsafe { TerminateJobObject(handle, 1) } == FALSE {
            return Err(BackendError::Job(format!(
                "TerminateJobObject failed: {}",
                io::Error::last_os_error()
            )));
        }
        // Poll briefly until the kernel reports zero active processes.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.active_processes(broker_job)? == 0 {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn active_processes(&self, broker_job: u64) -> Result<u32, BackendError> {
        let Some(&handle) = self.handles.get(&broker_job) else {
            return Ok(0);
        };
        // SAFETY: zero-initialized output struct for `QueryInformationJobObject`.
        let mut buf: PidList = unsafe { std::mem::zeroed() };
        let mut returned = 0_u32;
        // SAFETY: queries assigned processes of the live Job Object.
        let ok = unsafe {
            QueryInformationJobObject(
                handle,
                JobObjectBasicProcessIdList,
                (&raw mut buf).cast(),
                u32::try_from(size_of::<PidList>()).expect("invariant: PidList size fits in u32"),
                &raw mut returned,
            )
        };
        if ok == FALSE {
            return Err(BackendError::Job(format!(
                "QueryInformationJobObject failed: {}",
                io::Error::last_os_error()
            )));
        }
        Ok(buf.header.NumberOfAssignedProcesses)
    }

    fn close_job(&mut self, broker_job: u64) {
        if let Some(handle) = self.handles.remove(&broker_job)
            && !handle.is_null()
            && handle != INVALID_HANDLE_VALUE
        {
            // SAFETY: closes duplicated Job Object handle once.
            unsafe {
                CloseHandle(handle);
            }
        }
    }

    fn has_live_appcontainer(&self, app_container_name: &str) -> bool {
        let Ok(guard) = derive_sid(app_container_name) else {
            return false;
        };
        let target_sid = guard.0;

        let mut pids = vec![0_u32; 2048];
        let mut bytes_returned = 0_u32;
        // SAFETY: enumerates running PIDs on the host.
        let ok = unsafe {
            EnumProcesses(
                pids.as_mut_ptr(),
                u32::try_from(pids.len() * size_of::<u32>())
                    .expect("invariant: pid buffer byte length fits in u32"),
                &raw mut bytes_returned,
            )
        };
        if ok == FALSE {
            return false;
        }
        let count = (bytes_returned as usize) / size_of::<u32>();
        for &pid in &pids[..count.min(pids.len())] {
            if pid == 0 {
                continue;
            }
            // SAFETY: queries process token to check `TokenAppContainerSid`.
            let proc_handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
            if proc_handle.is_null() {
                continue;
            }
            let mut token: HANDLE = ptr::null_mut();
            // SAFETY: opens process token for `TokenAppContainerSid` query.
            let opened = unsafe { OpenProcessToken(proc_handle, TOKEN_QUERY, &raw mut token) };
            // SAFETY: closes process handle immediately after token query.
            unsafe {
                CloseHandle(proc_handle);
            }
            if opened == FALSE || token.is_null() {
                continue;
            }
            let mut buf = [0_u64; 32];
            let mut len = 0_u32;
            // SAFETY: `buf` is 8-byte aligned and sized for `TOKEN_APPCONTAINER_INFORMATION`.
            let got = unsafe {
                GetTokenInformation(
                    token,
                    TokenAppContainerSid,
                    buf.as_mut_ptr().cast(),
                    u32::try_from(std::mem::size_of_val(&buf))
                        .expect("invariant: token buffer fits in u32"),
                    &raw mut len,
                )
            };
            // SAFETY: closes process token handle.
            unsafe {
                CloseHandle(token);
            }
            if got != FALSE {
                // SAFETY: `buf` is 8-byte aligned and holds `TOKEN_APPCONTAINER_INFORMATION`.
                let info = unsafe { &*(buf.as_ptr().cast::<TOKEN_APPCONTAINER_INFORMATION>()) };
                if !info.TokenAppContainer.is_null() {
                    // SAFETY: compares two valid Windows SIDs.
                    let same = unsafe { EqualSid(info.TokenAppContainer, target_sid) != FALSE };
                    if same {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Real Windows Event Log sink (`RegisterEventSourceW` / `ReportEventW`).
pub struct Win32EventLog {
    handle: HANDLE,
}

// SAFETY: Win32 Event Log source handles (`RegisterEventSourceW`) are safe to
// transfer across threads when serialized by `Mutex`.
unsafe impl Send for Win32EventLog {}

impl Win32EventLog {
    /// Registers the `GoodfetchFilterBroker` Windows Event Log source.
    #[must_use]
    pub fn open() -> Self {
        let source = to_wide_nul(BROKER_EVENT_SOURCE);
        // SAFETY: opens local Windows Event Log source handle.
        let handle = unsafe { RegisterEventSourceW(ptr::null(), source.as_ptr()) };
        Self { handle }
    }
}

impl Drop for Win32EventLog {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: paired with `RegisterEventSourceW`.
            unsafe {
                let _ = DeregisterEventSource(self.handle);
            }
        }
    }
}

impl EventLogSink for Win32EventLog {
    fn write(&mut self, event: BrokerEvent) {
        if self.handle.is_null() {
            return;
        }
        let message = to_wide_nul(&event.to_string());
        let strings = [message.as_ptr()];
        let severity = match event.severity() {
            EventSeverity::Information => EVENTLOG_INFORMATION_TYPE,
            EventSeverity::Warning => EVENTLOG_WARNING_TYPE,
        };
        // SAFETY: `strings` and `message` stay live across `ReportEventW`.
        unsafe {
            let _ = ReportEventW(
                self.handle,
                severity,
                0,
                event.event_id(),
                ptr::null_mut(),
                1,
                0,
                strings.as_ptr(),
                ptr::null(),
            );
        }
    }
}

/// Real caller executable digest verifier reading the peer process's on-disk
/// executable via `QueryFullProcessImageNameW`.
#[derive(Default)]
pub struct Win32CallerVerifier;

impl CallerVerifier for Win32CallerVerifier {
    fn executable_digest(&self, caller_pid: u32) -> Result<String, BackendError> {
        // SAFETY: opens peer process for image-path query only.
        let proc_handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, caller_pid) };
        if proc_handle.is_null() {
            return Err(BackendError::Caller(format!(
                "OpenProcess(pid={caller_pid}) failed: {}",
                io::Error::last_os_error()
            )));
        }
        let mut buf = vec![0_u16; (MAX_PATH as usize) * 4];
        let mut len = u32::try_from(buf.len()).expect("invariant: path buffer fits in u32");
        // SAFETY: queries full Win32 executable path of `caller_pid`.
        let ok =
            unsafe { QueryFullProcessImageNameW(proc_handle, 0, buf.as_mut_ptr(), &raw mut len) };
        // SAFETY: closes process handle immediately.
        unsafe {
            CloseHandle(proc_handle);
        }
        if ok == FALSE {
            return Err(BackendError::Caller(format!(
                "QueryFullProcessImageNameW(pid={caller_pid}) failed: {}",
                io::Error::last_os_error()
            )));
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let bytes = std::fs::read(&path).map_err(|error| {
            BackendError::Caller(format!("cannot read caller executable `{path}`: {error}"))
        })?;
        Ok(sha256_digest(&bytes))
    }
}

/// Resolves the current process user's Windows SID string (`S-1-5-...`) for
/// building the named-pipe SDDL DACL.
///
/// # Errors
///
/// Returns an `io::Error` if the current token or SID cannot be queried.
pub fn current_user_sid() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: opens calling process token for query.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    let mut buf = [0_u64; 32];
    let mut len = 0_u32;
    // SAFETY: reads `TOKEN_USER` into 8-byte-aligned `buf`.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr().cast(),
            u32::try_from(std::mem::size_of_val(&buf)).expect("invariant: buffer size fits in u32"),
            &raw mut len,
        )
    };
    // SAFETY: closes token handle.
    unsafe {
        CloseHandle(token);
    }
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `buf` is 8-byte aligned and holds `TOKEN_USER` populated by `GetTokenInformation`.
    let user = unsafe { &*(buf.as_ptr().cast::<TOKEN_USER>()) };
    let mut str_sid: *mut u16 = ptr::null_mut();
    // SAFETY: formats SID into a heap-allocated wide string.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &raw mut str_sid) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    let result = wide_ptr_to_string(str_sid);
    // SAFETY: frees string allocated by `ConvertSidToStringSidW`.
    unsafe {
        LocalFree(str_sid.cast());
    }
    Ok(result)
}

/// Serves the named pipe at `pipe_name` (restricted by `allowed_user_sid`'s
/// SDDL DACL) until `stop_flag` becomes `true`, reconciling orphan filters on
/// startup and coordinating engine drain on stop.
///
/// # Errors
///
/// Returns an error string on fatal WFP, SDDL, or named-pipe failure.
pub fn serve_pipe_loop(
    pipe_name: &str,
    allowed_user_sid: &str,
    trust_root: AttestationTrustRoot,
    stop_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    let sddl = pipe_sddl_for_user_sid(allowed_user_sid).map_err(|e| e.to_string())?;
    let sddl_wide = to_wide_nul(&sddl);
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: parses protected SDDL restricting pipe access to installing user + SYSTEM + BA.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_wide.as_ptr(),
            SDDL_REVISION_1,
            &raw mut sd,
            ptr::null_mut(),
        )
    } == FALSE
    {
        return Err(format!(
            "ConvertStringSecurityDescriptorToSecurityDescriptorW failed: {}",
            io::Error::last_os_error()
        ));
    }
    let _sd_guard = SdGuard(sd);

    let wfp = Win32WfpStore::open().map_err(|e| e.to_string())?;
    let broker = Arc::new(Mutex::new(FilterBroker::new(
        wfp,
        Win32JobController::default(),
        Win32EventLog::open(),
        Win32CallerVerifier,
        trust_root,
        release_signer(),
    )));
    {
        let mut guard = broker
            .lock()
            .map_err(|_| "broker mutex poisoned".to_owned())?;
        guard
            .start_and_reconcile_orphans()
            .map_err(|e| e.to_string())?;
    }

    let pipe_wide = to_wide_nul(pipe_name);
    let mut first = true;
    while !stop_flag.load(Ordering::Acquire) {
        let sa = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
                .expect("invariant: SECURITY_ATTRIBUTES fits in u32"),
            lpSecurityDescriptor: sd,
            bInheritHandle: FALSE,
        };
        let open_mode = if first {
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            PIPE_ACCESS_DUPLEX
        };
        first = false;
        // SAFETY: creates one duplex byte-mode named pipe instance rejecting remote clients.
        let pipe = unsafe {
            CreateNamedPipeW(
                pipe_wide.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                &raw const sa,
            )
        };
        if pipe == INVALID_HANDLE_VALUE || pipe.is_null() {
            return Err(format!(
                "CreateNamedPipeW({pipe_name}) failed: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: waits for one client connection.
        let connected = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) };
        if connected == FALSE {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(ERROR_PIPE_CONNECTED.cast_signed()) {
                // SAFETY: closes unconnected pipe handle on error.
                unsafe {
                    CloseHandle(pipe);
                }
                continue;
            }
        }
        let broker_ref = Arc::clone(&broker);
        let stop_ref = Arc::clone(stop_flag);
        let pipe_raw = pipe as usize;
        let _ = std::thread::spawn(move || {
            handle_pipe_client(pipe_raw as HANDLE, &broker_ref, &stop_ref);
        });
    }

    if let Ok(mut guard) = broker.lock() {
        let _ = guard.begin_service_stop();
        let _ = guard.finish_service_stop();
    }
    Ok(())
}

fn handle_pipe_client(
    pipe: HANDLE,
    broker: &Arc<
        Mutex<FilterBroker<Win32WfpStore, Win32JobController, Win32EventLog, Win32CallerVerifier>>,
    >,
    stop_flag: &Arc<AtomicBool>,
) {
    let mut caller_pid = 0_u32;
    // SAFETY: reads connected client's PID from the live named pipe handle.
    let got_pid = unsafe { GetNamedPipeClientProcessId(pipe, &raw mut caller_pid) != FALSE };
    if !got_pid {
        // SAFETY: disconnects and closes pipe when client PID could not be read.
        unsafe {
            DisconnectNamedPipe(pipe);
            CloseHandle(pipe);
        }
        return;
    }
    // SAFETY: takes exclusive ownership of `pipe` for buffered line I/O.
    let mut file = unsafe { std::fs::File::from_raw_handle(pipe.cast()) };
    let mut authenticated_engine: Option<String> = None;
    let mut line = String::new();
    loop {
        if stop_flag.load(Ordering::Acquire) {
            if let Ok(notice) = serde_json::to_string(&BrokerResponse::ServiceStopping) {
                let _ = writeln!(file, "{notice}");
            }
            break;
        }
        line.clear();
        let read_res = BufReader::new(&file).read_line(&mut line);
        match read_res {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let response = match serde_json::from_str::<BrokerRequest>(trimmed) {
                    Ok(request) => {
                        let Ok(mut guard) = broker.lock() else {
                            break;
                        };
                        let resp = guard.handle_request(
                            caller_pid,
                            authenticated_engine.as_deref(),
                            request,
                        );
                        if let BrokerResponse::Authenticated { ref engine_id } = resp {
                            authenticated_engine = Some(engine_id.clone());
                        }
                        resp
                    }
                    Err(error) => BrokerResponse::Error {
                        code: "malformed_request".to_owned(),
                        message: error.to_string(),
                    },
                };
                if let Ok(encoded) = serde_json::to_string(&response)
                    && writeln!(file, "{encoded}").is_err()
                {
                    break;
                }
            }
            Err(error) if error.raw_os_error() == Some(ERROR_BROKEN_PIPE.cast_signed()) => break,
            Err(_) => break,
        }
    }

    // Engine disconnected or exited: terminate its tracked Job Objects and
    // remove its persistent WFP filters once empty.
    if let Some(engine_id) = authenticated_engine
        && let Ok(mut guard) = broker.lock()
    {
        let _ = guard.on_engine_exit(&engine_id);
    }
}

/// Registers and starts the `GoodfetchFilterBroker` Windows service pointing
/// at `binary_path` with `--user-sid <installing_user_sid>`.
///
/// # Errors
///
/// Returns an error message if Service Control Manager registration fails.
pub fn install_windows_service(
    binary_path: &Path,
    installing_user_sid: &str,
) -> Result<(), String> {
    let _ = pipe_sddl_for_user_sid(installing_user_sid).map_err(|e| e.to_string())?;
    // SAFETY: opens local Service Control Manager with full access (requires elevation).
    let scm = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_ALL_ACCESS) };
    if scm.is_null() {
        return Err(format!(
            "OpenSCManagerW failed (run `plugin setup` elevated): {}",
            io::Error::last_os_error()
        ));
    }
    let service_name = to_wide_nul(BROKER_SERVICE_NAME);
    let display_name = to_wide_nul(BROKER_DISPLAY_NAME);
    let cmd = format!(
        "\"{}\" --service --user-sid {}",
        binary_path.display(),
        installing_user_sid
    );
    let cmd_wide = to_wide_nul(&cmd);
    // SAFETY: creates or opens the Windows service.
    let mut service: SC_HANDLE = unsafe {
        CreateServiceW(
            scm,
            service_name.as_ptr(),
            display_name.as_ptr(),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            cmd_wide.as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
        )
    };
    if service.is_null() {
        // SAFETY: opens existing service entry when `CreateServiceW` saw a duplicate.
        service = unsafe { OpenServiceW(scm, service_name.as_ptr(), SERVICE_ALL_ACCESS) };
    }
    if service.is_null() {
        let err = io::Error::last_os_error();
        // SAFETY: closes SCM handle before returning error.
        unsafe {
            CloseServiceHandle(scm);
        }
        return Err(format!("CreateServiceW/OpenServiceW failed: {err}"));
    }
    // SAFETY: starts the service if not already running.
    let _ = unsafe { StartServiceW(service, 0, ptr::null()) };
    // SAFETY: closes service and SCM handles.
    unsafe {
        CloseServiceHandle(service);
        CloseServiceHandle(scm);
    }
    Ok(())
}

/// Stops the `GoodfetchFilterBroker` Windows service (if registered), deletes
/// EVERY persistent WFP filter under `PROVIDER_GUID` (logging each to the
/// Windows Event Log), and deletes the Windows service entry.
///
/// # Errors
///
/// Returns an error message if WFP cleanup fails.
pub fn uninstall_broker() -> Result<usize, String> {
    // 1. Stop and delete Windows service if present.
    // SAFETY: opens local SCM to stop and delete the service.
    let scm = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_ALL_ACCESS) };
    if !scm.is_null() {
        let service_name = to_wide_nul(BROKER_SERVICE_NAME);
        // SAFETY: opens service if installed.
        let service = unsafe { OpenServiceW(scm, service_name.as_ptr(), SERVICE_ALL_ACCESS) };
        if !service.is_null() {
            // SAFETY: zero-initialized `SERVICE_STATUS` output.
            let mut status: SERVICE_STATUS = unsafe { std::mem::zeroed() };
            // SAFETY: stops and deletes the installed service handle, then closes it.
            unsafe {
                let _ = ControlService(service, SERVICE_CONTROL_STOP, &raw mut status);
                let _ = DeleteService(service);
                CloseServiceHandle(service);
            }
        }
        // SAFETY: closes SCM handle.
        unsafe {
            CloseServiceHandle(scm);
        }
    }

    // 2. Remove every persistent WFP filter + sublayer + provider and log each.
    let wfp = Win32WfpStore::open().map_err(|e| e.to_string())?;
    let mut broker = FilterBroker::new(
        wfp,
        Win32JobController::default(),
        Win32EventLog::open(),
        Win32CallerVerifier,
        AttestationTrustRoot::bundled(),
        release_signer(),
    );
    let report = broker.uninstall_all_filters().map_err(|e| e.to_string())?;
    Ok(report.removed_filter_ids.len())
}

static SERVICE_STOP: AtomicBool = AtomicBool::new(false);
static SERVICE_USER_SID: Mutex<Option<String>> = Mutex::new(None);
static STATUS_HANDLE: Mutex<usize> = Mutex::new(0);

/// Runs under the Windows Service Control Manager (`StartServiceCtrlDispatcherW`).
///
/// # Errors
///
/// Returns an error message if SCM dispatch fails.
pub fn run_windows_service(user_sid: String) -> Result<(), String> {
    if let Ok(mut slot) = SERVICE_USER_SID.lock() {
        *slot = Some(user_sid);
    }
    let mut name = to_wide_nul(BROKER_SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: ptr::null_mut(),
            lpServiceProc: None,
        },
    ];
    // SAFETY: registers service main entrypoint with Windows SCM.
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == FALSE {
        return Err(format!(
            "StartServiceCtrlDispatcherW failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

unsafe extern "system" fn service_ctrl_handler(
    control: u32,
    _event_type: u32,
    _event_data: *mut core::ffi::c_void,
    _context: *mut core::ffi::c_void,
) -> u32 {
    if control == SERVICE_CONTROL_STOP {
        SERVICE_STOP.store(true, Ordering::Release);
        set_scm_state(SERVICE_STOP_PENDING);
    }
    0
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
    let name = to_wide_nul(BROKER_SERVICE_NAME);
    // SAFETY: registers SCM control handler for stop notifications.
    let handle = unsafe {
        RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(service_ctrl_handler), ptr::null())
    };
    if let Ok(mut slot) = STATUS_HANDLE.lock() {
        *slot = handle as usize;
    }
    set_scm_state(SERVICE_START_PENDING);
    let user_sid = SERVICE_USER_SID
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .or_else(|| current_user_sid().ok())
        .unwrap_or_default();
    set_scm_state(SERVICE_RUNNING);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_watcher = Arc::clone(&stop);
    let _ = std::thread::spawn(move || {
        while !SERVICE_STOP.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(50));
        }
        stop_watcher.store(true, Ordering::Release);
    });
    let _ = serve_pipe_loop(
        DEFAULT_PIPE_NAME,
        &user_sid,
        AttestationTrustRoot::bundled(),
        &stop,
    );
    set_scm_state(SERVICE_STOPPED);
}

fn set_scm_state(state: u32) {
    let handle = STATUS_HANDLE
        .lock()
        .map_or(ptr::null_mut(), |slot| *slot as SERVICE_STATUS_HANDLE);
    if handle.is_null() {
        return;
    }
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP
        } else {
            0
        },
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 5_000,
    };
    // SAFETY: reports current service state to Windows SCM.
    unsafe {
        let _ = SetServiceStatus(handle, &raw const status);
    }
}
