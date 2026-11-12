//! The executables' allocator, mimalloc (ADR 0024), set so that the pages a
//! freed block held leave the process's resident memory when it is freed,
//! so the peak follows the live heap (spec #1577, user story 28). mimalloc
//! v3 waits a second (`purge_delay`, 1,000 ms) before it gives a freed span
//! back to the OS, so a crawl's freed pages stay resident while the next
//! pages allocate. Set in code, not through `MIMALLOC_PURGE_DELAY`, which
//! the executable would otherwise pass on to the browsers it starts.
//!
//! It also starts with transparent huge pages off, unless
//! `governor.huge_pages` turns them back on (issue 2199).
#![expect(
    unsafe_code,
    reason = "mimalloc's options are set through its C function, `mi_option_set`, the first of them from a constructor's link section, and the process's huge-page flag through `prctl` (STYLE.md, `unsafe`)"
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

/// `mi_option_allow_thp`, its place in the same enumeration.
#[cfg(target_os = "linux")]
const ALLOW_THP: c_int = 43;

/// Turns transparent huge pages off for the process, the default of
/// `governor.huge_pages` (ADR 0138): it tells mimalloc not to allow them
/// before mimalloc starts. mimalloc's own constructor, of priority 101,
/// then calls `prctl(PR_SET_THP_DISABLE)` and never advises its arena, as
/// under `MIMALLOC_ALLOW_THP=0`, which the executable would otherwise
/// pass on to the browsers it starts. The entry's section name gives it
/// priority 100, so the loader runs it first; no other code of the
/// workspace runs before `main`.
#[cfg(target_os = "linux")]
#[used]
#[unsafe(link_section = ".init_array.00100")]
static HUGE_PAGES_OFF: extern "C" fn() = {
    extern "C" fn huge_pages_off() {
        // SAFETY: `mi_option_set` takes any option and value, writes the
        // option's own slot alone, and no other thread of the process
        // runs before `main`.
        unsafe { mi_option_set(ALLOW_THP, 0) }
    }
    huge_pages_off
};

/// Gives the process back the host's transparent huge pages, as
/// `governor.huge_pages: true` asks: it clears the flag mimalloc's
/// constructor set and lets mimalloc advise its arena again. Called from
/// [`super::main`] right after it read the machine's settings, before the
/// process starts a thread. Linux only; elsewhere the process never left
/// the host's state.
pub(super) fn huge_pages_on() {
    #[cfg(target_os = "linux")]
    // SAFETY: `prctl(PR_SET_THP_DISABLE, 0, 0, 0, 0)` takes no pointer and
    // only clears the process's flag; `mi_option_set` is called as in
    // `purge_at_once`, and no other thread of the process runs yet.
    unsafe {
        libc::prctl(libc::PR_SET_THP_DISABLE, 0, 0, 0, 0);
        mi_option_set(ALLOW_THP, 1);
    }
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
