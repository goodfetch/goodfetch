use rstest::rstest;
use serde_json::json;

use super::{
    ScheduleDefinition, ScheduleId, SchedulePreviewParams, ScheduleTarget, ScheduleTrigger,
};

#[rstest]
#[case::one_letter("a")]
#[case::digits_and_dashes("nightly-prices-2")]
#[case::made("s01jd8x0v4k3m2n1p0q9r8s7t6")]
#[case::longest(&"a".repeat(63))]
fn a_schedule_id_reads_lowercase_letters_digits_and_inner_dashes(#[case] text: &str) {
    assert_eq!(
        ScheduleId::parse(text).map(|id| id.as_str().to_owned()),
        Ok(text.to_owned())
    );
}

#[rstest]
#[case::empty("")]
#[case::upper_case("Nightly")]
#[case::leading_dash("-nightly")]
#[case::trailing_dash("nightly-")]
#[case::dot("nightly.prices")]
#[case::too_long(&"a".repeat(64))]
fn a_schedule_id_refuses_any_other_text(#[case] text: &str) {
    assert!(
        ScheduleId::parse(text).is_err(),
        "{text:?} read as a Schedule ID"
    );
}

#[test]
fn a_definition_with_an_unknown_field_is_refused() {
    let read = serde_json::from_value::<ScheduleDefinition>(json!({
        "target": { "recipe": "./prices.recipe.ts" },
        "trigger": { "every": 60_000 },
        "colour": "blue",
    }));

    let error = read.expect_err("an unknown field was read");
    assert!(error.to_string().contains("colour"), "{error}");
}

#[test]
fn a_definition_reads_its_target_and_every_trigger_as_given() {
    let definition: ScheduleDefinition = serde_json::from_value(json!({
        "id": "prices",
        "target": { "recipe": "./prices.recipe.ts", "inputs": { "city": "Lyon" }, "timeout": null },
        "trigger": { "every": 3_600_000, "anchor": "2026-11-25T09:00:00Z" },
        "time_zone": "Europe/Paris",
    }))
    .expect("a definition");

    assert!(matches!(
        &definition.target,
        ScheduleTarget::Recipe(target) if target.recipe == "./prices.recipe.ts" && target.timeout == Some(None)
    ));
    assert!(matches!(
        &definition.trigger,
        ScheduleTrigger::Every(every) if every.every == 3_600_000 && every.anchor.is_some()
    ));
    assert_eq!(
        serde_json::to_value(&definition).expect("written")["trigger"]["anchor"],
        json!("2026-11-25T09:00:00Z")
    );
}

#[test]
fn a_definition_reads_a_cron_trigger_and_preview_params_round_trip() {
    let definition: ScheduleDefinition = serde_json::from_value(json!({
        "target": { "recipe": "./prices.recipe.ts" },
        "trigger": { "cron": "0 9 * * MON-FRI" },
        "time_zone": "Europe/Paris",
    }))
    .expect("cron definition");

    assert!(matches!(
        &definition.trigger,
        ScheduleTrigger::Cron(cron) if cron.cron == "0 9 * * MON-FRI"
    ));

    let preview: SchedulePreviewParams = serde_json::from_value(json!({
        "trigger": { "cron": "30 2 * * *" },
        "time_zone": "Europe/Paris",
        "after": "2026-03-29T00:00:00Z",
        "count": 3,
    }))
    .expect("preview params");
    assert_eq!(preview.time_zone.as_deref(), Some("Europe/Paris"));
    assert!(preview.after.is_some());
    assert_eq!(preview.count, Some(3));
}

#[test]
fn schedule_setting_shapes_validate_and_reject_invalid_values() {
    use super::{
        SCHEDULE_CATCH_UP_SHAPE, SCHEDULE_PAUSE_TIMEOUT_SHAPE, SCHEDULE_PERIOD_LIMITS_SHAPE,
        ScheduleCapExceededDetails, ScheduleCapKind, ScheduleCatchUp, SchedulePauseTimeout,
        SchedulePeriodLimits,
    };

    assert_eq!(
        ScheduleCatchUp::read(&json!("until_next")),
        Ok(ScheduleCatchUp::UntilNext)
    );
    assert_eq!(
        ScheduleCatchUp::read(&json!(0)),
        Ok(ScheduleCatchUp::Duration(0))
    );
    assert_eq!(
        ScheduleCatchUp::read(&json!(60_000)),
        Ok(ScheduleCatchUp::Duration(60_000))
    );
    assert!(ScheduleCatchUp::read(&json!("always")).is_err());
    assert!(ScheduleCatchUp::read(&json!(-1)).is_err());
    for example in (SCHEDULE_CATCH_UP_SHAPE.examples)() {
        assert!((SCHEDULE_CATCH_UP_SHAPE.check)(&example).is_ok());
    }

    assert_eq!(
        SchedulePauseTimeout::read(&json!("next_occurrence")),
        Ok(SchedulePauseTimeout::NextOccurrence)
    );
    assert!(SchedulePauseTimeout::read(&json!(1_000)).is_ok());
    assert!(SchedulePauseTimeout::read(&json!(0)).is_err());
    assert!(SchedulePauseTimeout::read(&json!("never")).is_err());
    for example in (SCHEDULE_PAUSE_TIMEOUT_SHAPE.examples)() {
        assert!((SCHEDULE_PAUSE_TIMEOUT_SHAPE.check)(&example).is_ok());
    }

    for example in (SCHEDULE_PERIOD_LIMITS_SHAPE.examples)() {
        assert!((SCHEDULE_PERIOD_LIMITS_SHAPE.check)(&example).is_ok());
    }
    assert!(
        SchedulePeriodLimits::read(&json!({
            "day": [
                { "usage_kind": "pages", "amount": 100 },
                { "usage_kind": "pages", "amount": 200 }
            ]
        }))
        .is_err()
    );
    assert!(
        SchedulePeriodLimits::read(&json!({
            "day": [{ "usage_kind": "pages", "currency": "usd", "amount": 100 }]
        }))
        .is_err()
    );
    assert!(
        SchedulePeriodLimits::read(&json!({
            "day": [{ "usage_kind": "", "amount": 100 }]
        }))
        .is_err()
    );

    let details = ScheduleCapExceededDetails {
        cap: ScheduleCapKind::PeriodLimit,
        policy: "schedule.limits_max".to_owned(),
        policy_version: Some("2026.1".to_owned()),
    };
    let encoded = serde_json::to_value(&details).unwrap();
    assert_eq!(encoded["cap"], json!("period_limit"));
    assert_eq!(encoded["policy"], json!("schedule.limits_max"));
    assert_eq!(encoded["policy_version"], json!("2026.1"));
}
