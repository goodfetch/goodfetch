//! What a crawl's Run requires of its Sinks' Guarantee classes (ADR 0026):
//! `crawl.min_sink_guarantee` names the weakest class it accepts, and a
//! Sink whose probe proves a weaker one, or fails, refuses the Run's start
//! with `sink_guarantee_unmet`.

use goodfetch_engine::{CallError, named};
use goodfetch_protocol::foxhound::sink::{
    CRAWL_MIN_SINK_GUARANTEE, SINK_GUARANTEE_UNMET, SinkGuaranteeUnmetDetails,
};

use crate::contract::GuaranteeClass;

/// Whether `proved` is at least as strong as `required`.
#[must_use]
pub fn meets(proved: GuaranteeClass, required: GuaranteeClass) -> bool {
    strength(proved) >= strength(required)
}

/// Refuses the Sink `sink`, whose probe answered `answer`, when the Run
/// requires `minimum` of it; accepts any Sink when it requires none.
///
/// # Errors
///
/// `sink_guarantee_unmet`, naming the Sink, the class required, the class
/// proved or the probe's failure's cause.
pub fn require(
    minimum: Option<GuaranteeClass>,
    sink: &str,
    answer: Result<GuaranteeClass, String>,
) -> Result<(), CallError> {
    let Some(required) = minimum else {
        return Ok(());
    };
    let (proved, cause, found) = match answer {
        Ok(proved) if meets(proved, required) => return Ok(()),
        Ok(proved) => (
            Some(proved),
            None,
            format!("its probe proved `{}`", wire_name(proved)),
        ),
        Err(cause) => {
            let found = format!("its probe failed: {cause}");
            (None, Some(cause), found)
        }
    };
    Err(CallError::new(
        &SINK_GUARANTEE_UNMET,
        format!(
            "The Sink `{sink}` cannot meet `{}`'s `{}`, so the Run was not started: {found}. Deliver to a destination that proves `{}`, or lower `{}`.",
            named::setting(CRAWL_MIN_SINK_GUARANTEE.name),
            wire_name(required),
            wire_name(required),
            named::setting(CRAWL_MIN_SINK_GUARANTEE.name),
        ),
        &SinkGuaranteeUnmetDetails {
            sink: sink.to_owned(),
            required,
            proved,
            cause,
        },
    ))
}

/// How strong `class` is: the higher, the stronger.
fn strength(class: GuaranteeClass) -> u8 {
    match class {
        GuaranteeClass::AtLeastOnce => 0,
        GuaranteeClass::Keyed => 1,
        GuaranteeClass::RecordFenced => 2,
        GuaranteeClass::Fenced => 3,
    }
}

/// The wire name of `class`.
fn wire_name(class: GuaranteeClass) -> &'static str {
    match class {
        GuaranteeClass::Fenced => "fenced",
        GuaranteeClass::RecordFenced => "record_fenced",
        GuaranteeClass::Keyed => "keyed",
        GuaranteeClass::AtLeastOnce => "at_least_once",
    }
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::foxhound::sink::SinkGuaranteeUnmetDetails;

    use super::{GuaranteeClass, meets, require};

    const ALL: [GuaranteeClass; 4] = [
        GuaranteeClass::Fenced,
        GuaranteeClass::RecordFenced,
        GuaranteeClass::Keyed,
        GuaranteeClass::AtLeastOnce,
    ];

    #[test]
    fn a_class_meets_itself_and_every_weaker_one_only() {
        for (strong, proved) in ALL.iter().enumerate() {
            for (weak, required) in ALL.iter().enumerate() {
                assert_eq!(
                    meets(*proved, *required),
                    strong <= weak,
                    "{proved:?} against {required:?}"
                );
            }
        }
    }

    #[test]
    fn a_run_without_a_minimum_accepts_a_failed_probe() {
        assert!(require(None, "archive", Err("down".to_owned())).is_ok());
    }

    /// A Keyed Sink under a Fenced minimum is refused, the refusal naming
    /// the Sink and both classes.
    ///
    /// Covers: story:6-284
    #[test]
    fn a_keyed_sink_under_a_fenced_minimum_is_refused_naming_it() {
        let error = require(
            Some(GuaranteeClass::Fenced),
            "archive",
            Ok(GuaranteeClass::Keyed),
        )
        .expect_err("a Keyed Sink is below Fenced");
        assert_eq!(error.code(), "sink_guarantee_unmet");
        let details = error.details().cloned().expect("a code carries details");
        let details: SinkGuaranteeUnmetDetails =
            serde_json::from_value(serde_json::Value::Object(details))
                .expect("the details are the code's");
        assert_eq!(
            details,
            SinkGuaranteeUnmetDetails {
                sink: "archive".to_owned(),
                required: GuaranteeClass::Fenced,
                proved: Some(GuaranteeClass::Keyed),
                cause: None,
            }
        );
    }

    #[test]
    fn a_failed_probe_under_a_minimum_is_refused_with_its_cause() {
        let error = require(
            Some(GuaranteeClass::AtLeastOnce),
            "archive",
            Err("its SQLite database did not open".to_owned()),
        )
        .expect_err("a failed probe proves nothing");
        assert!(error.message().contains("its SQLite database did not open"));
        let details = error.details().expect("a code carries details");
        assert_eq!(
            details.get("cause"),
            Some(&serde_json::json!("its SQLite database did not open"))
        );
        assert_eq!(details.get("proved"), None);
    }
}
