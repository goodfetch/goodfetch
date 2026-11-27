//! Replicated control-state push, sync, monotonic version enforcement, and
//! per-member `[group_member.<name>]` settings isolation over loopback mutual
//! TLS (ADR 0055, issue 243).
#![expect(
    clippy::disallowed_methods,
    reason = "loopback tests write Lead Daemon TOML files from the test thread"
)]

use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::browser_cache::BROWSER_EXECUTABLE_PATH;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::group::{
    CONTROL_STATE_PUSH, ControlStateSnapshot, LeadEpoch, PEER_PROTOCOL_VERSION,
    PeerControlStatePushParams, PeerControlStatePushResult,
};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_NAVIGATION};
use goodfetch_test_support::{TestEnv, real_time_runtime};
use serde_json::json;

use super::super::loopback::{clock_at, identity, name, now};
use super::super::{PeerServices, Peering, ask_peer, serve};
use super::{replicate_control_state, sync_control_state};
use crate::control_state::{STORE_OPERATOR_POLICY, STORE_USER_CONFIGURATION};
use crate::group::identity::KeptIdentity;
use crate::group::store::{self, Certified, Joining, STORE};
use crate::group::{CERTIFICATE_LIFETIME, GroupAuthority, GroupMemberKey};
use crate::home::Home;
use crate::settings::{Settings, SettingsFiles};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::listen_peer;
use crate::user_store::UserStore;

struct PeerNode {
    _env: TestEnv,
    owner: Arc<TaskOwner>,
    settings: Settings,
    peering: Arc<Peering>,
    kept: KeptIdentity,
    store: UserStore,
    authority: Option<GroupAuthority>,
}

async fn start_lead_node(clock: crate::clock::Clock) -> PeerNode {
    let env = TestEnv::new();
    let id = identity();
    let home = Home::new(env.home().to_path_buf());
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let settings = Settings::new(
        SettingsFiles::of(&home, &id.product_version),
        Product::Collie,
        owner.part("settings"),
    );
    settings.set_group_member(Some("lead".to_owned()));
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
            PeerServices {
                settings: Some(settings.clone()),
                ..PeerServices::default()
            },
        )
        .unwrap(),
    );
    peering.hydrate_control_state().await;
    owner.spawn(serve(
        listener,
        Arc::clone(&peering),
        owner.part("peer link"),
    ));
    owner.spawn(replicate_control_state(
        Arc::clone(&peering),
        owner.part("control state replication"),
    ));
    PeerNode {
        _env: env,
        owner,
        settings,
        peering,
        kept,
        store,
        authority: Some(authority),
    }
}

async fn start_worker_node(
    lead: &PeerNode,
    member_name: &str,
    clock: crate::clock::Clock,
) -> PeerNode {
    let env = TestEnv::new();
    let id = identity();
    let home = Home::new(env.home().to_path_buf());
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let settings = Settings::new(
        SettingsFiles::of(&home, &id.product_version),
        Product::Collie,
        owner.part("settings"),
    );
    settings.set_group_member(Some(member_name.to_owned()));
    let store = UserStore::open(
        home.per_major(&id.product_version).join(STORE.file()),
        &STORE,
        None,
        Arc::clone(&owner),
    );
    let listener = listen_peer("127.0.0.1:0").await.unwrap();
    let address = listener.local_address().unwrap().to_string();

    let authority = lead.authority.as_ref().unwrap();
    let key = GroupMemberKey::generate().unwrap();
    let signed = authority
        .sign(&name(member_name), &key.public_key(), now())
        .unwrap();
    let token = crate::group::JoinToken::mint(now()).unwrap();
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
    let admitted = store::admit(
        &lead.store,
        secrecy::ExposeSecret::expose_secret(&token.token),
        joining,
        now(),
    )
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
    assert!(
        store::joined(&store, kept.clone(), LeadEpoch::new(1))
            .await
            .unwrap()
    );
    let peering = Arc::new(
        Peering::new(
            id,
            store.clone(),
            &kept,
            LeadEpoch::new(1),
            Vec::new(),
            clock,
            PeerServices {
                settings: Some(settings.clone()),
                ..PeerServices::default()
            },
        )
        .unwrap(),
    );
    peering.hydrate_control_state().await;
    owner.spawn(serve(
        listener,
        Arc::clone(&peering),
        owner.part("peer link"),
    ));
    owner.spawn(sync_control_state(
        Arc::clone(&peering),
        owner.part("control state sync"),
    ));
    PeerNode {
        _env: env,
        owner,
        settings,
        peering,
        kept,
        store,
        authority: None,
    }
}

async fn wait_for_store_version(node: &PeerNode, store: &str, min_version: u64) {
    for _ in 0..80 {
        if node.peering.control_state.lock().await.version(store) >= min_version {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "worker {} did not reach version {min_version} for {store}",
        node.kept.name
    );
}

async fn push_stale_and_verify(
    lead: &PeerNode,
    worker: &PeerNode,
    clock: crate::clock::Clock,
    stale_policy: &str,
) {
    let stale_push = PeerControlStatePushParams {
        protocol_version: PEER_PROTOCOL_VERSION,
        product: Product::Collie,
        product_version: identity().product_version,
        lead_epoch: LeadEpoch::new(1),
        snapshots: vec![ControlStateSnapshot {
            store: STORE_OPERATOR_POLICY.to_owned(),
            version: 1,
            content: Some(stale_policy.to_owned()),
        }],
    };
    let reply: PeerControlStatePushResult = ask_peer(
        &lead.kept,
        &worker.kept.name,
        &worker.kept.peer_address,
        clock,
        CONTROL_STATE_PUSH,
        &stale_push,
    )
    .await
    .unwrap();
    assert!(reply.versions[STORE_OPERATOR_POLICY] >= 2);
}

/// Lead Daemon replicates Operator policy and user configuration (including
/// `[group_member.<name>]` scopes) to every Group member, each member applies
/// only its own machine-specific scope, and stale/out-of-order pushes never
/// roll back a newer version.
///
/// Covers: story:3-93, story:3-107
#[test]
fn lead_replicates_policy_and_config_with_group_member_scope_and_ignores_stale_pushes() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = start_lead_node(clock).await;

        let policy_v1 = r#"version = "0.1"

[settings.timeout]
navigation = 45000

[group_member.worker-a.browser]
executable_path = "/opt/browsers/worker-a"

[group_member.worker-b.browser]
executable_path = "/opt/browsers/worker-b"
"#;
        let config_v1 = r#"version = "0.1"

[settings.timeout]
action = 12000
"#;
        std::fs::create_dir_all(lead.settings.files().operator_policy().parent().unwrap()).unwrap();
        std::fs::write(lead.settings.files().operator_policy(), policy_v1).unwrap();
        std::fs::write(lead.settings.files().user_configuration(), config_v1).unwrap();
        lead.settings.notify_written();

        let worker_a = start_worker_node(&lead, "worker-a", clock).await;
        let worker_b = start_worker_node(&lead, "worker-b", clock).await;
        for worker in [&worker_a, &worker_b] {
            wait_for_store_version(worker, STORE_OPERATOR_POLICY, 1).await;
            wait_for_store_version(worker, STORE_USER_CONFIGURATION, 1).await;
        }

        let resolved_a = worker_a
            .settings
            .snapshot()
            .await
            .resolved()
            .unwrap()
            .clone();
        let resolved_b = worker_b
            .settings
            .snapshot()
            .await
            .resolved()
            .unwrap()
            .clone();
        assert_eq!(
            resolved_a.effective(&TIMEOUT_NAVIGATION).unwrap().value,
            json!(45_000)
        );
        assert_eq!(
            resolved_a
                .effective(&BROWSER_EXECUTABLE_PATH)
                .unwrap()
                .value,
            json!("/opt/browsers/worker-a")
        );
        assert_eq!(
            resolved_a
                .effective(&BROWSER_EXECUTABLE_PATH)
                .unwrap()
                .layer,
            SettingsLayer::GroupMember
        );
        assert_eq!(
            resolved_b
                .effective(&BROWSER_EXECUTABLE_PATH)
                .unwrap()
                .value,
            json!("/opt/browsers/worker-b")
        );

        let policy_v2 = r#"version = "0.1"

[settings.timeout]
navigation = 60000

[group_member.worker-a.browser]
executable_path = "/opt/browsers/worker-a-v2"
"#;
        std::fs::write(lead.settings.files().operator_policy(), policy_v2).unwrap();
        lead.settings.notify_written();
        wait_for_store_version(&worker_a, STORE_OPERATOR_POLICY, 2).await;

        push_stale_and_verify(&lead, &worker_a, clock, policy_v1).await;
        let after_stale = worker_a
            .settings
            .snapshot()
            .await
            .resolved()
            .unwrap()
            .clone();
        assert_eq!(
            after_stale.effective(&TIMEOUT_NAVIGATION).unwrap().value,
            json!(60_000)
        );
        assert_eq!(
            after_stale
                .effective(&BROWSER_EXECUTABLE_PATH)
                .unwrap()
                .value,
            json!("/opt/browsers/worker-a-v2")
        );

        worker_a.owner.stop().await;
        worker_b.owner.stop().await;
        lead.owner.stop().await;
    });
}
