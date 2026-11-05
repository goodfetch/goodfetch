//! The executables' allocator, mimalloc (ADR 0024), set so that the pages a
//! freed block held leave the process's resident memory when it is freed,
//! so the peak follows the live heap (spec #1577, user story 28). mimalloc
//! v3 waits a second (`purge_delay`, 1,000 ms) before it gives a freed span
//! back to the OS, so a crawl's freed pages stay resident while the next
//! pages allocate. Set in code, not through `MIMALLOC_PURGE_DELAY`, which
//! the executable would otherwise pass on to the browsers it starts.
#![expect(
    unsafe_code,
    reason = "mimalloc's options are set through its C function, `mi_option_set` (STYLE.md, `unsafe`)"
)]

use std::ffi::{c_int, c_long};

// The executable's global allocator is mimalloc: link it here too, so the
// symbol below resolves in every binary that calls `main`.
use mimalloc as _;

/// `mi_option_purge_delay`, its place in mimalloc v3's `mi_option_e`
/// (`include/mimalloc.h`), which `libmimalloc-sys` does not name.
const PURGE_DELAY: c_int = 15;

unsafe extern "C" {
    /// `<mimalloc.h>`: sets `option` to `value` for the whole process.
    fn mi_option_set(option: c_int, value: c_long);
}

/// Has mimalloc give a freed span's pages back to the OS at once: a purge
/// delay of 0. Called first thing in [`super::main`], before the process
/// starts a thread, since mimalloc's options are not set thread-safely.
pub(super) fn purge_at_once() {
    // SAFETY: `mi_option_set` takes any option and value (it ignores an
    // option out of range), and no other thread of the process runs yet.
    unsafe { mi_option_set(PURGE_DELAY, 0) }
}

#[cfg(test)]
mod tests;
