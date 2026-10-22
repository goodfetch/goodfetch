//! Replays the run-state import fuzz target over its seeds and crashers
//! (ADR 0067).

mod run_state_import {
    goodfetch_test_support::fuzz_replay!(
        crate::runs::fuzz_run_state_import,
        "fuzz/run_state_import"
    );
}
