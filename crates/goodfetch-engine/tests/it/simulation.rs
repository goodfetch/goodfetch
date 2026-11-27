//! The engine's Simulation scenarios (ADR 0060), one module a behaviour
//! area, which `just simulate <scenario>` runs, and the gate's `just
//! simulation` from 16 seeds. A scenario's committed Fault plans, which
//! every run of it passes before generating others, are in
//! `fixtures/fault_plans/<scenario>/`.

mod control_state;
mod engine;
mod group;
mod group_limits;
mod usage;
