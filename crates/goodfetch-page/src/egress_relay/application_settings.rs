//! ALPS, the application-layer protocol settings a TLS client sends in its
//! encrypted extensions once the server accepted them (ADRs 0042 and
//! 0124), a payload per ALPN protocol. The relay's upstream handshake ends
//! before the browser's own handshake with the relay starts, so the
//! payload the relay sends a site is the one captured from the pinned
//! browser build when that build was pinned (`capture`, which
//! `pin-chrome-for-testing alps` runs), never one the browser sends at that
//! moment.
//!
//! `btls` sends ALPS with an empty payload only and reads none, so the
//! relay calls BoringSSL's own functions through `btls-sys`.

use std::collections::BTreeMap;

use btls::error::ErrorStack;
use btls::ssl::SslRef;
use foreign_types::ForeignTypeRef as _;

/// The ALPS payload a browser build sends, by the ALPN protocol it goes
/// with, such as `h2`.
pub(crate) type ApplicationSettings = BTreeMap<String, Vec<u8>>;

/// Offers ALPS for `protocol` on `ssl`, a client's or a server's, with
/// `settings` as this end's payload.
///
/// # Errors
///
/// BoringSSL's, when it refuses the protocol.
#[expect(
    unsafe_code,
    reason = "btls sends no ALPS payload, so the relay calls BoringSSL's `SSL_add_application_settings` through btls-sys (ADR 0124)"
)]
pub(crate) fn offer(ssl: &mut SslRef, protocol: &[u8], settings: &[u8]) -> Result<(), ErrorStack> {
    // SAFETY: `ssl.as_ptr()` is the live `SSL` that `ssl` borrows for the
    // whole call, and BoringSSL copies the `protocol.len()` and
    // `settings.len()` bytes the two pointers name before it returns,
    // keeping no pointer to them.
    let added = unsafe {
        btls_sys::SSL_add_application_settings(
            ssl.as_ptr(),
            protocol.as_ptr(),
            protocol.len(),
            settings.as_ptr(),
            settings.len(),
        )
    };
    if added == 1 {
        Ok(())
    } else {
        Err(ErrorStack::get())
    }
}

/// The ALPS payload the peer of `ssl` sent, once its handshake finished;
/// `None` when ALPS was not negotiated.
#[expect(
    unsafe_code,
    reason = "btls reads no ALPS payload, so the relay calls BoringSSL's `SSL_get0_peer_application_settings` through btls-sys (ADR 0124)"
)]
pub(crate) fn of_peer(ssl: &SslRef) -> Option<Vec<u8>> {
    // SAFETY: `ssl.as_ptr()` is the live `SSL` that `ssl` borrows for the
    // whole call; `SSL_has_application_settings` only reads it.
    if unsafe { btls_sys::SSL_has_application_settings(ssl.as_ptr()) } != 1 {
        return None;
    }
    let mut data: *const u8 = std::ptr::null();
    let mut length = 0_usize;
    // SAFETY: as above; BoringSSL sets `data` to `length` bytes it owns
    // inside `ssl`, which stay valid while `ssl` is borrowed, and they are
    // copied out before the borrow ends.
    unsafe {
        btls_sys::SSL_get0_peer_application_settings(ssl.as_ptr(), &raw mut data, &raw mut length);
        if data.is_null() {
            return Some(Vec::new());
        }
        Some(std::slice::from_raw_parts(data, length).to_vec())
    }
}
