//! Schedules (ADR 0036; spec #589): what starts a Run on its own, kept in
//! the per-user, per-major Schedule store beside the Run index, so every
//! Client of the user finds them again. A Product's engine registers the
//! store and the `schedule.*` operations through [`contribution`]: create,
//! list, get, and a Schedule's own info and delete. Its trigger is checked
//! when it is created (`trigger`), its ID and time zone settled then
//! (`definition`); nothing fires yet.

mod definition;
mod operations;
mod store;
mod trigger;

pub use operations::contribution;
