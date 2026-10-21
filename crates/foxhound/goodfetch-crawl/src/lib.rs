//! A Foxhound crawl's Run (ADRs 0003, 0087, 0093 and 0102): `crawl.start`
//! begins it from its seeds, each a work item in its run state, and its
//! delivery leases each work item to the program's handler as a
//! `crawl_request.handle` request, commits the Records the handler staged
//! once while the lease holds, retries an attempt that failed or got no
//! answer under the Run's policy, and finishes the Run once every work item
//! is done. Each work item's ending commits its outcome, which
//! `run.outcomes` and `run.failed_requests` read from committed state. Its
//! own crate under `crates/foxhound/`, since only Foxhound links it
//! (STYLE.md, reason 2); it fills the engine's interfaces: a Run's driver,
//! the tables a contribution keeps in every run file, and reads of them.

mod delivery;
mod lease;
mod outcomes;
mod policy;
mod start;
mod work_items;

pub use lease::Lease;
pub use start::contribution;
