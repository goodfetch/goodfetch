//! The menu's closing on macOS: `tray-icon` reports the click that opens
//! the menu but not its closing, so the Tray observes `AppKit`'s
//! `NSMenuDidEndTrackingNotification`, which the menu posts on the main
//! thread once its tracking ends, whether a choice was made or not.
#![expect(
    unsafe_code,
    reason = "`AppKit`'s notification name is an extern static, and adding a block observer is an unsafe Objective-C call"
)]

use std::ptr::NonNull;

use block2::RcBlock;
use objc2_app_kit::NSMenuDidEndTrackingNotification;
use objc2_foundation::{NSNotification, NSNotificationCenter};

/// Calls `closed` each time a menu of the Kennel closes, for the Kennel's
/// life.
pub(crate) fn observe(closed: impl Fn() + Send + Sync + 'static) {
    let block = RcBlock::new(move |_notification: NonNull<NSNotification>| closed());
    // SAFETY: `AppKit` defines the notification name, a constant `NSString`
    // that lives for the process.
    let name = unsafe { NSMenuDidEndTrackingNotification };
    // SAFETY: no object filters the notifications, so none has a type to
    // match; with no queue the block runs on the thread that posts, the
    // main thread for a menu's tracking; and the block is sendable, since
    // what it calls is `Send` and `Sync`.
    let registration = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            None,
            &block,
        )
    };
    // The center holds the copied block until the observer is removed,
    // which the Kennel never does: the registration lasts its life.
    drop(registration);
}
