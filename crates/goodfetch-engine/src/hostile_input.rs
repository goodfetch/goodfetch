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
//! `/` and `\` alike. ADR 0067's note for #644 puts the check in each
//! module that makes a file of such a name (a screenshot, Run trace, Plan
//! or download name); none calls it yet.

use goodfetch_protocol::hostile_input::{
    HEADER_VALUE_INVALID, HeaderCharacter, HeaderValueInvalidDetails, PATH_UNSAFE, PathHazard,
    PathUnsafeDetails,
};
use goodfetch_saved_file::{PathIssue, PathShape as SavedPathShape, check_relative_path};

use crate::call_error::CallError;

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
    let saved_shape = match shape {
        PathShape::Name => SavedPathShape::Name,
        PathShape::RelativePath => SavedPathShape::RelativePath,
    };
    check_relative_path(name, saved_shape).map_err(|issue| PathUnsafe {
        name: name.to_owned(),
        hazard: hazard_of(issue),
    })
}

/// Maps the shared path check's issue to the protocol's [`PathHazard`].
const fn hazard_of(issue: PathIssue) -> PathHazard {
    match issue {
        PathIssue::ParentTraversal => PathHazard::ParentTraversal,
        PathIssue::Absolute => PathHazard::Absolute,
        PathIssue::Drive => PathHazard::Drive,
        PathIssue::Verbatim => PathHazard::Verbatim,
        PathIssue::DeviceNamespace => PathHazard::DeviceNamespace,
        PathIssue::Unc => PathHazard::Unc,
        PathIssue::ReservedName => PathHazard::ReservedName,
        PathIssue::TrailingDotOrSpace => PathHazard::TrailingDotOrSpace,
        PathIssue::AlternateDataStream => PathHazard::AlternateDataStream,
        PathIssue::ForbiddenCharacter => PathHazard::ForbiddenCharacter,
        PathIssue::ControlCharacter => PathHazard::ControlCharacter,
        PathIssue::Normalisation => PathHazard::Normalisation,
        PathIssue::Overlong => PathHazard::Overlong,
        PathIssue::DotName => PathHazard::DotName,
        PathIssue::Separator => PathHazard::Separator,
    }
}

#[cfg(test)]
mod tests;
