//! The `challenges` family on `challenges.test`: the online copy's tasks
//! (#168), one per vendor it is configured with, listed only by the
//! online copy, so the local copy's task list stays as committed. Each is
//! judged by the vendor's own verdict, read on the server side, never by
//! the tool's report: the vendor's latest verdict on the copy must be a
//! pass. A failed one refutes the task; an inconclusive one, when the
//! vendor could not say, refutes it too, since it is never a pass
//! (ADR 0064), and the answer carries the verdict, so a Live check tells
//! the two apart. A consent vendor's task asks the tool to refuse its
//! Consent prompt, the verdict being the answer the vendor's cookies
//! record. The answer also names the build's age, as the Access family's
//! first rung does.

use serde::Serialize;

use crate::access::{self, AccessAnswer};
use crate::observation::Observations;
use crate::online::{self, Online, Outcome, Vendor, Verdict};
use crate::oracle::{Answer, Finding};
use crate::task::{self, Entry, Expected, Kind, Task};

pub(crate) const FAMILY: &str = "challenges";

/// The vendors in the order of their tasks' ordinals, from 1; ordinals are
/// append-only, so a new vendor goes last.
const VENDORS: [Vendor; 13] = [
    Vendor::Turnstile,
    Vendor::Recaptcha,
    Vendor::Hcaptcha,
    Vendor::Cap,
    Vendor::Altcha,
    Vendor::Anubis,
    Vendor::CloudflareBotScore,
    Vendor::AwsWaf,
    Vendor::Botid,
    Vendor::OneTrust,
    Vendor::Cookiebot,
    Vendor::CookieYes,
    Vendor::Klaro,
];

const DESCRIPTION: &str = "Open this page in the browser, pass the vendor's challenge on it or \
    in front of it, and submit its form if it has one. The vendor's own verdict, which the \
    Proving ground reads on the server side, judges the task.";

const CONSENT_DESCRIPTION: &str = "Open this page in the browser and refuse the Consent prompt \
    it shows. The answer the vendor records in its cookies on this site, which the Proving \
    ground reads on the server side, judges the task.";

/// The online copy's tasks, one per vendor `online` is configured with, in
/// the order of their ordinals, their start URLs on its public origins when
/// it is deployed, else on `port`.
pub(crate) fn tasks(online: &Online, port: Option<u16>) -> Vec<Task> {
    let configured: Vec<Vendor> = online.vendors().collect();
    VENDORS
        .into_iter()
        .zip(1_u32..)
        .filter(|(vendor, _)| configured.contains(vendor))
        .map(|(vendor, ordinal)| Task {
            host: online::HOST.to_owned(),
            entry: Entry {
                account: None,
                time_steps: None,
                id: format!("{FAMILY}/{ordinal}"),
                family: FAMILY,
                kind: if vendor.is_consent() {
                    Kind::Act
                } else {
                    Kind::Submit
                },
                start_url: format!(
                    "{}/{}",
                    online
                        .origin(vendor)
                        .map_or_else(|| task::origin(online::HOST, port), str::to_owned),
                    vendor.name()
                ),
                description: if vendor.is_consent() {
                    CONSENT_DESCRIPTION
                } else {
                    DESCRIPTION
                },
            },
            expected: Expected::default(),
        })
        .collect()
}

/// The oracle's answer for the family: the Access family's, and the
/// vendor's verdict it was judged by, if the vendor gave one.
#[derive(Serialize)]
pub(crate) struct ChallengeAnswer {
    #[serde(flatten)]
    answer: AccessAnswer,
    verdict: Option<Verdict>,
}

/// Judges `task`'s `answer` by its vendor's latest verdict `online` read.
pub(crate) fn judge(
    task: &str,
    answer: Answer,
    online: &Online,
    observations: &Observations,
) -> ChallengeAnswer {
    let vendor = task
        .strip_prefix(FAMILY)
        .and_then(|rest| rest.strip_prefix('/'))
        .and_then(|ordinal| ordinal.parse::<usize>().ok())
        .and_then(|ordinal| VENDORS.get(ordinal.checked_sub(1)?))
        .copied()
        .expect("invariant: a challenges task's ordinal names a vendor");
    let verdict = online.latest_verdict(vendor);
    let at = match vendor {
        Vendor::Anubis => format!("/{}", vendor.name()),
        _ if vendor.is_consent() => format!("/{}/answer", vendor.name()),
        _ => format!("/{}/verify", vendor.name()),
    };
    let finding = match &verdict {
        Some(verdict) if verdict.outcome == Outcome::Passed => None,
        Some(verdict) => Some(Finding {
            check: "vendor_verdict".to_owned(),
            at,
            expected: None,
            observed: serde_json::to_value(verdict)
                .expect("invariant: a verdict serializes to JSON"),
        }),
        None => Some(Finding {
            check: "vendor_verdict".to_owned(),
            at,
            expected: None,
            observed: "no_verdict".into(),
        }),
    };
    ChallengeAnswer {
        answer: access::judge(
            task,
            answer.refuted_by(finding.into_iter().collect()),
            observations,
        ),
        verdict,
    }
}
