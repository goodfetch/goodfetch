//! The one `unsafe` Windows confinement module (ADR 0033, STYLE.md, issue 873):
//! RAII wrappers around Win32 `AppContainer` profiles, restricted tokens,
//! alternate desktops, NTFS ACL grants/revocations, Job Objects with I/O
//! Completion Ports, and `CreateProcessAsUserW` with extended process-thread
//! attributes (`PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` and
//! `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`).
//!
//! Size: one concern — isolating every Windows `unsafe` Win32 confinement call
//! in a single auditable module (`win32_is_the_only_windows_module_that_opts_into_unsafe`).

#![expect(
    unsafe_code,
    reason = "Windows confinement calls Win32 AppContainer, restricted token, Job Object, desktop and ACL APIs via windows-sys (ADR 0033)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the launcher runs single-threaded outside any async worker (ADR 0033)"
)]

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use goodfetch_protocol::executables::is_smart_app_control_os_error;
use goodfetch_protocol::plugin::{Limits, PluginResourceLimit};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FALSE, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    LocalFree, SetHandleInformation, TRUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, GetSecurityInfo, REVOKE_ACCESS,
    SE_FILE_OBJECT, SE_WINDOW_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, SetSecurityInfo,
    TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    ACL, CreateRestrictedToken, DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE, FreeSid,
    GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_CAPABILITIES,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_ALL_ACCESS, TOKEN_QUERY, TokenIsAppContainer,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::IO::{CreateIoCompletionPort, GetQueuedCompletionStatus};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_LIMIT_JOB_TIME, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP,
    JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectAssociateCompletionPortInformation, JobObjectBasicProcessIdList,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, DESKTOP_CREATEMENU, DESKTOP_CREATEWINDOW, DESKTOP_READOBJECTS,
    DESKTOP_WRITEOBJECTS, HDESK,
};
use windows_sys::Win32::System::SystemServices::JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, GetProcessHandleCount, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess, OpenProcessToken, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::{ConfineError, ProcessSpec, classify_job_notification, cpu_time_to_100ns};

static INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(1);

const COMPLETION_KEY: usize = 0x6766_4a4f; // "gfJO"
const POLL_TIMEOUT_MS: u32 = 50;

fn to_wide_nul(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

fn path_to_wide_nul(path: &Path) -> Vec<u16> {
    // Strip `\\?\` extended prefix when canonicalized so NTFS ACL APIs and
    // working-directory calls accept the path uniformly.
    let display = path.as_os_str().to_string_lossy();
    let stripped = display.strip_prefix(r"\\?\").unwrap_or(&display);
    OsStr::new(stripped)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE) -> io::Result<Self> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: handle is owned by this RAII guard and closed once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

struct AppContainerProfile {
    name_wide: Vec<u16>,
    sid: PSID,
    owned_profile: bool,
}

impl AppContainerProfile {
    fn create_unique() -> Result<Self, ConfineError> {
        let seq = INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("goodfetch.plugin.{}.{seq}", std::process::id());
        let name_wide = to_wide_nul(OsStr::new(&name));
        let display = to_wide_nul(OsStr::new("Goodfetch Plugin"));
        let desc = to_wide_nul(OsStr::new("Goodfetch confined Plugin instance"));
        let mut sid: PSID = ptr::null_mut();
        // SAFETY: wide strings are NUL-terminated and live across the call;
        // zero capabilities at `none`.
        let hr = unsafe {
            CreateAppContainerProfile(
                name_wide.as_ptr(),
                display.as_ptr(),
                desc.as_ptr(),
                ptr::null(),
                0,
                &raw mut sid,
            )
        };
        if hr < 0 {
            // `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)` = 0x8007_00B7.
            let already_exists = (hr as u32) == (0x8007_0000 | ERROR_ALREADY_EXISTS);
            if already_exists {
                // SAFETY: derives SID for existing profile name.
                let hr2 = unsafe {
                    DeriveAppContainerSidFromAppContainerName(name_wide.as_ptr(), &raw mut sid)
                };
                if hr2 < 0 || sid.is_null() {
                    return Err(ConfineError::Unavailable(format!(
                        "DeriveAppContainerSidFromAppContainerName failed: HRESULT 0x{:08x}",
                        hr2 as u32
                    )));
                }
            } else {
                return Err(ConfineError::Unavailable(format!(
                    "CreateAppContainerProfile failed: HRESULT 0x{:08x}",
                    hr as u32
                )));
            }
        }
        if sid.is_null() {
            return Err(ConfineError::Unavailable(
                "CreateAppContainerProfile returned a null SID".to_owned(),
            ));
        }
        Ok(Self {
            name_wide,
            sid,
            owned_profile: true,
        })
    }

    fn open_existing(name: &str) -> Result<Self, ConfineError> {
        let name_wide = to_wide_nul(OsStr::new(name));
        let mut sid: PSID = ptr::null_mut();
        // SAFETY: `name_wide` is NUL-terminated.
        let hr =
            unsafe { DeriveAppContainerSidFromAppContainerName(name_wide.as_ptr(), &raw mut sid) };
        if hr < 0 || sid.is_null() {
            return Err(ConfineError::Unavailable(format!(
                "DeriveAppContainerSidFromAppContainerName failed: HRESULT 0x{:08x}",
                hr as u32
            )));
        }
        Ok(Self {
            name_wide,
            sid,
            owned_profile: false,
        })
    }

    fn sid(&self) -> PSID {
        self.sid
    }
}

impl Drop for AppContainerProfile {
    fn drop(&mut self) {
        if !self.sid.is_null() {
            // SAFETY: SID was allocated by `CreateAppContainerProfile` or
            // `DeriveAppContainerSidFromAppContainerName`.
            unsafe {
                FreeSid(self.sid);
            }
        }
        if self.owned_profile {
            // SAFETY: deletes the per-instance profile we created.
            unsafe {
                let _ = DeleteAppContainerProfile(self.name_wide.as_ptr());
            }
        }
    }
}

struct AlternateDesktop {
    name_wide: Vec<u16>,
    handle: HDESK,
}

impl AlternateDesktop {
    fn create_unique(app_container_sid: PSID) -> Result<Self, ConfineError> {
        let seq = INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("goodfetch_desk_{}_{seq}", std::process::id());
        let name_wide = to_wide_nul(OsStr::new(&name));
        let desired_access = DESKTOP_READOBJECTS
            | DESKTOP_WRITEOBJECTS
            | DESKTOP_CREATEWINDOW
            | DESKTOP_CREATEMENU
            | 0x000F_0000; // STANDARD_RIGHTS_REQUIRED
        // SAFETY: `name_wide` is NUL-terminated; creates an empty non-interactive
        // desktop on the current window station.
        let handle = unsafe {
            CreateDesktopW(
                name_wide.as_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                desired_access,
                ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(ConfineError::Confine(format!(
                "CreateDesktopW failed: {}",
                io::Error::last_os_error()
            )));
        }
        let desktop = Self { name_wide, handle };
        grant_handle_access(
            handle,
            SE_WINDOW_OBJECT,
            app_container_sid,
            DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS | DESKTOP_CREATEWINDOW | DESKTOP_CREATEMENU,
        )?;
        Ok(desktop)
    }

    fn lp_desktop(&mut self) -> *mut u16 {
        self.name_wide.as_mut_ptr()
    }
}

impl Drop for AlternateDesktop {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: closes the alternate desktop handle we created.
            unsafe {
                CloseDesktop(self.handle);
            }
        }
    }
}

fn grant_handle_access(
    handle: HANDLE,
    object_type: i32,
    sid: PSID,
    access: u32,
) -> Result<(), ConfineError> {
    let mut old_dacl: *mut ACL = ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: reads existing DACL on the live handle.
    let err = unsafe {
        GetSecurityInfo(
            handle,
            object_type,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut old_dacl,
            ptr::null_mut(),
            &raw mut sd,
        )
    };
    if err != 0 {
        return Err(ConfineError::Confine(format!(
            "GetSecurityInfo failed: win32 error {err}"
        )));
    }
    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: access,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.cast(),
        },
    };
    let mut new_dacl: *mut ACL = ptr::null_mut();
    // SAFETY: builds updated ACL with one explicit ACE for `sid`.
    let err = unsafe { SetEntriesInAclW(1, &raw const ea, old_dacl, &raw mut new_dacl) };
    if err != 0 {
        // SAFETY: frees security descriptor allocated by `GetSecurityInfo`.
        unsafe {
            LocalFree(sd);
        }
        return Err(ConfineError::Confine(format!(
            "SetEntriesInAclW failed: win32 error {err}"
        )));
    }
    // SAFETY: installs updated DACL on handle.
    let set_err = unsafe {
        SetSecurityInfo(
            handle,
            object_type,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            new_dacl,
            ptr::null_mut(),
        )
    };
    // SAFETY: frees ACL and security descriptor allocated by Win32.
    unsafe {
        LocalFree(new_dacl.cast());
        LocalFree(sd);
    }
    if set_err != 0 {
        return Err(ConfineError::Confine(format!(
            "SetSecurityInfo failed: win32 error {set_err}"
        )));
    }
    Ok(())
}

/// Tracks every path granted an AppContainer ACE and revokes it on drop.
struct AclGrants {
    sid: PSID,
    paths: Vec<PathBuf>,
}

impl AclGrants {
    fn new(sid: PSID) -> Self {
        Self {
            sid,
            paths: Vec::new(),
        }
    }

    fn grant(&mut self, path: &Path, access: u32) -> Result<(), ConfineError> {
        if !path.exists() {
            return Ok(());
        }
        apply_path_ace(path, self.sid, access, GRANT_ACCESS)?;
        self.paths.push(path.to_path_buf());
        Ok(())
    }
}

impl Drop for AclGrants {
    fn drop(&mut self) {
        for path in self.paths.drain(..).rev() {
            let _ = apply_path_ace(&path, self.sid, 0, REVOKE_ACCESS);
        }
    }
}

fn apply_path_ace(path: &Path, sid: PSID, access: u32, mode: i32) -> Result<(), ConfineError> {
    let mut wide = path_to_wide_nul(path);
    let mut old_dacl: *mut ACL = ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: reads existing DACL for the file or folder.
    let err = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut old_dacl,
            ptr::null_mut(),
            &raw mut sd,
        )
    };
    if err != 0 {
        return Err(ConfineError::Confine(format!(
            "GetNamedSecurityInfoW({}) failed: win32 error {err}",
            path.display()
        )));
    }
    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: access,
        grfAccessMode: mode,
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.cast(),
        },
    };
    let mut new_dacl: *mut ACL = ptr::null_mut();
    // SAFETY: modifies DACL copy with our explicit entry.
    let err = unsafe { SetEntriesInAclW(1, &raw const ea, old_dacl, &raw mut new_dacl) };
    if err != 0 {
        // SAFETY: frees security descriptor from `GetNamedSecurityInfoW`.
        unsafe {
            LocalFree(sd);
        }
        return Err(ConfineError::Confine(format!(
            "SetEntriesInAclW({}) failed: win32 error {err}",
            path.display()
        )));
    }
    // SAFETY: applies updated DACL to the file or directory.
    let set_err = unsafe {
        SetNamedSecurityInfoW(
            wide.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            new_dacl,
            ptr::null_mut(),
        )
    };
    // SAFETY: frees Win32 allocations.
    unsafe {
        LocalFree(new_dacl.cast());
        LocalFree(sd);
    }
    if set_err != 0 {
        return Err(ConfineError::Confine(format!(
            "SetNamedSecurityInfoW({}) failed: win32 error {set_err}",
            path.display()
        )));
    }
    Ok(())
}

fn create_restricted_token() -> Result<OwnedHandle, ConfineError> {
    let mut current: HANDLE = ptr::null_mut();
    // SAFETY: opens the calling process's primary token.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &raw mut current) } == FALSE
    {
        return Err(ConfineError::Confine(format!(
            "OpenProcessToken failed: {}",
            io::Error::last_os_error()
        )));
    }
    let current = OwnedHandle::new(current).map_err(|e| ConfineError::Confine(e.to_string()))?;
    let mut restricted: HANDLE = ptr::null_mut();
    // SAFETY: creates a restricted token with `DISABLE_MAX_PRIVILEGE` so every
    // privilege is stripped while preserving the user SID needed by the kernel
    // to synthesize the AppContainer lowbox token during `CreateProcessAsUserW`.
    let ok = unsafe {
        CreateRestrictedToken(
            current.raw(),
            DISABLE_MAX_PRIVILEGE,
            0,
            ptr::null(),
            0,
            ptr::null(),
            0,
            ptr::null(),
            &raw mut restricted,
        )
    };
    if ok == FALSE {
        return Err(ConfineError::Confine(format!(
            "CreateRestrictedToken failed: {}",
            io::Error::last_os_error()
        )));
    }
    OwnedHandle::new(restricted).map_err(|e| ConfineError::Confine(e.to_string()))
}

struct ConfinedJob {
    job: OwnedHandle,
    iocp: OwnedHandle,
}

impl ConfinedJob {
    fn new(limits: &Limits) -> Result<Self, ConfineError> {
        // SAFETY: anonymous Job Object.
        let raw_job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        let job = OwnedHandle::new(raw_job).map_err(ConfineError::Limits)?;

        // Extended limits: kill on close, memory, CPU time, active process count.
        // SAFETY: zero-initialized Win32 limit struct.
        let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
        ext.BasicLimitInformation.ActiveProcessLimit = limits.processes.max(1);
        if limits.memory > 0 {
            flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
            ext.JobMemoryLimit = usize::try_from(limits.memory).unwrap_or(usize::MAX);
        }
        if limits.cpu_time > 0 {
            flags |= JOB_OBJECT_LIMIT_JOB_TIME;
            ext.BasicLimitInformation.PerJobUserTimeLimit = cpu_time_to_100ns(limits.cpu_time);
        }
        ext.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: sets extended limits on the newly created Job Object.
        let ok = unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&raw const ext).cast(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .expect("invariant: Win32 limit struct size fits in u32"),
            )
        };
        if ok == FALSE {
            return Err(ConfineError::Limits(io::Error::last_os_error()));
        }

        // UI restrictions: isolate clipboard, handles, desktop, display, atoms.
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: JOB_OBJECT_UILIMIT_HANDLES
                | JOB_OBJECT_UILIMIT_READCLIPBOARD
                | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
                | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                | JOB_OBJECT_UILIMIT_GLOBALATOMS
                | JOB_OBJECT_UILIMIT_DESKTOP
                | JOB_OBJECT_UILIMIT_EXITWINDOWS,
        };
        // SAFETY: applies UI restrictions to the Job Object.
        let ok = unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectBasicUIRestrictions,
                (&raw const ui).cast(),
                u32::try_from(size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>())
                    .expect("invariant: UI restriction struct size fits in u32"),
            )
        };
        if ok == FALSE {
            return Err(ConfineError::Limits(io::Error::last_os_error()));
        }

        // Associate an I/O Completion Port so we observe Job limit notifications.
        // SAFETY: creates a standalone completion port.
        let raw_iocp =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, ptr::null_mut(), 0, 1) };
        let iocp = OwnedHandle::new(raw_iocp).map_err(ConfineError::Limits)?;
        let assoc = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: COMPLETION_KEY as *mut core::ffi::c_void,
            CompletionPort: iocp.raw(),
        };
        // SAFETY: associates completion port with the Job Object.
        let ok = unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectAssociateCompletionPortInformation,
                (&raw const assoc).cast(),
                u32::try_from(size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>())
                    .expect("invariant: completion-port struct size fits in u32"),
            )
        };
        if ok == FALSE {
            return Err(ConfineError::Limits(io::Error::last_os_error()));
        }
        Ok(Self { job, iocp })
    }

    fn assign(&self, process: HANDLE) -> Result<(), ConfineError> {
        // SAFETY: both handles are live.
        if unsafe { AssignProcessToJobObject(self.job.raw(), process) } == FALSE {
            return Err(ConfineError::Limits(io::Error::last_os_error()));
        }
        Ok(())
    }

    fn terminate(&self) {
        // SAFETY: terminates all remaining processes in the Job Object.
        unsafe {
            TerminateJobObject(self.job.raw(), 1);
        }
    }

    /// Sums the OS handle counts across every live process assigned to this
    /// Job Object (used to enforce `limits.open_files` on Windows).
    fn total_handle_count(&self) -> u32 {
        // Space for up to 64 PIDs in the job's process ID list.
        const MAX_PIDS: usize = 64;
        #[repr(C)]
        struct PidListBuffer {
            header: JOBOBJECT_BASIC_PROCESS_ID_LIST,
            extra: [usize; MAX_PIDS - 1],
        }
        // SAFETY: zero-initialized Win32 output buffer.
        let mut buf: PidListBuffer = unsafe { std::mem::zeroed() };
        let mut returned = 0_u32;
        // SAFETY: queries assigned process IDs from the live Job Object.
        let ok = unsafe {
            QueryInformationJobObject(
                self.job.raw(),
                JobObjectBasicProcessIdList,
                (&raw mut buf).cast(),
                u32::try_from(size_of::<PidListBuffer>())
                    .expect("invariant: PID buffer size fits in u32"),
                &raw mut returned,
            )
        };
        if ok == FALSE {
            return 0;
        }
        let count = usize::try_from(buf.header.NumberOfProcessIdsInList)
            .unwrap_or(0)
            .min(MAX_PIDS);
        let mut total = 0_u32;
        for index in 0..count {
            let pid = if index == 0 {
                buf.header.ProcessIdList[0]
            } else {
                buf.extra[index - 1]
            };
            let Ok(pid) = u32::try_from(pid) else {
                continue;
            };
            // SAFETY: opens process for handle-count query only.
            let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
            if let Ok(owned) = OwnedHandle::new(handle) {
                let mut handles = 0_u32;
                // SAFETY: reads process handle count into local `u32`.
                if unsafe { GetProcessHandleCount(owned.raw(), &raw mut handles) } != FALSE {
                    total = total.saturating_add(handles);
                }
            }
        }
        total
    }

    /// Polls the completion port for up to `timeout_ms` milliseconds.
    fn poll_notification(&self, timeout_ms: u32) -> Option<u32> {
        let mut msg_id = 0_u32;
        let mut key = 0_usize;
        let mut overlapped = ptr::null_mut();
        // SAFETY: polls completion port for Job Object messages.
        let ok = unsafe {
            GetQueuedCompletionStatus(
                self.iocp.raw(),
                &raw mut msg_id,
                &raw mut key,
                &raw mut overlapped,
                timeout_ms,
            )
        };
        if ok != FALSE && key == COMPLETION_KEY {
            Some(msg_id)
        } else {
            None
        }
    }
}

fn build_command_line(program: &Path, args: &[String]) -> Vec<u16> {
    let mut cmd = String::new();
    push_windows_arg(&mut cmd, &program.to_string_lossy());
    for arg in args {
        cmd.push(' ');
        push_windows_arg(&mut cmd, arg);
    }
    OsStr::new(&cmd)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Quotes one Windows command-line argument per Microsoft C runtime parsing
/// rules.
fn push_windows_arg(out: &mut String, arg: &str) {
    let needs_quotes = arg.is_empty() || arg.bytes().any(|b| matches!(b, b' ' | b'\t' | b'"'));
    if !needs_quotes {
        out.push_str(arg);
        return;
    }
    out.push('"');
    let mut backslashes = 0_usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                for _ in 0..(backslashes * 2 + 1) {
                    out.push('\\');
                }
                out.push('"');
                backslashes = 0;
            }
            other => {
                for _ in 0..backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push(other);
            }
        }
    }
    for _ in 0..(backslashes * 2) {
        out.push('\\');
    }
    out.push('"');
}

fn build_env_block(env: &[(&str, &str)]) -> Vec<u16> {
    let mut entries: Vec<(&str, &str)> = env.to_vec();
    entries.sort_by(|(a, _), (b, _)| a.to_ascii_uppercase().cmp(&b.to_ascii_uppercase()));
    let mut block = Vec::new();
    for (k, v) in entries {
        block.extend(OsStr::new(k).encode_wide());
        block.push(u16::from(b'='));
        block.extend(OsStr::new(v).encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

fn inheritable_stdio() -> (HANDLE, HANDLE, HANDLE, Vec<HANDLE>) {
    // SAFETY: reads this process's standard handles.
    let stdin = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    let stdout = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    let stderr = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    let mut list = Vec::with_capacity(3);
    for handle in [stdin, stdout, stderr] {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE && !list.contains(&handle) {
            // SAFETY: ensures the standard handle can be inherited by the
            // confined child when listed in `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.
            unsafe {
                let _ = SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
            }
            list.push(handle);
        }
    }
    (stdin, stdout, stderr, list)
}

/// Confines and runs `spec` under a Windows AppContainer, restricted token,
/// alternate desktop, minimal NTFS ACLs, and Job Object.
pub(super) fn run_confined(spec: &ProcessSpec<'_>) -> Result<i32, ConfineError> {
    let folder = std::fs::canonicalize(spec.folder).map_err(ConfineError::Folder)?;

    let profile = match spec.broker_grant {
        Some(grant) => AppContainerProfile::open_existing(&grant.app_container_name)?,
        None => AppContainerProfile::create_unique()?,
    };
    let mut grants = AclGrants::new(profile.sid());
    for path in &spec.policy.reads {
        grants.grant(path, FILE_GENERIC_READ)?;
    }
    for path in &spec.policy.executables {
        grants.grant(path, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?;
    }
    for path in &spec.policy.writes {
        grants.grant(path, FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE)?;
    }
    grants.grant(spec.program, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?;
    grants.grant(&folder, FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE)?;

    let mut desktop = AlternateDesktop::create_unique(profile.sid())?;
    let restricted_token = create_restricted_token()?;
    let job = ConfinedJob::new(&spec.policy.limits)?;

    let (stdin, stdout, stderr, mut handle_list) = inheritable_stdio();
    let attr_count = if handle_list.is_empty() { 1 } else { 2 };
    let mut attr_size = 0_usize;
    // SAFETY: queries required byte length for `attr_count` attributes.
    unsafe {
        InitializeProcThreadAttributeList(ptr::null_mut(), attr_count, 0, &raw mut attr_size);
    }
    let mut attr_buf = vec![0_u8; attr_size.max(64)];
    let attr_list = attr_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    // SAFETY: initializes attribute list inside owned `attr_buf`.
    if unsafe { InitializeProcThreadAttributeList(attr_list, attr_count, 0, &raw mut attr_size) }
        == FALSE
    {
        return Err(ConfineError::Confine(format!(
            "InitializeProcThreadAttributeList failed: {}",
            io::Error::last_os_error()
        )));
    }
    struct AttrListGuard(LPPROC_THREAD_ATTRIBUTE_LIST);
    impl Drop for AttrListGuard {
        fn drop(&mut self) {
            // SAFETY: paired with `InitializeProcThreadAttributeList`.
            unsafe {
                DeleteProcThreadAttributeList(self.0);
            }
        }
    }
    let _attr_guard = AttrListGuard(attr_list);

    let mut caps = SECURITY_CAPABILITIES {
        AppContainerSid: profile.sid(),
        Capabilities: ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    // SAFETY: `caps` lives until `CreateProcessAsUserW` completes.
    if unsafe {
        UpdateProcThreadAttribute(
            attr_list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            (&raw mut caps).cast(),
            size_of::<SECURITY_CAPABILITIES>(),
            ptr::null_mut(),
            ptr::null(),
        )
    } == FALSE
    {
        return Err(ConfineError::Confine(format!(
            "UpdateProcThreadAttribute(SECURITY_CAPABILITIES) failed: {}",
            io::Error::last_os_error()
        )));
    }
    if !handle_list.is_empty() {
        // SAFETY: `handle_list` lives until `CreateProcessAsUserW` completes.
        if unsafe {
            UpdateProcThreadAttribute(
                attr_list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handle_list.as_mut_ptr().cast(),
                handle_list.len() * size_of::<HANDLE>(),
                ptr::null_mut(),
                ptr::null(),
            )
        } == FALSE
        {
            return Err(ConfineError::Confine(format!(
                "UpdateProcThreadAttribute(HANDLE_LIST) failed: {}",
                io::Error::last_os_error()
            )));
        }
    }

    // SAFETY: zero-initialized `STARTUPINFOEXW` populated below.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>())
        .expect("invariant: STARTUPINFOEXW size fits in u32");
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin;
    startup.StartupInfo.hStdOutput = stdout;
    startup.StartupInfo.hStdError = stderr;
    startup.StartupInfo.lpDesktop = desktop.lp_desktop();
    startup.lpAttributeList = attr_list;

    let app_wide = path_to_wide_nul(spec.program);
    let mut cmd_wide = build_command_line(spec.program, spec.args);
    let dir_wide = path_to_wide_nul(&folder);
    let env_block = build_env_block(spec.env);

    // SAFETY: zero-initialized `PROCESS_INFORMATION` populated by OS.
    let mut proc_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let inherit = if handle_list.is_empty() { FALSE } else { TRUE };
    let flags = CREATE_SUSPENDED
        | EXTENDED_STARTUPINFO_PRESENT
        | CREATE_UNICODE_ENVIRONMENT
        | CREATE_NO_WINDOW;
    // SAFETY: all wide buffers, attribute list, security capabilities and
    // token handles outlive `CreateProcessAsUserW`.
    let created = unsafe {
        CreateProcessAsUserW(
            restricted_token.raw(),
            app_wide.as_ptr(),
            cmd_wide.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            inherit,
            flags,
            env_block.as_ptr().cast(),
            dir_wide.as_ptr(),
            &raw const startup.StartupInfo,
            &raw mut proc_info,
        )
    };
    if created == FALSE {
        let error = io::Error::last_os_error();
        if error
            .raw_os_error()
            .is_some_and(is_smart_app_control_os_error)
        {
            return Err(ConfineError::BlockedBySmartAppControl(
                spec.program.to_path_buf(),
            ));
        }
        return Err(ConfineError::Start(error));
    }

    let child_process = OwnedHandle::new(proc_info.hProcess).map_err(ConfineError::Start)?;
    let child_thread = OwnedHandle::new(proc_info.hThread).map_err(ConfineError::Start)?;

    if let Err(error) = job.assign(child_process.raw()) {
        // SAFETY: terminates the suspended process if Job assignment failed.
        unsafe {
            windows_sys::Win32::System::Threading::TerminateProcess(child_process.raw(), 1);
        }
        return Err(error);
    }
    // SAFETY: resumes the confined child's main thread now that it is inside
    // its Job Object.
    if unsafe { ResumeThread(child_thread.raw()) } == u32::MAX {
        job.terminate();
        return Err(ConfineError::Start(io::Error::last_os_error()));
    }
    drop(child_thread);

    let open_files_limit = spec.policy.limits.open_files;
    let mut limit_hit: Option<PluginResourceLimit> = None;
    loop {
        if let Some(msg) = job.poll_notification(POLL_TIMEOUT_MS) {
            if let Some(limit) = classify_job_notification(msg) {
                limit_hit = Some(limit);
                job.terminate();
                break;
            }
            if msg == JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO {
                break;
            }
        }
        if open_files_limit > 0 && job.total_handle_count() >= open_files_limit {
            limit_hit = Some(PluginResourceLimit::OpenFiles);
            job.terminate();
            break;
        }
        // SAFETY: non-blocking check whether the primary child exited.
        if unsafe { WaitForSingleObject(child_process.raw(), 0) } == WAIT_OBJECT_0 {
            while let Some(msg) = job.poll_notification(0) {
                if let Some(limit) = classify_job_notification(msg) {
                    limit_hit = Some(limit);
                    break;
                }
            }
            break;
        }
    }

    if let Some(limit) = limit_hit {
        job.terminate();
        return Err(ConfineError::LimitExceeded(limit));
    }

    let mut exit_code = 0_u32;
    // SAFETY: reads exit code of the exited child process.
    if unsafe { GetExitCodeProcess(child_process.raw(), &raw mut exit_code) } == FALSE {
        return Err(ConfineError::Start(io::Error::last_os_error()));
    }
    #[expect(
        clippy::cast_possible_wrap,
        reason = "Windows process exit codes are 32-bit values carried as i32 in std::process::exit"
    )]
    Ok(exit_code as i32)
}

/// Checks whether the current process is executing inside an AppContainer
/// (`TokenIsAppContainer != 0`).
pub(super) fn is_current_process_appcontainer() -> bool {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: opens calling process token for query.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == FALSE {
        return false;
    }
    let Ok(token) = OwnedHandle::new(token) else {
        return false;
    };
    let mut is_appcontainer = 0_u32;
    let mut returned = 0_u32;
    // SAFETY: queries `TokenIsAppContainer` (`u32`) from the process token.
    let ok = unsafe {
        GetTokenInformation(
            token.raw(),
            TokenIsAppContainer,
            (&raw mut is_appcontainer).cast(),
            u32::try_from(size_of::<u32>()).expect("invariant: u32 size fits in u32"),
            &raw mut returned,
        )
    };
    ok != FALSE && is_appcontainer != 0
}

/// Probes whether this Windows host can create and delete an AppContainer
/// profile (called once at startup by `windows::host::read`).
pub(super) fn probe_appcontainer() -> bool {
    AppContainerProfile::create_unique().is_ok()
}
