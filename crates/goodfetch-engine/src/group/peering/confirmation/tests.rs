//! Lead-epoch durability and majority confirmation over loopback on real time
//! (ADR 0055): a Group member durably records a raised lead epoch across
//! restarts and refuses lower ones; a restarting 3-member Lead Daemon stays
//! unconfirmed until it exchanges lead epochs with one other member (2 of 3)
//! or an operator confirms it; and a peer holding a higher lead epoch fences
//! the Lead Daemon off and durably raises its epoch.

use std::sync::Arc;

use goodfetch_protocol::group::{
    HELLO, LEAD_EPOCH, LeadEpoch, LeadEpochParams, LeadEpochResult, PEER_PROTOCOL_VERSION,
    PeerHelloParams, PeerHelloResult,
};
use goodfetch_test_support::real_time_runtime;

use super::super::loopback::{clock_at, closed_address, identity, lead, now};
use super::super::{PeerServices, Peering, ask_peer, serve};
use crate::group::CERTIFICATE_LIFETIME;
use crate::group::identity::KeptIdentity;
use crate::group::store;
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::listen_peer;

/// Starts a Group member listening on its own loopback port, serving `peering`.
async fn serving_member(
    lead: &super::super::loopback::Lead,
    name: &str,
    initial_epoch: LeadEpoch,
    clock: crate::clock::Clock,
) -> (
    KeptIdentity,
    Arc<Peering>,
    crate::user_store::UserStore,
    TaskOwner,
    goodfetch_test_support::TestEnv,
) {
    let listener = listen_peer("127.0.0.1:0").await.unwrap();
    let address = listener.local_address().unwrap().to_string();
    let (mut kept, certified) = lead.member(name, now() + CERTIFICATE_LIFETIME);
    kept.peer_address = address.clone();
    lead.admit_at(name, &address, certified).await;

    let env = goodfetch_test_support::TestEnv::new();
    let store = super::super::loopback::opened_store(&env);
    assert!(
        store::joined(&store, kept.clone(), initial_epoch)
            .await
            .unwrap()
    );
    let peering = Arc::new(
        Peering::new(
            identity(),
            store.clone(),
            &kept,
            initial_epoch,
            Vec::new(),
            clock,
            PeerServices::default(),
        )
        .unwrap(),
    );
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    owner.spawn(serve(
        listener,
        Arc::clone(&peering),
        owner.part("peer link"),
    ));
    (kept, peering, store, owner, env)
}

#[test]
fn a_group_member_durably_records_a_higher_lead_epoch_across_restarts() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let (worker, _peering, worker_store, owner, _env) =
            serving_member(&lead, "worker", LeadEpoch::new(1), clock).await;

        let raised: PeerHelloResult = ask_peer(
            &lead.kept,
            "worker",
            &worker.peer_address,
            clock,
            HELLO,
            &PeerHelloParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: identity().product,
                product_version: identity().product_version,
                lead_epoch: LeadEpoch::new(4),
            },
        )
        .await
        .unwrap();
        assert_eq!(raised.lead_epoch, LeadEpoch::new(4));
        owner.stop().await;

        // Reopen from the same SQLite store as after a restart: epoch 4 stays
        // durable and epoch 3 is refused with `lead_epoch_stale`.
        let (reloaded, reloaded_epoch) = store::identity(&worker_store).await.unwrap().unwrap();
        assert_eq!(reloaded_epoch, LeadEpoch::new(4));
        let listener = listen_peer("127.0.0.1:0").await.unwrap();
        let new_address = listener.local_address().unwrap().to_string();
        let restarted = Arc::new(
            Peering::new(
                identity(),
                worker_store.clone(),
                &reloaded,
                reloaded_epoch,
                Vec::new(),
                clock,
                PeerServices::default(),
            )
            .unwrap(),
        );
        let owner2 = TaskOwner::root(BlockingPolicy::Inline);
        owner2.spawn(serve(listener, Arc::clone(&restarted), owner2.part("peer")));

        let refused = ask_peer::<PeerHelloResult>(
            &lead.kept,
            "worker",
            &new_address,
            clock,
            HELLO,
            &PeerHelloParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: identity().product,
                product_version: identity().product_version,
                lead_epoch: LeadEpoch::new(3),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(refused.code(), "lead_epoch_stale", "{refused}");
        assert_eq!(
            refused
                .details()
                .and_then(|d| d.get("highest_lead_epoch"))
                .and_then(serde_json::Value::as_u64),
            Some(4)
        );
        owner2.stop().await;
    });
}

#[test]
fn a_restarting_three_member_lead_daemon_confirms_by_majority_once_one_peer_answers() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        // One reachable worker and one unreachable worker -> 3 members total.
        let (_w1, _p1, _s1, owner1, _env1) =
            serving_member(&lead, "w1", LeadEpoch::new(1), clock).await;
        let dead = closed_address().await;
        let (_w2, cert2) = lead.member("w2", now() + CERTIFICATE_LIFETIME);
        lead.admit_at("w2", &dead, cert2).await;

        let members: Vec<String> = store::members(&lead.store)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        let restarted = Peering::new(
            identity(),
            lead.store.clone(),
            &lead.kept,
            LeadEpoch::new(1),
            members,
            clock,
            PeerServices::default(),
        )
        .unwrap();

        assert!(restarted.is_unconfirmed());
        assert_eq!(
            restarted.require_confirmed_lead().unwrap_err().code(),
            "lead_unconfirmed"
        );

        restarted.confirm_round().await;

        assert!(!restarted.is_unconfirmed());
        assert_eq!(
            restarted.require_confirmed_lead().unwrap(),
            LeadEpoch::new(1)
        );
        owner1.stop().await;
    });
}

#[test]
fn a_restarting_lead_daemon_is_fenced_when_a_peer_holds_a_higher_epoch() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let (_w1, _p1, _s1, owner1, _env1) =
            serving_member(&lead, "w1", LeadEpoch::new(3), clock).await;
        let dead = closed_address().await;
        let (_w2, cert2) = lead.member("w2", now() + CERTIFICATE_LIFETIME);
        lead.admit_at("w2", &dead, cert2).await;

        let members: Vec<String> = store::members(&lead.store)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        let restarted = Peering::new(
            identity(),
            lead.store.clone(),
            &lead.kept,
            LeadEpoch::new(1),
            members,
            clock,
            PeerServices::default(),
        )
        .unwrap();

        restarted.confirm_round().await;

        assert_eq!(
            restarted.require_confirmed_lead().unwrap_err().code(),
            "lead_epoch_stale"
        );
        assert_eq!(
            restarted.confirm_by_operator().unwrap_err().code(),
            "lead_epoch_stale"
        );
        assert_eq!(
            store::identity(&lead.store).await.unwrap().unwrap().1,
            LeadEpoch::new(3)
        );
        owner1.stop().await;
    });
}

#[test]
fn operator_confirmation_unblocks_an_isolated_lead_daemon() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let dead1 = closed_address().await;
        let dead2 = closed_address().await;
        let (_, cert1) = lead.member("w1", now() + CERTIFICATE_LIFETIME);
        let (_, cert2) = lead.member("w2", now() + CERTIFICATE_LIFETIME);
        lead.admit_at("w1", &dead1, cert1).await;
        lead.admit_at("w2", &dead2, cert2).await;

        let members: Vec<String> = store::members(&lead.store)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.name)
            .collect();
        let restarted = Peering::new(
            identity(),
            lead.store.clone(),
            &lead.kept,
            LeadEpoch::new(1),
            members,
            clock,
            PeerServices::default(),
        )
        .unwrap();

        restarted.confirm_round().await;
        assert_eq!(
            restarted.require_confirmed_lead().unwrap_err().code(),
            "lead_unconfirmed"
        );

        let confirmed = restarted.confirm_by_operator().unwrap();
        assert!(confirmed.confirmed);
        assert!(confirmed.confirmed_by_operator);
        assert_eq!(
            restarted.require_confirmed_lead().unwrap(),
            LeadEpoch::new(1)
        );
    });
}

#[test]
fn a_peer_link_lead_epoch_message_raises_epoch_durably() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let (worker, _peering, worker_store, owner, _env) =
            serving_member(&lead, "worker", LeadEpoch::new(1), clock).await;

        // Open one link, say hello at 1, then send group.lead_epoch at 5.
        let mut stream =
            super::super::connect_peer_tls(&lead.kept, "worker", &worker.peer_address, clock)
                .await
                .unwrap();
        let _: PeerHelloResult = super::super::ask(
            &mut stream,
            &worker.peer_address,
            HELLO,
            &PeerHelloParams {
                protocol_version: PEER_PROTOCOL_VERSION,
                product: identity().product,
                product_version: identity().product_version,
                lead_epoch: LeadEpoch::new(1),
            },
        )
        .await
        .unwrap();
        let raised: LeadEpochResult = super::super::ask(
            &mut stream,
            &worker.peer_address,
            LEAD_EPOCH,
            &LeadEpochParams {
                lead_epoch: LeadEpoch::new(5),
            },
        )
        .await
        .unwrap();
        assert_eq!(raised.lead_epoch, LeadEpoch::new(5));
        assert_eq!(
            store::identity(&worker_store).await.unwrap().unwrap().1,
            LeadEpoch::new(5)
        );
        owner.stop().await;
    });
}
