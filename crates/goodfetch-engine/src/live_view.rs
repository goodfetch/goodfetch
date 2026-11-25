//! The Live view (ADRs 0031 and 0149): how a person watches one Session of a
//! Daemon, through a channel of its own beside the protocol's Connections.
//!
//! Its interface: [`LiveView`] is the driver, which a Daemon has one of.
//! [`LiveView::watch`] admits a viewer of a Session, up to
//! `live_view.max_viewers`, and [`channel::serve`] serves the WebSocket that
//! viewer opened. The driver alone owns the work of a frame: which tier
//! captures the Session and which follows one that failed, which frame a
//! viewer gets next, and when the capture stops. It decodes no frame, opens
//! no file, compiles nothing and starts no task for one.
//!
//! What it watches and where frames go pass two narrow traits: a
//! [`FrameSource`] captures one Session by one tier, and a [`VideoOutput`]
//! carries frames to one viewer. Whoever drives Sessions offers them as
//! [`WatchedSessions`], and each failure of a browser, an encoder or a
//! transport becomes one [`WatchFailure`] where it enters.

mod channel;
mod driver;
#[cfg(test)]
mod tests;

use std::fmt;
use std::pin::Pin;

use goodfetch_protocol::live_view::{LiveViewScreen, LiveViewTier};
use goodfetch_protocol::page::{PageId, SessionId};
use tokio::sync::watch;

pub use channel::fuzz_viewer_message;
pub(crate) use channel::{Admission, Channel, serve};
pub(crate) use driver::LiveView;
use driver::{Frame, NotWatched, Tiers, Viewing};

/// A boxed future of a frame source's or a video output's answer.
pub type Watching<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The longest side a frame is asked for, in device pixels.
const LONGEST_SIDE: f64 = 8192.0;

/// The Sessions a Live view can watch, which the crate that drives them
/// offers ([`Contribution::with_watched_sessions`](crate::composition::Contribution::with_watched_sessions)).
pub trait WatchedSessions: Send + Sync {
    /// The open Session `session`, as a Live view watches it; `None` when
    /// it is none of these.
    fn watch(&self, session: &SessionId) -> Option<WatchedSession>;
}

/// One open Session, as a Live view watches it.
pub struct WatchedSession {
    /// Its tabs, oldest first, as they are now; its sender dropped once the
    /// Session closed.
    pub tabs: watch::Receiver<Vec<PageId>>,
    /// What captures it, the best tier first: none started, each used once
    /// those before it failed.
    pub sources: Vec<Box<dyn FrameSource>>,
}

/// One tier's capture of one Session.
pub trait FrameSource: Send {
    /// The tier.
    fn tier(&self) -> LiveViewTier;

    /// Starts capturing `page` at `size`, once stopped or at first.
    ///
    /// # Errors
    ///
    /// Why it cannot: it is then stopped.
    ///
    /// # Cancel safety
    ///
    /// After dropping the future, [`FrameSource::stop`] stops what started.
    fn start<'a>(
        &'a mut self,
        page: &'a PageId,
        size: FrameSize,
    ) -> Watching<'a, Result<(), WatchFailure>>;

    /// The next frame, once one comes: the source then acknowledges it and
    /// captures on, whether a viewer takes it or not.
    ///
    /// # Errors
    ///
    /// Why no more come.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no frame.
    fn next(&mut self) -> Watching<'_, Result<SourceFrame, WatchFailure>>;

    /// Stops capturing, if it started.
    fn stop(&mut self) -> Watching<'_, ()>;
}

/// What carries frames to one viewer.
trait VideoOutput: Send {
    /// Sends `frame` to the viewer.
    ///
    /// # Errors
    ///
    /// Why it cannot, which ends the viewer's channel.
    fn send<'a>(&'a mut self, frame: &'a Frame) -> Watching<'a, Result<(), WatchFailure>>;
}

/// The size a frame is captured within, in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSize {
    /// Its width.
    pub width: u32,
    /// Its height.
    pub height: u32,
}

impl FrameSize {
    /// The smallest size that fills every one of `screens`, so each shows
    /// the Session at its own resolution; `None` for no screen.
    fn filling<'a>(screens: impl IntoIterator<Item = &'a LiveViewScreen>) -> Option<Self> {
        screens
            .into_iter()
            .map(|screen| Self {
                width: device_pixels(screen.width, screen.device_pixel_ratio),
                height: device_pixels(screen.height, screen.device_pixel_ratio),
            })
            .reduce(|one, other| Self {
                width: one.width.max(other.width),
                height: one.height.max(other.height),
            })
    }
}

/// A side of `css` CSS pixels in device pixels, from 1 to [`LONGEST_SIDE`]
/// whatever `ratio` a viewer says.
fn device_pixels(css: u32, ratio: f64) -> u32 {
    let ratio = if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a whole number from 1 to 8192"
    )]
    let side = (f64::from(css) * ratio).round().clamp(1.0, LONGEST_SIDE) as u32;
    side
}

/// A frame as its source gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceFrame {
    /// The shown page's width, in its CSS pixels.
    pub width: f64,
    /// The shown page's height, in its CSS pixels.
    pub height: f64,
    /// The frame, encoded as its source's tier says and then in base64.
    pub data: String,
}

/// Why a Live view's capture or its output failed: what every failure of a
/// browser, an encoder or a transport becomes where it enters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchFailure {
    /// The page shown is gone: the Session's next tab is shown instead.
    PageGone,
    /// The browser answered that it does not capture.
    CaptureRefused(String),
    /// The browser answered nothing in time.
    CaptureTimedOut,
    /// The browser is gone.
    BrowserGone,
    /// The viewer is gone.
    ViewerGone,
    /// A failure this engine does not know, as its source wrote it.
    Unrecognized(String),
}

impl fmt::Display for WatchFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PageGone => formatter.write_str("the page shown is gone"),
            Self::CaptureRefused(reason) => {
                write!(formatter, "the browser refused to capture: {reason}")
            }
            Self::CaptureTimedOut => formatter.write_str("the browser did not answer in time"),
            Self::BrowserGone => formatter.write_str("the browser is gone"),
            Self::ViewerGone => formatter.write_str("the viewer is gone"),
            Self::Unrecognized(failure) => formatter.write_str(failure),
        }
    }
}
