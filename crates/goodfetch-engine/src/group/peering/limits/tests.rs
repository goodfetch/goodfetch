//! Group-wide Usage ledger slices, local *group redirect* server, unreachable
//! Lead Daemon behavior, 10-minute token check cache, and group erasure
//! broadcasts over loopback mutual TLS (ADRs 0041, 0046 and 0055, issue 258).
#![expect(
    clippy::disallowed_methods,
    reason = "loopback tests write Lead Daemon files and run blocking Charging calls on test threads"
)]
#![expect(
    clippy::too_many_lines,
    reason = "end-to-end multi-node loopback scenarios assert each step sequentially in one test"
)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use data_encoding::HEXLOWER;
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::EngineInstance;
use goodfetch_protocol::group::{
    ErasureTarget, GROUP_LEAD_REACHABILITY_CHANGED, GroupLeadReachabilityChanged, LEAD_UNREACHABLE,
};
use goodfetch_protocol::home::ACCOUNTS_STORE;
use goodfetch_protocol::run::{LimitScope, LimitSetter, Period, PeriodLength, RunId};
use goodfetch_protocol::settings::TIMEOUT_NAVIGATION;
use goodfetch_protocol::usage::{CONNECTION, DAEMON, INSTALL, Limit, PERSON, REQUESTS, TOKEN};
use goodfetch_test_support::{TestEnv, real_time_runtime};
use jiff::Timestamp;
use rusqlite::Connection;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use super::super::control_state::{replicate_control_state, sync_control_state};
use super::super::erasure::retry_pending_erasures;
use super::super::loopback::{clock_at, identity, name, now};
use super::super::{PeerServices, Peering, serve};
use super::{
    GroupMemberLedger, TOKEN_CHECK_CACHE_MAX_TTL, TokenCheckCache, serve_local_redirect,
    start_local_redirect,
};
use crate::accounts::AccountsStore;
use crate::clock::Clock;
use crate::events::{Events, EventsOptions};
use crate::group::identity::KeptIdentity;
use crate::group::store::{self, Certified, Joining, STORE};
use crate::group::{CERTIFICATE_LIFETIME, GroupAuthority, GroupMemberKey, JoinToken};
use crate::home::Home;
use crate::ledger::{Charging, Holder, Ledger, Placement, RedirectClient, Refusal, SliceSizing};
use crate::metrics::Metrics;
use crate::response_cache::{CacheRequest, Lookup, Partition, ResponseCache, ResponseHead};
use crate::settings::{Settings, SettingsFiles};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::listen_peer;
use crate::user_store::UserStore;

struct GroupNode {
    _env: TestEnv,
    home: Home,
    owner: Arc<TaskOwner>,
    ledger: Ledger,
    work: CountedWork,
    settings: Settings,
    response_cache: ResponseCache,
    events: Events,
    peering: Arc<Peering>,
    kept: KeptIdentity,
    store: UserStore,
    authority: Option<GroupAuthority>,
}

fn sha256_hex(secret: &str) -> String {
    HEXLOWER.encode(&Sha256::digest(secret.as_bytes()))
}

fn make_services(
    home: &Home,
    owner: &Arc<TaskOwner>,
    settings: &Settings,
    ledger: &Ledger,
    response_cache: &ResponseCache,
    events: &Events,
    metrics: &Metrics,
) -> PeerServices {
    PeerServices {
        settings: Some(settings.clone()),
        ledger: Some(ledger.clone()),
        response_cache: Some(response_cache.clone()),
        events: Some(events.clone()),
        metrics: Some(metrics.clone()),
        home: Some(home.clone()),
        owner: Some(Arc::clone(owner)),
        ..PeerServices::default()
    }
}

async fn start_lead_node(clock: Clock) -> GroupNode {
    let env = TestEnv::new();
    let id = identity();
    let home = Home::new(env.home().to_path_buf());
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Threads));
    let work = CountedWork::default();
    let settings = Settings::new(
        SettingsFiles::of(&home, &id.product_version),
        Product::Collie,
        owner.part("settings"),
    );
    settings.set_group_member(Some("lead".to_owned()));
    let ledger = Ledger::at(home.usage_ledger(), None).counting(work.clone());
    ledger.set_own_redirect_member(Some("lead".to_owned()));
    let response_cache = ResponseCache::of(
        home.per_major(&id.product_version).join("response-cache"),
        None,
        Arc::clone(&owner),
        work.clone(),
    );
    let events = Events::start(
        EventsOptions::default(),
        EngineInstance::new("lead-inst"),
        &owner,
        work.clone(),
    );
    let metrics = Metrics::none();
    let store = UserStore::open(
        home.per_major(&id.product_version).join(STORE.file()),
        &STORE,
        None,
        Arc::clone(&owner),
    );
    let listener = listen_peer("127.0.0.1:0").await.unwrap();
    let address = listener.local_address().unwrap().to_string();
    let mut authority = GroupAuthority::create(now()).unwrap();
    let kept = KeptIdentity::lead(&mut authority, &name("lead"), address, now()).unwrap();
    let epoch = store::form(&store, authority.kept(), kept.clone(), now())
        .await
        .unwrap()
        .unwrap();
    let peering = Arc::new(
        Peering::new(
            id,
            store.clone(),
            &kept,
            epoch,
            vec![kept.name.clone()],
            clock,
            make_services(
                &home,
                &owner,
                &settings,
                &ledger,
                &response_cache,
                &events,
                &metrics,
            ),
        )
        .unwrap(),
    );
    peering.hydrate_control_state().await;
    let redirect = start_local_redirect(&peering).await;
    owner.spawn(serve(
        listener,
        Arc::clone(&peering),
        owner.part("peer link"),
    ));
    if let Some(redirect) = redirect {
        owner.spawn(serve_local_redirect(
            Arc::clone(&peering),
            redirect,
            owner.part("group redirect"),
        ));
    }
    owner.spawn(replicate_control_state(
        Arc::clone(&peering),
        owner.part("control state replication"),
    ));
    owner.spawn(retry_pending_erasures(
        Arc::clone(&peering),
        owner.part("erasure retry"),
    ));
    GroupNode {
        _env: env,
        home,
        owner,
        ledger,
        work,
        settings,
        response_cache,
        events,
        peering,
        kept,
        store,
        authority: Some(authority),
    }
}

async fn start_worker_node(lead: &mut GroupNode, member_name: &str, clock: Clock) -> GroupNode {
    let env = TestEnv::new();
    let id = identity();
    let home = Home::new(env.home().to_path_buf());
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Threads));
    let work = CountedWork::default();
    let settings = Settings::new(
        SettingsFiles::of(&home, &id.product_version),
        Product::Collie,
        owner.part("settings"),
    );
    settings.set_group_member(Some(member_name.to_owned()));
    let ledger = Ledger::at(home.usage_ledger(), None).counting(work.clone());
    ledger.set_own_redirect_member(Some(member_name.to_owned()));
    let response_cache = ResponseCache::of(
        home.per_major(&id.product_version).join("response-cache"),
        None,
        Arc::clone(&owner),
        work.clone(),
    );
    let events = Events::start(
        EventsOptions::default(),
        EngineInstance::new(format!("{member_name}-inst")),
        &owner,
        work.clone(),
    );
    let metrics = Metrics::none();
    let store = UserStore::open(
        home.per_major(&id.product_version).join(STORE.file()),
        &STORE,
        None,
        Arc::clone(&owner),
    );
    let listener = listen_peer("127.0.0.1:0").await.unwrap();
    let address = listener.local_address().unwrap().to_string();

    let authority = lead.authority.as_mut().unwrap();
    let key = GroupMemberKey::generate().unwrap();
    let signed = authority
        .sign(&name(member_name), &key.public_key(), now())
        .unwrap();
    let token = JoinToken::mint(now()).unwrap();
    assert!(
        store::record_token(&lead.store, &token, now())
            .await
            .unwrap()
    );
    let joining = Joining {
        name: member_name.to_owned(),
        peer_address: address.clone(),
        certificate: Certified::from(&signed),
    };
    let admitted = store::admit(&lead.store, token.token.expose_secret(), joining, now())
        .await
        .unwrap();
    assert_eq!(admitted, store::Admission::Admitted);
    lead.peering.on_member_added(member_name.to_owned());
    lead.peering.notify_control_state();

    let kept = KeptIdentity {
        name: member_name.to_owned(),
        key: key.kept(),
        certificate: signed.certificate,
        authority: authority.certificate().clone(),
        revocations: lead.kept.revocations.clone(),
        peer_address: address,
        lead_address: lead.kept.peer_address.clone(),
        lead_name: lead.kept.name.clone(),
        certificate_expires_at: now() + CERTIFICATE_LIFETIME,
        renew_at: now() + CERTIFICATE_LIFETIME,
    };
    let epoch = lead.peering.highest_epoch().await;
    assert!(store::joined(&store, kept.clone(), epoch).await.unwrap());

    let peering = Arc::new(
        Peering::new(
            id,
            store.clone(),
            &kept,
            epoch,
            vec![lead.kept.name.clone(), member_name.to_owned()],
            clock,
            make_services(
                &home,
                &owner,
                &settings,
                &ledger,
                &response_cache,
                &events,
                &metrics,
            ),
        )
        .unwrap(),
    );
    peering.hydrate_control_state().await;
    ledger.set_authority(Some(Arc::new(GroupMemberLedger::new(
        Arc::clone(&peering),
        tokio::runtime::Handle::current(),
    ))));
    let redirect = start_local_redirect(&peering).await;
    owner.spawn(serve(
        listener,
        Arc::clone(&peering),
        owner.part("peer link"),
    ));
    if let Some(redirect) = redirect {
        owner.spawn(serve_local_redirect(
            Arc::clone(&peering),
            redirect,
            owner.part("group redirect"),
        ));
    }
    owner.spawn(sync_control_state(
        Arc::clone(&peering),
        owner.part("control state sync"),
    ));
    GroupNode {
        _env: env,
        home,
        owner,
        ledger,
        work,
        settings,
        response_cache,
        events,
        peering,
        kept,
        store,
        authority: None,
    }
}

fn ts(at: SystemTime) -> Timestamp {
    Timestamp::try_from(at).unwrap()
}

fn make_limit(scope_kind: &str, scope_id: &str, amount: u64, period: Option<Period>) -> Limit {
    Limit {
        scope: LimitScope {
            kind: scope_kind.to_owned(),
            id: scope_id.to_owned(),
        },
        usage_kind: REQUESTS.to_owned(),
        currency: None,
        period,
        amount,
        must_understand: true,
        set_by: LimitSetter::Person,
    }
}

fn holder(run_id: &str, folder: &std::path::Path) -> Holder {
    Holder {
        run_id: run_id.to_owned(),
        epoch: 1,
        folder: folder.to_path_buf(),
    }
}

fn placement(scopes: Vec<LimitScope>) -> Placement {
    Placement {
        product: Product::Collie,
        scopes,
    }
}

fn scope(kind: &str, id: &str) -> LimitScope {
    LimitScope {
        kind: kind.to_owned(),
        id: id.to_owned(),
    }
}

fn cache_request(path: &str) -> CacheRequest {
    CacheRequest::new(
        "GET",
        &format!("https://cache.test{path}").parse().unwrap(),
        &[],
        Bytes::new(),
    )
}

fn fresh_head() -> ResponseHead {
    ResponseHead {
        status: 200,
        headers: vec![
            ("content-type".to_owned(), "text/html".to_owned()),
            ("cache-control".to_owned(), "max-age=600".to_owned()),
        ],
    }
}

async fn wait_for_cached_limits(node: &GroupNode, expected: usize) {
    for _ in 0..50 {
        if let Some(limits) = node.peering.cached_usage_limits().await
            && limits.len() == expected
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("worker did not receive {expected} cached usage_limits");
}

/// Group members (`lead`, `worker-a`, `worker-b`) and an Attached engine
/// redirected through `worker-a`'s *group redirect* row share group-wide
/// Limits across `install`, `daemon`, `person`, `token`, and `connection`
/// scopes; in-memory held-Slice charges perform 0 peer calls and 0 SQL
/// statements (`E3`), while taking a Slice costs 1 SQLite transaction on the
/// Lead Daemon.
///
/// Covers: story:3-112, story:3-115
#[test]
fn group_wide_limits_and_redirect_enforce_with_zero_in_memory_round_trips() {
    let runtime = real_time_runtime();
    runtime.block_on(async {
        let clock = clock_at(now());
        let mut lead = start_lead_node(clock).await;
        let worker_a = start_worker_node(&mut lead, "worker-a", clock).await;
        let worker_b = start_worker_node(&mut lead, "worker-b", clock).await;

        let id = identity();
        let lead_major = lead.home.per_major(&id.product_version);
        std::fs::create_dir_all(&lead_major).unwrap();
        std::fs::write(
            lead_major.join("operator-policy.toml"),
            "version = \"0.1\"\n\n[settings.politeness]\nsharing = \"per_daemon\"\n\n[settings.timeout]\nnavigation = 42000\n",
        )
        .unwrap();
        lead.settings.notify_written();

        for _ in 0..50 {
            let snap = worker_a.settings.snapshot().await;
            if let Ok(resolved) = snap.resolved()
                && resolved.get::<Duration>(&TIMEOUT_NAVIGATION) == Duration::from_secs(42)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Attached engine beside worker-a sees worker-a's group redirect row
        // and fetches cached policy + Politeness configuration through it.
        let attached_ledger = Ledger::at(worker_a.home.usage_ledger(), None);
        let redirect_row = attached_ledger
            .external_group_redirect()
            .unwrap()
            .expect("worker-a wrote its group redirect row");
        let redirect_client = RedirectClient::new(redirect_row);
        let redirect_cfg = worker_a
            .owner
            .blocking(move || redirect_client.fetch_config().unwrap())
            .await;
        assert_eq!(redirect_cfg.member_name, "worker-a");
        assert_eq!(
            redirect_cfg.politeness_sharing.as_deref(),
            Some("per_daemon")
        );
        assert!(
            redirect_cfg
                .operator_policy
                .as_deref()
                .is_some_and(|text| text.contains("navigation = 42000"))
        );

        let month = Some(Period {
            every: PeriodLength::Month,
            time_zone: "UTC".to_owned(),
        });
        // install limit = 500 -> SliceSizing::default() gives slice size 5
        lead.ledger
            .add_limit(&make_limit(INSTALL, "", 500, None), ts(now()))
            .unwrap();
        // person monthly limit = 6 -> slice size 1
        lead.ledger
            .add_limit(&make_limit(PERSON, "alice", 6, month), ts(now()))
            .unwrap();
        lead.ledger
            .add_limit(&make_limit(DAEMON, "0.4", 50, None), ts(now()))
            .unwrap();
        lead.ledger
            .add_limit(&make_limit(TOKEN, "ci", 50, None), ts(now()))
            .unwrap();
        lead.ledger
            .add_limit(&make_limit(CONNECTION, "conn-1", 50, None), ts(now()))
            .unwrap();

        let mut install_run = Charging::new(
            worker_a.ledger.clone(),
            holder("run-install", worker_a.home.root()),
            placement(vec![scope(INSTALL, "")]),
            SliceSizing::default(),
        );
        let t0 = ts(now());
        let lead_work = lead.work.clone();
        let lead_ledger = lead.ledger.clone();
        let first_work = worker_a.work.clone();
        let first_ledger = worker_a.ledger.clone();
        let first_root = worker_a.home.root().to_path_buf();
        let second_ledger = worker_b.ledger.clone();
        let second_root = worker_b.home.root().to_path_buf();
        worker_a
            .owner
            .blocking(move || {
                install_run.may_start(t0).unwrap();
                let before_tx = lead_work.read(Row::ChargeTransactions);
                let before_worker_stmts = first_work.read(Row::ChargeStatements);
                // Charge #1 takes a Slice of 5 from Lead Daemon (1 SQLite tx on Lead).
                install_run.charge(REQUESTS, 1, t0).unwrap();
                assert_eq!(
                    lead_work.read(Row::ChargeTransactions) - before_tx,
                    1,
                    "taking one Slice costs exactly one transaction on the Lead Daemon"
                );
                // Charges #2..=#5 spend the held Slice in memory: 0 statements, 0 Lead txs.
                let mid_tx = lead_work.read(Row::ChargeTransactions);
                let mid_stmts = lead_work.read(Row::ChargeStatements);
                let mid_worker_stmts = first_work.read(Row::ChargeStatements);
                for _ in 0..4 {
                    install_run.charge(REQUESTS, 1, t0).unwrap();
                }
                assert_eq!(lead_work.read(Row::ChargeTransactions), mid_tx);
                assert_eq!(lead_work.read(Row::ChargeStatements), mid_stmts);
                assert_eq!(first_work.read(Row::ChargeStatements), mid_worker_stmts);
                assert_eq!(mid_worker_stmts, before_worker_stmts);
                install_run.give_back().unwrap();

                // Enforce monthly person Limit (amount = 6) across worker-a, worker-b,
                // and the Attached engine beside worker-a.
                let scopes = vec![
                    scope(INSTALL, ""),
                    scope(PERSON, "alice"),
                    scope(DAEMON, "0.4"),
                    scope(TOKEN, "ci"),
                    scope(CONNECTION, "conn-1"),
                ];
                let mut run_a = Charging::new(
                    first_ledger,
                    holder("run-a", &first_root),
                    placement(scopes.clone()),
                    SliceSizing::default(),
                );
                let mut run_b = Charging::new(
                    second_ledger,
                    holder("run-b", &second_root),
                    placement(scopes.clone()),
                    SliceSizing::default(),
                );
                let mut run_attached = Charging::new(
                    attached_ledger,
                    holder("run-attached", &first_root),
                    placement(scopes),
                    SliceSizing::default(),
                );
                run_a.may_start(t0).unwrap();
                run_b.may_start(t0).unwrap();
                run_attached.may_start(t0).unwrap();

                for _ in 0..2 {
                    run_a.charge(REQUESTS, 1, t0).unwrap();
                    run_b.charge(REQUESTS, 1, t0).unwrap();
                    run_attached.charge(REQUESTS, 1, t0).unwrap();
                }

                for runner in [&mut run_a, &mut run_b, &mut run_attached] {
                    match runner.charge(REQUESTS, 1, t0) {
                        Err(Refusal::Reached(reached)) => {
                            assert_eq!(reached.scope.kind, PERSON);
                            assert_eq!(reached.scope.id, "alice");
                            assert_eq!(
                                reached.period.as_ref().map(|p| p.every),
                                Some(PeriodLength::Month)
                            );
                        }
                        other => panic!("expected Reached(person/month), got {other:?}"),
                    }
                }
                run_a.give_back().unwrap();
                run_b.give_back().unwrap();
                run_attached.give_back().unwrap();

                let usage = lead_ledger.usage(Product::Collie, t0).unwrap();
                let alice = usage
                    .iter()
                    .find(|entry| {
                        entry.limit.scope.kind == PERSON && entry.limit.scope.id == "alice"
                    })
                    .expect("alice limit present");
                assert_eq!((alice.charged, alice.held, alice.left), (6, 0, 0));
            })
            .await;

        worker_a.owner.stop().await;
        worker_b.owner.stop().await;
        lead.owner.stop().await;
    });
}

/// When the Lead Daemon becomes unreachable:
/// - live runs spend their held Slices in memory and stop with `lead_unreachable`
///   when needing a new Slice;
/// - new runs covered by a Limit fail with `lead_unreachable`, while runs with
///   no covering Limit still start;
/// - cached named-token checks stay valid for at most 10 minutes
///   (`TOKEN_CHECK_CACHE_MAX_TTL`), then fail with `lead_unreachable`, and
///   unknown tokens fail immediately with `lead_unreachable`;
/// - the last pushed Operator policy still applies;
/// - `group.lead_reachability_changed` is published on transitions.
///
/// Covers: story:3-112, story:3-115
#[test]
fn unreachable_lead_spends_held_slices_caches_tokens_and_emits_event() {
    let runtime = real_time_runtime();
    runtime.block_on(async {
        let clock = clock_at(now());
        let mut lead = start_lead_node(clock).await;
        let worker = start_worker_node(&mut lead, "worker-a", clock).await;
        let id = identity();

        let accounts_path = lead
            .home
            .per_major(&id.product_version)
            .join(ACCOUNTS_STORE);
        let token_secret = "gf_test_token_value_for_258";
        let token_hash = sha256_hex(token_secret);
        let unknown_hash = sha256_hex("gf_unknown_token_value_258");
        AccountsStore::open(&accounts_path, None)
            .unwrap()
            .add_named_token(
                "ci-bot",
                &SecretString::from(token_secret.to_owned()),
                ts(now()),
            )
            .unwrap();

        let lead_major = lead.home.per_major(&id.product_version);
        std::fs::create_dir_all(&lead_major).unwrap();
        std::fs::write(
            lead_major.join("operator-policy.toml"),
            "version = \"0.1\"\n\n[settings.timeout]\nnavigation = 33000\n",
        )
        .unwrap();
        lead.settings.notify_written();
        lead.ledger
            .add_limit(&make_limit(PERSON, "alice", 200, None), ts(now()))
            .unwrap();

        wait_for_cached_limits(&worker, 1).await;
        for _ in 0..50 {
            let snap = worker.settings.snapshot().await;
            if let Ok(resolved) = snap.resolved()
                && resolved.get::<Duration>(&TIMEOUT_NAVIGATION) == Duration::from_secs(33)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Verify named token while Lead Daemon is reachable (populates TokenCheckCache).
        assert_eq!(
            worker.peering.check_named_token(&token_hash).await.unwrap(),
            Some("ci-bot".to_owned())
        );

        let mut sub = worker
            .events
            .subscribe(None, false)
            .await
            .expect("event subscription opens");

        // Start a live run on worker and take 1 Slice (size 2), spending 1 and keeping 1 held.
        let t0 = ts(clock.wall());
        let worker_ledger = worker.ledger.clone();
        let worker_root = worker.home.root().to_path_buf();
        let mut live_run = worker
            .owner
            .blocking(move || {
                let mut charging = Charging::new(
                    worker_ledger,
                    holder("live-run", &worker_root),
                    placement(vec![scope(PERSON, "alice")]),
                    SliceSizing::default(),
                );
                charging.may_start(t0).unwrap();
                charging.charge(REQUESTS, 1, t0).unwrap();
                charging
            })
            .await;

        // Take the Lead Daemon completely offline.
        lead.owner.stop().await;
        drop(lead);

        // 1. Live run spends its remaining held Slice (1 left) with Lead unreachable,
        //    then fails closed with LeadUnreachable when needing a new Slice.
        let worker_ledger = worker.ledger.clone();
        let worker_root = worker.home.root().to_path_buf();
        worker
            .owner
            .blocking(move || {
                live_run.charge(REQUESTS, 1, t0).unwrap();
                let err = live_run
                    .charge(REQUESTS, 1, t0)
                    .expect_err("needing a new Slice with Lead unreachable must fail");
                assert!(
                    matches!(err, Refusal::LeadUnreachable(_)),
                    "expected LeadUnreachable, got {err:?}"
                );

                // 2. New run covered by alice's Limit fails may_start with LeadUnreachable.
                let mut covered = Charging::new(
                    worker_ledger.clone(),
                    holder("covered-new", &worker_root),
                    placement(vec![scope(PERSON, "alice")]),
                    SliceSizing::default(),
                );
                assert!(matches!(
                    covered.may_start(t0),
                    Err(Refusal::LeadUnreachable(_))
                ));

                // 3. New run with NO covering Limit still starts while Lead is unreachable.
                let mut uncovered = Charging::new(
                    worker_ledger,
                    holder("uncovered-new", &worker_root),
                    placement(vec![scope(PERSON, "bob")]),
                    SliceSizing::default(),
                );
                uncovered.may_start(t0).unwrap();
            })
            .await;

        // 4. Event `group.lead_reachability_changed` reported `reachable: false`.
        let batch = tokio::time::timeout(Duration::from_secs(5), sub.next(8))
            .await
            .expect("reachability event arrives")
            .expect("stream open");
        let reachability_event = batch
            .iter()
            .find(|env| env.event == GROUP_LEAD_REACHABILITY_CHANGED.name)
            .expect("group.lead_reachability_changed event published");
        let payload: GroupLeadReachabilityChanged =
            serde_json::from_value(serde_json::to_value(&reachability_event.data).unwrap())
                .unwrap();
        assert!(!payload.reachable);
        assert_eq!(payload.lead_name, "lead");

        // 5. Last pushed policy still applies while Lead is unreachable.
        let snap = worker.settings.snapshot().await;
        assert_eq!(
            snap.resolved()
                .unwrap()
                .get::<Duration>(&TIMEOUT_NAVIGATION),
            Duration::from_secs(33)
        );

        // 6. Cached token check serves cached token while Lead is unreachable and
        //    refuses unknown tokens with `lead_unreachable`.
        assert_eq!(
            worker.peering.check_named_token(&token_hash).await.unwrap(),
            Some("ci-bot".to_owned())
        );
        let unknown_err = worker
            .peering
            .check_named_token(&unknown_hash)
            .await
            .expect_err("unknown token fails when Lead is unreachable");
        assert_eq!(unknown_err.code(), LEAD_UNREACHABLE.name);

        // Verify exact 10-minute TTL boundary on TokenCheckCache.
        let cache = TokenCheckCache::default();
        cache.record(
            token_hash.clone(),
            true,
            Some("ci-bot".to_owned()),
            None,
            now(),
        );
        assert_eq!(
            cache
                .lookup_while_unreachable(
                    &token_hash,
                    now() + TOKEN_CHECK_CACHE_MAX_TTL - Duration::from_secs(1)
                )
                .unwrap(),
            Some("ci-bot".to_owned())
        );
        let expired = cache
            .lookup_while_unreachable(
                &token_hash,
                now() + TOKEN_CHECK_CACHE_MAX_TTL + Duration::from_secs(1),
            )
            .expect_err("cached token check must expire past 10 minutes");
        assert_eq!(expired.code(), LEAD_UNREACHABLE.name);

        worker.owner.stop().await;
    });
}

/// Erasures (a discarded Run and a deleted Saved identity) are broadcast by
/// the Lead Daemon to every Group member's local Response cache, collecting
/// acknowledgements and recording offline members as pending.
///
/// Covers: story:3-112
#[test]
fn erasure_broadcast_clears_caches_across_group_and_tracks_offline_members() {
    let runtime = real_time_runtime();
    runtime.block_on(async {
        let clock = clock_at(now());
        let mut lead = start_lead_node(clock).await;
        let worker_a = start_worker_node(&mut lead, "worker-a", clock).await;
        let worker_b = start_worker_node(&mut lead, "worker-b", clock).await;
        let id = identity();

        let run_id = RunId::parse("run-erased-258").unwrap();
        let req_run = cache_request("/products/run-erased");
        let req_ident = cache_request("/account/profile");
        let alice = Partition {
            person: "alice".to_owned(),
            ..Partition::default()
        };

        for node in [&lead, &worker_a, &worker_b] {
            assert!(
                node.response_cache
                    .store(
                        &alice,
                        &req_run,
                        &fresh_head(),
                        Bytes::from_static(b"run-body"),
                        Some(&run_id),
                        now(),
                    )
                    .await
            );
            assert!(
                node.response_cache
                    .store(
                        &alice,
                        &req_ident,
                        &fresh_head(),
                        Bytes::from_static(b"ident-body"),
                        None,
                        now(),
                    )
                    .await
            );
            // Tag the second cached response with the Saved identity partition name.
            let index_path = node
                .home
                .per_major(&id.product_version)
                .join("response-cache")
                .join("index.sqlite");
            let conn = Connection::open(&index_path).unwrap();
            conn.execute(
                "UPDATE entries SET partition = ?1 WHERE url LIKE '%/account/profile'",
                ["saved-identity-alice"],
            )
            .unwrap();
            drop(conn);

            assert!(matches!(
                node.response_cache.lookup(&alice, &req_run, now()).await,
                Lookup::Fresh(_)
            ));
            let stats = node.response_cache.stats("alice").await.unwrap();
            assert!(
                stats
                    .partitions
                    .iter()
                    .any(|p| p.partition == "saved-identity-alice" && p.entries == 1),
                "saved identity partition missing before erasure: {stats:?}"
            );
        }

        // Broadcast discarded Run from worker-a -> forwarded through Lead Daemon to all members.
        let run_result = worker_a
            .peering
            .broadcast_erasure(ErasureTarget::Run {
                run_id: run_id.clone(),
            })
            .await
            .unwrap();
        assert!(run_result.pending.is_empty(), "{run_result:?}");
        for expected in ["lead", "worker-a", "worker-b"] {
            assert!(
                run_result.acknowledged.iter().any(|m| m == expected),
                "missing ack from {expected}: {run_result:?}"
            );
        }
        for node in [&lead, &worker_a, &worker_b] {
            assert!(matches!(
                node.response_cache.lookup(&alice, &req_run, now()).await,
                Lookup::Miss
            ));
        }

        // Take worker-b offline, then broadcast Saved identity deletion from Lead Daemon.
        worker_b.owner.stop().await;
        let ident_result = lead
            .peering
            .broadcast_erasure(ErasureTarget::SavedIdentity {
                name: "saved-identity-alice".to_owned(),
            })
            .await
            .unwrap();
        assert!(ident_result.acknowledged.iter().any(|m| m == "lead"));
        assert!(ident_result.acknowledged.iter().any(|m| m == "worker-a"));
        assert_eq!(ident_result.pending, vec!["worker-b".to_owned()]);
        for node in [&lead, &worker_a] {
            let stats = node.response_cache.stats("alice").await.unwrap();
            assert!(
                !stats
                    .partitions
                    .iter()
                    .any(|p| p.partition == "saved-identity-alice"),
                "saved identity partition remained after erasure: {stats:?}"
            );
        }

        worker_a.owner.stop().await;
        lead.owner.stop().await;
    });
}
