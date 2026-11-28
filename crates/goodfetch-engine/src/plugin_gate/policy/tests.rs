//! Property and table tests of the pure Effects, `page_script` and
//! navigation-scope decision rules (ADRs 0010, 0014 and 0033; issue 845).

use std::collections::BTreeSet;

use goodfetch_protocol::plugin::approvals::{Approvals, Permission};
use goodfetch_protocol::plugin::{Grant, HookStageName};
use proptest::prelude::*;
use rstest::rstest;

use super::{
    ActionContext, AllowPlan, PluginAction, PolicyDecision, PolicyRefusal, classify_page_method,
    decide, is_observe_only_stage, is_page_stage, registrable_or_host,
};
use crate::egress::EgressClass;

const ALL_STAGES: [HookStageName; 12] = [
    HookStageName::from_static("before_request"),
    HookStageName::from_static("after_response"),
    HookStageName::BEFORE_NAVIGATION,
    HookStageName::AFTER_NAVIGATION,
    HookStageName::PAGE_READY,
    HookStageName::from_static("before_record_commit"),
    HookStageName::from_static("request_ended"),
    HookStageName::RUN_STARTED,
    HookStageName::CHECKPOINT_TAKEN,
    HookStageName::RUN_MIGRATED,
    HookStageName::LIMIT_REACHED,
    HookStageName::RUN_ENDED,
];

fn arb_action() -> impl Strategy<Value = PluginAction> {
    prop_oneof![
        (
            any::<bool>(),
            prop_oneof![
                Just(EgressClass::OperatorConfigured),
                Just(EgressClass::PageReachable),
            ],
        )
            .prop_map(|(is_effect, egress_class)| PluginAction::NetFetch {
                is_effect,
                egress_class,
            }),
        Just(PluginAction::PageRead),
        Just(PluginAction::PageScript),
        Just(PluginAction::PageMutate),
        Just(PluginAction::PageNavigate),
    ]
}

fn arb_stage() -> impl Strategy<Value = Option<HookStageName>> {
    proptest::option::of(proptest::sample::select(ALL_STAGES.to_vec()))
}

fn arb_host() -> impl Strategy<Value = &'static str> {
    proptest::sample::select(vec![
        "shop.test",
        "www.shop.test",
        "api.shop.test",
        "cdn.partner.test",
        "evil.example",
    ])
}

proptest! {
    /// Across every combination of action, Hook stage, login state, hosts,
    /// declared domains, manifest grants and Operator policy approvals:
    /// - A non-page Hook stage never allows any `page.*` action.
    /// - An observe-only stage never allows a page-reachable `net.fetch` Effect.
    /// - A `net.fetch` to an Operator-configured endpoint is never treated as
    ///   an Effect on the run's sites.
    /// - A `PageMutate`, a page-reachable `NetFetch` Effect, and a logged-in
    ///   `PageNavigate` are allowed ONLY when `Grant::Effects` is declared AND
    ///   `approved.effects` covers the target domain, and ALWAYS journal and
    ///   audit `effects`.
    /// - A `PageScript` is allowed ONLY when `Grant::PageScript` is declared
    ///   AND `approved.page_script` covers the target domain, ALWAYS audits
    ///   `page_script`, and journals as a possible Effect iff `logged_in`.
    /// - A `PageNavigate` is refused outside the page's registrable domain and
    ///   declared domains, and while `logged_in` declared domains NEVER widen
    ///   navigation beyond the page's registrable domain.
    ///
    /// Covers: story:7-49, story:7-50, story:7-60
    #[test]
    fn effects_page_script_and_navigation_scope_invariants_hold_for_every_method_and_stage(
        action in arb_action(),
        stage in arb_stage(),
        logged_in in any::<bool>(),
        page_host in arb_host(),
        target_host in arb_host(),
        declare_partner in any::<bool>(),
        grant_effects in any::<bool>(),
        grant_page_script in any::<bool>(),
        approve_effects_shop in any::<bool>(),
        approve_effects_partner in any::<bool>(),
        approve_script_shop in any::<bool>(),
        approve_script_partner in any::<bool>(),
    ) {
        let mut grants = BTreeSet::new();
        if grant_effects {
            grants.insert(Grant::Effects);
        }
        if grant_page_script {
            grants.insert(Grant::PageScript);
        }
        let mut approved = Approvals::default();
        if approve_effects_shop {
            approved.effects.insert("shop.test".to_owned());
        }
        if approve_effects_partner {
            approved.effects.insert("partner.test".to_owned());
        }
        if approve_script_shop {
            approved.page_script.insert("shop.test".to_owned());
        }
        if approve_script_partner {
            approved.page_script.insert("partner.test".to_owned());
        }
        let declared_domains = if declare_partner {
            vec!["partner.test".to_owned()]
        } else {
            Vec::new()
        };

        let ctx = ActionContext {
            action,
            stage,
            logged_in,
            page_host: Some(page_host),
            target_host,
            declared_domains: &declared_domains,
            grants: &grants,
            approved: &approved,
        };
        let decision = decide(&ctx);

        // 1. Stage guards.
        if let Some(stage_name) = stage {
            if action.is_page() && !is_page_stage(stage_name) {
                prop_assert_eq!(
                    decision,
                    PolicyDecision::Refuse(PolicyRefusal::StageForbidsPageCall(stage_name))
                );
                return Ok(());
            }
            if is_observe_only_stage(stage_name)
                && matches!(
                    action,
                    PluginAction::NetFetch {
                        is_effect: true,
                        egress_class: EgressClass::PageReachable,
                    }
                )
            {
                prop_assert_eq!(
                    decision,
                    PolicyDecision::Refuse(PolicyRefusal::StageForbidsEffect(stage_name))
                );
                return Ok(());
            }
        }

        let target_reg = registrable_or_host(target_host);
        let page_reg = registrable_or_host(page_host);
        let effects_approved = (target_reg == "shop.test" && approve_effects_shop)
            || (target_reg == "partner.test" && approve_effects_partner);
        let script_approved = (target_reg == "shop.test" && approve_script_shop)
            || (target_reg == "partner.test" && approve_script_partner);

        match action {
            PluginAction::NetFetch {
                egress_class: EgressClass::OperatorConfigured,
                ..
            }
            | PluginAction::NetFetch {
                is_effect: false,
                egress_class: EgressClass::PageReachable,
            }
            | PluginAction::PageRead => {
                prop_assert_eq!(
                    decision,
                    PolicyDecision::Allow(AllowPlan {
                        journal_effect: false,
                        audit: Vec::new(),
                    })
                );
            }
            PluginAction::NetFetch {
                is_effect: true,
                egress_class: EgressClass::PageReachable,
            }
            | PluginAction::PageMutate => {
                if !grant_effects {
                    prop_assert_eq!(
                        decision,
                        PolicyDecision::Refuse(PolicyRefusal::GrantUndeclared(Grant::Effects))
                    );
                } else if !effects_approved {
                    prop_assert_eq!(
                        decision,
                        PolicyDecision::DenyPermission(Permission::Effects {
                            domain: target_reg,
                        })
                    );
                } else {
                    prop_assert_eq!(
                        decision,
                        PolicyDecision::Allow(AllowPlan {
                            journal_effect: true,
                            audit: vec![Permission::Effects { domain: target_reg }],
                        })
                    );
                }
            }
            PluginAction::PageScript => {
                if !grant_page_script {
                    prop_assert_eq!(
                        decision,
                        PolicyDecision::Refuse(PolicyRefusal::GrantUndeclared(Grant::PageScript))
                    );
                } else if !script_approved {
                    prop_assert_eq!(
                        decision,
                        PolicyDecision::DenyPermission(Permission::PageScript {
                            domain: target_reg,
                        })
                    );
                } else {
                    prop_assert_eq!(
                        decision,
                        PolicyDecision::Allow(AllowPlan {
                            journal_effect: logged_in,
                            audit: vec![Permission::PageScript { domain: target_reg }],
                        })
                    );
                }
            }
            PluginAction::PageNavigate => {
                let same_reg = page_reg == target_reg;
                let declared_allows = !logged_in && declare_partner && target_reg == "partner.test";
                if !same_reg && !declared_allows {
                    prop_assert_eq!(
                        decision,
                        PolicyDecision::Refuse(PolicyRefusal::NavigationOutOfScope {
                            target_domain: target_reg,
                            page_domain: Some(page_reg),
                            logged_in,
                        })
                    );
                } else if logged_in {
                    if !grant_effects {
                        prop_assert_eq!(
                            decision,
                            PolicyDecision::Refuse(PolicyRefusal::GrantUndeclared(Grant::Effects))
                        );
                    } else if !effects_approved {
                        prop_assert_eq!(
                            decision,
                            PolicyDecision::DenyPermission(Permission::Effects {
                                domain: target_reg,
                            })
                        );
                    } else {
                        prop_assert_eq!(
                            decision,
                            PolicyDecision::Allow(AllowPlan {
                                journal_effect: true,
                                audit: vec![Permission::Effects { domain: target_reg }],
                            })
                        );
                    }
                } else {
                    prop_assert_eq!(
                        decision,
                        PolicyDecision::Allow(AllowPlan {
                            journal_effect: false,
                            audit: Vec::new(),
                        })
                    );
                }
            }
        }
    }
}

/// Every page/locator method maps to its exact action category.
///
/// Covers: story:7-49, story:7-50, story:7-60
#[rstest]
#[case::evaluate("page.evaluate", Some(PluginAction::PageScript))]
#[case::evaluate_all("locator.evaluate_all", Some(PluginAction::PageScript))]
#[case::init_script("page.add_init_script", Some(PluginAction::PageScript))]
#[case::wait_for_function("page.wait_for_function", Some(PluginAction::PageScript))]
#[case::locator_wait_for_function("locator.wait_for_function", Some(PluginAction::PageScript))]
#[case::cookies("page.cookies", Some(PluginAction::PageScript))]
#[case::storage("page.storage", Some(PluginAction::PageScript))]
#[case::input_value("locator.input_value", Some(PluginAction::PageScript))]
#[case::click("locator.click", Some(PluginAction::PageMutate))]
#[case::fill("locator.fill", Some(PluginAction::PageMutate))]
#[case::press("locator.press", Some(PluginAction::PageMutate))]
#[case::submit("locator.submit", Some(PluginAction::PageMutate))]
#[case::goto("page.goto", Some(PluginAction::PageNavigate))]
#[case::reload("page.reload", Some(PluginAction::PageNavigate))]
#[case::go_back("page.go_back", Some(PluginAction::PageNavigate))]
#[case::go_forward("page.go_forward", Some(PluginAction::PageNavigate))]
#[case::read("page.read", Some(PluginAction::PageRead))]
#[case::formats("page.formats", Some(PluginAction::PageRead))]
#[case::body("page.body", Some(PluginAction::PageRead))]
#[case::unknown("config.policy_read", None)]
fn page_methods_classify_to_their_action_category(
    #[case] method: &str,
    #[case] expected: Option<PluginAction>,
) {
    assert_eq!(classify_page_method(method), expected);
}
