//! The Live view's contracts (ADRs 0031 and 0149): the channel a person
//! watches one Session through, its settings and its Error code.
//!
//! The channel is a WebSocket on a Daemon's `daemon.address`, selected by
//! its handshake's path ([`CHANNEL_PATH`]), one per viewer of a Session and
//! separate from the protocol's Connections and their events. Each text
//! message is one JSON object, tagged by its `kind`: the viewer's
//! [`LiveViewViewerMessage`], of which the first is `open`, and the
//! engine's [`LiveViewEngineMessage`]. The bundle's `live_view` holds both.
//!
//! Frames are newest-wins under acknowledgement: the engine sends a frame
//! only once the viewer acknowledged the one before, and then the newest.
//! Everything else is in the order it was sent, and nothing of it is
//! dropped. This engine answers a kind it does not carry yet with an
//! `error` naming it, and the channel goes on.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion, Setting, ValueKind,
};
use crate::daemon::DaemonToken;
use crate::jsonrpc::ErrorObject;
use crate::page::{PageId, SessionId};
use crate::settings::SettingLimit;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The path of a channel's WebSocket handshake on `daemon.address`.
pub const CHANNEL_PATH: &str = "/live-view";

fn on() -> serde_json::Value {
    serde_json::Value::Bool(true)
}

/// `live_view.enabled` (ADRs 0031 and 0049).
pub const ENABLED: Setting = Setting {
    name: "live_view.enabled",
    description: "Whether a person may watch this machine's Sessions through a Daemon: on by default. A Daemon then accepts Live view channels on the loopback WebSocket of `daemon.address`, which it opens for them where nothing else did. Off, no channel opens, the Daemon listens there for none, and nothing is captured. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Boolean,
    default: on,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The values of `live_view.tier`.
pub const TIERS: &[&str] = &["auto", "screencast"];

fn auto() -> serde_json::Value {
    serde_json::Value::String("auto".to_owned())
}

/// `live_view.tier` (ADR 0031).
pub const TIER: Setting = Setting {
    name: "live_view.tier",
    description: "How a Live view captures its Session: `auto` (the default) takes the best tier the Session's browser offers, `screencast` for a headless one, and falls back to the next when one fails; `screencast` takes the browser's own screencast of the shown page, JPEG frames sized to the largest screen watching. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Choice(TIERS),
    default: auto,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn ten() -> serde_json::Value {
    serde_json::json!(10)
}

/// `live_view.max_viewers` (ADR 0031).
pub const MAX_VIEWERS: Setting = Setting {
    name: "live_view.max_viewers",
    description: "How many viewers may watch one Session at once: 10. The next one's channel is refused with `live_view_viewer_limit`. Set in user configuration or an Operator policy only.",
    kind: ValueKind::CountFromOne,
    default: ten,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// One viewer more than `live_view.max_viewers`.
pub const LIVE_VIEW_VIEWER_LIMIT: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "live_view_viewer_limit",
    description: "As many viewers as `live_view.max_viewers` allows already watch the Session, so the channel opened none. `limit` names the setting, its value and the layer that set it. Open the channel again once a viewer leaves, or raise the setting.",
    group: ErrorGroup::Refused,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `live_view_viewer_limit` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LiveViewViewerLimitDetails {
    /// `live_view.max_viewers`, its value and its layer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: SettingLimit,
}

/// A viewer's screen: the area its Live view fills.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LiveViewScreen {
    /// Its width, in CSS pixels.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub width: u32,
    /// Its height, in CSS pixels.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub height: u32,
    /// The device pixels of one of its CSS pixels, as the browser's
    /// `devicePixelRatio`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub device_pixel_ratio: f64,
}

/// One step of a channel's WebRTC negotiation, either way.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum LiveViewSignal {
    /// A session description that offers video.
    Offer {
        /// The description, as SDP.
        #[schemars(extend("arrived_in" = "0.1"))]
        sdp: String,
    },
    /// The session description that answers an offer.
    Answer {
        /// The description, as SDP.
        #[schemars(extend("arrived_in" = "0.1"))]
        sdp: String,
    },
    /// An ICE candidate.
    Candidate {
        /// The candidate, as its `candidate:` line.
        #[schemars(extend("arrived_in" = "0.1"))]
        candidate: String,
    },
}

/// What a pointer or a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveViewPress {
    /// It was pressed.
    Down,
    /// It was released.
    Up,
    /// The pointer moved.
    Move,
}

/// A viewer's input in the shown page, in the order it happened. A point is
/// in the shown page's CSS pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "device", rename_all = "snake_case")]
pub enum LiveViewInput {
    /// The mouse or a pen.
    Pointer {
        /// What it did.
        #[schemars(extend("arrived_in" = "0.1"))]
        press: LiveViewPress,
        /// Where, from the left.
        #[schemars(extend("arrived_in" = "0.1"))]
        x: f64,
        /// Where, from the top.
        #[schemars(extend("arrived_in" = "0.1"))]
        y: f64,
        /// The button, as `MouseEvent.button`; none for a move.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        button: Option<u8>,
        /// Which of a run of clicks it is, from 1.
        #[schemars(extend("arrived_in" = "0.1"))]
        click: u32,
        /// The modifier keys held.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[schemars(extend("arrived_in" = "0.1"))]
        modifiers: Vec<LiveViewModifier>,
    },
    /// The wheel.
    Wheel {
        /// Where, from the left.
        #[schemars(extend("arrived_in" = "0.1"))]
        x: f64,
        /// Where, from the top.
        #[schemars(extend("arrived_in" = "0.1"))]
        y: f64,
        /// How far it scrolls right.
        #[schemars(extend("arrived_in" = "0.1"))]
        delta_x: f64,
        /// How far it scrolls down.
        #[schemars(extend("arrived_in" = "0.1"))]
        delta_y: f64,
    },
    /// The keyboard.
    Key {
        /// What it did: pressed or released.
        #[schemars(extend("arrived_in" = "0.1"))]
        press: LiveViewPress,
        /// The physical key, as `KeyboardEvent.code`.
        #[schemars(extend("arrived_in" = "0.1"))]
        code: String,
        /// What it types under the person's layout, as `KeyboardEvent.key`.
        #[schemars(extend("arrived_in" = "0.1"))]
        key: String,
        /// The modifier keys held.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[schemars(extend("arrived_in" = "0.1"))]
        modifiers: Vec<LiveViewModifier>,
    },
    /// An input method's composition, as it stands.
    Composition {
        /// Its text.
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
        /// Whether it ended, which inserts the text.
        #[schemars(extend("arrived_in" = "0.1"))]
        ended: bool,
    },
    /// The touch screen.
    Touch {
        /// What the fingers did.
        #[schemars(extend("arrived_in" = "0.1"))]
        press: LiveViewPress,
        /// Where each finger still on the screen is; none once the last
        /// left.
        #[schemars(extend("arrived_in" = "0.1"))]
        points: Vec<LiveViewPoint>,
    },
}

/// A modifier key held during an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveViewModifier {
    /// Alt, or Option.
    Alt,
    /// Control.
    Control,
    /// Meta: Command, or the Windows key.
    Meta,
    /// Shift.
    Shift,
}

/// One finger's place on the shown page.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LiveViewPoint {
    /// The finger, the same number while it stays on the screen.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: u32,
    /// Where, from the left.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub x: f64,
    /// Where, from the top.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub y: f64,
}

/// What a viewer does with the control of a Session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveViewControlAct {
    /// Takes it, or asks its holder for it.
    Take,
    /// Hands it back.
    HandBack,
}

/// A viewer's answer to a native dialog or a basic-auth prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum LiveViewDialogAnswer {
    /// Accepts it.
    Accept {
        /// What a `prompt` is answered with.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        text: Option<String>,
    },
    /// Dismisses it.
    Dismiss,
    /// Signs in to a basic-auth prompt.
    SignIn {
        /// The user name.
        #[schemars(extend("arrived_in" = "0.1"))]
        user_name: String,
        /// The password.
        #[schemars(extend("arrived_in" = "0.1"))]
        password: String,
    },
}

/// A file a viewer gives the page's file chooser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LiveViewFile {
    /// Its name, with no folder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its bytes, in base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub data: String,
}

/// An explicit clipboard transfer, never made without a person's gesture.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transfer", rename_all = "snake_case")]
pub enum LiveViewClipboard {
    /// Pastes this text in the shown page.
    Paste {
        /// The text.
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
    },
    /// Copies the shown page's selection out; the engine's message of this
    /// transfer answers it.
    Copy,
}

/// A viewer's answer to the Human pause the Session waits in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum LiveViewPauseAnswer {
    /// The person is done.
    Done,
    /// The person declines.
    Decline {
        /// Why, for the program.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        note: Option<String>,
    },
}

/// A message a viewer sends on its channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveViewViewerMessage {
    /// Opens the channel, as its first message: within `timeout.hello`.
    Open {
        /// The Daemon's token.
        #[schemars(extend("arrived_in" = "0.1"))]
        token: DaemonToken,
        /// The Session to watch.
        #[schemars(extend("arrived_in" = "0.1"))]
        session: SessionId,
        /// The viewer's screen.
        #[schemars(extend("arrived_in" = "0.1"))]
        screen: LiveViewScreen,
    },
    /// The viewer's screen changed.
    Screen {
        /// Its screen now.
        #[schemars(extend("arrived_in" = "0.1"))]
        screen: LiveViewScreen,
    },
    /// The viewer drew a frame and takes the next one.
    Acknowledge {
        /// The frame's number.
        #[schemars(extend("arrived_in" = "0.1"))]
        frame: u64,
    },
    /// A step of the WebRTC negotiation.
    Signal {
        /// The step.
        #[schemars(extend("arrived_in" = "0.1"))]
        signal: LiveViewSignal,
    },
    /// Input in the shown page.
    Input {
        /// The input.
        #[schemars(extend("arrived_in" = "0.1"))]
        input: LiveViewInput,
    },
    /// Takes the Session's control or hands it back.
    Control {
        /// What it does.
        #[schemars(extend("arrived_in" = "0.1"))]
        act: LiveViewControlAct,
    },
    /// Shows another tab.
    Tab {
        /// The tab's page.
        #[schemars(extend("arrived_in" = "0.1"))]
        page: PageId,
    },
    /// Answers a dialog.
    Dialog {
        /// The dialog.
        #[schemars(extend("arrived_in" = "0.1"))]
        dialog: String,
        /// The answer.
        #[schemars(extend("arrived_in" = "0.1"))]
        answer: LiveViewDialogAnswer,
    },
    /// Gives a file chooser its files; none cancels it.
    Files {
        /// The file chooser.
        #[schemars(extend("arrived_in" = "0.1"))]
        chooser: String,
        /// The files.
        #[schemars(extend("arrived_in" = "0.1"))]
        files: Vec<LiveViewFile>,
    },
    /// A clipboard transfer the person asked for.
    Clipboard {
        /// The transfer.
        #[schemars(extend("arrived_in" = "0.1"))]
        clipboard: LiveViewClipboard,
    },
    /// Answers the Human pause the Session waits in.
    Pause {
        /// The answer.
        #[schemars(extend("arrived_in" = "0.1"))]
        answer: LiveViewPauseAnswer,
    },
}

/// One tab of a Session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LiveViewTab {
    /// Its page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page: PageId,
}

/// A tier a Session is captured by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveViewTier {
    /// The browser's own screencast of the shown page.
    Screencast,
}

/// Who holds a Session's control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveViewControl {
    /// The program.
    Automation,
    /// Nobody: the Session waits for a person.
    Waiting,
    /// This viewer.
    You,
    /// Another viewer.
    Another,
}

/// What a native dialog asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveViewDialogType {
    /// `alert`.
    Alert,
    /// `confirm`.
    Confirm,
    /// `prompt`.
    Prompt,
    /// Whether to leave the page.
    Beforeunload,
    /// A basic-auth prompt.
    BasicAuth,
}

/// Why a channel ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LiveViewEnd {
    /// The Session closed.
    SessionClosed,
    /// No tier could capture the Session any longer.
    CaptureLost,
    /// The Daemon is stopping.
    DaemonStopping,
}

/// A message the engine sends on a channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveViewEngineMessage {
    /// The channel is open: the answer to `open`.
    Opened {
        /// The Session it watches.
        #[schemars(extend("arrived_in" = "0.1"))]
        session: SessionId,
        /// The tier that captures it.
        #[schemars(extend("arrived_in" = "0.1"))]
        tier: LiveViewTier,
    },
    /// The tier that captures the Session from now on, the one before having
    /// failed.
    Tier {
        /// The tier.
        #[schemars(extend("arrived_in" = "0.1"))]
        tier: LiveViewTier,
    },
    /// The newest frame, sent once the viewer acknowledged the one before.
    Frame {
        /// Its number, larger than every earlier one of the channel.
        #[schemars(extend("arrived_in" = "0.1"))]
        frame: u64,
        /// The page it shows.
        #[schemars(extend("arrived_in" = "0.1"))]
        page: PageId,
        /// The shown page's width, in its CSS pixels.
        #[schemars(extend("arrived_in" = "0.1"))]
        width: f64,
        /// The shown page's height, in its CSS pixels.
        #[schemars(extend("arrived_in" = "0.1"))]
        height: f64,
        /// The frame, a JPEG in base64.
        #[schemars(extend("arrived_in" = "0.1"))]
        data: String,
    },
    /// The Session's tabs: after `opened`, and after each change.
    Tabs {
        /// The tabs, oldest first.
        #[schemars(extend("arrived_in" = "0.1"))]
        tabs: Vec<LiveViewTab>,
        /// The page shown; none when the Session has no tab.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        shown: Option<PageId>,
    },
    /// A step of the WebRTC negotiation.
    Signal {
        /// The step.
        #[schemars(extend("arrived_in" = "0.1"))]
        signal: LiveViewSignal,
    },
    /// Who holds the Session's control: after each change.
    Control {
        /// The holder.
        #[schemars(extend("arrived_in" = "0.1"))]
        holder: LiveViewControl,
    },
    /// A native dialog or a basic-auth prompt opened in the shown page.
    Dialog {
        /// The dialog, which its answer names.
        #[schemars(extend("arrived_in" = "0.1"))]
        dialog: String,
        /// What it asks.
        #[schemars(extend("arrived_in" = "0.1"))]
        asks: LiveViewDialogType,
        /// Its text.
        #[schemars(extend("arrived_in" = "0.1"))]
        message: String,
    },
    /// The shown page asks for files.
    FileChooser {
        /// The file chooser, which its answer names.
        #[schemars(extend("arrived_in" = "0.1"))]
        chooser: String,
        /// Whether it takes several files.
        #[schemars(extend("arrived_in" = "0.1"))]
        multiple: bool,
    },
    /// A download of the Session, which a Client fetches by its handle.
    Download {
        /// The download's handle.
        #[schemars(extend("arrived_in" = "0.1"))]
        download: String,
        /// The file name the site gave it.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
    },
    /// What the viewer asked to copy out.
    Clipboard {
        /// The text.
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
    },
    /// The Human pause the Session waits in; none once it ended.
    Pause {
        /// Why it waits.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        reason: Option<String>,
    },
    /// A message of the viewer's that the engine refused. When it answers
    /// `open`, the channel ends.
    Error {
        /// Why, as a call's error.
        #[schemars(extend("arrived_in" = "0.1"))]
        error: ErrorObject,
    },
    /// The channel ends, as its last message.
    Ended {
        /// Why.
        #[schemars(extend("arrived_in" = "0.1"))]
        reason: LiveViewEnd,
    },
}

/// The Live view's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[LIVE_VIEW_VIEWER_LIMIT],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[ENABLED, TIER, MAX_VIEWERS],
    hook_stages: &[],
    shared_type_values: &[],
};
