//! Loopback multi-member peer-link tests for Client routing, cached group Run
//! index lookups, `run_moved` refresh and retry-once, event stream redirect and
//! forwarding, Live view single-use token redirect and peer relay, headroom
//! reporting, and `_meta` Trace context propagation (ADRs 0055, 0056, 0132,
//! 0140 and 0149, issue 244).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{EngineInstance, Form, Surface};
use goodfetch_protocol::events::EventsSubscribeParams;
use goodfetch_protocol::governor::GovernorLevel;
use goodfetch_protocol::group::{
    GROUP_HEADROOM_REPORT, LeadEpoch, MemberCapabilities, MemberHeadroom, PEER_PROTOCOL_VERSION,
    PeerHeadroomReportParams, PeerHeadroomReportResult, PlacementRequest, RUN_MOVED,
};
use goodfetch_protocol::jsonrpc::{RequestId, TraceContext};
use goodfetch_protocol::live_view::{LiveViewEngineMessage, LiveViewScreen, LiveViewTier};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::{
    Lifecycle, Presence, RunAxes, RunId, RunIndexRow, RunKind, RunOwner, Tags,
};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::super::loopback::{self, Lead, clock_at, identity, now, opened_store};
use super::super::{PeerServices, Peering, ask_lead, serve};
use crate::engine_context::{
    EventsRoute, ForwardedCall, LiveViewRoute, LocalDispatcher, RoutedCall,
};
use crate::events::{Events, EventsOptions};
use crate::group::placement::MemberRecord;
use crate::group::store;
use crate::live_view::{
    FrameSize, FrameSource, LiveView, SourceFrame, WatchFailure, WatchedSession, WatchedSessions,
    Watching,
};
use crate::run_index::RunIndex;
use crate::sync::{AtomicUsize, Mutex};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::trace;
use crate::transport::listen_peer;

/// Local dispatcher used in loopback routing tests: captures every dispatched
/// call (including raw params & parsed W3C `_meta` traceparent) and returns a
/// deterministic JSON-RPC reply bearing the caller's `RequestId`.
struct RecordingDispatcher {
    member: String,
    calls: AtomicUsize,
    seen_traceparents: Mutex<Vec<Option<String>>>,
}

impl RecordingDispatcher {
    fn new(member: &str) -> Arc<Self> {
        Arc::new(Self {
            member: member.to_owned(),
            calls: AtomicUsize::new(0),
            seen_traceparents: Mutex::new(Vec::new()),
        })
    }

    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn traceparents(&self) -> Vec<Option<String>> {
        self.seen_traceparents.lock().unwrap().clone()
    }
}

impl LocalDispatcher for RecordingDispatcher {
    fn dispatch(&self, call: ForwardedCall) -> Pin<Box<dyn Future<Output = Vec<u8>> + Send>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let traceparent = call
            .params_raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|v| {
                v.get("_meta")
                    .and_then(|m| m.get("traceparent"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        // Verify W3C Trace context parses cleanly on the owning Group member.
        if let Some(ref tp) = traceparent {
            let received = trace::receive(&TraceContext {
                traceparent: Some(tp.clone()),
                tracestate: None,
            });
            assert!(
                received.parent.is_some(),
                "forwarded traceparent must parse as a valid remote parent span"
            );
        }
        self.seen_traceparents.lock().unwrap().push(traceparent);
        let member = self.member.clone();
        Box::pin(async move {
            let mut out = serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": call.id,
                "result": {
                    "served_by": member,
                    "method": call.method,
                }
            }))
            .unwrap();
            out.push(b'\n');
            out
        })
    }
}

struct StubSource {
    sent_first: bool,
}

impl StubSource {
    fn new() -> Self {
        Self { sent_first: false }
    }
}

impl FrameSource for StubSource {
    fn tier(&self) -> LiveViewTier {
        LiveViewTier::Screencast
    }

    fn start<'a>(
        &'a mut self,
        _page: &'a PageId,
        _size: FrameSize,
    ) -> Watching<'a, Result<(), WatchFailure>> {
        Box::pin(async { Ok(()) })
    }

    fn next(&mut self) -> Watching<'_, Result<SourceFrame, WatchFailure>> {
        if self.sent_first {
            Box::pin(std::future::pending())
        } else {
            self.sent_first = true;
            Box::pin(async {
                Ok(SourceFrame {
                    width: 800.0,
                    height: 600.0,
                    data: "AAAA".to_owned(),
                })
            })
        }
    }

    fn stop(&mut self) -> Watching<'_, ()> {
        Box::pin(async {})
    }
}

struct StubSessions {
    sessions: Mutex<BTreeMap<SessionId, watch::Sender<Vec<PageId>>>>,
}

impl StubSessions {
    fn with(session: SessionId) -> Arc<Self> {
        let (tx, _rx) = watch::channel(vec![PageId("p1".to_owned())]);
        let mut map = BTreeMap::new();
        map.insert(session, tx);
        Arc::new(Self {
            sessions: Mutex::new(map),
        })
    }
}

impl WatchedSessions for StubSessions {
    fn watch(&self, session: &SessionId) -> Option<WatchedSession> {
        let guard = self.sessions.lock().unwrap();
        let tx = guard.get(session)?;
        Some(WatchedSession {
            tabs: tx.subscribe(),
            sources: vec![Box::new(StubSource::new())],
        })
    }
}

struct PeerNode {
    peering: Arc<Peering>,
    peer_address: String,
    _owner: TaskOwner,
    _env: TestEnv,
}

async fn spawn_member(
    lead: &Lead,
    name: &str,
    clock: crate::clock::Clock,
    services: PeerServices,
) -> PeerNode {
    let env = TestEnv::new();
    let store = opened_store(&env);
    let listener = listen_peer("127.0.0.1:0").await.unwrap();
    let peer_address = listener.local_address().unwrap().to_string();
    let (mut kept, cert) = lead.member(name, now() + Duration::from_hours(12));
    kept.peer_address = peer_address.clone();
    lead.admit_at(name, &peer_address, cert).await;
    assert!(
        store::joined(&store, kept.clone(), LeadEpoch::new(1))
            .await
            .unwrap()
    );
    let peering = Arc::new(
        Peering::new(
            identity(),
            store,
            &kept,
            LeadEpoch::new(1),
            Vec::new(),
            clock,
            services,
        )
        .unwrap(),
    );
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    owner.spawn(serve(
        listener,
        Arc::clone(&peering),
        owner.part("peer link"),
    ));
    PeerNode {
        peering,
        peer_address,
        _owner: owner,
        _env: env,
    }
}

fn index_row(run_id: &str, owner: &str) -> RunIndexRow {
    RunIndexRow {
        run_id: RunId::parse(run_id).unwrap(),
        location: format!("/tmp/{run_id}"),
        kind: RunKind::Collie,
        needs: Product::Collie,
        tags: Tags::default(),
        lifecycle: Lifecycle::Running,
        verdict: None,
        completeness: None,
        started_at: now(),
        ended_at: None,
        owner: Some(RunOwner::GroupMember(owner.to_owned())),
        presence: Presence::Present,
        progress_message: None,
        stop_reason: None,
        replayed: false,
    }
}

async fn lead_with_index(clock: crate::clock::Clock) -> (Lead, RunIndex, Arc<TaskOwner>, TestEnv) {
    let env = TestEnv::new();
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let run_index = RunIndex::of(
        env.home().join("run-index.sqlite"),
        &identity().product_version,
        None,
        Arc::clone(&owner),
    );
    let services = PeerServices {
        run_index: Some(run_index.clone()),
        owner: Some(Arc::clone(&owner)),
        ..PeerServices::default()
    };
    let lead =
        loopback::lead_with_services(clock, now() + Duration::from_hours(12), services).await;
    (lead, run_index, owner, env)
}

/// A unary call on a Run another Group member owns is forwarded over mTLS
/// after a cached group Run index lookup, preserving `_meta` Trace context,
/// and when the Run moves, the former owner answers `run_moved` so the
/// receiving Group member refreshes its cache and retries once.
///
/// Covers: story:3-109, story:3-110
fn sample_progress_call(run_id: &RunId, traceparent: &str) -> ForwardedCall {
    let raw_params = json!({
        "run_id": run_id.as_str(),
        "_meta": {
            "traceparent": traceparent,
            "tracestate": "vendor=1"
        }
    })
    .to_string();
    ForwardedCall {
        id: RequestId::Number(42),
        method: "run.progress".to_owned(),
        params_raw: Some(raw_params),
        surface: Surface::Rust,
        form: Form::Catalog,
        behind_gateway: false,
        answers_as_a_person: false,
    }
}

#[test]
fn unary_call_forwards_with_trace_context_and_retries_once_after_run_moved() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = clock_at(now());
        let (lead, lead_index, lead_owner, _lead_index_env) = lead_with_index(clock).await;

        let disp_b = RecordingDispatcher::new("member-b");
        let disp_c = RecordingDispatcher::new("member-c");

        let member_a = spawn_member(&lead, "member-a", clock, PeerServices::default()).await;
        let member_b = spawn_member(
            &lead,
            "member-b",
            clock,
            PeerServices {
                local_dispatcher: Some(Arc::clone(&disp_b) as Arc<dyn LocalDispatcher>),
                ..PeerServices::default()
            },
        )
        .await;
        let member_c = spawn_member(
            &lead,
            "member-c",
            clock,
            PeerServices {
                local_dispatcher: Some(Arc::clone(&disp_c) as Arc<dyn LocalDispatcher>),
                ..PeerServices::default()
            },
        )
        .await;

        let run_id = RunId::parse("run-routed-1").unwrap();
        lead_index
            .write(index_row(run_id.as_str(), "member-b"))
            .await
            .unwrap();

        let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa09c902b7-01";
        let call = sample_progress_call(&run_id, traceparent);

        let RoutedCall::Forwarded(bytes) = member_a
            .peering
            .route_unary_call(call.clone(), Some(run_id.clone()), None)
            .await
            .unwrap()
        else {
            panic!("expected forwarded response from member-b");
        };
        let reply: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reply["id"], 42);
        assert_eq!(reply["result"]["served_by"], "member-b");
        assert_eq!(disp_b.count(), 1);
        assert_eq!(disp_b.traceparents(), vec![Some(traceparent.to_owned())]);
        assert_eq!(
            member_a.peering.cached_run_route(&run_id).map(|r| r.member),
            Some("member-b".to_owned())
        );

        member_b
            .peering
            .mark_run_moved(&run_id, Some("member-c".to_owned()));
        lead_index
            .write(index_row(run_id.as_str(), "member-c"))
            .await
            .unwrap();

        let RoutedCall::Forwarded(bytes2) = member_a
            .peering
            .route_unary_call(call.clone(), Some(run_id.clone()), None)
            .await
            .unwrap()
        else {
            panic!("expected forwarded response from member-c after retry");
        };
        let reply2: Value = serde_json::from_slice(&bytes2).unwrap();
        assert_eq!(reply2["result"]["served_by"], "member-c");
        assert_eq!(disp_b.count(), 1);
        assert_eq!(disp_c.count(), 1);
        assert_eq!(disp_c.traceparents(), vec![Some(traceparent.to_owned())]);
        assert_eq!(
            member_a.peering.cached_run_route(&run_id).map(|r| r.member),
            Some("member-c".to_owned())
        );

        member_c.peering.mark_run_moved(&run_id, None);
        let err = member_a
            .peering
            .route_unary_call(call, Some(run_id), None)
            .await
            .err()
            .expect("second consecutive run_moved must fail without looping");
        assert_eq!(err.code(), RUN_MOVED.name);

        lead_owner.stop().await;
    });
}

async fn report_member_b_headroom(member_b: &PeerNode, clock: crate::clock::Clock) {
    let kept_b = store::identity(&member_b.peering.store)
        .await
        .unwrap()
        .unwrap()
        .0;
    let report_b = PeerHeadroomReportParams {
        protocol_version: PEER_PROTOCOL_VERSION,
        product: identity().product,
        product_version: identity().product_version,
        lead_epoch: LeadEpoch::new(1),
        member: "member-b".to_owned(),
        headroom: MemberHeadroom {
            http_available: 16,
            browser_available: 8,
            model_available: 4,
            level: GovernorLevel::Normal,
            ready: true,
        },
        capabilities: MemberCapabilities {
            browsers: vec!["chrome".to_owned()],
            plugins: vec!["alpha".to_owned()],
            local_addresses: vec!["203.0.113.1".to_owned()],
            local_model_servers: vec!["ollama".to_owned()],
            session_sources: Vec::new(),
            advertise_url: Some("https://member-b.example.test".to_owned()),
            product_version: identity().product_version,
        },
    };
    let _: PeerHeadroomReportResult = ask_lead(&kept_b, clock, GROUP_HEADROOM_REPORT, &report_b)
        .await
        .unwrap();
}

async fn assert_live_view_redirect_and_relay(
    member_a: &PeerNode,
    member_b: &PeerNode,
    session_redirect: &SessionId,
    session_relay: &SessionId,
    clock: crate::clock::Clock,
) {
    let lv_route_b = member_a
        .peering
        .route_live_view_for(session_redirect)
        .await
        .unwrap();
    let LiveViewRoute::Redirect {
        url,
        token,
        owner: lv_owner,
    } = lv_route_b
    else {
        panic!("expected LiveViewRoute::Redirect for member-b");
    };
    assert_eq!(url, "https://member-b.example.test");
    assert_eq!(lv_owner, "member-b");
    assert!(member_b.peering.live_view_tokens.consume(
        session_redirect,
        token.expose(),
        clock.wall()
    ));
    assert!(!member_b.peering.live_view_tokens.consume(
        session_redirect,
        token.expose(),
        clock.wall()
    ));

    let lv_route_c = member_a
        .peering
        .route_live_view_for(session_relay)
        .await
        .unwrap();
    let LiveViewRoute::Relay {
        owner: relay_owner,
        peer_address: relay_addr,
    } = lv_route_c
    else {
        panic!("expected LiveViewRoute::Relay for member-c");
    };
    let mut relay = member_a
        .peering
        .open_live_view_relay_stream(
            relay_owner,
            relay_addr,
            session_relay.clone(),
            LiveViewScreen {
                width: 800,
                height: 600,
                device_pixel_ratio: 1.0,
            },
        )
        .await
        .unwrap();
    let first_msg = relay
        .from_owner
        .recv()
        .await
        .expect("relayed Opened arrives");
    let parsed: LiveViewEngineMessage = serde_json::from_str(&first_msg).unwrap();
    assert!(matches!(
        parsed,
        LiveViewEngineMessage::Opened { ref session, .. } if session == session_relay
    ));
}

async fn assert_peer_placement_and_zero_encode(
    lead: &Lead,
    member_a: &PeerNode,
    member_c: &PeerNode,
) {
    lead.peering.record_member_for_test(MemberRecord {
        name: "member-c".to_owned(),
        peer_address: member_c.peer_address.clone(),
        headroom: MemberHeadroom {
            http_available: 2,
            browser_available: 1,
            model_available: 1,
            level: GovernorLevel::Pressure,
            ready: true,
        },
        capabilities: MemberCapabilities {
            browsers: vec!["chromium".to_owned()],
            product_version: identity().product_version,
            ..MemberCapabilities::default()
        },
    });
    let placed = member_a
        .peering
        .place_run(&PlacementRequest {
            receiving_member: "member-a".to_owned(),
            required_browsers: vec!["chrome".to_owned()],
            required_plugins: vec!["alpha".to_owned()],
            required_local_addresses: vec!["203.0.113.1".to_owned()],
            required_local_model_servers: vec!["ollama".to_owned()],
            ..PlacementRequest::default()
        })
        .await
        .unwrap();
    assert_eq!(placed.member, "member-b");

    let work = CountedWork::default();
    let forwarded_line =
        crate::wire::Line::copying(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n".to_vec());
    assert!(!forwarded_line.encoded());
    assert_eq!(work.read(Row::MessageEncodes), 0);
}

async fn assert_events_redirect_and_forward(
    member_a: &PeerNode,
    member_c: &PeerNode,
    events_c: &Events,
    run_redirect: &RunId,
    run_fwd: &RunId,
) {
    let route_ev_b = member_a
        .peering
        .route_events_for(run_redirect)
        .await
        .unwrap();
    assert!(matches!(
        route_ev_b,
        EventsRoute::Redirect { ref url, ref owner }
        if url == "https://member-b.example.test" && owner == "member-b"
    ));

    let EventsRoute::Forward {
        owner: ev_owner,
        peer_address: ev_addr,
    } = member_a.peering.route_events_for(run_fwd).await.unwrap()
    else {
        panic!("expected EventsRoute::Forward for member-c");
    };
    assert_eq!(ev_owner, "member-c");
    assert_eq!(ev_addr, member_c.peer_address);

    let mut forwarded_rx = member_a
        .peering
        .forward_events_stream(
            ev_owner,
            ev_addr,
            EventsSubscribeParams {
                after: None,
                system_info: false,
                run_id: Some(run_fwd.clone()),
            },
        )
        .await
        .unwrap();
    events_c
        .publish_axes(RunAxes {
            run_id: run_fwd.clone(),
            lifecycle: Lifecycle::Running,
            verdict: None,
            completeness: None,
        })
        .await;
    let batch = forwarded_rx
        .recv()
        .await
        .expect("forwarded event batch arrives");
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].event, "run.axes_changed");
}

/// Group members report headroom & `advertise_url` to Lead; event streams and
/// Live view streams redirect when `advertise_url` is set (with a single-use
/// Live view token verified in the owner's memory) and forward/relay over the
/// peer link when `advertise_url` is unset.
///
/// Covers: story:3-109, story:3-110
#[test]
fn events_and_live_view_redirect_when_advertised_and_forward_otherwise() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = clock_at(now());
        let (lead, lead_index, lead_owner, _lead_index_env) = lead_with_index(clock).await;

        let member_a_owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let member_a = spawn_member(
            &lead,
            "member-a",
            clock,
            PeerServices {
                owner: Some(Arc::clone(&member_a_owner)),
                ..PeerServices::default()
            },
        )
        .await;

        let session_redirect = SessionId("s-redirect".to_owned());
        let lv_owner_b = TaskOwner::root(BlockingPolicy::Inline);
        let lv_b = LiveView::new(
            Arc::from([StubSessions::with(session_redirect.clone()) as Arc<dyn WatchedSessions>]),
            &lv_owner_b,
            clock,
        );
        let member_b = spawn_member(
            &lead,
            "member-b",
            clock,
            PeerServices {
                live_view: Some(lv_b),
                ..PeerServices::default()
            },
        )
        .await;

        let session_relay = SessionId("s-relay".to_owned());
        let lv_owner_c = TaskOwner::root(BlockingPolicy::Inline);
        let lv_c = LiveView::new(
            Arc::from([StubSessions::with(session_relay.clone()) as Arc<dyn WatchedSessions>]),
            &lv_owner_c,
            clock,
        );
        let events_owner_c = TaskOwner::root(BlockingPolicy::Inline);
        let events_c = Events::start(
            EventsOptions::default(),
            EngineInstance::new("member-c-inst"),
            &events_owner_c,
            CountedWork::default(),
        );
        let member_c = spawn_member(
            &lead,
            "member-c",
            clock,
            PeerServices {
                events: Some(events_c.clone()),
                live_view: Some(lv_c),
                ..PeerServices::default()
            },
        )
        .await;

        report_member_b_headroom(&member_b, clock).await;

        let run_redirect = RunId::parse("run-redirect").unwrap();
        let run_fwd = RunId::parse("run-fwd").unwrap();
        lead_index
            .write(index_row(run_redirect.as_str(), "member-b"))
            .await
            .unwrap();
        lead_index
            .write(index_row(run_fwd.as_str(), "member-c"))
            .await
            .unwrap();
        lead.peering
            .register_session_owner(session_redirect.clone(), "member-b".to_owned());
        lead.peering
            .register_session_owner(session_relay.clone(), "member-c".to_owned());

        assert_events_redirect_and_forward(
            &member_a,
            &member_c,
            &events_c,
            &run_redirect,
            &run_fwd,
        )
        .await;
        assert_live_view_redirect_and_relay(
            &member_a,
            &member_b,
            &session_redirect,
            &session_relay,
            clock,
        )
        .await;
        assert_peer_placement_and_zero_encode(&lead, &member_a, &member_c).await;

        events_owner_c.stop().await;
        lv_owner_b.stop().await;
        lv_owner_c.stop().await;
        member_a_owner.stop().await;
        lead_owner.stop().await;
    });
}
