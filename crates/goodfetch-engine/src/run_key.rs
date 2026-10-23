//! A Run's field encryption (ADRs 0015 and 0055, spec #564): what a Run
//! creates itself, such as cookies, tokens and login state, is sealed under
//! its own random 256-bit data key before it reaches run state, so a copied
//! Checkpoint is useless without the key.
//!
//! Each field is sealed with XChaCha20-Poly1305 under a random 192-bit
//! nonce, the Run ID, table, column and row as its associated data, each
//! length-prefixed, so a value moved to another row, column, table or Run
//! does not open. The data key is stored only wrapped: sealed the same way
//! under a key HKDF-SHA256 derives from the key-encryption key that
//! `run_state.key_reference` names, the Run ID as associated data. Neither
//! key is ever written in clear.

use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use goodfetch_protocol::run::RunId;
use hkdf::Hkdf;
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::Sha256;

/// The length of a data key, in bytes.
const KEY_LENGTH: usize = 32;

/// The length of a nonce, in bytes.
const NONCE_LENGTH: usize = 24;

/// The first byte of every sealed value and wrapped key: the version of
/// this layout, `version ‖ nonce ‖ ciphertext and tag`.
const LAYOUT: u8 = 1;

/// What a field's associated data starts with.
const FIELD: &[u8] = b"goodfetch run_key field";

/// What a wrapped data key's associated data starts with.
const WRAP: &[u8] = b"goodfetch run_key wrap";

/// HKDF-SHA256's `info` for the key that wraps a data key.
const WRAPPING_KEY: &[u8] = b"goodfetch run_state key-encryption key";

/// One Run's data key, which seals and opens its fields. Its bytes are
/// zeroized when it drops and never formatted.
pub struct RunKey {
    run_id: RunId,
    key: Zeroizing<[u8; KEY_LENGTH]>,
}

/// A data key as run state stores it, wrapped by a key-encryption key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wrapped(Vec<u8>);

/// Why a key was not made, or a value did not open.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KeyError {
    /// The OS generator gave no random bytes.
    #[error("the OS generator gave no random bytes: {0}")]
    NoRandomness(getrandom::Error),
    /// The value, or the wrapped key, does not open under this key at this
    /// place: it was moved, changed, or sealed under another key.
    #[error("the value does not open under this key at this place")]
    DoesNotOpen,
}

impl std::fmt::Debug for RunKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunKey")
            .field("run_id", &self.run_id.as_str())
            .finish_non_exhaustive()
    }
}

impl RunKey {
    /// A new random data key for the Run `run_id`, from the OS generator.
    ///
    /// # Errors
    ///
    /// `NoRandomness` when the OS gives no random bytes.
    pub fn generate(run_id: RunId) -> Result<Self, KeyError> {
        let mut key = Zeroizing::new([0; KEY_LENGTH]);
        getrandom::fill(key.as_mut_slice()).map_err(KeyError::NoRandomness)?;
        Ok(Self { run_id, key })
    }

    /// The data key `key` of the Run `run_id`, which a test builds.
    #[cfg(test)]
    pub(crate) fn from_bytes(run_id: RunId, key: [u8; KEY_LENGTH]) -> Self {
        Self {
            run_id,
            key: Zeroizing::new(key),
        }
    }

    /// The Run it seals for.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// `plaintext` sealed for the field of `table` and `column` in the row
    /// `row`.
    ///
    /// # Errors
    ///
    /// `NoRandomness` when the OS gives no nonce.
    pub fn seal(
        &self,
        table: &str,
        column: &str,
        row: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, KeyError> {
        let place = associated(FIELD, &[self.run_id.as_str(), table, column, row]);
        seal(&self.key, &place, plaintext)
    }

    /// The plaintext of `sealed`, the field of `table` and `column` in the
    /// row `row`.
    ///
    /// # Errors
    ///
    /// `DoesNotOpen` when it was sealed elsewhere, under another key, or
    /// changed since.
    pub fn open(
        &self,
        table: &str,
        column: &str,
        row: &str,
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        let place = associated(FIELD, &[self.run_id.as_str(), table, column, row]);
        open(&self.key, &place, sealed)
    }

    /// The data key wrapped by `kek`, the key-encryption key, as run state
    /// stores it.
    ///
    /// # Errors
    ///
    /// `NoRandomness` when the OS gives no nonce.
    pub fn wrap(&self, kek: &SecretString) -> Result<Wrapped, KeyError> {
        let place = associated(WRAP, &[self.run_id.as_str()]);
        seal(&wrapping_key(kek), &place, self.key.as_slice()).map(Wrapped)
    }

    /// The data key of the Run `run_id` that `wrapped` holds, unwrapped by
    /// `kek`.
    ///
    /// # Errors
    ///
    /// `DoesNotOpen` when `kek` is not the key that wrapped it, it was
    /// wrapped for another Run, or it changed since.
    pub fn unwrap(run_id: RunId, wrapped: &Wrapped, kek: &SecretString) -> Result<Self, KeyError> {
        let place = associated(WRAP, &[run_id.as_str()]);
        let opened = open(&wrapping_key(kek), &place, &wrapped.0)?;
        let mut key = Zeroizing::new([0; KEY_LENGTH]);
        if opened.len() != KEY_LENGTH {
            return Err(KeyError::DoesNotOpen);
        }
        key.copy_from_slice(&opened);
        Ok(Self { run_id, key })
    }

    /// The data key's bytes, which a test searches files for.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &[u8; KEY_LENGTH] {
        &self.key
    }
}

impl Wrapped {
    /// A wrapped key as run state holds it.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Its bytes, as run state stores them.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// The table that holds a Run's wrapped data key, once it has one, with
/// the Secret reference of the key-encryption key that wrapped it.
pub const TABLE: &str = "CREATE TABLE IF NOT EXISTS run_key (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    wrapped BLOB NOT NULL,
    key_reference TEXT NOT NULL
) STRICT";

/// A wrapped data key as a run file holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredKey {
    /// The wrapped key.
    pub wrapped: Wrapped,
    /// The Secret reference of the key-encryption key that wrapped it.
    pub key_reference: String,
}

/// Stores `stored` as the Run's wrapped data key in the run file
/// `connection` writes, in place of none.
///
/// # Errors
///
/// SQLite's, and a constraint failure when the file holds a key already.
pub fn store(connection: &rusqlite::Connection, stored: &StoredKey) -> rusqlite::Result<()> {
    connection
        .execute(
            "INSERT INTO run_key (id, wrapped, key_reference) VALUES (1, ?1, ?2)",
            (stored.wrapped.as_bytes(), stored.key_reference.as_str()),
        )
        .map(drop)
}

/// The Run's wrapped data key in the run file or Checkpoint `connection`
/// reads, if it has one.
///
/// # Errors
///
/// SQLite's.
pub fn stored(connection: &rusqlite::Connection) -> rusqlite::Result<Option<StoredKey>> {
    let mut statement = connection.prepare("SELECT wrapped, key_reference FROM run_key")?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(StoredKey {
        wrapped: Wrapped(row.get(0)?),
        key_reference: row.get(1)?,
    }))
}

/// The associated data of `fields` under `domain`: each field's length, as
/// four big-endian bytes, then the field, so no two lists of fields share
/// it.
fn associated(domain: &[u8], fields: &[&str]) -> Vec<u8> {
    let mut data = Vec::with_capacity(
        1 + domain.len() + fields.iter().map(|field| 4 + field.len()).sum::<usize>(),
    );
    data.push(LAYOUT);
    data.extend_from_slice(domain);
    for field in fields {
        let length = u32::try_from(field.len()).expect("invariant: a name is under 4 GiB");
        data.extend_from_slice(&length.to_be_bytes());
        data.extend_from_slice(field.as_bytes());
    }
    data
}

/// The key that wraps a data key: HKDF-SHA256 of the key-encryption key's
/// text.
fn wrapping_key(kek: &SecretString) -> Zeroizing<[u8; KEY_LENGTH]> {
    let mut key = Zeroizing::new([0; KEY_LENGTH]);
    Hkdf::<Sha256>::new(None, kek.expose_secret().as_bytes())
        .expand(WRAPPING_KEY, key.as_mut_slice())
        .expect("invariant: 32 bytes is a valid HKDF-SHA256 output length");
    key
}

/// `plaintext` sealed under `key` with `associated` as its associated data,
/// a fresh random nonce before it.
fn seal(key: &[u8; KEY_LENGTH], associated: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
    let mut nonce = [0; NONCE_LENGTH];
    getrandom::fill(&mut nonce).map_err(KeyError::NoRandomness)?;
    let ciphertext = XChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: associated,
            },
        )
        .expect("invariant: XChaCha20-Poly1305 seals any value under 256 GiB");
    let mut sealed = Vec::with_capacity(1 + NONCE_LENGTH + ciphertext.len());
    sealed.push(LAYOUT);
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

/// The plaintext of `sealed` under `key` with `associated` as its
/// associated data.
fn open(
    key: &[u8; KEY_LENGTH],
    associated: &[u8],
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, KeyError> {
    let Some((&LAYOUT, rest)) = sealed.split_first() else {
        return Err(KeyError::DoesNotOpen);
    };
    let Some((nonce, ciphertext)) = rest.split_at_checked(NONCE_LENGTH) else {
        return Err(KeyError::DoesNotOpen);
    };
    XChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: associated,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| KeyError::DoesNotOpen)
}

#[cfg(test)]
mod tests;
