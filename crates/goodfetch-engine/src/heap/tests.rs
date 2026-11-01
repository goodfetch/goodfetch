#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "the counts are std statics, which tests on several std threads move (ADR 0060)"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::AtomicPtr;

use super::{Counting, live};

/// The system allocator, counted, as an executable installs its own.
static COUNTED: Counting<System> = Counting::new(System);

/// One test at a time moves the count, which every test of the process
/// shares: under Miri they run in one process.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The bytes counted now, once something was.
fn counted() -> u64 {
    live().expect("a counted allocation was made")
}

/// Allocates `size` bytes through [`COUNTED`], so that the count starts.
fn allocate(size: usize) -> (*mut u8, Layout) {
    let layout = Layout::from_size_align(size, 8).unwrap();
    // SAFETY: the layout's size is not zero.
    let block = unsafe { COUNTED.alloc(layout) };
    assert!(!block.is_null());
    (block, layout)
}

fn free((block, layout): (*mut u8, Layout)) {
    // SAFETY: `block` was allocated by `COUNTED` with `layout`, once.
    unsafe { COUNTED.dealloc(block, layout) };
}

#[test]
fn a_block_counts_its_bytes_from_its_allocation_to_its_free() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    free(allocate(8));
    let before = counted();

    let block = allocate(1000);
    let allocated = counted();
    let layout = Layout::from_size_align(24, 8).unwrap();
    // SAFETY: the layout's size is not zero.
    let zeroed = unsafe { COUNTED.alloc_zeroed(layout) };
    assert!(!zeroed.is_null());
    let with_zeroed = counted();
    // SAFETY: `block` was allocated by `COUNTED` with its layout, and the
    // new size is not zero.
    let grown = unsafe { COUNTED.realloc(block.0, block.1, 3000) };
    assert!(!grown.is_null());
    let after_growth = counted();
    // SAFETY: `zeroed` was allocated by `COUNTED` with `layout`, once.
    unsafe { COUNTED.dealloc(zeroed, layout) };
    free((grown, Layout::from_size_align(3000, 8).unwrap()));

    assert_eq!(
        (
            allocated - before,
            with_zeroed - before,
            after_growth - before,
            counted()
        ),
        (1000, 1024, 3024, before)
    );
}

#[test]
fn a_block_freed_by_another_thread_than_its_own_leaves_nothing_counted() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    free(allocate(8));
    let before = counted();

    // Each thread ends before the count is read: what a thread counted
    // stays counted once it is gone.
    let (block, layout) = std::thread::spawn(|| {
        let (block, layout) = allocate(4096);
        (AtomicPtr::new(block), layout)
    })
    .join()
    .unwrap();
    let held = counted();
    std::thread::spawn(move || free((block.into_inner(), layout)))
        .join()
        .unwrap();

    assert_eq!((held - before, counted()), (4096, before));
}

#[test]
#[cfg_attr(miri, ignore = "starts more threads than Miri runs in a test's time")]
fn threads_past_the_slots_are_counted_too() {
    let _one = ONE_AT_A_TIME.lock().unwrap();
    free(allocate(8));
    let before = counted();

    let blocks: Vec<(AtomicPtr<u8>, Layout)> = (0..super::SLOTS + 64)
        .map(|_| {
            std::thread::spawn(|| {
                let (block, layout) = allocate(16);
                (AtomicPtr::new(block), layout)
            })
            .join()
            .unwrap()
        })
        .collect();
    let held = counted();
    for (block, layout) in blocks {
        free((block.into_inner(), layout));
    }

    assert_eq!(
        (held - before, counted()),
        (16 * (super::SLOTS as u64 + 64), before)
    );
}
