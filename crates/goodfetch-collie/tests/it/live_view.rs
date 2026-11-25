//! The Live view through Collie's executable (issue 959; ADR 0031): a
//! person's Client watches a headless Session of `daemon run` over a Live
//! view channel at its address's `/live-view`. Frames come sized to the
//! viewer's screen, one at a time as the viewer acknowledges them, the
//! newest only; more viewers join up to `live_view.max_viewers`; and with
//! `live_view.enabled` off no channel opens and nothing is captured. The
//! channel is a column of the endpoint table, each refusal in the Audit log.
#![expect(
    clippy::disallowed_types,
    reason = "a test opens its own WebSocket to the Daemon, outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use data_encoding::BASE64;
use goodfetch_protocol::live_view::CHANNEL_PATH;
use goodfetch_test_client::DaemonClient;
use goodfetch_test_support::{
    DaemonEndpoint, Endpoint, EndpointRunner, Listener, ListenerKind, Open, TestEnv, browser_cache,
};
use serde_json::{Value, json};
use tungstenite::handshake::client::generate_key;
use tungstenite::{Message, WebSocket};

use crate::daemon_run::{ANY_PORT, Foreground, opened, token};
use crate::endpoint::audit_verifier;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Run each test watches a page of.
const RUN: &str = "watched";

/// How long a viewer waits for a message: the first frame follows the
/// browser's screencast start.
const MESSAGE_DEADLINE: Duration = Duration::from_secs(60);

/// A foreground Daemon at a free loopback address, its user configuration
/// also `settings`, with a Run whose browser traces its CDP commands and
/// one page in it.
struct Watched {
    daemon: Foreground,
    address: SocketAddr,
    client: DaemonClient,
    session: Value,
    page: Value,
}

impl Watched {
    fn start(env: &TestEnv, settings: &str) -> Self {
        env.user_configuration(&format!("daemon.address = \"{ANY_PORT}\"\n{settings}"));
        let cache = browser_cache(env).unwrap();
        let mut daemon = Foreground::start(
            env,
            COLLIE,
            &[("GOODFETCH_BROWSERS_PATH", cache.to_str().unwrap())],
        );
        let endpoint = daemon.serving();
        let DaemonEndpoint::Tcp(address) = endpoint else {
            panic!("`daemon run` listens on TCP");
        };
        let mut client = opened(env, &endpoint);
        let started = client.call(
            1,
            "run.start",
            &json!({ "run_id": RUN, "settings": { "browser": { "cdp_command_log": true } } }),
        );
        assert!(started.get("result").is_some(), "{started}");
        let page = client.call(2, "run.new_page", &json!({ "run_id": RUN }));
        let (session, page) = (
            page["result"]["session_id"].clone(),
            page["result"]["page_id"].clone(),
        );
        assert!(session.is_string(), "{page}");
        Self {
            daemon,
            address,
            client,
            session,
            page,
        }
    }

    /// Has the page change its colour at each animation frame, so the
    /// browser's screencast has a new frame to give at each.
    fn animate(&mut self) {
        let animated = self.client.call(
            3,
            "page.evaluate",
            &json!({
                "page_id": self.page,
                "function": "() => { let n = 0; const step = () => { n += 1; document.documentElement.style.background = `hsl(${n % 360} 80% 50%)`; requestAnimationFrame(step); }; requestAnimationFrame(step); }",
            }),
        );
        assert!(animated.get("result").is_some(), "{animated}");
    }

    /// Finishes the Run and stops the Daemon; whether it exited with 0.
    fn stop(mut self) -> bool {
        let _ = self.client.call(9, "run.finish", &json!({ "run_id": RUN }));
        drop(self.client);
        self.daemon.terminate()
    }
}

/// A viewer's screen of `width` by `height` CSS pixels, at
/// `device_pixel_ratio`.
fn screen(width: u32, height: u32, device_pixel_ratio: f64) -> Value {
    json!({ "width": width, "height": height, "device_pixel_ratio": device_pixel_ratio })
}

/// The WebSocket handshake at the Live view's path of `address`, from the
/// Daemon's own origin.
fn handshake(address: SocketAddr) -> Result<WebSocket<TcpStream>, tungstenite::Error> {
    let request = tungstenite::http::Request::builder()
        .uri(format!("ws://{address}{CHANNEL_PATH}"))
        .header("Host", address.to_string())
        .header("Origin", format!("http://{address}"))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key())
        .body(())
        .unwrap();
    let stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(MESSAGE_DEADLINE)).unwrap();
    tungstenite::client(request, stream)
        .map(|(socket, _)| socket)
        .map_err(|error| match error {
            tungstenite::HandshakeError::Failure(error) => error,
            tungstenite::HandshakeError::Interrupted(_) => {
                panic!("a blocking handshake is never interrupted")
            }
        })
}

/// A viewer of `watched`'s Session on `screen`: its channel, once it sent
/// `open`.
fn viewer(env: &TestEnv, watched: &Watched, screen: &Value) -> WebSocket<TcpStream> {
    let mut socket = handshake(watched.address).unwrap();
    let open = json!({
        "kind": "open",
        "token": token(env),
        "session": watched.session,
        "screen": screen,
    });
    socket.send(Message::text(open.to_string())).unwrap();
    socket
}

/// The channel's next message; `None` once it closed.
fn next(socket: &mut WebSocket<TcpStream>) -> Option<Value> {
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => return Some(serde_json::from_str(&text).unwrap()),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

/// The channel's next message of `kind`, and the kinds read before it.
fn next_of(socket: &mut WebSocket<TcpStream>, kind: &str) -> (Value, Vec<String>) {
    let mut before = Vec::new();
    loop {
        let message = next(socket).unwrap_or_else(|| panic!("closed before `{kind}`: {before:?}"));
        if message["kind"] == json!(kind) {
            return (message, before);
        }
        before.push(message["kind"].as_str().unwrap_or_default().to_owned());
    }
}

/// Acknowledges the frame `frame`.
fn acknowledge(socket: &mut WebSocket<TcpStream>, frame: &Value) {
    let acknowledged = json!({ "kind": "acknowledge", "frame": frame["frame"] });
    socket
        .send(Message::text(acknowledged.to_string()))
        .unwrap();
}

/// The kinds of every message the channel sends within `window`.
#[expect(
    clippy::disallowed_methods,
    reason = "the test watches a real Daemon's socket for a span of wall-clock time"
)]
fn sent_within(socket: &mut WebSocket<TcpStream>, window: Duration) -> Vec<String> {
    socket.get_ref().set_read_timeout(Some(window)).unwrap();
    let ends = Instant::now() + window;
    let mut kinds = Vec::new();
    while Instant::now() < ends {
        match socket.read() {
            Ok(Message::Text(text)) => {
                let message: Value = serde_json::from_str(&text).unwrap();
                kinds.push(message["kind"].as_str().unwrap_or_default().to_owned());
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("{error}"),
        }
    }
    socket
        .get_ref()
        .set_read_timeout(Some(MESSAGE_DEADLINE))
        .unwrap();
    kinds
}

/// The width and height a JPEG's start-of-frame marker gives.
fn jpeg_size(data: &Value) -> (u32, u32) {
    let bytes = BASE64.decode(data.as_str().unwrap().as_bytes()).unwrap();
    let mut at = 2;
    while at + 9 < bytes.len() {
        assert_eq!(bytes[at], 0xFF, "a JPEG segment starts with 0xFF");
        let marker = bytes[at + 1];
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        if matches!(marker, 0xC0..=0xC3) {
            let height = u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]);
            let width = u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]);
            return (u32::from(width), u32::from(height));
        }
        at += 2 + length;
    }
    panic!("the frame is no JPEG with a start of frame");
}

/// A viewer's channel opens on the Session, lists its tabs and gives
/// frames of its page, each a JPEG no larger than the viewer's screen in
/// device pixels and filling it on one side; the browser started its
/// screencast.
///
/// Covers: story:12-38, story:12-144, setting:live_view.enabled, setting:live_view.tier
#[test]
fn a_viewer_gets_frames_sized_to_its_screen() {
    let env = TestEnv::new();
    let mut watched = Watched::start(&env, "");
    watched.animate();

    let mut socket = viewer(&env, &watched, &screen(320, 240, 2.0));
    let opened = next(&mut socket).unwrap();
    let (tabs, _) = next_of(&mut socket, "tabs");
    let (frame, _) = next_of(&mut socket, "frame");
    let (width, height) = jpeg_size(&frame["data"]);
    let _ = socket.close(None);
    let (session, page) = (watched.session.clone(), watched.page.clone());
    let exited = watched.stop();
    let traced = crate::page::traced_methods(&env, RUN);

    assert_eq!(
        (
            opened,
            (tabs["shown"].clone(), frame["page"].clone()),
            width <= 640 && height <= 480 && (width >= 636 || height >= 476),
            traced.contains("Page.startScreencast"),
            exited,
        ),
        (
            json!({ "kind": "opened", "session": session, "tier": "screencast" }),
            (page.clone(), page),
            true,
            true,
            true,
        ),
        "a frame of {width}x{height}, of the tabs {tabs}"
    );
}

/// A viewer that does not acknowledge its frame gets no other, however
/// many the page draws meanwhile; once it acknowledges, it gets the newest,
/// its number past every frame it skipped.
///
/// Covers: story:12-38
#[test]
fn a_slow_viewer_gets_only_the_newest_frame() {
    let env = TestEnv::new();
    let mut watched = Watched::start(&env, "");
    watched.animate();

    let mut socket = viewer(&env, &watched, &screen(320, 240, 1.0));
    let (first, _) = next_of(&mut socket, "frame");
    let unacknowledged = sent_within(&mut socket, Duration::from_secs(2));
    acknowledge(&mut socket, &first);
    let (newest, between) = next_of(&mut socket, "frame");
    let _ = socket.close(None);
    let exited = watched.stop();
    let (first, newest) = (
        first["frame"].as_u64().unwrap(),
        newest["frame"].as_u64().unwrap(),
    );

    assert_eq!(
        (
            unacknowledged.contains(&"frame".to_owned()),
            between.contains(&"frame".to_owned()),
            newest > first + 1,
            exited,
        ),
        (false, false, true, true),
        "frame {first}, then {unacknowledged:?}, then {between:?} and frame {newest}"
    );
}

/// A second and a third viewer join the Session one watches already, and
/// each gets the Session's frames.
///
/// Covers: story:12-38
#[test]
fn more_viewers_join_a_watched_session() {
    let env = TestEnv::new();
    let mut watched = Watched::start(&env, "");
    watched.animate();

    let mut sockets = [
        screen(320, 240, 1.0),
        screen(640, 480, 1.0),
        screen(400, 300, 1.0),
    ]
    .map(|screen| viewer(&env, &watched, &screen));
    let opened = sockets
        .iter_mut()
        .map(|socket| next(socket).unwrap()["kind"].clone())
        .collect::<Vec<_>>();
    let framed = sockets
        .iter_mut()
        .map(|socket| next_of(socket, "frame").0["page"] == watched.page)
        .collect::<Vec<_>>();
    for socket in &mut sockets {
        let _ = socket.close(None);
    }
    let exited = watched.stop();

    assert_eq!(
        (opened, framed, exited),
        (vec![json!("opened"); 3], vec![true; 3], true)
    );
}

/// The viewer past `live_view.max_viewers` gets `live_view_viewer_limit`,
/// naming the setting, its value and its layer, as its channel's last
/// message, while the viewers within it keep watching.
///
/// Covers: story:12-38, setting:live_view.max_viewers, error:live_view_viewer_limit
#[test]
fn the_viewer_past_the_limit_is_refused_naming_the_setting() {
    let env = TestEnv::new();
    let mut watched = Watched::start(&env, "live_view.max_viewers = 1\n");
    watched.animate();

    let mut first = viewer(&env, &watched, &screen(320, 240, 1.0));
    let (_, _) = next_of(&mut first, "opened");
    let mut past = viewer(&env, &watched, &screen(320, 240, 1.0));
    let refused = next(&mut past).unwrap();
    let ended = next(&mut past);
    let (frame, _) = next_of(&mut first, "frame");
    let _ = first.close(None);
    let page = watched.page.clone();
    let exited = watched.stop();

    assert_eq!(
        (
            refused["kind"].clone(),
            refused["error"]["data"]["code"].clone(),
            refused["error"]["data"]["limit"].clone(),
            ended,
            frame["page"] == page,
            exited,
        ),
        (
            json!("error"),
            json!("live_view_viewer_limit"),
            json!({ "key": "live_view.max_viewers", "value": 1, "layer": "user_configuration" }),
            None,
            true,
            true,
        ),
        "{refused}"
    );
}

/// With `live_view.enabled` off, the Daemon answers a Live view handshake
/// `404`, a closed door, and its browser never starts a screencast.
///
/// Covers: story:12-144, setting:live_view.enabled
#[test]
fn with_the_live_view_off_no_channel_opens_and_nothing_is_captured() {
    let env = TestEnv::new();
    let mut watched = Watched::start(&env, "live_view.enabled = false\n");
    watched.animate();

    let answered = match handshake(watched.address) {
        Err(tungstenite::Error::Http(response)) => Some(response.status().as_u16()),
        Err(error) => panic!("{error}"),
        Ok(_) => None,
    };
    let exited = watched.stop();
    let traced = crate::page::traced_methods(&env, RUN);

    assert_eq!(
        (
            answered,
            traced.contains("Page.startScreencast"),
            traced.is_empty(),
            exited,
        ),
        (Some(404), false, false, true)
    );
}

/// The Live view channel, read as the endpoint table's column: it passes
/// every row, each refusal in the Audit log with its listener and reason.
///
/// Covers: story:12-38
#[test]
fn the_live_view_channel_passes_its_endpoint_table_rows() {
    let env = TestEnv::new();
    let watched = Watched::start(&env, "");
    let open = json!({ "session": watched.session, "screen": screen(320, 240, 1.0) });
    let listener = Listener::new(
        ListenerKind::LiveView,
        Endpoint::LiveView {
            address: watched.address,
            path: CHANNEL_PATH.to_owned(),
            open: Open::new(open.as_object().unwrap().clone()),
        },
        token(&env),
    )
    .with_other_listener_token("a-private-endpoints-token");

    let ran = EndpointRunner::new(audit_verifier(&env, "live_view")).run(&listener);
    let exited = watched.stop();

    assert_eq!((ran, exited), (Ok(()), true));
}
