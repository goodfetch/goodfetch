//! macOS only: whether the status item is visible (the notch question), and
//! desktop notices through `UNUserNotificationCenter`.

use std::ptr::NonNull;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, MainThreadMarker, define_class, msg_send};
use objc2_app_kit::NSWindowOcclusionState;
use objc2_foundation::{NSBundle, NSError, NSObject, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationSettings,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tray_icon::TrayIcon;

use crate::log;

/// Every reading that could tell a notch-hidden icon from a shown one.
pub fn visibility(tray: &TrayIcon) -> String {
    let rect = tray.rect();
    let Some(mtm) = MainThreadMarker::new() else {
        return "not on the main thread".into();
    };
    let Some(item) = tray.ns_status_item() else {
        return format!("no status item; rect() {rect:?}");
    };
    let Some(button) = item.button(mtm) else {
        return "the status item has no button".into();
    };
    let Some(window) = button.window() else {
        return "the button has no window".into();
    };
    let frame = window.frame();
    let occlusion_visible = window
        .occlusionState()
        .contains(NSWindowOcclusionState::Visible);
    let notch = match window.screen() {
        Some(screen) => {
            let screen_frame = screen.frame();
            let left = screen.auxiliaryTopLeftArea();
            let right = screen.auxiliaryTopRightArea();
            let notch_from = screen_frame.origin.x + left.origin.x + left.size.width;
            let notch_to = screen_frame.origin.x + right.origin.x;
            let has_notch = left.size.width > 0.0 && right.size.width > 0.0;
            let under = has_notch
                && frame.origin.x + frame.size.width > notch_from
                && frame.origin.x < notch_to;
            format!(
                "screen x {:.0}..{:.0}, notch {} (x {notch_from:.0}..{notch_to:.0}), icon overlaps it: {under}",
                screen_frame.origin.x,
                screen_frame.origin.x + screen_frame.size.width,
                if has_notch { "present" } else { "absent" },
            )
        }
        None => "the window has no screen".into(),
    };
    format!(
        "occlusion visible: {occlusion_visible}, window visible: {}, frame x {:.0} y {:.0} w {:.0} h {:.0}; {notch}; rect() {rect:?}",
        window.isVisible(),
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
    )
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "KennelSpikeNoticeDelegate"]
    struct NoticeDelegate;

    unsafe impl NSObjectProtocol for NoticeDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NoticeDelegate {
        // An accessory app may count as frontmost after a menu click; without
        // this, macOS would hold the banner back.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            log("the system asks how to show the notice while the spike is frontmost: banner");
            handler
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }
    }
);

impl NoticeDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

pub struct Notices {
    _delegate: Retained<NoticeDelegate>,
}

impl Notices {
    /// `None` outside a `.app`, where `UNUserNotificationCenter` aborts.
    pub fn new() -> Option<Self> {
        let Some(identifier) = NSBundle::mainBundle().bundleIdentifier() else {
            log("not inside a .app bundle: desktop notices are off (run ./run.sh app)");
            return None;
        };
        log(format!("inside a .app, bundle identifier {identifier}"));
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = NoticeDelegate::new();
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let report = RcBlock::new(|settings: NonNull<UNNotificationSettings>| {
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            log(format!(
                "notice authorization at launch: {} (after a rebuild, 'authorized' means the grant survived)",
                status_name(status.0)
            ));
        });
        center.getNotificationSettingsWithCompletionHandler(&report);
        Some(Self {
            _delegate: delegate,
        })
    }

    pub fn send(&self) {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let then_post = RcBlock::new(|granted: Bool, error: *mut NSError| {
            log(format!(
                "authorization request answered: granted {}, error {}",
                granted.as_bool(),
                describe(error)
            ));
            if granted.as_bool() {
                post();
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &then_post,
        );
    }
}

fn post() {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str("Kennel spike"));
    content.setBody(&NSString::from_str(
        "A test desktop notice from an ad hoc signed .app.",
    ));
    let identifier = NSString::from_str(&format!(
        "kennel-spike-{}",
        crate::TICKS.load(std::sync::atomic::Ordering::Relaxed)
    ));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
    let done = RcBlock::new(|error: *mut NSError| {
        log(format!(
            "notice handed to the system: error {} (did a banner appear?)",
            describe(error)
        ));
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&done));
}

fn status_name(status: isize) -> &'static str {
    match status {
        0 => "not determined",
        1 => "denied",
        2 => "authorized",
        3 => "provisional",
        4 => "ephemeral",
        _ => "unknown",
    }
}

fn describe(error: *mut NSError) -> String {
    match unsafe { error.as_ref() } {
        Some(error) => error.localizedDescription().to_string(),
        None => "none".into(),
    }
}
