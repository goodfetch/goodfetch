//! Where a folder still holds a Secret canary (ADR 0067): every file under
//! it is searched for each canary's value raw and in the encodings
//! Goodfetch produces, JSON-escaped, URL-encoded (as Goodfetch, a browser's
//! form and JavaScript's `encodeURIComponent` encode it), base64 (standard and
//! URL-safe, at each of the three alignments a value can start at inside a
//! longer base64 text) and hexadecimal (lower and upper case). The forms are
//! those of test support's canary search, whose shared fixture this module's
//! tests read, so the reset gate's scan and the tests' scans find the same
//! hits; the harness depends on no Goodfetch crate but `goodfetch-protocol`,
//! so it keeps its own copy of the forms.
#![expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use regex::bytes::Regex;

/// A canary's name and the forms it is searched in, each with its
/// encoding's name, as a literal search that runs at `memchr`'s speed
/// through a Browser cache's or a Daemon's copied executable's megabytes.
type Forms<'a> = (&'a str, Vec<(&'static str, Regex)>);

/// One place a file holds a canary.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Held {
    /// The canary's name, as the caller gave it.
    pub(crate) canary: String,
    /// The file.
    pub(crate) file: PathBuf,
    /// The encoding it was found in: `raw`, `json_escaped`, `url_encoded`,
    /// `base64` or `hex`.
    pub(crate) encoding: &'static str,
}

/// Every place a file under `root`, or `root` itself, holds one of
/// `canaries`, each a name and a value; a `root` that does not exist holds
/// none.
///
/// # Errors
///
/// When a file or a folder under it cannot be read.
pub(crate) fn held_under(root: &Path, canaries: &[(&str, &str)]) -> io::Result<Vec<Held>> {
    let forms: Vec<Forms<'_>> = canaries
        .iter()
        .map(|(name, value)| {
            let literal = |form: Vec<u8>| {
                let text =
                    String::from_utf8(form).expect("invariant: every form of a text is text");
                Regex::new(&regex::escape(&text)).expect("invariant: an escaped literal compiles")
            };
            let searched = forms(value)
                .into_iter()
                .map(|(encoding, form)| (encoding, literal(form)))
                .collect();
            (*name, searched)
        })
        .collect();
    let mut held = Vec::new();
    for file in files(root)? {
        // A file the engine removed since the folder was listed, such as a
        // WAL, holds nothing any more.
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for (name, forms) in &forms {
            for (encoding, form) in forms {
                if form.is_match(&bytes) {
                    held.push(Held {
                        canary: (*name).to_owned(),
                        file: file.clone(),
                        encoding,
                    });
                }
            }
        }
    }
    Ok(held)
}

/// Every regular file under `path`, or `path` itself when it is one: a
/// socket, such as a Daemon's, holds no bytes, and a symbolic link is not
/// followed, so nothing outside `path` is read.
fn files(path: &Path) -> io::Result<Vec<PathBuf>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if metadata.is_file() {
        return Ok(vec![path.to_owned()]);
    }
    if !metadata.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(path)? {
        files.extend(self::files(&entry?.path())?);
    }
    files.sort();
    Ok(files)
}

/// The forms `value` is searched in, each once.
fn forms(value: &str) -> Vec<(&'static str, Vec<u8>)> {
    let bytes = value.as_bytes();
    let mut forms: Vec<(&'static str, Vec<u8>)> = Vec::new();
    let mut add = |encoding: &'static str, form: Vec<u8>| {
        if !form.is_empty() && forms.iter().all(|(_, earlier)| *earlier != form) {
            forms.push((encoding, form));
        }
    };
    add("raw", bytes.to_vec());
    let quoted = serde_json::to_string(value).expect("invariant: a string serializes to JSON");
    add(
        "json_escaped",
        quoted.as_bytes()[1..quoted.len() - 1].to_vec(),
    );
    add("url_encoded", url_encoded(bytes, UNRESERVED).into_bytes());
    add(
        "url_encoded",
        url_encoded(bytes, FORM_KEPT)
            .replace("%20", "+")
            .into_bytes(),
    );
    add(
        "url_encoded",
        url_encoded(bytes, URI_COMPONENT_KEPT).into_bytes(),
    );
    for alphabet in [
        &data_encoding::BASE64_NOPAD,
        &data_encoding::BASE64URL_NOPAD,
    ] {
        for alignment in 0..3 {
            add("base64", base64_at(alphabet, bytes, alignment));
        }
    }
    add("hex", data_encoding::HEXLOWER.encode(bytes).into_bytes());
    add("hex", data_encoding::HEXUPPER.encode(bytes).into_bytes());
    forms
}

/// The marks RFC 3986 leaves unreserved.
const UNRESERVED: &[u8] = b"-._~";

/// The marks a browser keeps as it encodes a form
/// (`application/x-www-form-urlencoded`), which writes a space `+`.
const FORM_KEPT: &[u8] = b"*-._";

/// The marks JavaScript's `encodeURIComponent` keeps.
const URI_COMPONENT_KEPT: &[u8] = b"-_.!~*'()";

/// `value` percent-encoded, every byte but the ASCII letters, digits and
/// `kept`.
fn url_encoded(value: &[u8], kept: &[u8]) -> String {
    let mut encoded = String::new();
    for byte in value {
        if byte.is_ascii_alphanumeric() || kept.contains(byte) {
            encoded.push(char::from(*byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("invariant: writing to a String succeeds");
        }
    }
    encoded
}

/// The base64 characters that encode `value` alone when it starts
/// `alignment` bytes into a group of three: the characters shared with the
/// bytes before or after it are cut.
fn base64_at(alphabet: &data_encoding::Encoding, value: &[u8], alignment: usize) -> Vec<u8> {
    let mut padded = vec![0; alignment];
    padded.extend_from_slice(value);
    let encoded = alphabet.encode(&padded).into_bytes();
    let start = [0, 2, 3][alignment];
    let end = padded.len() / 3 * 4 + padded.len() % 3;
    if start >= end {
        return Vec::new();
    }
    encoded[start..end].to_vec()
}

#[cfg(test)]
mod tests;
