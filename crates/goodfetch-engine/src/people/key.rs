//! The Console key (ADRs 0031 and 0098): 32 bytes of the OS generator in a
//! `0600` file of its own in Goodfetch home, outside the per-major folders,
//! so every major of the user shares it. Every sign-in's verifier and CSRF
//! token is an HMAC under it, so replacing it ends every sign-in of every
//! major at once. Its calls block: callers run them under their owner's
//! blocking policy.
#![expect(
    clippy::disallowed_methods,
    reason = "the Console key's calls run under their caller's blocking policy"
)]

use std::path::{Path, PathBuf};

use crate::durable;

/// How many bytes of the OS generator the key holds.
const KEY_BYTES: usize = 32;

/// Why the Console key could not be read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KeyError {
    /// The file system failed.
    #[error("the Console key at `{path}`: {source}")]
    Io {
        /// The key's file.
        path: PathBuf,
        /// What failed.
        source: std::io::Error,
    },
    /// The OS generator gave no key.
    #[error("the OS generator gave no Console key: {0}")]
    Generator(getrandom::Error),
}

/// The Console key's file.
#[derive(Clone, Debug)]
pub struct ConsoleKey {
    path: PathBuf,
}

impl ConsoleKey {
    /// The key at `path`.
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Its file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The key's bytes, written first when the file holds none; a file
    /// that holds no key of its length is replaced.
    pub fn read_or_create(&self) -> Result<Vec<u8>, KeyError> {
        match std::fs::read(&self.path) {
            Ok(text) => {
                if let Some(key) = decode(&text) {
                    return Ok(key);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(KeyError::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        }
        self.replace()
    }

    /// Replaces the key with a new one from the OS generator, through a
    /// file renamed over it, so a reader never sees half of one, and the
    /// key it answers is the key after a power cut (ADR 0002).
    pub fn replace(&self) -> Result<Vec<u8>, KeyError> {
        let io = |source| KeyError::Io {
            path: self.path.clone(),
            source,
        };
        let mut key = vec![0; KEY_BYTES];
        getrandom::fill(&mut key).map_err(KeyError::Generator)?;
        if let Some(folder) = self.path.parent() {
            durable::create_folder(folder).map_err(io)?;
        }
        // `0600` from its creation on, as `durable::replace` writes it.
        durable::replace(&self.path, data_encoding::HEXLOWER.encode(&key).as_bytes())
            .map_err(io)?;
        Ok(key)
    }
}

/// The key a file's text holds, in hex; none for any other text.
fn decode(text: &[u8]) -> Option<Vec<u8>> {
    let key = data_encoding::HEXLOWER.decode(text.trim_ascii()).ok()?;
    (key.len() == KEY_BYTES).then_some(key)
}
