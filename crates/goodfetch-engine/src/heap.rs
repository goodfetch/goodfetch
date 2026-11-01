//! The live heap (#1734, ADR 0134's soak note): the bytes the process's own
//! allocations hold now, which its executable's global allocator counts as
//! it allocates and frees, and `daemon.status` answers. Not the private
//! memory the operating system sees: the allocator wrapped here keeps pages
//! it freed for later allocations, by the megabyte and per thread, so a
//! Daemon with no leak can hold more private memory crawl after crawl while
//! what it allocated stays flat (#1729's heap profiles).
//!
//! Each thread counts on a slot of its own, with a plain read and write
//! rather than a locked add, so counting costs an allocation little; a
//! thread keeps its slot once it ends, since the blocks it allocated may
//! outlive it, and the count is the sum of every slot. Past [`SLOTS`]
//! threads, the rest share one counter. A block one thread allocates and
//! another frees counts up on the first's slot and down on the other's.
//! Only an executable counts: an engine an SDK crate links runs on its
//! host's allocator, and answers no live heap.
#![expect(
    unsafe_code,
    reason = "a global allocator implements `GlobalAlloc`, an unsafe trait, by calling the allocator it wraps (STYLE.md, `unsafe`)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a global allocator counts on statics, built at compile time, which loom's atomics cannot be; no loom model allocates through it (ADR 0060)"
)]

use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

/// How many threads count on a slot of their own.
const SLOTS: usize = 1024;

/// What a thread that has no slot yet holds as its slot.
const UNCLAIMED: usize = usize::MAX;

/// One thread's count: the bytes it allocated less the bytes it freed,
/// below zero when it freed others' blocks. A cache line of its own, so a
/// thread's writes never slow another's.
#[repr(align(128))]
struct Slot(AtomicI64);

static OWN_SLOTS: [Slot; SLOTS] = [const { Slot(AtomicI64::new(0)) }; SLOTS];

/// How many slots threads claimed, which may pass [`SLOTS`].
static CLAIMED: AtomicUsize = AtomicUsize::new(0);

/// The count of the threads past [`SLOTS`].
static SHARED: AtomicI64 = AtomicI64::new(0);

/// Whether a [`Counting`] allocator counted anything in this process.
static COUNTING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// This thread's slot, an index of [`OWN_SLOTS`], or [`UNCLAIMED`]; one
    /// past them counts on [`SHARED`]. A constant with no destructor, so
    /// reading it never allocates.
    static OWN: Cell<usize> = const { Cell::new(UNCLAIMED) };
}

/// A global allocator, `A`, that counts the bytes its blocks hold, for
/// [`live`].
#[derive(Debug)]
pub struct Counting<A>(A);

impl<A> Counting<A> {
    /// `allocator`, counted.
    pub const fn new(allocator: A) -> Self {
        Self(allocator)
    }
}

/// Counts `bytes` more, or fewer when below zero, on this thread's slot.
fn count(bytes: i64) {
    let slot = OWN
        .try_with(|own| {
            if own.get() == UNCLAIMED {
                COUNTING.store(true, Ordering::Relaxed);
                own.set(CLAIMED.fetch_add(1, Ordering::Relaxed).min(SLOTS));
            }
            own.get()
        })
        .unwrap_or(SLOTS);
    match OWN_SLOTS.get(slot) {
        // Only this thread writes its slot, so a read and a write add
        // without a lock; a reader of the count sees one or the other.
        Some(Slot(held)) => held.store(
            held.load(Ordering::Relaxed).wrapping_add(bytes),
            Ordering::Relaxed,
        ),
        None => {
            SHARED.fetch_add(bytes, Ordering::Relaxed);
        }
    }
}

/// The bytes of a block of `size`, as a count.
fn bytes(size: usize) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

// SAFETY: every call is the wrapped allocator's, with the caller's own
// arguments, and answers what it answered; counting reads and writes
// atomics and a constant thread-local, and never allocates.
unsafe impl<A: GlobalAlloc> GlobalAlloc for Counting<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller meets `alloc`'s contract.
        let block = unsafe { self.0.alloc(layout) };
        if !block.is_null() {
            count(bytes(layout.size()));
        }
        block
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller meets `alloc_zeroed`'s contract.
        let block = unsafe { self.0.alloc_zeroed(layout) };
        if !block.is_null() {
            count(bytes(layout.size()));
        }
        block
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        // SAFETY: the caller meets `dealloc`'s contract.
        unsafe { self.0.dealloc(block, layout) };
        count(-bytes(layout.size()));
    }

    unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller meets `realloc`'s contract.
        let moved = unsafe { self.0.realloc(block, layout, new_size) };
        if !moved.is_null() {
            count(bytes(new_size).wrapping_sub(bytes(layout.size())));
        }
        moved
    }
}

/// The bytes the process's blocks hold now, as its [`Counting`] global
/// allocator counted them; none when it has none.
pub fn live() -> Option<u64> {
    if !COUNTING.load(Ordering::Relaxed) {
        return None;
    }
    let claimed = CLAIMED.load(Ordering::Relaxed).min(SLOTS);
    let held = OWN_SLOTS[..claimed]
        .iter()
        .fold(SHARED.load(Ordering::Relaxed), |held, Slot(slot)| {
            held.wrapping_add(slot.load(Ordering::Relaxed))
        });
    // A thread may free a block another allocated before the reader sees
    // the allocation's count: a moment's count below zero is none.
    Some(u64::try_from(held).unwrap_or(0))
}

#[cfg(test)]
mod tests;
