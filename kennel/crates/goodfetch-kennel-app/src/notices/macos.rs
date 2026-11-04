//! The desktop notices on macOS, through `UNUserNotificationCenter`, as
//! #1915's throwaway build showed them on the owner's Mac.
//!
//! - **Only from the `.app`.** The framework aborts in a process with no
//!   bundle identifier, and refuses an `.app` whose binary was run
//!   directly, with no prompt. Started through `LaunchServices`, an ad hoc
//!   build gets the prompt, and the grant survives rebuilds under the same
//!   bundle identifier.
//! - **The permission** is asked for once the adapter is built, so the
//!   prompt shows at launch, and again before each notice, which macOS
//!   answers at once once the person has chosen. A notice is posted only
//!   when it is granted.
//! - **Banners while frontmost.** After a menu click the Kennel may count
//!   as the frontmost app, and macOS then holds a notice back unless the
//!   center's delegate asks for a banner: the adapter's delegate does.
#![expect(
    unsafe_code,
    reason = "the center's delegate is an Objective-C class whose protocols are unsafe to implement, its init is a message to its superclass, and an `NSError` comes as a raw pointer"
)]

use std::cell::Cell;

use block2::{DynBlock, RcBlock};
use goodfetch_kennel::state::Notice;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

use super::Adapter;

/// What each notice's identifier starts with: each notice has its own, so
/// none replaces another in the Notification Center.
const IDENTIFIER: &str = "dev.goodfetch.kennel.notice";

define_class!(
    // SAFETY: `NSObject` has no subclassing requirement, and the class
    // adds no ivar and overrides no method of it.
    #[unsafe(super(NSObject))]
    #[name = "GoodfetchKennelNoticePresenter"]
    struct Presenter;

    // SAFETY: `NSObject` implements the protocol.
    unsafe impl NSObjectProtocol for Presenter {}

    // SAFETY: the method's signature is the protocol's.
    unsafe impl UNUserNotificationCenterDelegate for Presenter {
        /// A banner, and a place in the Notification Center, even while
        /// the Kennel counts as frontmost.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }
    }
);

impl Presenter {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: `NSObject`'s `init` takes no argument and returns the
        // initialised object.
        unsafe { msg_send![super(this), init] }
    }
}

/// The adapter on `UNUserNotificationCenter`.
pub(crate) struct Center {
    /// The center's delegate, which it holds weakly.
    presenter: Retained<Presenter>,
    /// How many notices this adapter showed, which numbers their
    /// identifiers.
    shown: Cell<u64>,
}

impl Center {
    /// The adapter, which asks for the permission at once; none when the
    /// Kennel does not run as its `.app`, where the center would abort.
    pub(crate) fn new() -> Option<Self> {
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            tracing::warn!(
                "desktop notices need the Kennel started as Kennel.app through LaunchServices (`just kennel`, `open`): none is shown"
            );
            return None;
        }
        let presenter = Presenter::new();
        UNUserNotificationCenter::currentNotificationCenter()
            .setDelegate(Some(ProtocolObject::from_ref(&*presenter)));
        authorized(|| {});
        Some(Self {
            presenter,
            shown: Cell::new(0),
        })
    }
}

impl Adapter for Center {
    fn show(&self, notice: Notice) {
        let number = self.shown.get() + 1;
        self.shown.set(number);
        let identifier = format!("{IDENTIFIER}.{}.{number}", std::process::id());
        authorized(move || post(&identifier, &notice));
    }
}

impl Drop for Center {
    fn drop(&mut self) {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let ours = center.delegate().is_some_and(|delegate| {
            std::ptr::addr_eq(
                Retained::as_ptr(&delegate),
                Retained::as_ptr(&self.presenter),
            )
        });
        if ours {
            center.setDelegate(None);
        }
    }
}

/// Asks for the permission to show alerts, and calls `then` on the
/// system's thread once it is granted. The first time, macOS shows the
/// prompt; after that it answers at once.
fn authorized(then: impl Fn() + Send + Sync + 'static) {
    let answered = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        if granted.as_bool() {
            then();
        } else {
            tracing::info!(
                why = %describe(error),
                "desktop notices are not allowed for the Kennel (System Settings › Notifications)"
            );
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert,
            &answered,
        );
}

/// Hands the notice to the system, at once.
fn post(identifier: &str, notice: &Notice) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&notice.title));
    content.setBody(&NSString::from_str(&notice.body));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(identifier),
        &content,
        None,
    );
    let added = RcBlock::new(|error: *mut NSError| {
        if !error.is_null() {
            tracing::warn!(why = %describe(error), "a desktop notice was not shown");
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&added));
}

/// What `error` says, from a completion handler's argument.
fn describe(error: *mut NSError) -> String {
    // SAFETY: the system passes null or a valid `NSError` that lives for
    // the handler's call, during which this runs.
    match unsafe { error.as_ref() } {
        Some(error) => error.localizedDescription().to_string(),
        None => "no reason given".to_owned(),
    }
}
