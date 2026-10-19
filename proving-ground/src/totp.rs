//! TOTP codes (RFC 6238): HMAC-SHA-1 over the count of 30-second steps since
//! the Unix epoch, truncated to six digits, as authenticator apps and the
//! engine's `totp` Secret placeholders compute them (ADR 0092).

use hmac::{Hmac, Mac as _};
use sha1::Sha1;

/// The length of one TOTP step.
pub(crate) const STEP_SECONDS: u64 = 30;

/// The six-digit code of `key` at `unix_seconds`.
pub(crate) fn code(key: &[u8], unix_seconds: u64) -> String {
    let mut mac =
        Hmac::<Sha1>::new_from_slice(key).expect("invariant: HMAC takes a key of any length");
    mac.update(&(unix_seconds / STEP_SECONDS).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    // RFC 4226's dynamic truncation: the low nibble of the last byte picks
    // four bytes, read without their top bit.
    let offset = usize::from(hash[hash.len() - 1] & 0x0f);
    let truncated = u32::from_be_bytes(
        hash[offset..offset + 4]
            .try_into()
            .expect("invariant: an offset below 16 leaves four of SHA-1's 20 bytes"),
    ) & 0x7fff_ffff;
    format!("{:06}", truncated % 1_000_000)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::code;

    /// RFC 6238's SHA-1 test key.
    const KEY: &[u8] = b"12345678901234567890";

    /// The last six digits of RFC 6238's eight-digit SHA-1 vectors
    /// (Appendix B).
    #[rstest]
    #[case::the_first_step(59, "287082")]
    #[case::in_2005(1_111_111_109, "081804")]
    #[case::in_2009(1_234_567_890, "005924")]
    #[case::in_2033(2_000_000_000, "279037")]
    fn a_code_is_rfc_6238s(#[case] unix_seconds: u64, #[case] expected: &str) {
        assert_eq!(code(KEY, unix_seconds), expected);
    }
}
