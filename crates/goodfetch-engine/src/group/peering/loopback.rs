//! A Lead Daemon serving its peer listener over loopback on real time, and
//! the Group members it signs certificates for: what the tests of renewal
//! and leaving share.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::group::LeadEpoch;
use goodfetch_test_support::TestEnv;
use secrecy::ExposeSecret;

use super::{Peering, serve};
use crate::clock::Clock;
use crate::group::identity::KeptIdentity;
use crate::group::store::{self, Certified, Joining, STORE};
use crate::group::{GroupAuthority, GroupMemberKey, GroupMemberName, JoinToken};
use crate::product::{Channel, Identity};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::listen_peer;
use crate::user_store::UserStore;

pub(super) fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_hours(500_000)
}

/// The test Clock, reading `at` now and moving on real time; made on the
/// runtime it runs on.
#[expect(
    clippy::disallowed_methods,
    reason = "a test drives the engine's time through the test Clock (ADR 0059)"
)]
pub(super) fn clock_at(at: SystemTime) -> Clock {
    Clock::paused(at)
}

pub(super) fn identity() -> Identity {
    Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.4.0").unwrap(),
        channel: Channel::Homebrew,
    }
}

pub(super) fn opened_store(env: &TestEnv) -> UserStore {
    UserStore::open(
        env.home().join("0.1").join(STORE.file()),
        &STORE,
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    )
}

fn opened(env: &TestEnv) -> UserStore {
    opened_store(env)
}

pub(super) fn name(text: &str) -> GroupMemberName {
    GroupMemberName::new(text).unwrap()
}

/// A Lead Daemon serving its peer listener on loopback.
pub(super) struct Lead {
    authority: GroupAuthority,
    pub(super) kept: KeptIdentity,
    pub(super) peering: Arc<Peering>,
    pub(super) store: UserStore,
    _owner: TaskOwner,
    _env: TestEnv,
}

/// The Lead Daemon `lead` of a group formed at `now()`, its certificate
/// due for renewal at `renew_at`, serving at `clock`.
pub(super) async fn lead(clock: Clock, renew_at: SystemTime) -> Lead {
    let env = TestEnv::new();
    let store = opened(&env);
    let listener = listen_peer("127.0.0.1:0").await.unwrap();
    let address = listener.local_address().unwrap().to_string();
    let mut authority = GroupAuthority::create(now()).unwrap();
    let kept = KeptIdentity {
        renew_at,
        ..KeptIdentity::lead(&mut authority, &name("lead"), address, now()).unwrap()
    };
    let first = store::form(&store, authority.kept(), kept.clone(), now())
        .await
        .unwrap()
        .unwrap();
    let peering = Arc::new(
        Peering::new(
            identity(),
            store.clone(),
            &kept,
            first,
            vec!["lead".to_owned()],
            clock,
        )
        .unwrap(),
    );
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    owner.spawn(serve(
        listener,
        Arc::clone(&peering),
        owner.part("peer link"),
    ));
    Lead {
        authority,
        kept,
        peering,
        store,
        _owner: owner,
        _env: env,
    }
}

impl Lead {
    /// Admits the Group member `member` at `peer_address` with a join token,
    /// the authority having signed `certificate` for it.
    pub(super) async fn admit_at(&self, member: &str, peer_address: &str, certificate: Certified) {
        let token = JoinToken::mint(now()).unwrap();
        assert!(
            store::record_token(&self.store, &token, now())
                .await
                .unwrap()
        );
        let joining = Joining {
            name: member.to_owned(),
            peer_address: peer_address.to_owned(),
            certificate,
        };
        let admitted = store::admit(&self.store, token.token.expose_secret(), joining, now())
            .await
            .unwrap();
        assert_eq!(admitted, store::Admission::Admitted);
    }

    /// Admits the Group member `member` with a join token, the authority
    /// having signed `certificate` for it.
    pub(super) async fn admit(&self, member: &str, certificate: Certified) {
        self.admit_at(member, "127.0.0.1:7001", certificate).await;
    }

    /// The identity of the Group member `member`, admitted, which renews
    /// its certificate at `renew_at`.
    pub(super) async fn admitted(&self, member: &str, renew_at: SystemTime) -> KeptIdentity {
        let (kept, certificate) = self.member(member, renew_at);
        self.admit(member, certificate).await;
        kept
    }

    /// The identity of the Group member `member`, whose certificate the
    /// authority signed at `now()` and which renews it at `renew_at`, and
    /// what the Lead Daemon records of that certificate.
    pub(super) fn member(&self, member: &str, renew_at: SystemTime) -> (KeptIdentity, Certified) {
        let key = GroupMemberKey::generate().unwrap();
        let signed = self
            .authority
            .sign(&name(member), &key.public_key(), now())
            .unwrap();
        let certified = Certified::from(&signed);
        let kept = KeptIdentity {
            name: member.to_owned(),
            key: key.kept(),
            certificate: signed.certificate,
            authority: self.authority.certificate().clone(),
            revocations: self.kept.revocations.clone(),
            peer_address: "127.0.0.1:7001".to_owned(),
            lead_address: self.kept.peer_address.clone(),
            lead_name: self.kept.name.clone(),
            certificate_expires_at: signed.not_after,
            renew_at,
        };
        (kept, certified)
    }
}

/// A Group member that joined as `kept`, at `clock`.
pub(super) struct Member {
    pub(super) peering: Peering,
    pub(super) store: UserStore,
    _env: TestEnv,
}

pub(super) async fn joined(kept: &KeptIdentity, clock: Clock) -> Member {
    let env = TestEnv::new();
    let store = opened(&env);
    assert!(
        store::joined(&store, kept.clone(), LeadEpoch::new(1))
            .await
            .unwrap()
    );
    let peering = Peering::new(
        identity(),
        store.clone(),
        kept,
        LeadEpoch::new(1),
        Vec::new(),
        clock,
    )
    .unwrap();
    Member {
        peering,
        store,
        _env: env,
    }
}

pub(super) async fn stored(store: &UserStore) -> KeptIdentity {
    store::identity(store).await.unwrap().unwrap().0
}

/// A loopback `host:port` where nothing listens any more.
pub(super) async fn closed_address() -> String {
    let closed = listen_peer("127.0.0.1:0").await.unwrap();
    closed.local_address().unwrap().to_string()
}
