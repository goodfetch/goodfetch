use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;

use super::{Approvals, Origin, Permission, PluginRefusal, admit, stated};
use crate::plugin::PluginManifest;

/// A Plugin at `direct` that reaches `shop.test`, uses `env:TOKEN`, wants
/// its value and to cause Effects and run page script, with a browser for
/// two platforms.
fn manifest() -> PluginManifest {
    PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": "shop-sink",
            "version": "1.0.0",
            "kinds": ["notification"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "shop-sink",
            "network_level": "direct",
            "permissions": {
                "domains": ["shop.test"],
                "secrets": ["env:TOKEN"],
                "grants": ["secret_value", "effects", "page_script"],
            },
            "browsers": [{
                "name": "shop-browser",
                "platforms": {
                    "linux-x64": { "url": "https://cdn.shop.test/l.zip", "digest": "sha256:aa" },
                    "macos-arm64": { "url": "https://cdn.shop.test/m.zip", "digest": "sha256:bb" },
                },
            }],
        })
        .to_string()
        .as_bytes(),
    )
    .expect("the test manifest reads")
}

fn domain(name: &str) -> String {
    name.to_owned()
}

/// Covers: story:7-18
#[test]
fn each_permission_the_manifest_asks_for_is_listed_on_its_own() {
    let needed = Permission::needed_by(&manifest());
    assert_eq!(
        needed,
        vec![
            Permission::Effects {
                domain: domain("shop.test")
            },
            Permission::PageScript {
                domain: domain("shop.test")
            },
            Permission::Secret {
                reference: "env:TOKEN".to_owned()
            },
            Permission::SecretValue {
                reference: "env:TOKEN".to_owned()
            },
            Permission::Browser {
                name: "shop-browser".to_owned(),
                platform: "linux-x64".to_owned(),
                digest: "sha256:aa".to_owned(),
            },
            Permission::Browser {
                name: "shop-browser".to_owned(),
                platform: "macos-arm64".to_owned(),
                digest: "sha256:bb".to_owned(),
            },
            Permission::Direct,
        ]
    );
}

#[test]
fn a_manifest_that_asks_for_nothing_needs_no_approval() {
    let mut quiet = manifest();
    quiet.permissions = crate::plugin::Permissions::default();
    quiet.browsers.clear();
    quiet.network_level = crate::plugin::NetworkLevel::None;
    assert_eq!(Permission::needed_by(&quiet), Vec::new());
}

/// Covers: story:7-109
#[test]
fn a_secret_reference_states_both_limits() {
    let statement = stated(Permission::Secret {
        reference: "env:TOKEN".to_owned(),
    });
    assert!(
        statement
            .cannot_check
            .contains("a bound service that stores or publishes what it receives"),
        "{}",
        statement.cannot_check
    );
    assert!(
        statement
            .cannot_check
            .contains("an encoding the scrub does not know returns it"),
        "{}",
        statement.cannot_check
    );
}

/// Covers: story:7-18
#[test]
fn every_permission_states_what_it_lets_and_what_the_engine_cannot_check() {
    for permission in Permission::needed_by(&manifest()) {
        let statement = stated(permission.clone());
        assert!(!statement.lets.is_empty(), "{permission:?}");
        assert!(!statement.cannot_check.is_empty(), "{permission:?}");
    }
}

/// Covers: story:7-56
#[test]
fn direct_approval_states_what_the_engine_cannot_check() {
    let statement = stated(Permission::Direct);
    for phrase in [
        "configured destinations",
        "DNS rebinding",
        "loopback",
        "cloud-metadata",
        "Effects rule",
    ] {
        assert!(
            statement.cannot_check.contains(phrase),
            "expected `{phrase}` in: {}",
            statement.cannot_check
        );
    }
}

fn any_permission() -> impl Strategy<Value = Permission> {
    let word = "[a-z]{1,6}";
    prop_oneof![
        word.prop_map(|domain| Permission::Effects { domain }),
        word.prop_map(|domain| Permission::PageScript { domain }),
        word.prop_map(|domain| Permission::SessionCredentials { domain }),
        word.prop_map(|reference| Permission::Secret { reference }),
        word.prop_map(|reference| Permission::SecretValue { reference }),
        (word, word, word).prop_map(|(name, platform, digest)| Permission::Browser {
            name,
            platform,
            digest
        }),
        Just(Permission::Direct),
    ]
}

proptest! {
    /// Approving a set of permissions covers exactly those: each approval
    /// is separate, so approving one never covers another.
    #[test]
    fn approvals_cover_exactly_what_was_approved(
        approved in prop::collection::vec(any_permission(), 0..8),
        asked in any_permission(),
    ) {
        let mut approvals = Approvals::default();
        for permission in &approved {
            approvals.approve(permission);
        }
        let same_approval = |given: &Permission| match (given, &asked) {
            (
                Permission::Browser { digest: given, .. },
                Permission::Browser { digest: wanted, .. },
            ) => given == wanted,
            _ => given == &asked,
        };
        prop_assert_eq!(approvals.covers(&asked), approved.iter().any(same_approval));
    }

    /// Approvals read back as written, through the Operator policy's JSON.
    #[test]
    fn approvals_read_back(approved in prop::collection::vec(any_permission(), 0..8)) {
        let mut approvals = Approvals::default();
        for permission in &approved {
            approvals.approve(permission);
        }
        let value = serde_json::to_value(&approvals).expect("approvals serialize");
        let read: Approvals = serde_json::from_value(value).expect("approvals read back");
        prop_assert_eq!(read, approvals);
    }
}

const DIGEST: &str = "sha256:11";

/// Covers: story:7-16, story:7-17
#[rstest]
#[case::first_party_by_default(Origin::FirstParty, None, None, Ok(()))]
#[case::third_party_by_default(Origin::ThirdParty, None, None, Err(PluginRefusal::NotAllowed))]
#[case::private_is_not_first_party(Origin::Private, None, None, Err(PluginRefusal::NotAllowed))]
#[case::third_party_allowed(Origin::ThirdParty, Some(true), None, Ok(()))]
#[case::private_allowed(Origin::Private, Some(true), None, Ok(()))]
#[case::first_party_denied(Origin::FirstParty, Some(false), None, Err(PluginRefusal::Denied))]
#[case::third_party_denied(Origin::ThirdParty, Some(false), None, Err(PluginRefusal::Denied))]
#[case::pinned_to_its_digest(Origin::ThirdParty, Some(true), Some(DIGEST), Ok(()))]
#[case::pinned_to_another_digest(
    Origin::ThirdParty,
    Some(true),
    Some("sha256:22"),
    Err(PluginRefusal::Digest { pinned: "sha256:22".to_owned() })
)]
#[case::first_party_pinned_to_another_digest(
    Origin::FirstParty,
    None,
    Some("sha256:22"),
    Err(PluginRefusal::Digest { pinned: "sha256:22".to_owned() })
)]
fn admission_follows_the_operator_policy(
    #[case] origin: Origin,
    #[case] allow: Option<bool>,
    #[case] pinned: Option<&str>,
    #[case] expected: Result<(), PluginRefusal>,
) {
    assert_eq!(admit(origin, allow, pinned, Some(DIGEST)), expected);
}

#[test]
fn a_pin_refuses_a_plugin_whose_digest_is_unknown() {
    assert_eq!(
        admit(Origin::ThirdParty, Some(true), Some(DIGEST), None),
        Err(PluginRefusal::Digest {
            pinned: DIGEST.to_owned()
        })
    );
}
