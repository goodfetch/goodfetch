use std::net::{IpAddr, Ipv4Addr};

use super::{LayerGap, LayerGaps, RunFailed, SsrfLayers, UnknownRunner};
use crate::ssrf::{EgressRule, Family, Outcome, Refusal, SsrfCase, SsrfTable};

/// The fixture layers' public stand-in.
const PUBLIC: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));

const PRIVATE_ADDRESS: Refusal = Refusal::EgressRefused {
    rule: EgressRule::PrivateAddress,
};

/// A table of one encoding, one redirect and one domain policy case.
fn table() -> SsrfTable {
    SsrfTable::parse(
        r#"
        [[cases]]
        name = "decimal_integer"
        family = "encoding"
        url = "http://2130706433/"
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }

        [[cases]]
        name = "status_302_to_loopback"
        family = "redirect"
        url = "http://redirect.ssrf.test/"
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }

        [[cases]]
        name = "unlisted_host"
        family = "domain_policy"
        url = "http://elsewhere.ssrf.test/"
        target = "public"
        refusal = { code = "egress_refused", rule = "domain_policy" }
        "#,
    )
    .unwrap()
}

/// A test file holding the runner `name`, taking its cases by its name.
fn holding(name: &str) -> String {
    format!(
        "#[test]\nfn {name}() {{\n    let runner = Runner::committed(\"client\", file!(), \"{name}\");\n}}\n"
    )
}

/// The test files: `tests/client.rs` holds `every_case` and `redirects`.
fn sources(path: &str) -> Option<String> {
    (path == "tests/client.rs").then(|| holding("every_case") + &holding("redirects"))
}

const EVERY_LAYER: &str = r#"every_layer = ["encoding", "redirect"]"#;

fn layers(text: &str) -> SsrfLayers {
    SsrfLayers::parse(&format!("{EVERY_LAYER}\n{text}")).unwrap()
}

#[test]
fn layers_whose_runners_run_every_family_pass_the_check() {
    let layers = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [
            { test = "tests/client.rs::every_case", families = ["encoding", "domain_policy"] },
            { test = "tests/client.rs::redirects", families = ["redirect"] },
        ]
        "#,
    );

    let result = layers.check(&table(), sources);

    assert_eq!(result, Ok(()));
}

#[test]
fn a_built_layer_with_no_runner_fails_the_check() {
    let layers = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [
            { test = "tests/client.rs::every_case", families = ["encoding", "redirect", "domain_policy"] },
        ]

        [layers.relay]
        summary = "The relay."
        not_run = { encoding = "None reaches it.", redirect = "None reaches it." }
        "#,
    );

    let result = layers.check(&table(), sources);

    assert_eq!(
        result,
        Err(LayerGaps {
            gaps: vec![LayerGap::NoRunner {
                layer: "relay".to_owned()
            }]
        })
    );
}

#[test]
fn a_layer_whose_runners_skip_a_family_every_layer_meets_fails_the_check() {
    let layers = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [
            { test = "tests/client.rs::every_case", families = ["encoding", "domain_policy"] },
        ]
        "#,
    );

    let result = layers.check(&table(), sources);

    assert_eq!(
        result,
        Err(LayerGaps {
            gaps: vec![
                LayerGap::FamilySkipped {
                    layer: "client".to_owned(),
                    family: Family::Redirect,
                },
                // No other layer runs it either.
                LayerGap::FamilyOfNoLayer {
                    family: Family::Redirect
                },
            ]
        })
    );
}

#[test]
fn a_family_the_layer_says_it_never_meets_is_no_gap_unless_it_also_runs_it() {
    let excused = layers(
        r#"
        [layers.classification]
        summary = "The classification in process."
        runners = [{ test = "tests/client.rs::every_case", families = ["encoding", "domain_policy"] }]
        not_run = { redirect = "It follows no redirect." }

        [layers.client]
        summary = "The engine's client."
        runners = [{ test = "tests/client.rs::redirects", families = ["redirect"] }]
        not_run = { encoding = "Another layer parses the URL." }
        "#,
    );
    let both = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [
            { test = "tests/client.rs::every_case", families = ["encoding", "redirect", "domain_policy"] },
        ]
        not_run = { redirect = "It follows no redirect." }
        "#,
    );

    let results = (
        excused.check(&table(), sources),
        both.check(&table(), sources),
    );

    assert_eq!(
        results,
        (
            Ok(()),
            Err(LayerGaps {
                gaps: vec![LayerGap::RunAndNotRun {
                    layer: "client".to_owned(),
                    family: Family::Redirect,
                }]
            })
        )
    );
}

#[test]
fn a_family_of_the_table_no_layer_runs_fails_the_check_unless_a_pending_layer_will() {
    let unrun = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [{ test = "tests/client.rs::every_case", families = ["encoding", "redirect"] }]
        "#,
    );
    let pending = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [{ test = "tests/client.rs::every_case", families = ["encoding", "redirect"] }]

        [layers.webhooks]
        summary = "Notification webhooks."
        pending = { joins = "area 11", families = ["domain_policy"] }
        "#,
    );

    let results = (
        unrun.check(&table(), sources),
        pending.check(&table(), sources),
    );

    assert_eq!(
        results,
        (
            Err(LayerGaps {
                gaps: vec![LayerGap::FamilyOfNoLayer {
                    family: Family::DomainPolicy
                }]
            }),
            Ok(())
        )
    );
}

#[test]
fn a_pending_layer_naming_a_runner_fails_the_check() {
    let layers = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [{ test = "tests/client.rs::every_case", families = ["encoding", "redirect", "domain_policy"] }]
        pending = { joins = "area 11", families = [] }
        "#,
    );

    let result = layers.check(&table(), sources);

    assert_eq!(
        result,
        Err(LayerGaps {
            gaps: vec![LayerGap::PendingWithRunners {
                layer: "client".to_owned()
            }]
        })
    );
}

#[test]
fn a_runner_its_file_does_not_hold_or_take_its_cases_by_name_fails_the_check() {
    let layers = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [
            { test = "tests/client.rs::every_case", families = ["encoding", "redirect", "domain_policy"] },
            { test = "tests/client.rs::gone", families = ["encoding"] },
            { test = "tests/other.rs::every_case", families = ["encoding"] },
            { test = "tests/named.rs::every_case", families = ["encoding"] },
        ]
        "#,
    );
    // `named.rs` holds the function, but takes its cases by another name.
    let sources = |path: &str| match path {
        "tests/named.rs" => Some(holding("every_case").replace("\"every_case\"", "\"other\"")),
        path => sources(path),
    };

    let result = layers.check(&table(), sources);

    assert_eq!(
        result,
        Err(LayerGaps {
            gaps: [
                "tests/client.rs::gone",
                "tests/other.rs::every_case",
                "tests/named.rs::every_case"
            ]
            .into_iter()
            .map(|test| LayerGap::RunnerNotFound {
                layer: "client".to_owned(),
                test: test.to_owned(),
            })
            .collect()
        })
    );
}

#[test]
fn a_runner_takes_every_case_of_the_families_its_entry_gives_it() {
    let layers = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [{ test = "tests/client.rs::every_case", families = ["redirect", "encoding"] }]
        "#,
    );

    let runner = layers
        .runner(&table(), "client", "tests/client.rs", "every_case")
        .unwrap();
    let unknown = layers.runner(&table(), "relay", "tests/client.rs", "every_case");

    assert_eq!(
        runner
            .cases()
            .iter()
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>(),
        ["status_302_to_loopback", "decimal_integer"]
    );
    assert_eq!(
        unknown,
        Err(UnknownRunner {
            layer: "relay".to_owned(),
            test: "tests/client.rs::every_case".to_owned(),
        })
    );
}

/// A control reaching the public stand-in.
fn control() -> Vec<SsrfCase> {
    SsrfTable::parse(
        r#"
        [[cases]]
        name = "public_only"
        family = "mixed_answers"
        url = "http://public.ssrf.test/"
        target = "public"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap()
    .cases
}

/// A layer that refuses every case but the control, which it lets
/// through to the public stand-in when `reaching`.
fn refusing_all_but_the_control(reaching: bool) -> impl FnMut(&SsrfCase) -> Outcome {
    move |case| {
        if case.name == "public_only" {
            Outcome::connected(reaching.then_some(PUBLIC))
        } else {
            Outcome::refused(case.refusal.clone())
        }
    }
}

#[test]
fn a_run_passes_only_with_a_control_that_reaches_its_target() {
    let layers = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [{ test = "tests/client.rs::every_case", families = ["encoding"] }]
        "#,
    );
    let runner = layers
        .runner(&table(), "client", "tests/client.rs", "every_case")
        .unwrap();

    let passed = runner.run(&control(), PUBLIC, refusing_all_but_the_control(true));
    let uncontrolled = runner.run(&[], PUBLIC, refusing_all_but_the_control(true));
    let held = runner.run(&control(), PUBLIC, refusing_all_but_the_control(false));

    assert_eq!(passed, Ok(()));
    assert_eq!(uncontrolled.map_err(|failed| failed.no_control), Err(true));
    assert_eq!(
        held.map_err(|failed| failed
            .held
            .iter()
            .map(|miss| (miss.case.clone(), miss.reached_target))
            .collect::<Vec<_>>()),
        Err(vec![("public_only".to_owned(), false)])
    );
}

#[test]
fn a_run_lists_every_case_its_layer_let_through() {
    let layers = layers(
        r#"
        [layers.client]
        summary = "The engine's client."
        runners = [{ test = "tests/client.rs::every_case", families = ["encoding", "redirect"] }]
        "#,
    );
    let runner = layers
        .runner(&table(), "client", "tests/client.rs", "every_case")
        .unwrap();

    let result = runner.run(&control(), PUBLIC, |case| match case.family {
        Family::Redirect => Outcome::connected([IpAddr::V4(Ipv4Addr::LOCALHOST)]),
        _ => Outcome::connected([PUBLIC]),
    });

    let Err(RunFailed { let_through, .. }) = result else {
        panic!("the run fails: {result:?}");
    };
    assert_eq!(
        let_through
            .iter()
            .map(|miss| (
                miss.case.as_str(),
                miss.refusal.clone(),
                miss.reached_target
            ))
            .collect::<Vec<_>>(),
        [
            ("decimal_integer", None, false),
            ("status_302_to_loopback", None, true),
        ]
    );
    assert_eq!(let_through[0].expected, PRIVATE_ADDRESS);
}
