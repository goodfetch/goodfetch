//! Portable path safety checks for Saved files and engine-managed file names
//! (ADRs 0067 and 0116): one canonical rule shared between the engine
//! (`hostile_input` and `run_files::Saving`) and Collie's client crate
//! (`file_saving::FileSaving`), so Linux, macOS and Windows refuse the same
//! hostile names alike.

use std::fmt;
use std::path::Path;

use icu_normalizer::ComposingNormalizerBorrowed;

/// The most bytes one part of a path holds: a file system's name.
const NAME_MAX_BYTES: usize = 255;

/// The ASCII bytes Windows forbids in a name, beside the separators and the
/// colon, which the checks name on their own.
const FORBIDDEN_BYTES: [u8; 6] = [b'<', b'>', b'"', b'|', b'?', b'*'];

/// The device names Windows reserves, in upper case, alone or before an
/// extension: the console's, the ports, with their superscript digits, and
/// the console's input and output.
const RESERVED: [&str; 32] = [
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "COM0",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT0",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
    "CONIN$",
    "CONOUT$",
];

/// What kind of path or name from outside input is being checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathShape {
    /// One file name, such as an Export name, a Plan's or a Key-value store key.
    Name,
    /// A path under the folder it is written in, to a subfolder or not,
    /// such as a screenshot's or a Run trace's.
    RelativePath,
    /// A program-supplied save path (`export.save_as`, `download.save_as`,
    /// `run export --out`), relative or rooted on the host OS.
    SavePath,
}

/// What makes a file name or save path unsafe on Linux, macOS or Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathIssue {
    /// A `..` part, which climbs out of its folder.
    ParentTraversal,
    /// A path from the root.
    Absolute,
    /// A Windows drive, absolute or drive-relative.
    Drive,
    /// A Windows verbatim path, `\\?\`.
    Verbatim,
    /// A Windows device path, `\\.\`.
    DeviceNamespace,
    /// A Windows UNC path.
    Unc,
    /// A name Windows reserves for a device, with or without an extension.
    ReservedName,
    /// A trailing dot or space, which Windows removes.
    TrailingDotOrSpace,
    /// An NTFS alternate data stream, after a colon.
    AlternateDataStream,
    /// A character Windows forbids in a name: `<`, `>`, `"`, `|`, `?` or `*`.
    ForbiddenCharacter,
    /// A control character or a NUL.
    ControlCharacter,
    /// A name not in Unicode's composed form (NFC).
    Normalisation,
    /// A part longer than the 255 bytes a file system's name holds.
    Overlong,
    /// An empty part, or a `.` part.
    DotName,
    /// A folder separator where one name alone is taken, or a backslash in a
    /// Unix path.
    Separator,
}

impl fmt::Display for PathIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::ParentTraversal => "it contains a parent traversal (`..`)",
            Self::Absolute => "it is an absolute path outside its folder",
            Self::Drive => "it names a Windows drive path",
            Self::Verbatim => "it uses a Windows verbatim path prefix (`\\\\?\\`)",
            Self::DeviceNamespace => "it uses a Windows device namespace prefix (`\\\\.\\`)",
            Self::Unc => "it uses a UNC path prefix (`\\\\`)",
            Self::ReservedName => "it uses a Windows reserved device name",
            Self::TrailingDotOrSpace => "a path segment ends with a dot or space",
            Self::AlternateDataStream => "it contains a colon (alternate data stream)",
            Self::ForbiddenCharacter => "it contains a character forbidden on Windows",
            Self::ControlCharacter => "it contains a control character",
            Self::Normalisation => "it is not in Unicode composed form (NFC)",
            Self::Overlong => "a path segment exceeds 255 bytes",
            Self::DotName => "it names no file",
            Self::Separator => "it contains a disallowed path separator",
        };
        f.write_str(description)
    }
}

/// Judges `path`, from outside input, against the cross-OS safety rules for
/// `shape` (ADRs 0067 and 0116) before any folder or file is created.
///
/// # Errors
///
/// [`PathIssue`] naming the first hazard found.
pub fn check_path(path: &Path, shape: PathShape) -> Result<(), PathIssue> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let bytes = path.as_os_str().as_bytes();
        match shape {
            PathShape::SavePath => check_save_bytes(bytes),
            PathShape::Name | PathShape::RelativePath => check_relative_bytes(bytes, shape),
        }
    }
    #[cfg(not(unix))]
    {
        let Some(text) = path.to_str() else {
            return reject_non_utf8(path);
        };
        match shape {
            PathShape::SavePath => check_save_bytes(text.as_bytes()),
            PathShape::Name | PathShape::RelativePath => {
                check_relative_bytes(text.as_bytes(), shape)
            }
        }
    }
}

#[cfg(not(unix))]
fn reject_non_utf8(path: &Path) -> Result<(), PathIssue> {
    use std::path::Component;

    for component in path.components() {
        match component {
            Component::ParentDir => return Err(PathIssue::ParentTraversal),
            Component::Prefix(_) => return Err(PathIssue::Drive),
            Component::RootDir => return Err(PathIssue::Absolute),
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Err(PathIssue::Normalisation)
}

fn check_relative_bytes(bytes: &[u8], shape: PathShape) -> Result<(), PathIssue> {
    if bytes.is_empty() {
        return Err(PathIssue::DotName);
    }
    if has_control_bytes(bytes) {
        return Err(PathIssue::ControlCharacter);
    }
    if let Some(issue) = root_issue_bytes(bytes) {
        return Err(issue);
    }
    if !is_nfc_normalized(bytes) {
        return Err(PathIssue::Normalisation);
    }
    let parts: Vec<&[u8]> = bytes.split(|&byte| byte == b'/' || byte == b'\\').collect();
    if let Some(issue) = parts.iter().find_map(|part| part_issue_bytes(part)) {
        // An empty last part is a trailing separator, which a name never
        // takes and a path does not need.
        let trailing = parts.last().is_some_and(|last| last.is_empty())
            && parts
                .iter()
                .rev()
                .skip(1)
                .all(|part| part_issue_bytes(part).is_none());
        if !(trailing && shape == PathShape::Name) {
            return Err(issue);
        }
    }
    if shape == PathShape::Name && parts.len() > 1 {
        return Err(PathIssue::Separator);
    }
    Ok(())
}

fn check_save_bytes(bytes: &[u8]) -> Result<(), PathIssue> {
    if bytes.is_empty() {
        return Err(PathIssue::DotName);
    }
    if has_control_bytes(bytes) {
        return Err(PathIssue::ControlCharacter);
    }
    if bytes.ends_with(b"/") || bytes.ends_with(b"\\") {
        return Err(PathIssue::DotName);
    }
    let tail = strip_host_root_bytes(bytes)?;
    if tail.is_empty() {
        return Err(PathIssue::DotName);
    }
    let parts: Vec<&[u8]> = tail.split(|&byte| byte == b'/' || byte == b'\\').collect();
    let last_index = parts.len() - 1;
    for (index, part) in parts.iter().enumerate() {
        // Leading and intermediate `.` segments (such as `./out.json` or
        // `dir/./out.json`) name the current folder safely on every OS; bare
        // `.` and trailing `/.` name no file and remain refused (ADR 0116).
        if *part == b"." && index < last_index {
            continue;
        }
        if let Some(issue) = part_issue_bytes(part) {
            return Err(issue);
        }
        if !is_nfc_normalized(part) {
            return Err(PathIssue::Normalisation);
        }
    }
    if !cfg!(windows) && tail.contains(&b'\\') {
        return Err(PathIssue::Separator);
    }
    Ok(())
}

fn has_control_bytes(bytes: &[u8]) -> bool {
    bytes
        .utf8_chunks()
        .any(|chunk| chunk.valid().chars().any(char::is_control))
}

fn is_nfc_normalized(bytes: &[u8]) -> bool {
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    bytes
        .utf8_chunks()
        .all(|chunk| nfc.is_normalized(chunk.valid()))
}

fn strip_host_root_bytes(bytes: &[u8]) -> Result<&[u8], PathIssue> {
    match root_issue_bytes(bytes) {
        Some(PathIssue::Verbatim) => Err(PathIssue::Verbatim),
        Some(PathIssue::DeviceNamespace) => Err(PathIssue::DeviceNamespace),
        Some(PathIssue::Unc) => Err(PathIssue::Unc),
        Some(PathIssue::Drive) => {
            if cfg!(windows)
                && bytes
                    .get(2)
                    .is_some_and(|&byte| byte == b'/' || byte == b'\\')
            {
                Ok(&bytes[3..])
            } else {
                Err(PathIssue::Drive)
            }
        }
        Some(PathIssue::Absolute) => {
            if bytes.first() == Some(&b'\\') && !cfg!(windows) {
                return Err(PathIssue::Separator);
            }
            Ok(&bytes[1..])
        }
        _ => Ok(bytes),
    }
}

/// What makes `bytes`, as a whole, start outside its folder: a verbatim,
/// device or UNC path, a path from the root, or a drive.
fn root_issue_bytes(bytes: &[u8]) -> Option<PathIssue> {
    let separator = |byte: u8| byte == b'/' || byte == b'\\';
    match bytes {
        [a, b, b'?', d, ..] if separator(*a) && separator(*b) && separator(*d) => {
            Some(PathIssue::Verbatim)
        }
        [a, b, b'.', d, ..] if separator(*a) && separator(*b) && separator(*d) => {
            Some(PathIssue::DeviceNamespace)
        }
        [a, b, ..] if separator(*a) && separator(*b) => Some(PathIssue::Unc),
        [a, ..] if separator(*a) => Some(PathIssue::Absolute),
        [letter, b':', ..] if letter.is_ascii_alphabetic() => Some(PathIssue::Drive),
        _ => None,
    }
}

/// What makes one part of a path unsafe, if anything.
fn part_issue_bytes(part: &[u8]) -> Option<PathIssue> {
    if part.is_empty() || part == b"." {
        return Some(PathIssue::DotName);
    }
    if part == b".." {
        return Some(PathIssue::ParentTraversal);
    }
    if part.contains(&b':') {
        return Some(PathIssue::AlternateDataStream);
    }
    if part.iter().any(|byte| FORBIDDEN_BYTES.contains(byte)) {
        return Some(PathIssue::ForbiddenCharacter);
    }
    if part.ends_with(b".") || part.ends_with(b" ") {
        return Some(PathIssue::TrailingDotOrSpace);
    }
    if is_reserved_bytes(part) {
        return Some(PathIssue::ReservedName);
    }
    (part.len() > NAME_MAX_BYTES).then_some(PathIssue::Overlong)
}

/// Whether `part` is a device name Windows reserves: alone, or before an
/// extension, in any case, spaces before the extension dropped as Windows
/// drops them.
fn is_reserved_bytes(part: &[u8]) -> bool {
    let stem = part.split(|&byte| byte == b'.').next().unwrap_or(part);
    let end = stem
        .iter()
        .rposition(|&byte| byte != b' ')
        .map_or(0, |index| index + 1);
    std::str::from_utf8(&stem[..end]).is_ok_and(|text| {
        let upper = text.to_uppercase();
        RESERVED.contains(&upper.as_str())
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{PathIssue, PathShape, check_path};

    #[test]
    fn save_path_allows_leading_and_intermediate_current_dir_and_refuses_bare_or_trailing_dot() {
        for ok in [
            "./out.json",
            "dir/./out.json",
            "./a/./b.json",
            "/./out.json",
            "./.hidden",
        ] {
            assert_eq!(
                check_path(Path::new(ok), PathShape::SavePath),
                Ok(()),
                "{ok}"
            );
        }
        for bad in [".", "./", "dir/.", "./.", "/.", "dir/./"] {
            assert_eq!(
                check_path(Path::new(bad), PathShape::SavePath),
                Err(PathIssue::DotName),
                "{bad}"
            );
        }
        assert_eq!(
            check_path(Path::new("./../out.json"), PathShape::SavePath),
            Err(PathIssue::ParentTraversal)
        );
        assert_eq!(
            check_path(Path::new("./out.json"), PathShape::RelativePath),
            Err(PathIssue::DotName)
        );
        assert_eq!(
            check_path(Path::new("./out.json"), PathShape::Name),
            Err(PathIssue::DotName)
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_hostile_and_escape_paths_are_refused_for_every_shape() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        let overlong = vec![b'\xff'; 256];
        let cases: &[(&[u8], PathShape, PathIssue)] = &[
            (b"/\xff", PathShape::Name, PathIssue::Absolute),
            (b"../\xff", PathShape::Name, PathIssue::ParentTraversal),
            (b"sub/\xff", PathShape::Name, PathIssue::Separator),
            (b"sub\\\xff", PathShape::Name, PathIssue::Separator),
            (b"CON.\xff", PathShape::Name, PathIssue::ReservedName),
            (
                b"\xff\x00name",
                PathShape::Name,
                PathIssue::ControlCharacter,
            ),
            (
                b"\xff:stream",
                PathShape::Name,
                PathIssue::AlternateDataStream,
            ),
            (b"\xff<bad", PathShape::Name, PathIssue::ForbiddenCharacter),
            (b"\xff.", PathShape::Name, PathIssue::TrailingDotOrSpace),
            (b"\xff ", PathShape::Name, PathIssue::TrailingDotOrSpace),
            (overlong.as_slice(), PathShape::Name, PathIssue::Overlong),
            (b"/\xff", PathShape::RelativePath, PathIssue::Absolute),
            (
                b"../\xff",
                PathShape::RelativePath,
                PathIssue::ParentTraversal,
            ),
            (
                b"a/../\xff",
                PathShape::RelativePath,
                PathIssue::ParentTraversal,
            ),
            (b"C:\\\xff", PathShape::RelativePath, PathIssue::Drive),
            (
                b"\\\\?\\foo\\\xff",
                PathShape::RelativePath,
                PathIssue::Verbatim,
            ),
            (
                b"\\\\host\\share\\\xff",
                PathShape::RelativePath,
                PathIssue::Unc,
            ),
            (
                b"dir/CON.\xff",
                PathShape::RelativePath,
                PathIssue::ReservedName,
            ),
            (
                b"dir/\xff:ads",
                PathShape::RelativePath,
                PathIssue::AlternateDataStream,
            ),
            (
                b"dir/\xff?",
                PathShape::RelativePath,
                PathIssue::ForbiddenCharacter,
            ),
            (
                b"dir/\xff.",
                PathShape::RelativePath,
                PathIssue::TrailingDotOrSpace,
            ),
            (b"../\xff", PathShape::SavePath, PathIssue::ParentTraversal),
            (
                b"dir/../\xff",
                PathShape::SavePath,
                PathIssue::ParentTraversal,
            ),
            (b"C:\\\xff", PathShape::SavePath, PathIssue::Drive),
            (
                b"\\\\?\\foo\\\xff",
                PathShape::SavePath,
                PathIssue::Verbatim,
            ),
            (
                b"\\\\.\\pipe\\\xff",
                PathShape::SavePath,
                PathIssue::DeviceNamespace,
            ),
            (
                b"\\\\host\\share\\\xff",
                PathShape::SavePath,
                PathIssue::Unc,
            ),
            (b"dir\\\xff", PathShape::SavePath, PathIssue::Separator),
            (
                b"dir/CON.\xff",
                PathShape::SavePath,
                PathIssue::ReservedName,
            ),
            (
                b"dir/\xff\x07",
                PathShape::SavePath,
                PathIssue::ControlCharacter,
            ),
            (
                b"dir/\xff:ads",
                PathShape::SavePath,
                PathIssue::AlternateDataStream,
            ),
            (
                b"dir/\xff*",
                PathShape::SavePath,
                PathIssue::ForbiddenCharacter,
            ),
            (
                b"dir/\xff.",
                PathShape::SavePath,
                PathIssue::TrailingDotOrSpace,
            ),
            (
                b"dir/\xff ",
                PathShape::SavePath,
                PathIssue::TrailingDotOrSpace,
            ),
            (
                overlong.as_slice(),
                PathShape::SavePath,
                PathIssue::Overlong,
            ),
            (b"\xff/.", PathShape::SavePath, PathIssue::DotName),
        ];

        for &(bytes, shape, expected) in cases {
            let path = Path::new(OsStr::from_bytes(bytes));
            assert_eq!(
                check_path(path, shape),
                Err(expected),
                "{bytes:?} {shape:?}"
            );
        }

        // Benign non-UTF-8 file names with no hostile bytes or escape components stay accepted.
        assert_eq!(
            check_path(Path::new(OsStr::from_bytes(b"\xff.json")), PathShape::Name),
            Ok(())
        );
        assert_eq!(
            check_path(
                Path::new(OsStr::from_bytes(b"./\xff.json")),
                PathShape::SavePath
            ),
            Ok(())
        );
    }
}
