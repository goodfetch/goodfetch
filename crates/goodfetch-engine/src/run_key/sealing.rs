//! The sealing a Run's key does (spec #564): XChaCha20-Poly1305 under a
//! random 192-bit nonce, the place's names, each length-prefixed, as the
//! associated data; a field's place names its Run, table, column and row,
//! a wrapped data key's its Run. A data key is wrapped under the key
//! HKDF-SHA256 derives from the key-encryption key's text. The key's bytes
//! leave a [`RunKey`](super::RunKey) only through these functions.

use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::Sha256;

use super::KeyError;

/// The length of a data key, in bytes.
pub(crate) const KEY_LENGTH: usize = 32;

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

/// `plaintext` sealed under `key` for the field of `table` and `column` in
/// the row `row` of the Run `run_id`.
pub(super) fn seal_field(
    key: &[u8; KEY_LENGTH],
    run_id: &str,
    [table, column, row]: [&str; 3],
    plaintext: &[u8],
) -> Result<Vec<u8>, KeyError> {
    seal(
        key,
        &associated(FIELD, &[run_id, table, column, row]),
        plaintext,
    )
}

/// The plaintext of `sealed` under `key`, the field of `table` and
/// `column` in the row `row` of the Run `run_id`.
pub(super) fn open_field(
    key: &[u8; KEY_LENGTH],
    run_id: &str,
    [table, column, row]: [&str; 3],
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, KeyError> {
    open(
        key,
        &associated(FIELD, &[run_id, table, column, row]),
        sealed,
    )
}

/// The data key `key` of the Run `run_id` wrapped by `kek`.
pub(super) fn wrap(
    key: &[u8; KEY_LENGTH],
    run_id: &str,
    kek: &SecretString,
) -> Result<Vec<u8>, KeyError> {
    seal(&wrapping_key(kek), &associated(WRAP, &[run_id]), key)
}

/// The data key of the Run `run_id` that `wrapped` holds, unwrapped by
/// `kek`.
///
/// # Errors
///
/// `DoesNotOpen` when `kek` is not the key that wrapped it, it was wrapped
/// for another Run, or it changed since.
pub(crate) fn unwrap(
    run_id: &str,
    wrapped: &[u8],
    kek: &SecretString,
) -> Result<Zeroizing<[u8; KEY_LENGTH]>, KeyError> {
    let opened = open(&wrapping_key(kek), &associated(WRAP, &[run_id]), wrapped)?;
    let mut key = Zeroizing::new([0; KEY_LENGTH]);
    if opened.len() != KEY_LENGTH {
        return Err(KeyError::DoesNotOpen);
    }
    key.copy_from_slice(&opened);
    Ok(key)
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
