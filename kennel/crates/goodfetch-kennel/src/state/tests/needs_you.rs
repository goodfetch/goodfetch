//! The Needs you items in the menu, with the actions a person resolves
//! them by, the calls those choices make, and the engine's refusals.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::needs_you::{
    NeedsYouAllowConsentDomainParams, NeedsYouDenyWallDomainParams, NeedsYouDismissParams,
    NeedsYouRemoveRefetchDomainParams,
};
use goodfetch_protocol::usage::UsageRaiseParams;
use rstest::rstest;
use serde_json::{Value, json};

use super::super::{Action, Choice, Effect, Input, MenuChange, Row, State, Verdict};
use crate::client::{Call, CallFailure, ConnectionChange, DaemonEvent, DaemonIdentity};
use crate::settings::Settings;

/// A State that reached a Foxhound Daemon.
fn reached() -> State {
    let mut state = State::new();
    state.apply(Input::Connection(ConnectionChange::Connected(
        DaemonIdentity {
            product: Product::Foxhound,
            product_version: ProductVersion::parse("0.4.1").unwrap(),
        },
    )));
    state
}

fn listed(item: &Value) -> Input {
    Input::Events(vec![DaemonEvent::NeedsYou(
        serde_json::from_value(json!({"change": "listed", "item": item})).unwrap(),
    )])
}

fn storage_level(id: &str, since: &str) -> Value {
    json!({
        "id": id,
        "kind": "storage_level",
        "since": since,
        "summary": "The filesystem of /data is at the low storage level",
        "actions": ["needs_you.dismiss"],
        "details": {"level": "low", "folder": "/data"},
    })
}

/// The menu as a person reads it: a detail indented, a separator as
/// `---`, a submenu as its label followed by `>`, and its rows under it
/// after `> `.
fn lines(state: &State) -> Vec<String> {
    let mut lines = Vec::new();
    for row in state.view().rows {
        read(&row, "", &mut lines);
    }
    lines
}

fn read(row: &Row, prefix: &str, lines: &mut Vec<String>) {
    match row {
        Row::Text(line) | Row::Choice { label: line, .. } => lines.push(format!("{prefix}{line}")),
        Row::Detail(line) => lines.push(format!("{prefix}  {line}")),
        Row::Separator => lines.push(format!("{prefix}---")),
        Row::Submenu { label, rows } => {
            lines.push(format!("{prefix}{label} >"));
            for row in rows {
                read(row, &format!("{prefix}> "), lines);
            }
        }
    }
}

/// Every choice the menu offers, submenus included, with its label.
fn choices(state: &State) -> Vec<(String, Choice)> {
    fn collect(rows: &[Row], found: &mut Vec<(String, Choice)>) {
        for row in rows {
            match row {
                Row::Choice { label, choice } => found.push((label.clone(), choice.clone())),
                Row::Submenu { rows, .. } => collect(rows, found),
                Row::Text(_) | Row::Detail(_) | Row::Separator => {}
            }
        }
    }
    let mut found = Vec::new();
    collect(&state.view().rows, &mut found);
    found
}

#[test]
fn the_menu_lists_the_needs_you_items_longest_waiting_first_each_with_its_actions() {
    let mut state = reached();

    state.apply(listed(&storage_level("storage:b", "2026-11-03T10:00:00Z")));
    state.apply(listed(&storage_level("storage:a", "2026-11-03T09:00:00Z")));

    assert_eq!(state.view().verdict, Verdict::NeedsYou);
    assert_eq!(
        lines(&state),
        [
            "Foxhound Daemon 0.4.1",
            "---",
            "Needs you",
            "The filesystem of /data is at the low storage level >",
            "> The filesystem of /data is at the low storage level",
            "> Dismiss",
            "The filesystem of /data is at the low storage level >",
            "> The filesystem of /data is at the low storage level",
            "> Dismiss",
            "---",
            "Open details",
            "Quit",
        ]
    );
    let dismissed: Vec<Choice> = choices(&state)
        .into_iter()
        .map(|(_, choice)| choice)
        .filter(|choice| !matches!(choice, Choice::Quit | Choice::OpenDetails))
        .collect();
    assert_eq!(
        dismissed,
        [
            Choice::NeedsYou {
                item: "storage:a".to_owned(),
                action: Action::Dismiss,
            },
            Choice::NeedsYou {
                item: "storage:b".to_owned(),
                action: Action::Dismiss,
            },
        ]
    );
}

fn limit_stopped(recipe: Option<&str>, currency: Option<&str>, amount: u64) -> Value {
    let mut details = json!({
        "limit": {
            "scope": {"kind": "daemon", "id": "0.4"},
            "usage_kind": "page_loads",
            "currency": currency,
            "period": {"every": "day", "time_zone": "Europe/Paris"},
            "resets_at": "2026-11-04T00:00:00+01:00",
        },
        "amount": amount,
        "needs": "foxhound",
    });
    let mut actions = vec!["usage.raise"];
    if let Some(recipe) = recipe {
        details["recipe"] = json!(recipe);
        actions.push("run.resume");
    }
    actions.push("needs_you.dismiss");
    json!({
        "id": "run_stopped_by_limit:shop-crawl:2026-11-03T10:00:00Z",
        "kind": "run_stopped_by_limit",
        "since": "2026-11-03T10:00:00Z",
        "run": "shop-crawl",
        "summary": "The Run shop-crawl stopped: a Limit on page_loads has nothing left.",
        "actions": actions,
        "details": details,
    })
}

fn consent_wall() -> Value {
    json!({
        "id": "consent_wall:news.example",
        "kind": "consent_wall",
        "since": "2026-11-03T10:00:00Z",
        "run": "shop-crawl",
        "summary": "A Run met the consent wall of news.example.",
        "actions": ["needs_you.allow_consent_domain", "needs_you.dismiss"],
        "details": {"domain": "news.example"},
    })
}

fn refetch_stopped(only_star: bool) -> Value {
    json!({
        "id": "wall_refetch_stopped:shop.example",
        "kind": "wall_refetch_stopped",
        "since": "2026-11-03T10:00:00Z",
        "run": "shop-crawl",
        "summary": "A Run's refetch stopped on shop.example at a login page.",
        "actions": [
            "needs_you.remove_refetch_domain",
            "needs_you.deny_wall_domain",
            "needs_you.dismiss",
        ],
        "details": {
            "domain": "shop.example",
            "cause": "login_page",
            "layer": "user",
            "file": "/home/person/.config/goodfetch/config.toml",
            "only_star": only_star,
        },
    })
}

fn raised(amount: u64, currency: Option<&str>) -> Call {
    Call::RaiseLimit(
        serde_json::from_value::<UsageRaiseParams>(json!({
            "scope": "daemon",
            "scope_id": "0.4",
            "usage_kind": "page_loads",
            "currency": currency,
            "period": {"every": "day", "time_zone": "Europe/Paris"},
            "amount": amount,
        }))
        .unwrap(),
    )
}

fn dismissed(item: &str) -> Call {
    Call::Dismiss(NeedsYouDismissParams {
        item: item.to_owned(),
    })
}

/// The lines of the one item's submenu, and the call each of its choices
/// makes.
fn submenu_and_calls(state: &mut State) -> (Vec<String>, Vec<Call>) {
    let menu = lines(state);
    let opened = menu
        .iter()
        .position(|line| line == "Needs you")
        .expect("the menu has a Needs you section");
    let submenu: Vec<String> = menu[opened + 1..]
        .iter()
        .take_while(|line| *line != "---")
        .cloned()
        .collect();
    let mut calls = Vec::new();
    for (_, choice) in choices(state) {
        if matches!(choice, Choice::Quit | Choice::OpenDetails) {
            continue;
        }
        for effect in state.choose(choice) {
            match effect {
                Effect::Call(call) => calls.push(call),
                other => panic!("a call, not {other:?}"),
            }
        }
    }
    (submenu, calls)
}

#[rstest]
#[case::a_limit_raised_to_twice_what_it_allows(
    limit_stopped(None, None, 500),
    &[
        "shop-crawl stopped by a Limit on page_loads >",
        "> The Run shop-crawl stopped: a Limit on page_loads has nothing left.",
        "> Raise the Limit to 1000",
        "> Dismiss",
    ],
    vec![raised(1000, None), dismissed("run_stopped_by_limit:shop-crawl:2026-11-03T10:00:00Z")],
)]
#[case::a_recipes_run_named_by_its_recipe_and_a_price_in_its_currency(
    limit_stopped(Some("shop"), Some("EUR"), 1_250_000),
    &[
        "shop stopped by a Limit on page_loads >",
        "> The Run shop-crawl stopped: a Limit on page_loads has nothing left.",
        "> Raise the Limit to 2.5 EUR",
        "> Dismiss",
    ],
    vec![raised(2_500_000, Some("EUR")), dismissed("run_stopped_by_limit:shop-crawl:2026-11-03T10:00:00Z")],
)]
#[case::a_limit_of_nothing_is_raised_in_the_cli(
    limit_stopped(None, None, 0),
    &[
        "shop-crawl stopped by a Limit on page_loads >",
        "> The Run shop-crawl stopped: a Limit on page_loads has nothing left.",
        "> Dismiss",
    ],
    vec![dismissed("run_stopped_by_limit:shop-crawl:2026-11-03T10:00:00Z")],
)]
#[case::a_consent_wall(
    consent_wall(),
    &[
        "Consent wall on news.example >",
        "> A Run met the consent wall of news.example.",
        "> Accept the consent walls of news.example",
        "> Dismiss",
    ],
    vec![
        Call::AllowConsentDomain(NeedsYouAllowConsentDomainParams { domain: "news.example".to_owned() }),
        dismissed("consent_wall:news.example"),
    ],
)]
#[case::a_stopped_refetch(
    refetch_stopped(false),
    &[
        "Refetch stopped on shop.example >",
        "> A Run's refetch stopped on shop.example at a login page.",
        "> Remove shop.example from the refetch domains",
        "> Deny shop.example: no Run reads past its Walls",
        "> Dismiss",
    ],
    vec![
        Call::RemoveRefetchDomain(NeedsYouRemoveRefetchDomainParams {
            domain: "shop.example".to_owned(),
            instead_of_star: None,
        }),
        Call::DenyWallDomain(NeedsYouDenyWallDomainParams {
            domain: "shop.example".to_owned(),
            reason: "Denied by a person from the Kennel's menu".to_owned(),
        }),
        dismissed("wall_refetch_stopped:shop.example"),
    ],
)]
#[case::a_stopped_refetch_only_star_lists_is_removed_in_the_cli(
    refetch_stopped(true),
    &[
        "Refetch stopped on shop.example >",
        "> A Run's refetch stopped on shop.example at a login page.",
        "> Deny shop.example: no Run reads past its Walls",
        "> Dismiss",
    ],
    vec![
        Call::DenyWallDomain(NeedsYouDenyWallDomainParams {
            domain: "shop.example".to_owned(),
            reason: "Denied by a person from the Kennel's menu".to_owned(),
        }),
        dismissed("wall_refetch_stopped:shop.example"),
    ],
)]
fn each_action_an_item_allows_calls_the_operation_it_is_for(
    #[case] item: Value,
    #[case] submenu: &[&str],
    #[case] calls: Vec<Call>,
) {
    let mut state = reached();
    state.apply(listed(&item));

    let (shown, made) = submenu_and_calls(&mut state);

    assert_eq!(shown, submenu);
    assert_eq!(made, calls);
}

#[test]
fn a_read_only_kennel_lists_the_items_and_offers_no_action() {
    let mut state = reached();
    state.apply(Input::Settings(Ok(Settings {
        controls: false,
        ..Settings::default()
    })));

    state.apply(listed(&consent_wall()));

    assert_eq!(state.view().verdict, Verdict::NeedsYou);
    let (submenu, calls) = submenu_and_calls(&mut state);
    assert_eq!(
        submenu,
        [
            "Consent wall on news.example >",
            "> A Run met the consent wall of news.example.",
        ]
    );
    assert_eq!(calls, []);
}

#[test]
fn an_action_of_an_item_that_left_the_list_does_nothing() {
    let mut state = reached();
    state.apply(listed(&consent_wall()));
    let offered = Choice::NeedsYou {
        item: "consent_wall:news.example".to_owned(),
        action: Action::AllowConsentDomain,
    };
    state.apply(Input::Events(vec![DaemonEvent::NeedsYou(
        serde_json::from_value(
            json!({"change": "removed", "id": "consent_wall:news.example", "reason": "resolved"}),
        )
        .unwrap(),
    )]));

    let effects = state.choose(offered);

    assert_eq!(effects, []);
    assert_eq!(state.view().verdict, Verdict::Idle);
}

/// The call choosing to accept the consent walls of `news.example` makes.
fn accepted(state: &mut State) -> Call {
    let effects = state.choose(Choice::NeedsYou {
        item: "consent_wall:news.example".to_owned(),
        action: Action::AllowConsentDomain,
    });
    match effects.as_slice() {
        [Effect::Call(call)] => call.clone(),
        other => panic!("one call, not {other:?}"),
    }
}

fn origin() -> std::time::Instant {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test's origin for the instants it gives the State, which reads no clock itself"
    )]
    std::time::Instant::now()
}

#[test]
fn an_action_the_engine_refuses_shows_its_errors_message_made_safe() {
    let mut state = reached();
    state.apply(listed(&consent_wall()));
    let call = accepted(&mut state);

    state.apply(Input::Answered(
        call,
        Err(CallFailure {
            code: "setting_locked".to_owned(),
            message: "The policy \u{202e}locks\u{202c} it:\n see https://admin:hunter2@p.example/"
                .to_owned(),
        }),
    ));

    assert_eq!(
        lines(&state)[..2],
        [
            "Foxhound Daemon 0.4.1",
            "Accepting the consent walls of news.example failed: The policy locks it: see https://admin:••••@p.example/",
        ]
    );
    // The item waits still.
    assert_eq!(state.view().verdict, Verdict::NeedsYou);
}

#[test]
fn a_refusal_stays_until_the_menu_that_showed_it_closes() {
    let mut state = reached();
    state.apply(listed(&consent_wall()));
    let call = accepted(&mut state);
    state.apply(Input::Answered(
        call,
        Err(CallFailure {
            code: "connection_lost".to_owned(),
            message: "The Connection to the Daemon was lost before it answered".to_owned(),
        }),
    ));
    let refusal = "Accepting the consent walls of news.example failed: The Connection to the Daemon was lost before it answered";

    state.apply(Input::Menu(MenuChange::Opened(origin())));
    let while_open = lines(&state).contains(&refusal.to_owned());
    state.apply(Input::Menu(MenuChange::Closed));

    assert!(while_open, "the open menu shows the refusal");
    assert!(!lines(&state).contains(&refusal.to_owned()));
}

#[test]
fn an_action_the_engine_answers_shows_nothing_more() {
    let mut state = reached();
    state.apply(listed(&consent_wall()));
    let before = lines(&state);
    let call = accepted(&mut state);

    state.apply(Input::Answered(call, Ok(())));

    assert_eq!(lines(&state), before);
}
