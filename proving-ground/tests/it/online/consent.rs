//! The online copy's consent vendors (#168): `/<vendor>` embeds the
//! vendor's real Consent prompt, and `/<vendor>/answer` reads the answer
//! from the consent cookies that vendor set on the copy's own domain, so a
//! refusal passes and an acceptance fails by the vendor's own record. The
//! tests send the cookies each vendor's prompt sets, as a stand-in for it.

use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::{HOST, challenge_tasks, online, verdicts};
use crate::proving_ground::{ProvingGround, submission};

const ONETRUST: &str = "[onetrust]\ndomain_script = \"0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b\"\n";

const COOKIEBOT: &str = "[cookiebot]\ncbid = \"8a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d\"\n";

const COOKIEYES: &str = "[cookieyes]\nclient_id = \"3f2e1d0c9b8a7f6e5d4c3b2a\"\n";

const KLARO: &str = "[klaro]\n";

/// The copy of `config`'s consent vendors, which read no secret.
fn consent_copy(env: &TestEnv, config: &str) -> ProvingGround {
    online(env, config, ("UNUSED", "unused"))
}

/// OneTrust's cookies once its prompt was answered with `groups`.
fn onetrust(groups: &str) -> String {
    format!(
        "OptanonAlertBoxClosed=2026-11-24T10:00:00.000Z; OptanonConsent=isGpcEnabled=0&datestamp=Tue+Nov+24+2026&version=202411.1.0&isIABGlobal=false&consentId=abc&interactionCount=1&landingPath=NotLandingPage&groups={}",
        groups.replace(':', "%3A").replace(',', "%2C")
    )
}

/// Cookiebot's cookie once its prompt was answered with the three
/// categories' states.
fn cookiebot(preferences: bool, statistics: bool, marketing: bool) -> String {
    format!(
        "CookieConsent={{stamp:%27nW1ypT8z%2BkQ%3D%3D%27%2Cnecessary:true%2Cpreferences:{preferences}%2Cstatistics:{statistics}%2Cmarketing:{marketing}%2Cmethod:%27explicit%27%2Cver:1%2Cutc:1795521600000%2Cregion:%27fr%27}}"
    )
}

/// Klaro's cookie once its prompt was answered with `consents`.
fn klaro(consents: &Value) -> String {
    let text = consents.to_string();
    let encoded: String = text
        .bytes()
        .map(|byte| match byte {
            b'{' | b'}' | b'"' | b':' | b',' => format!("%{byte:02X}"),
            other => char::from(other).to_string(),
        })
        .collect();
    format!("klaro={encoded}")
}

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::onetrust(
    ONETRUST,
    "/onetrust",
    r#"src="https://cdn.cookielaw.org/scripttemplates/otSDKStub.js" data-domain-script="0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b""#
)]
#[case::cookiebot(
    COOKIEBOT,
    "/cookiebot",
    r#"src="https://consent.cookiebot.com/uc.js" data-cbid="8a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d""#
)]
#[case::cookieyes(
    COOKIEYES,
    "/cookieyes",
    r#"src="https://cdn-cookieyes.com/client_data/3f2e1d0c9b8a7f6e5d4c3b2a/script.js""#
)]
#[case::klaro(
    KLARO,
    "/klaro",
    r#"src="https://cdn.jsdelivr.net/npm/klaro@0.7.21/dist/klaro.js""#
)]
fn a_consent_vendors_page_embeds_its_prompt(
    #[case] config: &str,
    #[case] path: &str,
    #[case] embed: &str,
) {
    let env = TestEnv::new();
    let proving_ground = consent_copy(&env, config);

    let page = proving_ground.get(&proving_ground.url(HOST, path));

    assert_eq!(page.status, 200);
    assert!(page.body.contains(embed), "{}", page.body);
}

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::onetrust_refused(ONETRUST, onetrust("C0001:1,C0002:0,C0003:0,C0004:0"), json!({ "vendor": "onetrust", "outcome": "passed" }))]
#[case::onetrust_accepted(ONETRUST, onetrust("C0001:1,C0002:1,C0003:1,C0004:1"), json!({ "vendor": "onetrust", "outcome": "failed", "reason": "accepted" }))]
#[case::onetrust_partial(ONETRUST, onetrust("C0001:1,C0002:1,C0003:0,C0004:0"), json!({ "vendor": "onetrust", "outcome": "failed", "reason": "partially_accepted" }))]
#[case::onetrust_unanswered(
    ONETRUST,
    "OptanonConsent=isGpcEnabled=0&groups=C0001%3A1%2CC0002%3A0".to_owned(),
    json!({ "vendor": "onetrust", "outcome": "failed", "reason": "unanswered" })
)]
#[case::cookiebot_refused(COOKIEBOT, cookiebot(false, false, false), json!({ "vendor": "cookiebot", "outcome": "passed" }))]
#[case::cookiebot_accepted(COOKIEBOT, cookiebot(true, true, true), json!({ "vendor": "cookiebot", "outcome": "failed", "reason": "accepted" }))]
#[case::cookiebot_partial(COOKIEBOT, cookiebot(true, false, false), json!({ "vendor": "cookiebot", "outcome": "failed", "reason": "partially_accepted" }))]
#[case::cookiebot_not_required(COOKIEBOT, "CookieConsent=-1".to_owned(), json!({ "vendor": "cookiebot", "outcome": "inconclusive", "reason": "consent_not_required" }))]
#[case::cookieyes_refused(
    COOKIEYES,
    "cookieyes-consent=consentid:abc,consent:no,action:yes,necessary:yes,functional:no,analytics:no,performance:no,advertisement:no,other:no".to_owned(),
    json!({ "vendor": "cookieyes", "outcome": "passed" })
)]
#[case::cookieyes_accepted(
    COOKIEYES,
    "cookieyes-consent=consentid:abc,consent:yes,action:yes,necessary:yes,functional:yes,analytics:yes,performance:yes,advertisement:yes,other:yes".to_owned(),
    json!({ "vendor": "cookieyes", "outcome": "failed", "reason": "accepted" })
)]
#[case::cookieyes_unanswered(
    COOKIEYES,
    "cookieyes-consent=consentid:abc,consent:no,action:no,necessary:yes,functional:no,analytics:no,performance:no,advertisement:no,other:no".to_owned(),
    json!({ "vendor": "cookieyes", "outcome": "failed", "reason": "unanswered" })
)]
#[case::klaro_refused(KLARO, klaro(&json!({ "analytics": false, "marketing": false })), json!({ "vendor": "klaro", "outcome": "passed" }))]
#[case::klaro_partial(KLARO, klaro(&json!({ "analytics": true, "marketing": false })), json!({ "vendor": "klaro", "outcome": "failed", "reason": "partially_accepted" }))]
#[case::no_cookie(KLARO, "other=1".to_owned(), json!({ "vendor": "klaro", "outcome": "failed", "reason": "unanswered" }))]
#[case::malformed(KLARO, "klaro=%7Bnot-json".to_owned(), json!({ "vendor": "klaro", "outcome": "inconclusive", "reason": "malformed_cookie" }))]
fn the_vendors_consent_cookie_is_read_as_its_answer(
    #[case] config: &str,
    #[case] cookie: String,
    #[case] verdict: Value,
) {
    let env = TestEnv::new();
    let proving_ground = consent_copy(&env, config);
    let vendor = verdict["vendor"].as_str().expect("a vendor").to_owned();

    let page = proving_ground.get_with_cookie(
        &proving_ground.url(HOST, &format!("/{vendor}/answer")),
        Some(&cookie),
    );

    let mut expected = json!({ "task": null });
    expected
        .as_object_mut()
        .expect("an object")
        .extend(verdict.as_object().expect("an object").clone());
    assert_eq!(page.status, 200);
    assert_eq!(verdicts(&proving_ground), json!([expected]));
}

/// Covers: story:2-38, story:2-49
#[test]
fn the_online_copy_lists_a_task_per_consent_vendor() {
    let env = TestEnv::new();
    let proving_ground = consent_copy(&env, &format!("{ONETRUST}{COOKIEBOT}{COOKIEYES}{KLARO}"));

    let tasks = challenge_tasks(&proving_ground);

    let expected: Vec<(String, String)> = ["onetrust", "cookiebot", "cookieyes", "klaro"]
        .into_iter()
        .zip(10..)
        .map(|(vendor, ordinal)| {
            (
                format!("challenges/{ordinal}"),
                proving_ground.url(HOST, &format!("/{vendor}")),
            )
        })
        .collect();
    assert_eq!(tasks, expected);
}

/// Covers: story:2-38, story:2-49
#[rstest]
#[case::refused(cookiebot(false, false, false), "verified", None)]
#[case::accepted(
    cookiebot(true, true, true),
    "false_success",
    Some(json!([{
        "check": "vendor_verdict",
        "at": "/cookiebot/answer",
        "observed": { "vendor": "cookiebot", "outcome": "failed", "reason": "accepted" },
    }]))
)]
fn the_recorded_answer_judges_the_consent_task(
    #[case] cookie: String,
    #[case] outcome: &str,
    #[case] findings: Option<Value>,
) {
    let env = TestEnv::new();
    let proving_ground = consent_copy(&env, COOKIEBOT);
    proving_ground.get_with_cookie(
        &proving_ground.url(HOST, "/cookiebot/answer"),
        Some(&cookie),
    );

    let answer = proving_ground.judge(&submission("challenges/11", "success", &[]));

    assert_eq!(answer["outcome"], outcome, "{answer}");
    if let Some(findings) = findings {
        assert_eq!(answer["findings"], findings, "{answer}");
    }
}
