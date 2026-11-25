//! `identity_report`: one Identity profile surface's values in every context
//! the consistency probes run script in, and in the headers of each
//! context's requests, compared with the main frame's (ADR 0074). The
//! observations here are written as the Proving ground lists them: each
//! context's reports, and the report requests that carried them.

use goodfetch_test_support::{
    IdentityReport, PageValues, ProbeContext, ValueSource, identity_report,
};
use serde_json::{Value, json};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) Chrome/156.0.0.0 Safari/537.36";

/// One context's report of a surface, and the headers its report request
/// sent.
struct Reported {
    context: ProbeContext,
    surface: &'static str,
    observed: Value,
    headers: Vec<(&'static str, &'static str)>,
}

/// `context`'s report of `surface` as `observed`, its request sending the
/// stock user agent.
fn reported(context: ProbeContext, surface: &'static str, observed: Value) -> Reported {
    Reported {
        context,
        surface,
        observed,
        headers: vec![("user-agent", USER_AGENT)],
    }
}

/// The observations of a Proving ground that received `reports`.
fn observations(reports: &[Reported]) -> Value {
    let requests: Vec<Value> = reports
        .iter()
        .enumerate()
        .map(|(sequence, report)| {
            json!({
                "sequence": sequence,
                "connection": 0,
                "task": null,
                "host": report.context.host(),
                "protocol": "h2",
                "kind": "request",
                "method": "POST",
                "target": format!("/reports?page={}", report.context.page()),
                "pseudo_header_order": null,
                "headers": report.headers,
                "cookies": [],
                "message_signature": null,
                "tls": null,
                "http2": null,
                "status": 204,
                "timing": { "arrived_microseconds": sequence, "answered_microseconds": sequence + 1 },
                "bytes": {},
                "body": {},
            })
        })
        .collect();
    let reports: Vec<Value> = reports
        .iter()
        .map(|report| {
            json!({
                "task": null,
                "page": report.context.page(),
                "check": report.surface,
                "observed": report.observed.to_string(),
            })
        })
        .collect();
    json!({ "requests": requests, "reports": reports })
}

/// Every context's report of the stock user agent, each as `change` leaves
/// it.
fn user_agent_everywhere(change: impl Fn(&mut Reported)) -> Vec<Reported> {
    ProbeContext::ALL
        .into_iter()
        .map(|context| {
            let mut report = reported(context, "user_agent", json!(USER_AGENT));
            change(&mut report);
            report
        })
        .collect()
}

/// The report of `surface` among `observations`.
fn report_of(observations: &Value, surface: &str) -> IdentityReport {
    let values = PageValues::read(observations).unwrap();
    let page_values = values
        .iter()
        .find(|values| values.surface == surface)
        .unwrap();
    identity_report(page_values)
}

/// The same value in every context and header passes, and a worker's
/// `null` for the screen, which a worker has none of, is left out.
#[test]
fn a_surface_every_context_and_header_agree_on_is_consistent() {
    let mut reports = user_agent_everywhere(|_| {});
    reports.extend(ProbeContext::ALL.into_iter().map(|context| {
        let window = matches!(
            context,
            ProbeContext::MainFrame | ProbeContext::CrossOriginFrame
        );
        reported(
            context,
            "screen_width",
            if window { json!(800) } else { Value::Null },
        )
    }));
    let observations = observations(&reports);

    let user_agent = report_of(&observations, "user_agent");
    let screen = report_of(&observations, "screen_width");

    assert!(user_agent.is_consistent(), "{user_agent}");
    assert!(screen.is_consistent(), "{screen}");
}

/// A worker whose patch did not reach it reports the stock value: that
/// worker's value is flagged, named by its context.
#[test]
fn one_worker_reporting_another_value_is_flagged() {
    let stock = "Mozilla/5.0 (X11; Linux x86_64) HeadlessChrome/156.0.0.0 Safari/537.36";
    let reports = user_agent_everywhere(|report| {
        if report.context == ProbeContext::SharedWorker {
            report.observed = json!(stock);
        }
    });

    let report = report_of(&observations(&reports), "user_agent");

    assert!(!report.is_consistent());
    assert_eq!(
        report.differing,
        [(
            ValueSource::Script(ProbeContext::SharedWorker),
            json!(stock)
        )]
    );
    assert!(
        report.to_string().contains("the shared worker gives"),
        "{report}"
    );
}

/// Every script agrees but a context's requests send another user agent
/// than its script reads: the header is flagged.
#[test]
fn a_header_disagreeing_with_the_scripts_is_flagged() {
    let stock = "Mozilla/5.0 (X11; Linux x86_64) HeadlessChrome/156.0.0.0 Safari/537.36";
    let reports = user_agent_everywhere(|report| {
        if report.context == ProbeContext::ServiceWorker {
            report.headers = vec![("User-Agent", stock)];
        }
    });

    let report = report_of(&observations(&reports), "user_agent");

    assert_eq!(
        report.differing,
        [(
            ValueSource::Headers(ProbeContext::ServiceWorker),
            json!(stock)
        )]
    );
}

/// A context whose probe failed reports nothing, which would hide the very
/// difference the test looks for: it is flagged.
#[test]
fn a_context_that_reported_nothing_is_flagged() {
    let mut reports = user_agent_everywhere(|_| {});
    reports.retain(|report| report.context != ProbeContext::CrossOriginFrame);

    let report = report_of(&observations(&reports), "user_agent");

    assert!(!report.is_consistent());
    assert_eq!(report.unreported, [ProbeContext::CrossOriginFrame]);
}
