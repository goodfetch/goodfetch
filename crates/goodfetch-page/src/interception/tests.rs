//! The interception's tables, without a browser: what a page's or a
//! Session's end takes out of them, and where a handler's failure waits;
//! and the headers step 7 sends a request on with when its Session signs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use goodfetch_clean_up_rules::matching::ResourceKind;
use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_engine::CallError;
use goodfetch_engine::egress::{Policy, RequestKind};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::request_signing::RequestSigner;
use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::{PageId, SessionId};
use secrecy::SecretString;
use serde_json::{Map, Value, json};

use super::navigations::PendingNavigation;
use super::{
    EngineRequests, Interception, Learned, LeftFailure, PageInterception, PageState, lock, steps,
};
use crate::cdp::{PausedId, RequestHeaders};
use crate::page_value::Permit;

/// The interception state of a page or a service worker of the Session
/// `session` in `interception`'s tables: its handlers' failures wait in
/// `failure`, the page's own slot, or its Session's for a worker.
fn state_of(
    interception: &Interception,
    session: &SessionId,
    failure: Option<LeftFailure>,
) -> PageState {
    let mut tables = lock(&interception.shared.tables);
    let pending = Arc::clone(tables.pending.entry(session.clone()).or_default());
    let routes = Arc::clone(tables.sessions.entry(session.clone()).or_default());
    let session_failure = Arc::clone(tables.failures.entry(session.clone()).or_default());
    PageState {
        routes: Arc::default(),
        session: routes,
        failure: failure.unwrap_or_else(|| Arc::clone(&session_failure)),
        session_failure,
        cosmetic: None,
        engine_requests: EngineRequests {
            policy: Policy::new(&["*"], &[] as &[&str]).expect("invariant: `*` is a host pattern"),
            blocking: steps::Blocking {
                never_block: Arc::new(NeverBlock::new()),
                lists: None,
                choices: goodfetch_clean_up_rules::matching::Choices::default(),
                work: goodfetch_counted_work::CountedWork::default(),
            },
            signing: None,
        },
        pending,
        navigation: PendingNavigation::default(),
        navigating: None,
        files: Mutex::new(None),
        own: Mutex::new(None),
        lists_on: false,
        retrying: AtomicBool::new(false),
        lists_blocked: std::sync::atomic::AtomicU64::new(0),
        learned: Arc::new(Learned::default()),
        on_document: None,
    }
}

/// The side of the interception of the page `page` of the Session
/// `session`, held as its start holds it once the browser paused its
/// requests.
pub(crate) fn hold_page(
    interception: &Interception,
    page: &PageId,
    session: &SessionId,
) -> PageInterception {
    let state = Arc::new(state_of(
        interception,
        session,
        Some(LeftFailure::default()),
    ));
    PageInterception::joined(page, &state)
}

/// The page and Session the tests hold.
fn page_and_session() -> (PageId, SessionId) {
    (
        PageId("page-1".to_owned()),
        SessionId("session-1".to_owned()),
    )
}

/// A page that closes before its navigation's `load` gives back the permit
/// a service worker's request took for that navigation, so a pooled
/// Session's domain is not held for its life.
#[test]
fn a_page_that_closes_before_load_gives_back_its_navigations_permit() {
    let interception = Interception::default();
    let (page, session) = page_and_session();
    interception.open_session(&session);
    let held = hold_page(&interception, &page, &session);
    held.state.navigation.started("http://a.test/#top");
    let carried = held.state.pending.take("http://a.test/");
    let released = Arc::new(AtomicBool::new(false));
    let releasing = Arc::clone(&released);
    held.state
        .navigation
        .hold(Permit::new(move || releasing.store(true, Ordering::SeqCst)));

    held.close();

    assert_eq!(
        (
            carried.is_some_and(|carried| Arc::ptr_eq(&carried, &held.state)),
            released.load(Ordering::SeqCst),
            held.state.pending.any(),
        ),
        (true, true, false)
    );
}

/// A service worker's request to a page's pending navigation takes it
/// once: a second request to its URL carries none.
#[test]
fn a_pending_navigation_is_taken_once() {
    let interception = Interception::default();
    let (page, session) = page_and_session();
    interception.open_session(&session);
    let held = hold_page(&interception, &page, &session);
    held.state.navigation.started("http://a.test/");

    let first = held.state.pending.take("http://a.test/#top");
    let second = held.state.pending.take("http://a.test/");

    assert_eq!((first.is_some(), second.is_some()), (true, false));
}

/// A Session route's handler that fails on a service worker's request,
/// which belongs to no page, leaves its failure on the Session: the next
/// call on a page of the Session raises it, once.
#[test]
fn a_failure_on_a_service_workers_request_waits_for_the_sessions_next_call() {
    let interception = Interception::default();
    let (page, session) = page_and_session();
    interception.open_session(&session);
    let held = hold_page(&interception, &page, &session);
    let worker = state_of(&interception, &session, None);

    worker.leave(CallError::new(
        &CLOSED,
        "the handler failed",
        &serde_json::json!({}),
    ));

    assert_eq!(
        (
            held.take_failure().map(|failure| failure.code()),
            held.take_failure().is_none(),
        ),
        (Some(CLOSED.name), true)
    );
}

/// RFC 9421, Appendix B.1.4's Ed25519 test key, as an `identified`
/// Session's Secret holds it.
const RFC_KEY: &str = r#"{"kty":"OKP","crv":"Ed25519","d":"n4Ni-HpISpVObnQMW0wOhCKROaIKqKtW_2ZYb2p9KcU","x":"JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs"}"#;

#[expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Entropy (ADR 0059)"
)]
fn signer() -> RequestSigner {
    RequestSigner::new(
        &SecretString::from(RFC_KEY.to_owned()),
        "https://bot.example",
        Entropy::seeded(1551),
    )
    .expect("the key signs")
}

/// The headers step 8 sends a request to `url` on with, the request
/// carrying `headers` under `overrides` and its Session signing with
/// `signing`; none when it is sent on with its own.
fn sent_headers(
    url: &str,
    headers: &Map<String, Value>,
    overrides: &steps::Overrides,
    signing: Option<&RequestSigner>,
) -> Option<Vec<(String, String)>> {
    let state = state_of(&Interception::default(), &page_and_session().1, None);
    let request_id = PausedId::new("request-1".to_owned());
    let paused = steps::Paused {
        id: &request_id,
        url,
        method: "GET",
        overrides,
        headers: RequestHeaders::of(headers),
        kind: ResourceKind::Xhr,
        source_url: url,
        egress: RequestKind::Page,
    };
    let EngineRequests {
        policy, blocking, ..
    } = &state.engine_requests;
    match steps::send_on(&paused, policy, blocking, signing, false) {
        Ok(steps::Sent::Continue(command)) => command.headers.map(|headers| {
            headers
                .into_iter()
                .map(|header| (header.name, header.value))
                .collect()
        }),
        other => panic!("the request is sent on: {other:?}"),
    }
}

/// The names of `headers`.
fn names(headers: &[(String, String)]) -> Vec<&str> {
    headers.iter().map(|(name, _)| name.as_str()).collect()
}

/// An `identified` Session signs each request over the headers it carries,
/// in place of any signature a page put on it (#1551).
#[test]
fn an_identified_session_signs_a_request_in_place_of_the_signature_it_carried() {
    let carried = json!({ "Accept": "text/html", "signature": "sig1=:forged:" });
    let carried = carried.as_object().expect("an object");

    let sent = sent_headers(
        "https://identity.test/",
        carried,
        &steps::Overrides::default(),
        Some(&signer()),
    )
    .expect("the request is sent with its signature");

    assert_eq!(
        names(&sent),
        ["Accept", "Signature-Agent", "Signature-Input", "Signature"]
    );
    assert!(!sent.iter().any(|(_, value)| value.contains("forged")));
}

/// A route's headers replace the request's whole, and the signature is
/// added to them.
#[test]
fn a_routes_headers_are_signed_in_place_of_the_requests() {
    let overrides = steps::Overrides {
        headers: Some([("X-Route".to_owned(), "1".to_owned())].into()),
        ..steps::Overrides::default()
    };

    let sent = sent_headers(
        "https://identity.test/",
        json!({ "Accept": "text/html" })
            .as_object()
            .expect("an object"),
        &overrides,
        Some(&signer()),
    )
    .expect("the request is sent with its signature");

    assert_eq!(
        names(&sent),
        ["X-Route", "Signature-Agent", "Signature-Input", "Signature"]
    );
}

/// A Session that does not sign sends a request on with its own headers.
#[test]
fn a_session_that_does_not_sign_keeps_the_requests_own_headers() {
    let sent = sent_headers(
        "https://identity.test/",
        json!({ "Accept": "text/html" })
            .as_object()
            .expect("an object"),
        &steps::Overrides::default(),
        None,
    );

    assert_eq!(sent, None);
}
