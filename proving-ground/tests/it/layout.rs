//! The `responsive` family (#1536): a layout whose plan changes at viewport
//! widths and whose density changes at device pixel ratios, by media
//! queries alone, with an image the browser picks by its device pixel
//! ratio. The layout read in each task's viewport is verified; one read in
//! another viewport is a false success, which the image the Proving ground
//! saw requested also refutes.
//! Covers: story:2-33

use rstest::rstest;
use serde_json::json;

use crate::browser;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 1536;

#[rstest]
#[case::phone("responsive/1", 390, 844, 3)]
#[case::tablet("responsive/2", 820, 1180, 2)]
#[case::desktop("responsive/3", 1440, 900, 1)]
fn the_layout_read_in_its_tasks_viewport_is_verified(
    #[case] task: &str,
    #[case] width: u32,
    #[case] height: u32,
    #[case] device_scale_factor: u32,
) {
    let proving_ground = ProvingGround::start(SEED);
    let context = json!({
        "viewport": { "width": width, "height": height },
        "deviceScaleFactor": device_scale_factor,
    });

    let answer = browser::judged(
        &proving_ground,
        task,
        "layout",
        json!({ "context": context }),
    );

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

#[test]
fn the_layout_read_in_a_desktop_viewport_is_a_false_success_for_the_phone_task() {
    let proving_ground = ProvingGround::start(SEED);
    let context = json!({ "viewport": { "width": 1440, "height": 900 }, "deviceScaleFactor": 1 });

    let answer = browser::judged(
        &proving_ground,
        "responsive/1",
        "layout",
        json!({ "context": context }),
    );

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    let wrong: Vec<_> = answer["wrong"]
        .as_array()
        .expect("`wrong` is an array")
        .iter()
        .map(|wrong| (wrong["field"].clone(), wrong["submitted"].clone()))
        .collect();
    assert_eq!(wrong[0], (json!("breakpoint"), json!("wide")), "{answer}");
    assert_eq!(wrong[1], (json!("density"), json!("1x")), "{answer}");
    assert_eq!(wrong[2].0, json!("plan"), "{answer}");
    assert_eq!(
        answer["findings"],
        json!([{
            "check": "density_image",
            "at": "/layout",
            "expected": "/dot-3x.svg",
            "observed": "/dot-1x.svg",
        }])
    );
}

#[test]
fn the_layout_page_holds_every_breakpoints_plan_and_every_density() {
    let proving_ground = ProvingGround::start(SEED);

    let page = proving_ground
        .get(&browser::start_url(&proving_ground, "responsive/1"))
        .body;

    for shown in [
        "data-breakpoint=\"narrow\"",
        "data-breakpoint=\"medium\"",
        "data-breakpoint=\"wide\"",
    ] {
        assert!(page.contains(shown), "{page}");
    }
    assert!(
        page.contains("srcset=\"/dot-1x.svg 1x, /dot-2x.svg 2x, /dot-3x.svg 3x\""),
        "{page}"
    );
}
