//! The Simulation harness's own scenarios (ADR 0060), one module a
//! scenario, which `just simulate <scenario>` runs, and the gate's `just
//! simulation` from 16 seeds. A scenario's committed Fault plans, which
//! every run of it passes before generating others, are in
//! `fixtures/fault_plans/<scenario>/`.

mod exchange;
