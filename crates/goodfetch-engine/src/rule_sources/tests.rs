//! A Clean-up rules source as an Operator-configured endpoint (ADRs 0017
//! and 0047; spec #564's two egress classes): `egress.no_private_endpoints`
//! refuses a source that resolves to a private address before any
//! connection, and without it the source is reached; its certificate is
//! trusted as a site's, under the authorities listed for its own domain.
//!
//! Covers: story:4-16, setting:egress.no_private_endpoints
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::egress::EgressRule;
use goodfetch_protocol::settings::SettingsLayer;

use super::{SourceFailure, Sourcing, get};
use crate::clock::Clock;
use crate::dns::HostMapping;
use crate::egress::{EndpointPolicy, Refused};
use crate::settings::{Layers, Values, resolve};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A source's URL whose name the `resolve` mapping sends to loopback, on a
/// port nothing listens on.
const SOURCE: &str = "https://rules.test:9/filters.txt";

/// What a fetch may do with `endpoints`, `rules.test` mapped to loopback.
fn sourcing(endpoints: EndpointPolicy) -> Sourcing {
    let mut sourcing = Sourcing {
        endpoints,
        ..Sourcing::new(Clock::system())
    };
    sourcing.fetching.resolve =
        HostMapping::default().with("rules.test", vec!["127.0.0.1".parse().unwrap()]);
    sourcing
}

/// What a GET of the source came to.
fn fetched(endpoints: EndpointPolicy) -> SourceFailure {
    goodfetch_test_support::runtime()
        .block_on(get(SOURCE, &[], &sourcing(endpoints), 1024))
        .unwrap_err()
}

#[test]
fn a_source_at_a_private_address_is_refused_under_no_private_endpoints() {
    let refused = fetched(EndpointPolicy::new(true));
    // The positive control: without the setting the source is reached for,
    // and only the closed port fails it.
    let reached = fetched(EndpointPolicy::default());

    let SourceFailure::Refused(refused) = refused else {
        panic!("the private source is refused: {refused:?}");
    };
    assert_eq!(
        refused,
        Refused {
            url: SOURCE.to_owned(),
            rule: EgressRule::PrivateAddress,
            address: Some("127.0.0.1".parse().unwrap()),
            setting: Some("egress.no_private_endpoints"),
        }
    );
    assert!(
        matches!(&reached, SourceFailure::Network { host } if host == "rules.test"),
        "{reached:?}"
    );
}

/// A source trusts only the authorities `tls.trusted_cas` lists for its own
/// domain beside Mozilla's roots, never another domain's (ADR 0042): its
/// trust is a site's, read from the same settings.
/// Covers: setting:tls.trusted_cas
#[test]
fn a_source_trusts_only_the_authorities_listed_for_its_own_domain() {
    let env = goodfetch_test_support::TestEnv::new();
    let file = env.folder().join("other-authority.der");
    let key = rcgen::KeyPair::generate().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    #[expect(
        clippy::disallowed_methods,
        reason = "the test writes its authority's file before any engine runs"
    )]
    std::fs::write(&file, params.self_signed(&key).unwrap().der()).unwrap();
    let serde_json::Value::Object(user) =
        serde_json::json!({ "tls": { "trusted_cas": { "other.test": [file] } } })
    else {
        unreachable!("a table");
    };
    let layers = Layers {
        user_configuration: Values::read(SettingsLayer::UserConfiguration, &user, Product::Collie)
            .unwrap(),
        ..Layers::default()
    };
    let settings = resolve(&layers, Product::Collie, Product::Collie).unwrap();

    let sourcing = goodfetch_test_support::runtime().block_on(Sourcing::of(
        &settings,
        Clock::system(),
        &TaskOwner::root(BlockingPolicy::Inline),
    ));

    let trust = &sourcing.trust;
    assert_eq!(
        (
            trust.scope_of("rules.test"),
            trust.scopes()[0].authorities.len(),
            trust.scope_of("www.other.test"),
        ),
        (None, 0, Some("other.test"))
    );
}
