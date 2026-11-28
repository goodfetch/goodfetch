//! The startup probe on Windows (ADRs 0033 and 0049, issue 873): checks
//! whether this Windows host can create an `AppContainer` profile and reports
//! that the filter broker is not yet available (until #879 probes its named
//! pipe).

use crate::host::HostFacts;

use super::win32;

/// Probes whether `AppContainer` confinement is available on this Windows host.
#[must_use]
pub fn read() -> HostFacts {
    HostFacts {
        windows_appcontainer_refused: !win32::probe_appcontainer(),
        windows_filter_broker: Some(false),
        ..HostFacts::NO_NAMESPACE_NEEDED
    }
}
