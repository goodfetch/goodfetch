use rstest::rstest;
use serde_json::json;

use super::{ScheduleDefinition, ScheduleId, ScheduleTarget, ScheduleTrigger};

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
