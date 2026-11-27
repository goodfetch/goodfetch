//! Certificate renewal through the Group member's interface, a Lead Daemon
//! serving over loopback on real time: a Group member due to renew keeps
//! the certificate the Lead Daemon signs for a new key, presents it, and
//! opens the next peer link with it; the Lead Daemon renews its own; one
//! that asks under another name, or is no Group member, is refused; a
//! renewal that fails is tried again after a tenth of the time left; an
//! expired certificate is not renewed.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::group::LeadEpoch;
use goodfetch_test_support::{TestEnv, real_time_runtime};
use rstest::rstest;
use rustls::pki_types::CertificateDer;
use secrecy::ExposeSecret;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::super::{Peering, serve};
use crate::clock::Clock;
use crate::group::identity::KeptIdentity;
use crate::group::store::{self, Joining, STORE};
use crate::group::{
    CERTIFICATE_LIFETIME, GroupAuthority, GroupMemberKey, GroupMemberName, JoinToken, connecting,
};
use crate::product::{Channel, Identity};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::listen_peer;
use crate::user_store::UserStore;

/// How far apart two readings of the same moment may be: the time a test
/// takes.
const SLACK: Duration = Duration::from_mins(1);

fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_hours(500_000)
}

/// The test Clock, reading `at` now and moving on real time; made on the
/// runtime it runs on.
#[expect(
    clippy::disallowed_methods,
    reason = "a test drives the engine's time through the test Clock (ADR 0059)"
)]
fn clock_at(at: SystemTime) -> Clock {
    Clock::paused(at)
}

fn identity() -> Identity {
    Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.4.0").unwrap(),
        channel: Channel::Homebrew,
    }
}

fn opened(env: &TestEnv) -> UserStore {
    UserStore::open(
        env.home().join("0.1").join(STORE.file()),
        &STORE,
        None,
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    )
}

fn name(text: &str) -> GroupMemberName {
    GroupMemberName::new(text).unwrap()
}

/// A Lead Daemon serving its peer listener on loopback.
struct Lead {
    authority: GroupAuthority,
    kept: KeptIdentity,
    peering: Arc<Peering>,
    store: UserStore,
    _owner: TaskOwner,
    _env: TestEnv,
}

/// The Lead Daemon `lead` of a group formed at `now()`, its certificate
/// due for renewal at `renew_at`, serving at `clock`.
async fn lead(clock: Clock, renew_at: SystemTime) -> Lead {
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
    let peering = Arc::new(Peering::new(identity(), store.clone(), &kept, first, clock).unwrap());
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
    /// Admits the Group member `member` with a join token.
    async fn admit(&self, member: &str) {
        let token = JoinToken::mint(now()).unwrap();
        assert!(
            store::record_token(&self.store, &token, now())
                .await
                .unwrap()
        );
        let joining = Joining {
            name: member.to_owned(),
            peer_address: "127.0.0.1:7001".to_owned(),
        };
        let admitted = store::admit(&self.store, token.token.expose_secret(), joining, now())
            .await
            .unwrap();
        assert_eq!(admitted, store::Admission::Admitted);
    }

    /// The identity of the Group member `member`, whose certificate the
    /// authority signed at `now()` and which renews it at `renew_at`.
    fn member(&self, member: &str, renew_at: SystemTime) -> KeptIdentity {
        let key = GroupMemberKey::generate().unwrap();
        let signed = self
            .authority
            .sign(&name(member), &key.public_key(), now())
            .unwrap();
        KeptIdentity {
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
        }
    }
}

/// A Group member that joined as `kept`, at `clock`.
struct Member {
    peering: Peering,
    store: UserStore,
    _env: TestEnv,
}

async fn joined(kept: &KeptIdentity, clock: Clock) -> Member {
    let env = TestEnv::new();
    let store = opened(&env);
    assert!(
        store::joined(&store, kept.clone(), LeadEpoch::new(1))
            .await
            .unwrap()
    );
    let peering = Peering::new(identity(), store.clone(), kept, LeadEpoch::new(1), clock).unwrap();
    Member {
        peering,
        store,
        _env: env,
    }
}

async fn stored(store: &UserStore) -> KeptIdentity {
    store::identity(store).await.unwrap().unwrap().0
}

/// The certificate `peering`'s listener presents to the Group member
/// `client`, which dials it as `server`.
async fn presented(
    peering: &Peering,
    client: &KeptIdentity,
    server: &GroupMemberName,
) -> CertificateDer<'static> {
    let (near, far) = tokio::io::duplex(64 * 1024);
    let config = connecting(client.presented(), &client.trust(), peering.clock).unwrap();
    let connector = TlsConnector::from(Arc::new(config));
    let acceptor: TlsAcceptor = peering.acceptor();
    let (connected, accepted) = tokio::join!(
        connector.connect(server.server_name(), near),
        acceptor.accept(far)
    );
    accepted.unwrap();
    let connected = connected.unwrap();
    connected.get_ref().1.peer_certificates().unwrap()[0].clone()
}

/// Asserts that `wait` is two thirds of a certificate's life, less the
/// test's time.
fn two_thirds_of_a_life(wait: Duration) {
    let expected = CERTIFICATE_LIFETIME * 2 / 3;
    assert!(
        wait <= expected && wait + SLACK > expected,
        "{wait:?}, not about {expected:?}"
    );
}

#[test]
fn a_group_member_due_keeps_and_presents_the_certificate_the_lead_daemon_signs() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        lead.admit("worker").await;
        let before = lead.member("worker", now());
        let member = joined(&before, clock).await;

        let wait = member.peering.renew_when_due().await.unwrap();

        let after = stored(&member.store).await;
        assert_ne!(after.certificate, before.certificate);
        assert_ne!(after.key.secret_pkcs8_der(), before.key.secret_pkcs8_der());
        assert!(after.certificate_expires_at > before.certificate_expires_at);
        two_thirds_of_a_life(wait);
        assert_eq!(
            presented(&member.peering, &lead.kept, &name("worker")).await,
            after.certificate
        );
        member
            .peering
            .renewed(after, clock.wall())
            .await
            .expect("the renewed certificate opens the next peer link");
    });
}

#[test]
fn the_lead_daemon_renews_its_own_certificate() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now()).await;

        let wait = lead.peering.renew_when_due().await.unwrap();

        let after = stored(&lead.store).await;
        assert_ne!(after.certificate, lead.kept.certificate);
        two_thirds_of_a_life(wait);
        assert_eq!(
            presented(&lead.peering, &lead.member("worker", now()), &name("lead")).await,
            after.certificate
        );
    });
}

#[rstest]
#[case::under_another_members_name("worker", true, "other", "invalid_params")]
#[case::that_is_no_group_member("stranger", false, "stranger", "invalid_request")]
fn the_lead_daemon_refuses_a_renewal(
    #[case] certified: &str,
    #[case] admitted: bool,
    #[case] asks_as: &str,
    #[case] code: &str,
) {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        if admitted {
            lead.admit(certified).await;
        }
        let certificate = lead.member(certified, now());
        let member = joined(&certificate, clock).await;
        let asking = KeptIdentity {
            name: asks_as.to_owned(),
            ..certificate
        };

        let refused = member.peering.ask_lead(&asking).await.unwrap_err();

        assert_eq!(refused.code(), code, "{refused}");
    });
}

#[test]
fn a_renewal_that_fails_is_tried_again_after_a_tenth_of_the_time_left() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let closed = listen_peer("127.0.0.1:0").await.unwrap();
        let unreachable = closed.local_address().unwrap().to_string();
        drop(closed);
        let before = KeptIdentity {
            lead_address: unreachable,
            ..lead.member("worker", now())
        };
        let member = joined(&before, clock).await;

        let retry = member.peering.renew_when_due().await.unwrap();

        let expected = CERTIFICATE_LIFETIME / 10;
        assert!(
            retry <= expected && retry + SLACK > expected,
            "{retry:?}, not about {expected:?}"
        );
        assert_eq!(stored(&member.store).await.certificate, before.certificate);
    });
}

#[test]
fn an_expired_certificate_is_not_renewed() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now() + CERTIFICATE_LIFETIME + Duration::from_secs(1));
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        lead.admit("worker").await;
        let before = lead.member("worker", now());
        let member = joined(&before, clock).await;

        assert_eq!(member.peering.renew_when_due().await, None);
        assert_eq!(stored(&member.store).await.certificate, before.certificate);
    });
}
