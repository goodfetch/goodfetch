//! A block mimalloc frees leaves the process's resident memory at once,
//! and transparent huge pages are off until `governor.huge_pages` turns
//! them on.
#![expect(
    unsafe_code,
    reason = "the test allocates through mimalloc's `GlobalAlloc`, an unsafe trait, as the executables' global allocator does, and reads mimalloc's option and the process's huge-page flag through their C calls (STYLE.md, `unsafe`)"
)]

use std::alloc::{GlobalAlloc, Layout};

use goodfetch_test_support::private_resident_bytes_of;
use mimalloc::MiMalloc;

/// A block far above mimalloc's small and medium pages, as a crawl's page
/// bodies and trees add up to.
const BLOCK: usize = 64 << 20;

/// The OS's page.
const PAGE: usize = 4096;

#[test]
fn a_freed_block_leaves_the_resident_memory_when_it_is_freed() {
    super::purge_at_once();
    let layout = Layout::from_size_align(BLOCK, PAGE).unwrap();
    // SAFETY: `layout`'s size is not zero.
    let block = unsafe { MiMalloc.alloc(layout) };
    assert!(!block.is_null());
    for offset in (0..BLOCK).step_by(PAGE) {
        // SAFETY: `offset` is inside the block just allocated.
        unsafe { block.add(offset).write_volatile(1) };
    }
    let held = private_resident_bytes_of(std::process::id()).unwrap();
    // SAFETY: `block` was allocated above with `layout`, and is freed once.
    unsafe { MiMalloc.dealloc(block, layout) };
    let left = private_resident_bytes_of(std::process::id()).unwrap();
    let given_back = held.saturating_sub(left);
    assert!(
        given_back >= (BLOCK / 8 * 7) as u64,
        "freeing a block of {BLOCK} bytes gave back {given_back} resident bytes ({held} held, {left} left)"
    );
}

unsafe extern "C" {
    /// `<mimalloc.h>`: the value of `option` for the whole process.
    fn mi_option_get(option: std::ffi::c_int) -> std::ffi::c_long;
}

/// Whether the process has huge pages turned off for it, and whether
/// mimalloc allows them.
fn huge_pages() -> (bool, bool) {
    // SAFETY: `prctl(PR_GET_THP_DISABLE, 0, 0, 0, 0)` takes no pointer and
    // changes nothing; `mi_option_get` takes any option.
    unsafe {
        (
            libc::prctl(libc::PR_GET_THP_DISABLE, 0, 0, 0, 0) == 1,
            mi_option_get(super::ALLOW_THP) == 1,
        )
    }
}

/// The test's process links the allocator's constructor, so it starts with
/// huge pages off and mimalloc not allowing them; turning them on clears
/// the flag and allows them in mimalloc again.
#[test]
fn huge_pages_start_off_and_turning_them_on_clears_the_flag_and_allows_them() {
    let before = huge_pages();

    super::huge_pages_on();

    assert_eq!((before, huge_pages()), ((true, false), (false, true)));
}
