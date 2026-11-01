//! A sign-in's offline cache key (ADR 0031; spec #592's `offline`): the key
//! the Console's browser cache is encrypted under, which the page keeps only
//! wrapped. It is wrapped with XChaCha20-Poly1305 under the sign-in's own
//! wrapping key, which the accounts store keeps and never sends, the
//! sign-in's identifier as associated data, under a random 192-bit nonce:
//! `version ‖ nonce ‖ ciphertext and tag`. Only a request carrying the
//! sign-in's cookie has it unwrapped, so once the sign-in ends, its row and
//! wrapping key gone, the cache it encrypts can no longer be read.

use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use secrecy::zeroize::Zeroizing;

/// The length of a cache key and of a wrapping key, in bytes.
pub(super) const KEY_LENGTH: usize = 32;

/// The length of a nonce, in bytes.
const NONCE_LENGTH: usize = 24;

/// The first byte of every wrapped key: the version of its layout.
const LAYOUT: u8 = 1;

/// What a wrapped key's associated data starts with.
const WRAP: &[u8] = b"goodfetch console offline cache key\0";

/// Why a cache key was not wrapped or unwrapped.
#[derive(Debug)]
pub(super) enum Unwrapped {
    /// The OS generator gave no key or nonce.
    Generator(getrandom::Error),
    /// The wrapped key does not open under this sign-in's wrapping key: it
    /// is another sign-in's, or was changed.
    DoesNotOpen,
}

/// A new cache key from the OS generator, and its wrapped form under the
/// hex `wrapping_key` of the sign-in `id`.
pub(super) fn created(
    wrapping_key: &str,
    id: &str,
) -> Result<(Zeroizing<Vec<u8>>, Vec<u8>), Unwrapped> {
    let mut key = Zeroizing::new(vec![0; KEY_LENGTH]);
    getrandom::fill(key.as_mut_slice()).map_err(Unwrapped::Generator)?;
    let mut nonce = [0; NONCE_LENGTH];
    getrandom::fill(&mut nonce).map_err(Unwrapped::Generator)?;
    let cipher = cipher(wrapping_key)?;
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &key,
                aad: &associated(id),
            },
        )
        .map_err(|_| Unwrapped::DoesNotOpen)?;
    let mut wrapped = Vec::with_capacity(1 + NONCE_LENGTH + sealed.len());
    wrapped.push(LAYOUT);
    wrapped.extend_from_slice(&nonce);
    wrapped.extend_from_slice(&sealed);
    Ok((key, wrapped))
}

/// The cache key `wrapped` holds, under the hex `wrapping_key` of the
/// sign-in `id`.
pub(super) fn opened(
    wrapping_key: &str,
    id: &str,
    wrapped: &[u8],
) -> Result<Zeroizing<Vec<u8>>, Unwrapped> {
    let Some((&LAYOUT, rest)) = wrapped.split_first() else {
        return Err(Unwrapped::DoesNotOpen);
    };
    if rest.len() < NONCE_LENGTH {
        return Err(Unwrapped::DoesNotOpen);
    }
    let (nonce, sealed) = rest.split_at(NONCE_LENGTH);
    let key = cipher(wrapping_key)?
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: &associated(id),
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| Unwrapped::DoesNotOpen)?;
    if key.len() == KEY_LENGTH {
        Ok(key)
    } else {
        Err(Unwrapped::DoesNotOpen)
    }
}

/// The cipher of the hex `wrapping_key`; a row whose key is no 32 bytes of
/// hex opens nothing.
fn cipher(wrapping_key: &str) -> Result<XChaCha20Poly1305, Unwrapped> {
    let bytes = Zeroizing::new(
        data_encoding::HEXLOWER
            .decode(wrapping_key.as_bytes())
            .map_err(|_| Unwrapped::DoesNotOpen)?,
    );
    if bytes.len() != KEY_LENGTH {
        return Err(Unwrapped::DoesNotOpen);
    }
    Ok(XChaCha20Poly1305::new(Key::from_slice(&bytes)))
}

/// The associated data of the sign-in `id`'s wrapped key.
fn associated(id: &str) -> Vec<u8> {
    [WRAP, id.as_bytes()].concat()
}
