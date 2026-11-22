//! A Foxhound crawl's Session pools (ADRs 0006, 0021, 0035, 0059, 0088 and
//! 0093): the Run's pool, and one of its own for each Label that sets what
//! a login rests on; the Sessions each keeps, each request served by a new Session while
//! the pool has room and then by one drawn from `Entropy` among those open
//! at its domain's proxy tier; each Session scored on how its pages ended
//! and retired past its error score, its uses or its age; a block that
//! escalates retiring its Session and moving its domain one proxy tier up,
//! where the replacement opens; and every Session kept in run state, so a
//! resumed crawl keeps them, each revalidated on its first page. Its own
//! crate under `crates/foxhound/`, since only Foxhound links it (STYLE.md,
//! reason 2).

pub mod escalation;
mod labels;
mod pool;
mod settings;
mod store;

use goodfetch_engine::Contribution;

pub use labels::{LabelPools, PoolKey};
pub use pool::{Effect, LocalAddressing, Rotated, SessionPool, Wanted};
pub use settings::{Affinity, Growth, PoolSettings};
pub use store::{Changes, SendsFrom, TABLES};

/// What the Session pool adds to the engine: the tables that keep a
/// crawl's Sessions and its domains' proxy tiers in every run file.
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-session-pool").with_run_state_tables(store::TABLES)
}
