//! The desktop notices' adapters (ADR 0141): one interface, and an adapter
//! per platform that shows the notices the State decides.
//!
//! - **macOS** (`macos`): `UNUserNotificationCenter`, from the `.app`
//!   started through `LaunchServices`. It asks for the permission once
//!   built, and presents banners even while the Kennel counts as
//!   frontmost.
//! - **Linux and Windows** (`desktop`): `notify-rust`, on the blocking
//!   threads, so the UI thread never waits on the desktop's notification
//!   service.
//!
//! The Kennel builds an adapter only while the settings turn desktop
//! notices on, and drops it when they turn them off: with `notifications`
//! off, no notice code runs and macOS is never asked for the permission.

#[cfg(not(target_os = "macos"))]
mod desktop;
#[cfg(target_os = "macos")]
mod macos;

use goodfetch_kennel::state::Notice;
use goodfetch_kennel::tasks::Tasks;

/// Shows desktop notices.
pub(crate) trait Adapter {
    /// Shows `notice`, without blocking the caller. A notice the platform
    /// refuses, the permission withheld included, is logged and dropped.
    fn show(&self, notice: Notice);
}

/// The platform's adapter, or none where the platform cannot show the
/// Kennel's notices: on macOS, a Kennel not started as its `.app`.
#[cfg_attr(
    target_os = "macos",
    expect(
        unused_variables,
        reason = "the macOS adapter starts no task: the system calls it back"
    )
)]
#[cfg_attr(
    not(target_os = "macos"),
    expect(
        clippy::unnecessary_wraps,
        reason = "only macOS's adapter can be unavailable, outside its `.app`"
    )
)]
pub(crate) fn adapter(tasks: &Tasks) -> Option<Box<dyn Adapter>> {
    #[cfg(target_os = "macos")]
    {
        macos::Center::new().map(|center| Box::new(center) as Box<dyn Adapter>)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some(Box::new(desktop::Desktop::new(tasks.clone())))
    }
}
