//! Page bodies as files in their Run's folder (ADR 0024): each one under
//! `bodies/`, named by the lowercase hex SHA-256 of its bytes, so a body
//! stored twice is one file, and written whole before its name appears, so
//! a reader never finds a partial one; its name is synced too, so a body the
//! run state names survives a power cut with it (ADR 0002).

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use crate::durable;

/// The folder of a Run's bodies, inside its folder.
const BODIES: &str = "bodies";

/// The file of the body `body` in the Run folder `folder`, or `None` when
/// `body` is no body's ID.
pub(crate) fn file(folder: &Path, body: &str) -> Option<PathBuf> {
    let is_id = body.len() == 64
        && body
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    is_id.then(|| folder.join(BODIES).join(body))
}

/// Writes `bytes` as a body of the Run folder `folder`, and answers its ID
/// and its file. It blocks: run it through the owner's blocking policy.
///
/// # Errors
///
/// When the file cannot be written.
pub(crate) fn store(folder: &Path, bytes: &[u8]) -> io::Result<(String, PathBuf)> {
    let id = data_encoding::HEXLOWER.encode(&Sha256::digest(bytes));
    let file = file(folder, &id).expect("invariant: a SHA-256 in lowercase hex is a body's ID");
    let bodies = folder.join(BODIES);
    durable::create_folder(&bodies)?;
    let mut partial = tempfile::NamedTempFile::new_in(&bodies)?;
    partial.write_all(bytes)?;
    partial.as_file().sync_all()?;
    partial.persist(&file).map_err(|error| error.error)?;
    durable::sync_folder(&bodies);
    Ok((id, file))
}
