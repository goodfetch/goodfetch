//! Portable path safety checks for Saved files and engine-managed file names
//! (ADRs 0067 and 0116): one canonical rule shared between the engine
//! (`hostile_input` and `run_files::Saving`) and Collie's client crate
//! (`file_saving::FileSaving`), so Linux, macOS and Windows refuse the same
//! hostile names alike.

use std::path::{Component, Path, PathBuf};

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

/// What a name from outside input names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathShape {
    /// One file name, such as an Export name, a Plan's or a Key-value store key.
    Name,
    /// A path under the folder it is written in, to a subfolder or not,
    /// such as a screenshot's or a Run trace's.
    RelativePath,
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

impl PathIssue {
    /// A concise human-readable reason for `path_not_writable` error messages.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
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
        }
    }
}

/// Judges `name`, from outside input, as a [`PathShape`] the engine would
/// write a file under.
///
/// # Errors
///
/// [`PathIssue`] with the first hazard found: the whole path's, then each
/// part's, then a separator where one name is taken.
pub fn check_relative_path(name: &str, shape: PathShape) -> Result<(), PathIssue> {
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

/// Resolves an Export file `name` inside `folder`, refusing every hostile name
/// (traversal, absolute path, drive, UNC, verbatim, device namespace,
/// reserved name, trailing dot/space, ADS, forbidden/control chars, >255 bytes,
/// decomposed Unicode, or separators) on every OS alike.
///
/// # Errors
///
/// [`PathIssue`] naming why `name` cannot be resolved inside `folder`.
pub fn resolve_in_folder(folder: &Path, name: &str) -> Result<PathBuf, PathIssue> {
    check_relative_path(name, PathShape::Name)?;
    Ok(folder.join(name))
}

/// Judges a program-supplied save path (`export.save_as`, `download.save_as`,
/// `run export --out`), whether relative or rooted on the host OS, before any
/// folder or temporary file is created (ADRs 0067 and 0116).
///
/// Allows legitimate relative paths (`out/books.csv`) and host-native absolute
/// paths (`/tmp/out/books.csv` on Unix, `C:\Users\alice\books.csv` on Windows)
/// while refusing `..`, verbatim/device/UNC prefixes, drive-relative paths,
/// backslashes on Unix, and any segment with a Windows reserved name, trailing
/// dot/space, alternate data stream colon, Windows-forbidden or control
/// character, >255-byte length, or non-NFC Unicode decomposition.
///
/// # Errors
///
/// [`PathIssue`] naming why `path` cannot be written safely across OSes.
pub fn check_save_path(path: &Path) -> Result<(), PathIssue> {
    if path.as_os_str().is_empty() {
        return Err(PathIssue::DotName);
    }
    let Some(text) = path.to_str() else {
        for component in path.components() {
            match component {
                Component::ParentDir => return Err(PathIssue::ParentTraversal),
                Component::CurDir => return Err(PathIssue::DotName),
                _ => {}
            }
        }
        return path.file_name().map(|_| ()).ok_or(PathIssue::DotName);
    };
    if text.chars().any(char::is_control) {
        return Err(PathIssue::ControlCharacter);
    }
    if text.ends_with('/') || text.ends_with('\\') {
        return Err(PathIssue::DotName);
    }
    let tail = match root_issue(text) {
        Some(PathIssue::Verbatim) => return Err(PathIssue::Verbatim),
        Some(PathIssue::DeviceNamespace) => return Err(PathIssue::DeviceNamespace),
        Some(PathIssue::Unc) => return Err(PathIssue::Unc),
        Some(PathIssue::Drive) => {
            if cfg!(windows)
                && text
                    .as_bytes()
                    .get(2)
                    .is_some_and(|byte| *byte == b'/' || *byte == b'\\')
            {
                &text[3..]
            } else {
                return Err(PathIssue::Drive);
            }
        }
        Some(PathIssue::Absolute) => {
            if text.starts_with('\\') && !cfg!(windows) {
                return Err(PathIssue::Separator);
            }
            &text[1..]
        }
        _ => text,
    };
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
