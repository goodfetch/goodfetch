//! A Session's events, which the engine publishes on its event stream (ADR
//! 0081), read through the Connection's own Subscription, played by the
//! Scripted engine as the Attached engine: the first Session stream sends
//! `events.subscribe` with its credits before its `session.on`, the next
//! shares it, each hands only its own Session's pages as working pages, a
//! dropped stream gives back the credits of the events it never read, the
//! last one dropped cancels the Subscription, and a stream whose
//! `session.on` the engine answers still hands the pages published before
//! the answer. Rust has no handler callbacks, so `listener_failed` has no
//! Rust form.

use goodfetch_collie_client::transport::{AttachedEngine, Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, blocking, events};
use goodfetch_test_support::{ATTACHED_SCRIPT, TestEnv, Tool, engine_script, tools};
use serde_json::{Value, json};

use crate::connection::hello;

/// The script both forms play: two Sessions' streams, then a third.
const SCRIPT: &str = "sdk-rust/page/session-listened.json";

/// The Scripted engine as the Attached engine of `env`'s home, playing
/// the corpus's script `name` with this crate's `hello`.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script before any async worker runs"
)]
pub(crate) fn scripted_engine(env: &TestEnv, name: &str) -> Endpoint {
    let mut script: Value =
        serde_json::from_slice(&std::fs::read(engine_script(name)).unwrap()).unwrap();
    script["steps"][0]["expect"]["params"] = serde_json::to_value(hello()).unwrap();
    let installed = env.home().join(ATTACHED_SCRIPT);
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::write(&installed, serde_json::to_vec(&script).unwrap()).unwrap();
    Endpoint::Attached(AttachedEngine {
        executable: tools()
            .unwrap()
            .path(Tool::ScriptedEngine)
            .unwrap()
            .to_path_buf(),
        environment: env
            .env()
            .iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect(),
    })
}

/// What both forms read: the pages the streams handed, in the order read,
/// what the first two pages' `evaluate` answered, and whether the last
/// stream ended once its `session.on` was answered.
type Read = ([String; 5], [Option<Value>; 2], bool);

/// What the script makes each form read: the first Session's first page,
/// the second Session's two, past the first's page, the first Session's
/// second, past the second's last, then the new stream's page; the first
/// two reached by their `evaluate`, and the new stream ended.
fn expected() -> Read {
    (
        ["page-1", "page-3", "page-4", "page-2", "page-6"].map(str::to_owned),
        [Some(json!(1)), Some(json!(2))],
        true,
    )
}

/// Covers: story:11-61, story:11-62, story:11-63
#[test]
fn a_sessions_streams_share_one_subscription_and_each_hands_its_own_sessions_pages() {
    let env = TestEnv::new();
    let engine = scripted_engine(&env, SCRIPT);
    let read = goodfetch_test_support::real_time_runtime().block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &engine,
            hello(),
        )
        .await
        .unwrap();
        let run = connection.start().await.unwrap();
        let first = run.new_session().await.unwrap();
        let second = run.new_session().await.unwrap();

        let mut firsts = first.events(events::Page);
        let mut seconds = second.events(events::Page);
        // The first Session's last page is the batch's last event, so the
        // second's unread page is already in its stream.
        let pages = [
            firsts.next().await.unwrap().unwrap(),
            seconds.next().await.unwrap().unwrap(),
            seconds.next().await.unwrap().unwrap(),
            firsts.next().await.unwrap().unwrap(),
        ];
        let values = [
            pages[0].evaluate("() => 1").await.unwrap(),
            pages[1].evaluate("() => 1").await.unwrap(),
        ];
        drop(seconds);
        drop(firsts);
        let mut thirds = first.events(events::Page);
        let third = thirds.next().await.unwrap().unwrap();
        let ended = thirds.next().await.is_none();
        drop(thirds);

        run.finish().await.unwrap();
        connection.close().await.unwrap();
        let [one, two, three, four] = &pages;
        (
            [one, two, three, four, &third].map(|page| page.id().0.clone()),
            values,
            ended,
        )
    });

    assert_eq!(read, expected());
}

/// The same script through `blocking`: each Session's `events` iterates
/// its own pages, each a working blocking page.
/// Covers: story:11-61, story:11-62, story:11-63
#[test]
fn a_sessions_events_iterate_its_own_pages_through_blocking() {
    let env = TestEnv::new();
    let engine = scripted_engine(&env, SCRIPT);
    let one = std::num::NonZeroUsize::MIN;
    let owned = goodfetch_runtime::named(goodfetch_runtime::Threads {
        workers: Some(one),
        blocking: one,
    })
    .unwrap();
    let (opened, _said) = goodfetch_test_support::real_time_runtime()
        .block_on(ClientConnection::open(
            ConnectionRuntime::owned(owned),
            &Network::Os,
            &engine,
            hello(),
        ))
        .unwrap();
    let connection = blocking::ClientConnection::from(opened);
    let run = connection.start().send().unwrap();
    let first = run.new_session().send().unwrap();
    let second = run.new_session().send().unwrap();

    let mut firsts = first.events(events::Page);
    let mut seconds = second.events(events::Page);
    let pages = [
        firsts.next().unwrap().unwrap(),
        seconds.next().unwrap().unwrap(),
        seconds.next().unwrap().unwrap(),
        firsts.next().unwrap().unwrap(),
    ];
    let values = [
        pages[0].evaluate("() => 1").send().unwrap(),
        pages[1].evaluate("() => 1").send().unwrap(),
    ];
    drop(seconds);
    drop(firsts);
    let mut thirds = first.events(events::Page);
    let third = thirds.next().unwrap().unwrap();
    let ended = thirds.next().is_none();
    drop(thirds);

    run.finish().send().unwrap();
    connection.close().unwrap();
    let [one, two, three, four] = &pages;
    let read = (
        [one, two, three, four, &third].map(|page| page.id().0.clone()),
        values,
        ended,
    );

    assert_eq!(read, expected());
}
