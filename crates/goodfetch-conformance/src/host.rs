//! The platform and runtime the harness's own cases run on, as its result
//! documents name them.

use goodfetch_protocol::result_document::Platform;

/// The runtime of the cases the harness runs itself.
pub(crate) const RUNTIME: &str = "rust";

/// The operating system and architecture this build runs on.
pub(crate) fn platform() -> Platform {
    Platform {
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
    }
}
