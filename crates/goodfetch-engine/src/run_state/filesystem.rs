//! The check of a run file's folder before the file opens (ADR 0049).
//! SQLite's locking cannot be relied on over a network filesystem or a FUSE
//! mount of object storage, so the engine keeps no run state on NFS (which
//! EFS, Filestore and Azure Files' NFS shares are), on SMB (which Azure
//! Files' SMB shares are) or on such a mount. `statfs` names the
//! filesystem; on Linux a FUSE mount's tool is read from the process's
//! mount table, and any other FUSE mount is kept. macOS refuses NFS and
//! SMB, and checks no FUSE mount; other systems check nothing.
#![cfg_attr(
    any(target_os = "linux", target_os = "macos"),
    expect(
        unsafe_code,
        reason = "`statfs` is a system call, which `libc` binds only as an unsafe function"
    )
)]

use std::path::{Path, PathBuf};

use goodfetch_protocol::run::RunStateFilesystem;

use super::RunStateError;

#[cfg(test)]
mod tests;

/// A run file's folder, refused for the filesystem it is on (ADR 0049).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedFolder {
    /// The folder.
    pub folder: PathBuf,
    /// The kind of filesystem.
    pub filesystem: RunStateFilesystem,
    /// Its name, such as `nfs` or `fuse.s3fs`.
    pub found: String,
}

/// Refuses the run file at `file` when its folder is on a filesystem whose
/// locking SQLite cannot rely on. A folder whose filesystem cannot be read
/// is kept, and opening the file then says what is wrong.
///
/// # Errors
///
/// `FilesystemUnsupported`, naming the filesystem.
pub(super) fn check(file: &Path) -> Result<(), RunStateError> {
    let Some(folder) = file.parent() else {
        return Ok(());
    };
    match refused(folder) {
        Some((filesystem, found)) => Err(RunStateError::FilesystemUnsupported(UnsupportedFolder {
            folder: folder.to_owned(),
            filesystem,
            found,
        })),
        None => Ok(()),
    }
}

/// The FUSE tools that mount an object store as a folder.
#[cfg(any(target_os = "linux", test))]
const OBJECT_STORAGE_TOOLS: [&str; 8] = [
    "s3fs",
    "mountpoint-s3",
    "goofys",
    "geesefs",
    "gcsfuse",
    "blobfuse",
    "blobfuse2",
    "rclone",
];

/// What a Linux filesystem's magic number, `statfs`'s `f_type`, says.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Magic {
    /// A filesystem the engine refuses whatever is mounted.
    Refused(RunStateFilesystem),
    /// A FUSE mount, refused only when its tool mounts object storage.
    Fuse,
    /// Any other filesystem.
    Kept,
}

/// `NFS_SUPER_MAGIC`.
#[cfg(any(target_os = "linux", test))]
const NFS: u32 = 0x6969;
/// `SMB_SUPER_MAGIC`, the old `smbfs`.
#[cfg(any(target_os = "linux", test))]
const SMB: u32 = 0x517b;
/// `CIFS_SUPER_MAGIC`.
#[cfg(any(target_os = "linux", test))]
const CIFS: u32 = 0xff53_4d42;
/// `SMB2_SUPER_MAGIC`, which the `cifs` module answers for an SMB 2 or 3
/// share.
#[cfg(any(target_os = "linux", test))]
const SMB2: u32 = 0xfe53_4d42;
/// `FUSE_SUPER_MAGIC`.
#[cfg(any(target_os = "linux", test))]
const FUSE: u32 = 0x6573_5546;

#[cfg(any(target_os = "linux", test))]
fn magic(f_type: u32) -> Magic {
    match f_type {
        NFS => Magic::Refused(RunStateFilesystem::Nfs),
        SMB | CIFS | SMB2 => Magic::Refused(RunStateFilesystem::Smb),
        FUSE => Magic::Fuse,
        _ => Magic::Kept,
    }
}

/// A filesystem's short name, as the error names it.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn found(filesystem: RunStateFilesystem) -> String {
    match filesystem {
        RunStateFilesystem::Nfs => "nfs",
        RunStateFilesystem::Smb => "smb",
        RunStateFilesystem::FuseObjectStorage => "fuse",
    }
    .to_owned()
}

/// A mount of `/proc/self/mountinfo`: its type and its source.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, PartialEq, Eq)]
struct Mount<'a> {
    fstype: &'a str,
    source: &'a str,
}

/// The mount `mountinfo` holds `folder` on: of the mount points `folder`
/// is within, the longest, and of mounts on one point, the latest.
#[cfg(any(target_os = "linux", test))]
fn mount_of<'a>(mountinfo: &'a str, folder: &Path) -> Option<Mount<'a>> {
    let mut best: Option<(usize, Mount<'a>)> = None;
    for line in mountinfo.lines() {
        let mut fields = line.split(' ');
        let Some(point) = fields.nth(4) else {
            continue;
        };
        let point = unescaped(point);
        let mut after = fields.skip_while(|field| *field != "-").skip(1);
        let (Some(fstype), Some(source)) = (after.next(), after.next()) else {
            continue;
        };
        let longer = best
            .as_ref()
            .is_none_or(|(length, _)| point.len() >= *length);
        if longer && folder.starts_with(&point) {
            best = Some((point.len(), Mount { fstype, source }));
        }
    }
    best.map(|(_, mount)| mount)
}

/// A mount point as `mountinfo` escapes it: a space, a tab, a newline and
/// a backslash as three octal digits after a backslash.
#[cfg(any(target_os = "linux", test))]
fn unescaped(point: &str) -> String {
    let mut unescaped = String::with_capacity(point.len());
    let mut rest = point;
    while let Some(at) = rest.find('\\') {
        unescaped.push_str(&rest[..at]);
        let escape = rest.get(at + 1..at + 4);
        if let Some(byte) = escape.and_then(|digits| u8::from_str_radix(digits, 8).ok()) {
            unescaped.push(char::from(byte));
            rest = &rest[at + 4..];
        } else {
            unescaped.push('\\');
            rest = &rest[at + 1..];
        }
    }
    unescaped.push_str(rest);
    unescaped
}

/// The object-storage tool behind a FUSE mount, by its subtype (`fuse.s3fs`)
/// or, for a plain `fuse` mount, its source (`mountpoint-s3`, or a path or
/// `tool#bucket` naming the tool).
#[cfg(any(target_os = "linux", test))]
fn object_storage_tool(mount: &Mount<'_>) -> Option<&'static str> {
    let name = mount.fstype.strip_prefix("fuse.").unwrap_or_else(|| {
        let tool = mount.source.split(['#', ':']).next().unwrap_or_default();
        tool.rsplit('/').next().unwrap_or_default()
    });
    OBJECT_STORAGE_TOOLS
        .iter()
        .copied()
        .find(|tool| *tool == name)
}

#[cfg(target_os = "linux")]
fn refused(folder: &Path) -> Option<(RunStateFilesystem, String)> {
    let answer = statfs(folder)?;
    match magic(u32::try_from(answer.f_type).ok()?) {
        Magic::Refused(filesystem) => Some((filesystem, found(filesystem))),
        Magic::Fuse => {
            #[expect(
                clippy::disallowed_methods,
                reason = "it runs on the tasks module's blocking policy, inside `RunState::open`'s blocking call"
            )]
            let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
            let folder = std::fs::canonicalize(folder).ok()?;
            let tool = object_storage_tool(&mount_of(&mountinfo, &folder)?)?;
            let filesystem = RunStateFilesystem::FuseObjectStorage;
            Some((filesystem, format!("{}.{tool}", found(filesystem))))
        }
        Magic::Kept => None,
    }
}

#[cfg(target_os = "macos")]
fn refused(folder: &Path) -> Option<(RunStateFilesystem, String)> {
    let answer = statfs(folder)?;
    let name: Vec<u8> = answer
        .f_fstypename
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| byte.to_ne_bytes()[0])
        .collect();
    let filesystem = match name.as_slice() {
        b"nfs" => RunStateFilesystem::Nfs,
        b"smbfs" => RunStateFilesystem::Smb,
        _ => return None,
    };
    Some((filesystem, found(filesystem)))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn refused(_folder: &Path) -> Option<(RunStateFilesystem, String)> {
    None
}

/// `statfs` of `folder`, or none when it failed, which is logged.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn statfs(folder: &Path) -> Option<libc::statfs> {
    use std::os::unix::ffi::OsStrExt as _;

    let path = std::ffi::CString::new(folder.as_os_str().as_bytes()).ok()?;
    let mut answer = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is a NUL-terminated string that outlives the call, and
    // `answer` is writable memory the size of a `statfs`.
    let status = unsafe { libc::statfs(path.as_ptr(), answer.as_mut_ptr()) };
    if status != 0 {
        tracing::debug!(
            folder = %folder.display(),
            error = %std::io::Error::last_os_error(),
            "statfs of the run folder failed, so its filesystem is not checked"
        );
        return None;
    }
    // SAFETY: `statfs` returned 0, so it filled `answer`.
    Some(unsafe { answer.assume_init() })
}
