use super::*;

#[test]
fn the_committed_manifest_pins_two_models_of_each_served_format() {
    let manifest = Manifest::parse(include_str!("../../../test-support/fixtures/models.toml"))
        .expect("the committed manifest is in its format");

    for (format, pins) in &manifest.formats {
        let waits = matches!(pins, Pins::Waits(_));
        assert_eq!(waits, *format == Format::Gemini, "{format:?}: {pins:?}");
    }
}

#[test]
fn a_model_of_two_sizes_names_each() {
    let model = Due {
        reference: "openai/gpt-9",
        retires: Date::constant(2026, 12, 1),
        source: "https://example.com/deprecations",
        pins: vec![
            (Format::OpenAiCompatible, Size::Small),
            (Format::AnthropicMessages, Size::Larger),
        ],
    };

    let body = body(&model, Date::constant(2026, 12, 2));

    assert!(
        body.contains(
            "pins as the small model of the OpenAI-compatible format, and as the larger model of the Anthropic Messages format, retires on 2026-12-01, yesterday"
        ),
        "{body}"
    );
}
