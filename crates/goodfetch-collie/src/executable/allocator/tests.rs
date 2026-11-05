//! A block mimalloc frees leaves the process's resident memory at once.
#![expect(
    unsafe_code,
    reason = "the test allocates through mimalloc's `GlobalAlloc`, an unsafe trait, as the executables' global allocator does (STYLE.md, `unsafe`)"
)]

use std::alloc::{GlobalAlloc, Layout};

use goodfetch_test_support::private_resident_bytes_of;
use mimalloc::MiMalloc;

/// A block far above mimalloc's small and medium pages, as a crawl's page
/// bodies and trees add up to.
const BLOCK: usize = 64 << 20;

/// The OS's page.
const PAGE: usize = 4096;

#[cfg(target_os = "linux")]
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
