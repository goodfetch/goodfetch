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

pub(crate) mod sealing;

use goodfetch_protocol::run::RunId;
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;

use sealing::KEY_LENGTH;

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
        sealing::seal_field(
            &self.key,
            self.run_id.as_str(),
            [table, column, row],
            plaintext,
        )
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
        sealing::open_field(
            &self.key,
            self.run_id.as_str(),
            [table, column, row],
            sealed,
        )
    }

    /// The data key wrapped by `kek`, the key-encryption key, as run state
    /// stores it.
    ///
    /// # Errors
    ///
    /// `NoRandomness` when the OS gives no nonce.
    pub fn wrap(&self, kek: &SecretString) -> Result<Wrapped, KeyError> {
        sealing::wrap(&self.key, self.run_id.as_str(), kek).map(Wrapped)
    }

    /// The data key of the Run `run_id` that `wrapped` holds, unwrapped by
    /// `kek`.
    ///
    /// # Errors
    ///
    /// `DoesNotOpen` when `kek` is not the key that wrapped it, it was
    /// wrapped for another Run, or it changed since.
    pub fn unwrap(run_id: RunId, wrapped: &Wrapped, kek: &SecretString) -> Result<Self, KeyError> {
        let key = sealing::unwrap(run_id.as_str(), &wrapped.0, kek)?;
        Ok(Self { run_id, key })
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

#[cfg(test)]
mod tests;
