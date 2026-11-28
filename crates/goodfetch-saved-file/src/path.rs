//! Portable path safety checks for Saved files and engine-managed file names
//! (ADRs 0067 and 0116): one canonical rule shared between the engine
//! (`hostile_input` and `run_files::Saving`) and Collie's client crate
//! (`file_saving::FileSaving`), so Linux, macOS and Windows refuse the same
//! hostile names alike.

use std::fmt;
use std::path::{Component, Path};

use icu_normalizer::ComposingNormalizerBorrowed;

/// The most bytes one part of a path holds: a file system's name.
const NAME_MAX_BYTES: usize = 255;

/// The characters Windows forbids in a name, beside the separators and the
/// colon, which the checks name on their own.
const FORBIDDEN: [char; 6] = ['<', '>', '"', '|', '?', '*'];

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
    match shape {
        PathShape::SavePath => check_save(path),
        PathShape::Name | PathShape::RelativePath => {
            let Some(name) = path.to_str() else {
                return check_non_utf8(path);
            };
            check_relative(name, shape)
        }
    }
}

fn check_relative(name: &str, shape: PathShape) -> Result<(), PathIssue> {
    if name.is_empty() {
        return Err(PathIssue::DotName);
    }
    if name.chars().any(char::is_control) {
        return Err(PathIssue::ControlCharacter);
    }
    if let Some(issue) = root_issue(name) {
        return Err(issue);
    }
    if !ComposingNormalizerBorrowed::new_nfc().is_normalized(name) {
        return Err(PathIssue::Normalisation);
    }
    let parts: Vec<&str> = name.split(['/', '\\']).collect();
    if let Some(issue) = parts.iter().find_map(|part| part_issue(part)) {
        // An empty last part is a trailing separator, which a name never
        // takes and a path does not need.
        let trailing = parts.last().is_some_and(|last| last.is_empty())
            && parts
                .iter()
                .rev()
                .skip(1)
                .all(|part| part_issue(part).is_none());
        if !(trailing && shape == PathShape::Name) {
            return Err(issue);
        }
    }
    if shape == PathShape::Name && parts.len() > 1 {
        return Err(PathIssue::Separator);
    }
    Ok(())
}

fn check_non_utf8(path: &Path) -> Result<(), PathIssue> {
    for component in path.components() {
        match component {
            Component::ParentDir => return Err(PathIssue::ParentTraversal),
            Component::CurDir => return Err(PathIssue::DotName),
            _ => {}
        }
    }
    path.file_name().map(|_| ()).ok_or(PathIssue::DotName)
}

fn check_save(path: &Path) -> Result<(), PathIssue> {
    if path.as_os_str().is_empty() {
        return Err(PathIssue::DotName);
    }
    let Some(text) = path.to_str() else {
        return check_non_utf8(path);
    };
    if text.chars().any(char::is_control) {
        return Err(PathIssue::ControlCharacter);
    }
    if text.ends_with('/') || text.ends_with('\\') {
        return Err(PathIssue::DotName);
    }
    let tail = strip_host_root(text)?;
    if tail.is_empty() {
        return Err(PathIssue::DotName);
    }
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    for part in tail.split(['/', '\\']) {
        if let Some(issue) = part_issue(part) {
            return Err(issue);
        }
        if !nfc.is_normalized(part) {
            return Err(PathIssue::Normalisation);
        }
    }
    if !cfg!(windows) && tail.contains('\\') {
        return Err(PathIssue::Separator);
    }
    Ok(())
}

fn strip_host_root(text: &str) -> Result<&str, PathIssue> {
    match root_issue(text) {
        Some(PathIssue::Verbatim) => Err(PathIssue::Verbatim),
        Some(PathIssue::DeviceNamespace) => Err(PathIssue::DeviceNamespace),
        Some(PathIssue::Unc) => Err(PathIssue::Unc),
        Some(PathIssue::Drive) => {
            if cfg!(windows)
                && text
                    .as_bytes()
                    .get(2)
                    .is_some_and(|byte| *byte == b'/' || *byte == b'\\')
            {
                Ok(&text[3..])
            } else {
                Err(PathIssue::Drive)
            }
        }
        Some(PathIssue::Absolute) => {
            if text.starts_with('\\') && !cfg!(windows) {
                return Err(PathIssue::Separator);
            }
            Ok(&text[1..])
        }
        _ => Ok(text),
    }
}

/// What makes `name`, as a whole, start outside its folder: a verbatim,
/// device or UNC path, a path from the root, or a drive.
fn root_issue(name: &str) -> Option<PathIssue> {
    let separator = |character: char| character == '/' || character == '\\';
    let mut characters = name.chars();
    let (first, second, third, fourth) = (
        characters.next(),
        characters.next(),
        characters.next(),
        characters.next(),
    );
    match (first, second, third, fourth) {
        (Some(a), Some(b), Some('?'), Some(d)) if separator(a) && separator(b) && separator(d) => {
            Some(PathIssue::Verbatim)
        }
        (Some(a), Some(b), Some('.'), Some(d)) if separator(a) && separator(b) && separator(d) => {
            Some(PathIssue::DeviceNamespace)
        }
        (Some(a), Some(b), ..) if separator(a) && separator(b) => Some(PathIssue::Unc),
        (Some(a), ..) if separator(a) => Some(PathIssue::Absolute),
        (Some(letter), Some(':'), ..) if letter.is_ascii_alphabetic() => Some(PathIssue::Drive),
        _ => None,
    }
}

/// What makes one part of a path unsafe, if anything.
fn part_issue(part: &str) -> Option<PathIssue> {
    if part.is_empty() || part == "." {
        return Some(PathIssue::DotName);
    }
    if part == ".." {
        return Some(PathIssue::ParentTraversal);
    }
    if part.contains(':') {
        return Some(PathIssue::AlternateDataStream);
    }
    if part.contains(FORBIDDEN) {
        return Some(PathIssue::ForbiddenCharacter);
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return Some(PathIssue::TrailingDotOrSpace);
    }
    if is_reserved(part) {
        return Some(PathIssue::ReservedName);
    }
    (part.len() > NAME_MAX_BYTES).then_some(PathIssue::Overlong)
}

/// Whether `part` is a device name Windows reserves: alone, or before an
/// extension, in any case, spaces before the extension dropped as Windows
/// drops them.
fn is_reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end_matches(' ');
    let upper = stem.to_uppercase();
    RESERVED.contains(&upper.as_str())
}
