//! Hostile input a caller or a configuration hands the engine (ADR 0067):
//! a header value it would send, and a name or path from outside input it
//! would write a file under. Each is judged before it is used, so nothing it
//! carries reaches the wire, the browser or the file system.
//!
//! A header value holding a CR, an LF or a NUL is refused with
//! `header_value_invalid`, before it reaches CDP or a connection: it could
//! end its header and start another. Chromium refuses one too (ADR 0067's
//! note of 2026-10-20), but the engine's own check comes first, the same on
//! every path a header takes.
//!
//! A name or path is refused with `path_unsafe` when it would leave its
//! folder or is no safe file name on Linux, macOS or Windows, on every OS
//! alike, so a Run moved to another machine names the same files: a `..`,
//! `.` or empty part; an absolute, drive, UNC, verbatim or device path; a
//! name Windows reserves for a device, with or without an extension, and in
//! any case; an alternate data stream; a character Windows forbids or a
//! control character; a trailing dot or space; a part longer than 255
//! bytes; a name not in Unicode's composed form, which APFS would open as
//! the composed name it equals. A [`PathShape::Name`] takes one name, a
//! [`PathShape::RelativePath`] a path to a subfolder, its parts split on
//! `/` and `\` alike.

use goodfetch_protocol::hostile_input::{
    HEADER_VALUE_INVALID, HeaderCharacter, HeaderValueInvalidDetails, PATH_UNSAFE, PathHazard,
    PathUnsafeDetails,
};
use icu_normalizer::ComposingNormalizerBorrowed;

use crate::call_error::CallError;

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

/// A header value the engine refuses to send: `header_value_invalid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderValueInvalid {
    /// The header's name.
    pub name: String,
    /// The first character its value may not hold.
    pub character: HeaderCharacter,
}

impl HeaderValueInvalid {
    /// The refusal as the call fails with: `header_value_invalid`, naming
    /// the header and the character, never the value, which may hold a
    /// Secret.
    pub fn error(&self) -> CallError {
        let character = match self.character {
            HeaderCharacter::Cr => "a CR",
            HeaderCharacter::Lf => "an LF",
            HeaderCharacter::Nul => "a NUL",
        };
        CallError::new(
            &HEADER_VALUE_INVALID,
            format!(
                "the value of the header `{}` holds {character}, which no header value may hold",
                self.name
            ),
            &HeaderValueInvalidDetails {
                name: self.name.clone(),
                character: self.character,
            },
        )
    }
}

/// Judges the value of the header `name` before it is sent.
///
/// # Errors
///
/// [`HeaderValueInvalid`] for a value holding a CR, an LF or a NUL.
pub fn check_header_value(name: &str, value: &str) -> Result<(), HeaderValueInvalid> {
    let character = value.chars().find_map(|character| match character {
        '\r' => Some(HeaderCharacter::Cr),
        '\n' => Some(HeaderCharacter::Lf),
        '\0' => Some(HeaderCharacter::Nul),
        _ => None,
    });
    character.map_or(Ok(()), |character| {
        Err(HeaderValueInvalid {
            name: name.to_owned(),
            character,
        })
    })
}

/// Judges each of `headers`, names and values, in order.
///
/// # Errors
///
/// The first [`HeaderValueInvalid`].
pub fn check_headers<'a>(
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<(), HeaderValueInvalid> {
    headers
        .into_iter()
        .try_for_each(|(name, value)| check_header_value(name, value))
}

/// What a name from outside input names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathShape {
    /// One file name, such as a Plan's or a Key-value store key.
    Name,
    /// A path under the folder it is written in, to a subfolder or not,
    /// such as a screenshot's or a Run trace's.
    RelativePath,
}

/// A name or path the engine refuses to write under: `path_unsafe`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathUnsafe {
    /// The name or path, as given.
    pub name: String,
    /// What makes it unsafe.
    pub hazard: PathHazard,
}

impl PathUnsafe {
    /// The refusal as the call fails with: `path_unsafe`, naming the path
    /// and its hazard.
    pub fn error(&self) -> CallError {
        CallError::new(
            &PATH_UNSAFE,
            format!(
                "`{}` is no safe name on every OS ({:?})",
                self.name.escape_debug(),
                self.hazard
            ),
            &PathUnsafeDetails {
                name: self.name.clone(),
                hazard: self.hazard,
            },
        )
    }
}

/// Judges `name`, from outside input, as a [`PathShape`] the engine would
/// write a file under.
///
/// # Errors
///
/// [`PathUnsafe`] with the first hazard found: the whole path's, then
/// each part's, then a separator where one name is taken.
pub fn check_path(name: &str, shape: PathShape) -> Result<(), PathUnsafe> {
    hazard(name, shape).map_or(Ok(()), |hazard| {
        Err(PathUnsafe {
            name: name.to_owned(),
            hazard,
        })
    })
}

/// The first hazard of `name` as a `shape`, if any.
fn hazard(name: &str, shape: PathShape) -> Option<PathHazard> {
    if name.is_empty() {
        return Some(PathHazard::DotName);
    }
    if name.chars().any(char::is_control) {
        return Some(PathHazard::ControlCharacter);
    }
    if let Some(hazard) = root_hazard(name) {
        return Some(hazard);
    }
    if !ComposingNormalizerBorrowed::new_nfc().is_normalized(name) {
        return Some(PathHazard::Normalisation);
    }
    let parts: Vec<&str> = name.split(['/', '\\']).collect();
    if let Some(hazard) = parts.iter().find_map(|part| part_hazard(part)) {
        // An empty last part is a trailing separator, which a name never
        // takes and a path does not need.
        let trailing = parts.last().is_some_and(|last| last.is_empty())
            && parts
                .iter()
                .rev()
                .skip(1)
                .all(|part| part_hazard(part).is_none());
        if !(trailing && shape == PathShape::Name) {
            return Some(hazard);
        }
    }
    (shape == PathShape::Name && parts.len() > 1).then_some(PathHazard::Separator)
}

/// What makes `name`, as a whole, start outside its folder: a verbatim,
/// device or UNC path, a path from the root, or a drive.
fn root_hazard(name: &str) -> Option<PathHazard> {
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
            Some(PathHazard::Verbatim)
        }
        (Some(a), Some(b), Some('.'), Some(d)) if separator(a) && separator(b) && separator(d) => {
            Some(PathHazard::DeviceNamespace)
        }
        (Some(a), Some(b), ..) if separator(a) && separator(b) => Some(PathHazard::Unc),
        (Some(a), ..) if separator(a) => Some(PathHazard::Absolute),
        (Some(letter), Some(':'), ..) if letter.is_ascii_alphabetic() => Some(PathHazard::Drive),
        _ => None,
    }
}

/// What makes one part of a path unsafe, if anything.
fn part_hazard(part: &str) -> Option<PathHazard> {
    if part.is_empty() || part == "." {
        return Some(PathHazard::DotName);
    }
    if part == ".." {
        return Some(PathHazard::ParentTraversal);
    }
    if part.contains(':') {
        return Some(PathHazard::AlternateDataStream);
    }
    if part.contains(FORBIDDEN) {
        return Some(PathHazard::ForbiddenCharacter);
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return Some(PathHazard::TrailingDotOrSpace);
    }
    if is_reserved(part) {
        return Some(PathHazard::ReservedName);
    }
    (part.len() > NAME_MAX_BYTES).then_some(PathHazard::Overlong)
}

/// Whether `part` is a device name Windows reserves: alone, or before an
/// extension, in any case, spaces before the extension dropped as Windows
/// drops them.
fn is_reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end_matches(' ');
    let upper = stem.to_uppercase();
    RESERVED.contains(&upper.as_str())
}

#[cfg(test)]
mod tests;
