//! One viewer's channel: its `open`, held to the Daemon's token as a
//! Connection's `hello` is, then the Session's news and frames one way and
//! the viewer's screen and acknowledgements the other. A first message that
//! is no `open`, or whose token is missing or wrong, closes the channel
//! unanswered, as does one too long or too late, so a stranger learns
//! nothing; the caller records the refusal. Every message but a frame
//! passes the masks of the live Runs, as a Connection's do.

use std::future::Future;
use std::num::NonZeroU64;
use std::time::Duration;

use futures_util::stream::SplitSink;
use futures_util::{Sink, SinkExt as _, Stream, StreamExt as _};
use goodfetch_protocol::audit::RefusalReason;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::live_view::LiveViewScreen;
use goodfetch_protocol::live_view::{
    ENABLED, LIVE_VIEW_VIEWER_LIMIT, LiveViewEnd, LiveViewEngineMessage, LiveViewTier,
    LiveViewViewerLimitDetails, LiveViewViewerMessage, MAX_VIEWERS, TIER,
};
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use serde_json::Value;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::{Error, Message};

use super::{Frame, LiveView, NotWatched, Tiers, VideoOutput, Viewing, WatchFailure, Watching};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::endpoint::same_token;
use crate::mask::{Masks, Scope};
use crate::settings::{Resolved, get_or_default};

/// How long a channel the engine closed still reads what its viewer sends,
/// waiting for the viewer to close too, and how long its last message may
/// take to leave.
const LINGER: Duration = Duration::from_secs(2);

/// What a Daemon's Live view settings say, read as a channel opens.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Admission {
    /// `live_view.enabled`.
    pub(crate) enabled: bool,
    /// `live_view.max_viewers`, its value and its layer.
    pub(super) max_viewers: SettingLimit,
    /// `live_view.tier`.
    pub(super) tiers: Tiers,
}

impl Admission {
    /// What `resolved` says, or the catalog's defaults when the machine's
    /// settings could not be read.
    pub(crate) fn of(resolved: Option<&Resolved>) -> Self {
        let max: NonZeroU64 = get_or_default(resolved, &MAX_VIEWERS);
        let (layer, origin) = resolved
            .and_then(|resolved| resolved.effective(&MAX_VIEWERS))
            .map_or((SettingsLayer::Default, None), |effective| {
                (effective.layer, effective.origin)
            });
        let tier: &str = get_or_default(resolved, &TIER);
        Self {
            enabled: get_or_default(resolved, &ENABLED),
            max_viewers: SettingLimit {
                key: MAX_VIEWERS.name.to_owned(),
                value: max.get().into(),
                layer,
                origin,
            },
            tiers: match tier {
                "screencast" => Tiers::Only(LiveViewTier::Screencast),
                _ => Tiers::Any,
            },
        }
    }

    fn max_viewers(&self) -> u64 {
        self.max_viewers.value.as_u64().unwrap_or(1)
    }
}

/// What a channel needs of its Daemon.
pub(crate) struct Channel {
    /// The Daemon's Live view.
    pub(crate) live_view: LiveView,
    /// The Daemon's token, which `open` carries.
    pub(crate) token: std::sync::Arc<str>,
    /// The live Runs' masks.
    pub(crate) masks: Masks,
    /// The Daemon's clock.
    pub(crate) clock: Clock,
    /// Its settings, as the channel opened.
    pub(crate) admission: Admission,
    /// When `open` must have come by, from `timeout.hello`; none for no
    /// bound.
    pub(crate) deadline: Option<Instant>,
    /// How long `open` may be, from `network.max_hello_size`.
    pub(crate) max_open_size: usize,
}

/// How a viewer's first message opens its channel.
enum Opening {
    /// It watches this Session on this screen.
    Open(SessionId, LiveViewScreen),
    /// Its token was right, its message not: answered, then closed.
    Invalid(String),
    /// Closed unanswered, for this reason.
    Refused(RefusalReason),
    /// The viewer left first.
    Gone,
}

/// Why a channel's watch ended.
enum Ending {
    /// What the viewer is told last.
    Told(LiveViewEngineMessage),
    /// The viewer went away, or its channel failed.
    Gone,
}

/// Serves a viewer's channel over `socket` until it ends, or until
/// `closing` ends, which tells the viewer the Daemon is stopping.
///
/// # Errors
///
/// Why its first message was refused unanswered, which the caller records.
///
/// # Cancel safety
///
/// Dropping the future drops the channel and leaves the Session.
pub(crate) async fn serve<S>(
    channel: Channel,
    socket: S,
    closing: impl Future<Output = ()>,
) -> Result<(), RefusalReason>
where
    S: Stream<Item = Result<Message, Error>> + Sink<Message, Error = Error> + Send + Unpin,
{
    let (mut sink, mut stream) = socket.split();
    let opening = {
        let clock = channel.clock;
        let deadline = channel.deadline;
        let reading = opening(&mut stream, &channel);
        tokio::select! {
            biased;
            opening = reading => opening,
            () = sleep_until(clock, deadline) => Opening::Refused(RefusalReason::HelloTimedOut),
        }
    };
    let (session, screen) = match opening {
        Opening::Open(session, screen) => (session, screen),
        Opening::Refused(reason) => {
            end(&mut sink, &mut stream, channel.clock, None).await;
            return Err(reason);
        }
        Opening::Invalid(reason) => {
            let error = CallError::protocol(ProtocolFailure::InvalidParams, reason);
            let refused = error_message(&error);
            end(
                &mut sink,
                &mut stream,
                channel.clock,
                Some((refused, &channel.masks)),
            )
            .await;
            return Ok(());
        }
        Opening::Gone => return Ok(()),
    };
    let admission = &channel.admission;
    let watched =
        channel
            .live_view
            .watch(&session, screen, admission.max_viewers(), admission.tiers);
    let mut viewing = match watched {
        Ok(viewing) => viewing,
        Err(not_watched) => {
            let last = refusal(not_watched, &session, admission);
            end(
                &mut sink,
                &mut stream,
                channel.clock,
                Some((last, &channel.masks)),
            )
            .await;
            return Ok(());
        }
    };
    let opened = LiveViewEngineMessage::Opened {
        session,
        tier: viewing.tier(),
    };
    let ending = {
        let watching = watch(&mut viewing, &mut sink, &mut stream, &channel.masks, opened);
        tokio::select! {
            biased;
            () = closing => Ending::Told(LiveViewEngineMessage::Ended {
                reason: LiveViewEnd::DaemonStopping,
            }),
            ending = watching => ending,
        }
    };
    drop(viewing);
    match ending {
        Ending::Told(last) => {
            end(
                &mut sink,
                &mut stream,
                channel.clock,
                Some((last, &channel.masks)),
            )
            .await;
        }
        Ending::Gone => {}
    }
    Ok(())
}

/// The last message of a channel `open` asked for in vain.
fn refusal(
    not_watched: NotWatched,
    session: &SessionId,
    admission: &Admission,
) -> LiveViewEngineMessage {
    let error = match not_watched {
        NotWatched::ViewerLimit => CallError::new(
            &LIVE_VIEW_VIEWER_LIMIT,
            format!(
                "{} viewers watch Session `{}` already, as many as `live_view.max_viewers` allows",
                admission.max_viewers(),
                session.0
            ),
            &LiveViewViewerLimitDetails {
                limit: admission.max_viewers.clone(),
            },
        ),
        NotWatched::UnknownSession => CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("no open Session of this Daemon is `{}`", session.0),
        ),
        NotWatched::NoTier => {
            return LiveViewEngineMessage::Ended {
                reason: LiveViewEnd::CaptureLost,
            };
        }
    };
    error_message(&error)
}

fn error_message(error: &CallError) -> LiveViewEngineMessage {
    LiveViewEngineMessage::Error {
        error: error.to_error_object(),
    }
}

/// Reads the viewer's first message and what it opens.
async fn opening<S>(stream: &mut S, channel: &Channel) -> Opening
where
    S: Stream<Item = Result<Message, Error>> + Unpin,
{
    let bytes = loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) => break text.as_bytes().to_vec(),
            Some(Ok(Message::Binary(bytes))) => break bytes.to_vec(),
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
            Some(Err(Error::Capacity(_))) => {
                return Opening::Refused(RefusalReason::HelloTooLarge);
            }
            Some(Ok(Message::Close(_)) | Err(_)) | None => return Opening::Gone,
        }
    };
    if bytes.len() > channel.max_open_size {
        return Opening::Refused(RefusalReason::HelloTooLarge);
    }
    opened(&bytes, &channel.token)
}

/// What the first message `bytes` opens, under the Daemon's `token`.
fn opened(bytes: &[u8], token: &str) -> Opening {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Opening::Refused(RefusalReason::HelloNotFirst);
    };
    if value.get("kind").and_then(Value::as_str) != Some("open") {
        return Opening::Refused(RefusalReason::HelloNotFirst);
    }
    let Some(given) = value.get("token").and_then(Value::as_str) else {
        return Opening::Refused(RefusalReason::TokenMissing);
    };
    if !same_token(given, token) {
        return Opening::Refused(RefusalReason::TokenWrong);
    }
    match serde_json::from_value::<LiveViewViewerMessage>(value) {
        Ok(LiveViewViewerMessage::Open {
            session, screen, ..
        }) => Opening::Open(session, screen),
        Ok(_) => Opening::Refused(RefusalReason::HelloNotFirst),
        Err(error) => Opening::Invalid(format!("`open` is not a Live view's: {error}")),
    }
}

/// Watches the Session until the channel ends: `opened` first, then each
/// change and each frame the viewer is due.
async fn watch<S>(
    viewing: &mut Viewing,
    sink: &mut SplitSink<S, Message>,
    stream: &mut futures_util::stream::SplitStream<S>,
    masks: &Masks,
    opened: LiveViewEngineMessage,
) -> Ending
where
    S: Stream<Item = Result<Message, Error>> + Sink<Message, Error = Error> + Send + Unpin,
{
    if send(sink, masks, &opened).await.is_err() {
        return Ending::Gone;
    }
    loop {
        for news in viewing.news() {
            if matches!(news, LiveViewEngineMessage::Ended { .. }) {
                return Ending::Told(news);
            }
            if send(sink, masks, &news).await.is_err() {
                return Ending::Gone;
            }
        }
        if viewing.send_due(&mut Output(sink)).await.is_err() {
            return Ending::Gone;
        }
        let answer = tokio::select! {
            biased;
            () = viewing.changed() => None,
            message = stream.next() => match message {
                Some(Ok(Message::Text(text))) => received(viewing, text.as_bytes()),
                Some(Ok(Message::Binary(bytes))) => received(viewing, &bytes),
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => None,
                Some(Err(Error::Capacity(_))) => {
                    return Ending::Told(error_message(&CallError::protocol(
                        ProtocolFailure::MessageTooLarge,
                        "a Live view message is longer than a channel reads",
                    )));
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => return Ending::Gone,
            },
        };
        if let Some(answer) = answer
            && send(sink, masks, &error_message(&answer)).await.is_err()
        {
            return Ending::Gone;
        }
    }
}

/// Takes the viewer's message `bytes`: the error it is answered with, if
/// any.
fn received(viewing: &mut Viewing, bytes: &[u8]) -> Option<CallError> {
    match read(bytes) {
        Ok(LiveViewViewerMessage::Screen { screen }) => {
            viewing.set_screen(screen);
            None
        }
        Ok(LiveViewViewerMessage::Acknowledge { frame }) => {
            viewing.acknowledge(frame);
            None
        }
        Ok(LiveViewViewerMessage::Open { .. }) => Some(CallError::protocol(
            ProtocolFailure::InvalidRequest,
            "the channel is open already",
        )),
        Ok(other) => Some(CallError::protocol(
            ProtocolFailure::MethodNotFound,
            format!(
                "this engine's Live view does not carry `{}` yet",
                kind(&other)
            ),
        )),
        Err((failure, reason)) => Some(CallError::protocol(failure, reason)),
    }
}

/// A viewer's message `bytes`, or the failure it is and why.
fn read(bytes: &[u8]) -> Result<LiveViewViewerMessage, (ProtocolFailure, String)> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        (
            ProtocolFailure::ParseError,
            format!("a Live view message is no JSON: {error}"),
        )
    })?;
    serde_json::from_value(value).map_err(|error| {
        (
            ProtocolFailure::InvalidRequest,
            format!("no Live view message: {error}"),
        )
    })
}

/// `message`'s `kind`.
fn kind(message: &LiveViewViewerMessage) -> &'static str {
    match message {
        LiveViewViewerMessage::Open { .. } => "open",
        LiveViewViewerMessage::Screen { .. } => "screen",
        LiveViewViewerMessage::Acknowledge { .. } => "acknowledge",
        LiveViewViewerMessage::Signal { .. } => "signal",
        LiveViewViewerMessage::Input { .. } => "input",
        LiveViewViewerMessage::Control { .. } => "control",
        LiveViewViewerMessage::Tab { .. } => "tab",
        LiveViewViewerMessage::Dialog { .. } => "dialog",
        LiveViewViewerMessage::Files { .. } => "files",
        LiveViewViewerMessage::Clipboard { .. } => "clipboard",
        LiveViewViewerMessage::Pause { .. } => "pause",
    }
}

/// Sends `message`, masked, as one text message.
async fn send<S>(
    sink: &mut SplitSink<S, Message>,
    masks: &Masks,
    message: &LiveViewEngineMessage,
) -> Result<(), Error>
where
    S: Sink<Message, Error = Error>,
{
    let line = serde_json::to_vec(message).expect("invariant: an engine message encodes");
    let line = masks.apply_json_line(line, Scope::Returned);
    let text = String::from_utf8(line).expect("invariant: a masked message is JSON");
    sink.send(Message::text(text)).await
}

/// A viewer's frames, sent on its channel.
struct Output<'a, S>(&'a mut SplitSink<S, Message>);

impl<S> VideoOutput for Output<'_, S>
where
    S: Sink<Message, Error = Error> + Send,
{
    fn send<'a>(&'a mut self, frame: &'a Frame) -> Watching<'a, Result<(), WatchFailure>> {
        Box::pin(async move {
            self.0
                .send(Message::Text(frame.text()))
                .await
                .map_err(|_| WatchFailure::ViewerGone)
        })
    }
}

/// Ends the channel: sends `last`, masked, if any, closes, then reads and
/// drops what the viewer still sends until it closes too, for at most
/// [`LINGER`] in all.
async fn end<S>(
    sink: &mut SplitSink<S, Message>,
    stream: &mut futures_util::stream::SplitStream<S>,
    clock: Clock,
    last: Option<(LiveViewEngineMessage, &Masks)>,
) where
    S: Stream<Item = Result<Message, Error>> + Sink<Message, Error = Error> + Unpin,
{
    let ending = async {
        if let Some((last, masks)) = last {
            let _ = send(sink, masks, &last).await;
        }
        let _ = sink.close().await;
        while let Some(Ok(_)) = stream.next().await {}
    };
    tokio::select! {
        () = ending => {}
        () = clock.sleep(LINGER) => {}
    }
}

/// Sleeps on `clock` until `deadline`, or for ever when there is none.
async fn sleep_until(clock: Clock, deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => clock.sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// The fuzz target's body (ADR 0067): reads `data` as a viewer's first
/// message under a fixed token and as a later one; a message read must
/// encode and read back. It panics only on a bug.
pub fn fuzz_viewer_message(data: &[u8]) {
    let _ = opened(data, "fuzz-token");
    let Ok(message) = read(data) else {
        return;
    };
    let text = serde_json::to_vec(&message).expect("invariant: a read message encodes");
    let again = read(&text).expect("a message read reads back once encoded");
    assert_eq!(kind(&again), kind(&message), "a message keeps its kind");
}
