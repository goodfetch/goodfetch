//! A Secret placeholder's binding as the settings give it (ADR 0092): a
//! placeholder no scope binds raises `secret_unbound`, listing the
//! placeholders bound and never a value, even once the bound ones'
//! values are resolved and held for the Run.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::secrets::placeholders::{BoundPlaceholder, PlaceholderSource};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use serde_json::{Value, json};

use super::{driver, run_under};
use crate::secrets::{Purpose, binding};
use crate::settings::{Layers, Resolved, Values, resolve};

/// The Run's settings with `bound` as its `secrets.placeholders`.
fn binding_in_the_run(bound: &Value) -> Resolved {
    let Value::Object(set) = json!({ "secrets": { "placeholders": bound } }) else {
        unreachable!("a table")
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &set, Product::Collie).unwrap(),
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

/// Story 4-51 and A04-38: placeholders bound to a typed password's and a
/// TOTP seed's references, and one to a code source, are each looked up by
/// name, and the bound references' values resolved for github.com; a
/// placeholder no scope binds then raises `secret_unbound`, which names it
/// and the three bound and holds no value, nor does any file the Run wrote.
/// Covers: story:4-51, error:secret_unbound, setting:secrets.placeholders
#[test]
fn a_placeholder_no_scope_binds_is_unbound_naming_the_bound_never_a_value() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let password = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let seed = canaries.plant(SecretKind::TotpSeed).value().to_owned();
    let settings = binding_in_the_run(&json!({
        "github": { "reference": "env:GITHUB_PASSWORD", "domains": ["github.com"] },
        "github_otp": { "reference": "env:GITHUB_TOTP", "domains": ["github.com"], "totp": true },
        "inbox": { "code_source": "inbox", "domains": ["github.com"] },
    }));
    let secrets = goodfetch_test_support::runtime().block_on(async {
        run_under(
            &driver(
                &env,
                &[("GITHUB_PASSWORD", &password), ("GITHUB_TOTP", &seed)],
            ),
            &settings,
        )
    });
    for placeholder in ["github", "github_otp"] {
        let bound = binding(&settings, placeholder).unwrap();
        let PlaceholderSource::Reference { reference, .. } = &bound.source else {
            panic!("{placeholder} is bound to a reference");
        };
        goodfetch_test_support::runtime()
            .block_on(secrets.resolve(
                reference,
                &Purpose {
                    placeholder,
                    domain: Some("github.com"),
                    domains: &bound.domains,
                    layer: Some(SettingsLayer::Run),
                    run_id: None,
                },
            ))
            .unwrap();
    }

    let unbound = crate::run::secret_error(&binding(&settings, "gitlab").unwrap_err());

    let object = serde_json::to_vec(&unbound.to_error_object()).unwrap();
    assert_eq!(
        (
            unbound.code(),
            unbound.details().cloned().map(Value::Object),
            binding(&settings, "inbox").ok(),
            canaries
                .leaks(&[
                    Output::Read {
                        name: "secret_unbound",
                        bytes: &object,
                    },
                    Output::Path(env.home()),
                ])
                .unwrap(),
        ),
        (
            "secret_unbound",
            Some(json!({ "placeholder": "gitlab", "bound": ["github", "github_otp", "inbox"] })),
            Some(BoundPlaceholder {
                source: PlaceholderSource::CodeSource("inbox".to_owned()),
                domains: vec!["github.com".to_owned()],
            }),
            Vec::new(),
        )
    );
}
