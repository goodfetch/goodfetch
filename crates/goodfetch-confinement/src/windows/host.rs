//! The startup probe on Windows (ADRs 0033 and 0049, issues 873 and 879):
//! checks whether this Windows host can create an `AppContainer` profile and
//! whether `goodfetch-filter-broker`'s named pipe is available.

use crate::host::HostFacts;

use super::win32;

/// Probes whether `AppContainer` confinement and `goodfetch-filter-broker`
/// are available on this Windows host.
#[must_use]
pub fn read() -> HostFacts {
    HostFacts {
        windows_appcontainer_refused: !win32::probe_appcontainer(),
        windows_filter_broker: Some(win32::probe_filter_broker()),
        ..HostFacts::NO_NAMESPACE_NEEDED
    }
}
