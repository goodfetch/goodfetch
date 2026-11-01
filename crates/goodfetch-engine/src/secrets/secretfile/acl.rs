//! Windows' equivalent of mode `0600` for the encrypted local file's
//! identity (spec #564): its access list may grant access to its owner,
//! SYSTEM and Administrators only, and no access list at all, which
//! grants everyone everything, is refused.
#![expect(
    unsafe_code,
    reason = "reads a file's security descriptor through the Win32 API, which has no safe binding"
)]

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, IsWellKnownSid,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, WinBuiltinAdministratorsSid,
    WinLocalSystemSid,
};

/// `ACCESS_ALLOWED_ACE_TYPE`, the only kind of entry that grants access.
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

/// Whether the access list of the file at `path` grants access only to its
/// owner, SYSTEM and Administrators.
pub(super) fn owner_only(path: &Path) -> io::Result<bool> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut owner: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call,
    // and each out-pointer is a valid place for the pointer it receives.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut owner,
            std::ptr::null_mut(),
            &raw mut dacl,
            std::ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(
            i32::try_from(status).unwrap_or(i32::MAX),
        ));
    }
    let _freed = Freed(descriptor);
    if dacl.is_null() {
        return Ok(false);
    }
    let mut size = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: `dacl` points into `descriptor`, alive until `_freed` drops,
    // and `size` is an `ACL_SIZE_INFORMATION` of the length given.
    let read = unsafe {
        GetAclInformation(
            dacl,
            (&raw mut size).cast::<c_void>(),
            u32::try_from(size_of::<ACL_SIZE_INFORMATION>())
                .expect("invariant: the structure's size fits a u32"),
            AclSizeInformation,
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    for index in 0..size.AceCount {
        let mut entry: *mut c_void = std::ptr::null_mut();
        // SAFETY: `index` is below the list's entry count, and `entry`
        // receives a pointer into the list.
        if unsafe { GetAce(dacl, index, &raw mut entry) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: every entry starts with an `ACE_HEADER`.
        let header = unsafe { &*entry.cast::<ACE_HEADER>() };
        if header.AceType != ACCESS_ALLOWED_ACE_TYPE {
            continue;
        }
        // SAFETY: an `ACCESS_ALLOWED_ACE_TYPE` entry is an
        // `ACCESS_ALLOWED_ACE`, whose SID starts at `SidStart`.
        let sid: PSID =
            unsafe { (&raw mut (*entry.cast::<ACCESS_ALLOWED_ACE>()).SidStart).cast::<c_void>() };
        // SAFETY: `sid` and `owner` are valid SIDs inside `descriptor`.
        let allowed = unsafe {
            EqualSid(sid, owner) != 0
                || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
        };
        if !allowed {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A security descriptor the system allocated, freed when dropped.
struct Freed(PSECURITY_DESCRIPTOR);

impl Drop for Freed {
    fn drop(&mut self) {
        // SAFETY: `GetNamedSecurityInfoW` allocated it with `LocalAlloc`,
        // and nothing reads it after this.
        unsafe {
            LocalFree(self.0);
        }
    }
}
