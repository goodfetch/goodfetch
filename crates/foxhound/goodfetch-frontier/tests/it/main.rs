//! `goodfetch-frontier` through its public interface, in a process of its
//! own: the Frontier at scale (ADR 0134), and the Scope's seeds within
//! the Run state's memory budget (#2169), measured by the process's own
//! allocator.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

#[global_allocator]
static ALLOCATOR: goodfetch_engine::heap::Counting<std::alloc::System> =
    goodfetch_engine::heap::Counting::new(std::alloc::System);

mod at_scale;
mod seeds_in_memory;
