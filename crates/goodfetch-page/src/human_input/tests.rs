#![expect(
    clippy::disallowed_methods,
    reason = "unit tests seed Entropy directly (ADR 0059)"
)]

use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;
use goodfetch_engine::entropy::Entropy;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;

use super::recordings::{CURSORY_FILE, cursory_pin, replacement_path};
use super::{
    EVENTS_PER_GESTURE, HumanInput, KeyKind, Keystroke, Point, Recordings, RecordingsError, Shapes,
    Viewport,
};

/// Replays a typing plan into a buffer, applying `Backspace` to the last
/// typed character.
fn replay(strokes: &[Keystroke]) -> String {
    let mut buffer = String::new();
    for stroke in strokes {
        match stroke.kind {
            KeyKind::Character(ch) => buffer.push(ch),
            KeyKind::Backspace => {
                buffer.pop();
            }
        }
    }
    buffer
}

const VIEWPORT: Viewport = Viewport {
    width: 1280.0,
    height: 720.0,
};

/// Cursory's format, gzip-compressed, holding `recordings`.
fn compressed(recordings: &serde_json::Value) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(recordings.to_string().as_bytes())
        .unwrap();
    encoder.finish().unwrap()
}

/// A recording of an arc from `(0, 0)` to `(length, 0)` bowing `bow`
/// pixels off the line, sampled every 16 ms, in Cursory's format.
fn arc(length: f64, bow: f64, samples: u32) -> serde_json::Value {
    let points: Vec<[f64; 2]> = (0..=samples)
        .map(|sample| {
            let t = f64::from(sample) / f64::from(samples);
            [length * t, bow * (std::f64::consts::PI * t).sin()]
        })
        .collect();
    let timing: Vec<f64> = (0..=samples)
        .map(|sample| 1000.0 + 16.0 * f64::from(sample))
        .collect();
    json!({
        "start": points[0], "end": points[points.len() - 1], "points": points,
        "timing": timing, "dx": length, "dy": 0.0, "length": length,
    })
}

/// A handful of arcs of several lengths, one with more samples than a
/// gesture's bound.
fn recordings() -> Recordings {
    let file = json!([
        arc(80.0, 12.0, 20),
        arc(300.0, -40.0, 45),
        arc(600.0, 60.0, 70),
        arc(900.0, 90.0, 160),
        arc(1400.0, -120.0, 90),
        arc(0.2, 0.0, 10),
    ]);
    Recordings::parse(&compressed(&file)).unwrap()
}

fn shapes(recorded: bool) -> Arc<Shapes> {
    Arc::new(if recorded {
        Shapes::Recorded(recordings())
    } else {
        Shapes::GhostCursor
    })
}

/// The farthest a path's points are from the straight line between its
/// ends.
fn bow(from: Point, points: &[Point]) -> f64 {
    let to = points[points.len() - 1];
    let length = from.distance(to);
    points
        .iter()
        .map(|point| {
            ((to.x - from.x) * (from.y - point.y) - (from.x - point.x) * (to.y - from.y)).abs()
                / length
        })
        .fold(0.0, f64::max)
}

fn point() -> impl Strategy<Value = Point> {
    (0.0..1280.0_f64, 0.0..720.0_f64).prop_map(|(x, y)| Point { x, y })
}

proptest! {
    /// For every seed, a path from wherever the pointer is ends exactly on
    /// its target, stays in the viewport, sends at most the gesture's
    /// bound of moves and fits the time the guard gives it, whatever the
    /// shapes; so does the next path, from where the first ended.
    ///
    /// Covers: story:8-43, setting:stealth.human_input
    #[test]
    fn a_path_ends_on_its_target_within_the_guards_deadline(
        seed: u64,
        recorded: bool,
        targets in (point(), point()),
        budget in 0..3_000_u64,
    ) {
        let budget = Duration::from_millis(budget);
        let mut input = HumanInput::new(Entropy::seeded(seed), shapes(recorded));
        for target in [targets.0, targets.1] {
            let moves = input.path_to(target, VIEWPORT, budget);

            prop_assert!(!moves.is_empty() && moves.len() <= EVENTS_PER_GESTURE);
            prop_assert_eq!(moves[moves.len() - 1].to, target);
            prop_assert!(moves.iter().map(|step| step.after).sum::<Duration>() <= budget);
            prop_assert!(moves.iter().all(|step| (0.0..VIEWPORT.width).contains(&step.to.x) && (0.0..VIEWPORT.height).contains(&step.to.y)));
            prop_assert_eq!(input.at(), Some(target));
        }
    }

    /// For every seed, a scroll's notches add up to its distance, are at
    /// most the gesture's bound, and fit the time the guard gives them.
    ///
    /// Covers: story:8-42
    #[test]
    fn a_scroll_adds_up_to_its_distance_within_the_deadline(
        seed: u64,
        delta in (-20_000.0..20_000.0_f64, -20_000.0..20_000.0_f64),
        budget in 0..3_000_u64,
    ) {
        let budget = Duration::from_millis(budget);
        let notches = HumanInput::new(Entropy::seeded(seed), shapes(false)).wheel(delta, budget);

        prop_assert!(!notches.is_empty() && notches.len() <= EVENTS_PER_GESTURE);
        let sum = notches.iter().fold((0.0, 0.0), |sum, notch| (sum.0 + notch.delta.0, sum.1 + notch.delta.1));
        prop_assert!((sum.0 - delta.0).abs() < 1e-6 && (sum.1 - delta.1).abs() < 1e-6);
        prop_assert!(notches.iter().map(|notch| notch.after).sum::<Duration>() <= budget);
    }

    /// For every seed, typing produces the requested text verbatim after
    /// any slipped-key corrections, within the guard's deadline.
    ///
    /// Covers: story:8-42
    #[test]
    fn typing_reproduces_the_exact_text_within_the_deadline(
        seed: u64,
        text in "[a-zA-Z0-9 ._-]{0,80}",
        budget in 0..5_000_u64,
    ) {
        let budget = Duration::from_millis(budget);
        let strokes = HumanInput::new(Entropy::seeded(seed), shapes(false)).typing(&text, budget);

        prop_assert_eq!(replay(&strokes), text);
        prop_assert!(strokes.iter().map(|step| step.after).sum::<Duration>() <= budget);
    }

    /// For every seed, a press-and-hold approaches the target, holds with
    /// micro-tremors inside the viewport and within the event bound, and
    /// finishes inside the guard's deadline.
    ///
    /// Covers: story:8-42
    #[test]
    fn press_and_hold_stays_on_target_within_the_deadline(
        seed: u64,
        recorded: bool,
        target in point(),
        hold in 0..2_000_u64,
        budget in 0..3_000_u64,
    ) {
        let hold = Duration::from_millis(hold);
        let budget = Duration::from_millis(budget);
        let mut input = HumanInput::new(Entropy::seeded(seed), shapes(recorded));
        let plan = input.press_and_hold(target, hold, VIEWPORT, budget);

        prop_assert!(!plan.approach.is_empty());
        prop_assert!(plan.approach.len() + plan.hold_moves.len() <= EVENTS_PER_GESTURE);
        prop_assert_eq!(plan.approach[plan.approach.len() - 1].to, target);
        let total: Duration = plan
            .approach
            .iter()
            .chain(&plan.hold_moves)
            .map(|step| step.after)
            .sum::<Duration>()
            + plan.release_after;
        prop_assert!(total <= budget);
        let in_viewport = plan.hold_moves.iter().all(|step| {
            (0.0..VIEWPORT.width).contains(&step.to.x)
                && (0.0..VIEWPORT.height).contains(&step.to.y)
        });
        prop_assert!(in_viewport);
        prop_assert_eq!(input.at(), Some(target));
    }
}

/// Typing occasionally makes a neighbouring-key slip and corrects it with
/// `Backspace` before producing the requested text.
///
/// Covers: story:8-42
#[test]
fn typing_makes_and_corrects_neighbouring_key_slips() {
    let text = "the quick brown fox jumps over the lazy dog 1234567890";
    let mut saw_slip = false;
    for seed in 0..20 {
        let strokes = HumanInput::new(Entropy::seeded(seed), shapes(false))
            .typing(text, Duration::from_secs(30));
        assert_eq!(replay(&strokes), text);
        if strokes
            .iter()
            .any(|step| matches!(step.kind, KeyKind::Backspace))
        {
            saw_slip = true;
        }
    }
    assert!(
        saw_slip,
        "expected at least one corrected slip across 20 seeds"
    );
}

/// Cursory's pinned artifact is a runtime Browser cache download of kind
/// `mouse_movements` whose replacement path sits inside the cache.
///
/// Covers: story:8-43
#[test]
fn cursory_pin_targets_mouse_movements_cache_entry() {
    let pin = cursory_pin();
    assert_eq!(
        pin.id.kind,
        goodfetch_protocol::browser_cache::ArtifactKind::MouseMovements
    );
    assert!(pin.url.starts_with("https://"));
    assert!(pin.url.ends_with(CURSORY_FILE));
    assert!(pin.archive_digest.is_some());
    assert!(
        replacement_path(std::path::Path::new("/cache"))
            .ends_with("mouse_movements/cursory/trajectories.json.gz")
    );
}

/// A long move is a curve, not a straight line, from the recordings and
/// from the ghost-cursor port alike, and takes some time.
///
/// Covers: story:8-43
#[rstest]
fn a_long_move_is_not_a_straight_line(#[values(true, false)] recorded: bool) {
    let from = Point { x: 100.0, y: 600.0 };
    let to = Point { x: 900.0, y: 200.0 };
    for seed in 0..20 {
        let mut input = HumanInput::new(Entropy::seeded(seed), shapes(recorded));
        input.path_to(from, VIEWPORT, Duration::from_secs(2));

        let moves = input.path_to(to, VIEWPORT, Duration::from_secs(2));

        let points: Vec<Point> = moves.iter().map(|step| step.to).collect();
        assert!(bow(from, &points) > 5.0, "seed {seed}: {points:?}");
        assert!(moves.len() > 5, "seed {seed}");
        assert!(moves.iter().map(|step| step.after).sum::<Duration>() > Duration::from_millis(100));
    }
}

/// One seed gives one path; another seed gives another.
#[rstest]
fn a_seed_gives_one_path(#[values(true, false)] recorded: bool) {
    let path = |seed| {
        HumanInput::new(Entropy::seeded(seed), shapes(recorded)).path_to(
            Point { x: 640.0, y: 360.0 },
            VIEWPORT,
            Duration::from_secs(1),
        )
    };

    assert_eq!(path(7), path(7));
    assert_ne!(path(7), path(8));
}

/// A file that is not Cursory's gzip-compressed JSON, or holds no usable
/// movement, is refused, and the Run falls back on the ghost-cursor port.
#[rstest]
#[case::not_gzip(b"[]".to_vec(), "Unreadable")]
#[case::not_the_format(compressed(&json!({ "points": [] })), "Format")]
#[case::no_usable_movement(compressed(&json!([arc(0.2, 0.0, 10), { "points": [[0, 0], [5, 5]], "timing": [0] }])), "Empty")]
fn unusable_recordings_are_refused(#[case] file: Vec<u8>, #[case] expected: &str) {
    let error = Recordings::parse(&file).unwrap_err();

    let kind = match error {
        RecordingsError::Unreadable(_) => "Unreadable",
        RecordingsError::Format(_) => "Format",
        RecordingsError::Empty => "Empty",
    };
    assert_eq!(kind, expected);
}
