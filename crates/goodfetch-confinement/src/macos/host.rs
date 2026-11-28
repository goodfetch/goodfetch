//! The startup probe on macOS (ADRs 0033 and 0049, issue 867): checks whether
//! this macOS host allows Seatbelt (`sandbox_init`) in a forked child without
//! sandboxing the engine process itself.

use crate::host::HostFacts;

use super::seatbelt;

/// Probes whether Seatbelt confinement is available on this macOS host.
#[must_use]
pub fn read() -> HostFacts {
    HostFacts {
        seatbelt_refused: !seatbelt::probe_seatbelt(),
        ..HostFacts::NO_NAMESPACE_NEEDED
    }
}
