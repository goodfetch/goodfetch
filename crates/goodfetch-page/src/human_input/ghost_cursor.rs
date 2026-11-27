//! A Rust port of ghost-cursor's path (MIT, Xetera's `ghost-cursor`,
//! `src/math.ts` and `src/spoof.ts` at 5525a783fcf833a60f1b0746a8f80653ccfee2ba):
//! a cubic Bézier curve whose two anchors lie on one random side of the
//! straight line, sampled at a count of steps Fitts's law sets, with an
//! overshoot past a far target and a correction back onto it. Ghost-cursor
//! sends its moves back to back; their timing here is Goodfetch's own, a
//! duration that grows with the distance as Fitts's law has it, spread
//! across the moves.

use goodfetch_engine::entropy::Entropy;

use super::{Move, Point, between, millis, unit, whole};

/// The distance past which a path overshoots its target first.
const OVERSHOOT_PAST: f64 = 500.0;

/// How far from the target an overshoot ends.
const OVERSHOOT_RADIUS: f64 = 120.0;

/// The spread of the correction back from an overshoot.
const CORRECTION_SPREAD: f64 = 10.0;

/// The target width Fitts's law is taken at when none is known.
const DEFAULT_WIDTH: f64 = 100.0;

/// The moves from `from` to `to`, the last one on `to`.
pub(super) fn path(entropy: &mut Entropy, from: Point, to: Point) -> Vec<Move> {
    if from.distance(to) > OVERSHOOT_PAST {
        let angle = unit(entropy) * std::f64::consts::TAU;
        let radius = unit(entropy) * OVERSHOOT_RADIUS;
        let past = Point {
            x: to.x + angle.cos() * radius,
            y: to.y + angle.sin() * radius,
        };
        let mut moves = curve(entropy, from, past, None);
        moves.extend(curve(entropy, past, to, Some(CORRECTION_SPREAD)));
        moves
    } else {
        curve(entropy, from, to, None)
    }
}

/// One curve's moves from `from` to `to`, its anchors `spread` from the
/// line, or as far as the distance, between 2 and 200 pixels.
fn curve(entropy: &mut Entropy, from: Point, to: Point, spread: Option<f64>) -> Vec<Move> {
    let distance = from.distance(to);
    let spread = spread.unwrap_or_else(|| distance.clamp(2.0, 200.0));
    let side = if unit(entropy) < 0.5 { -1.0 } else { 1.0 };
    let mut anchors = [
        anchor(entropy, from, to, spread * side),
        anchor(entropy, from, to, spread * side),
    ];
    anchors.sort_by(|a, b| a.x.total_cmp(&b.x));
    let controls = [from, anchors[0], anchors[1], to];
    let length = curve_length(&controls);
    let fitts = 2.0 * (length * 0.8 / DEFAULT_WIDTH + 1.0).log2();
    let base = unit(entropy) * 25.0;
    let steps = whole((((fitts + 1.0).log2() + base) * 3.0).ceil()).max(1);
    let duration = (120.0 + 140.0 * (distance / 100.0 + 1.0).log2()) * between(entropy, (0.8, 1.2));
    let each = millis(duration / f64::from(steps));
    (1..=steps)
        .map(|step| Move {
            to: bezier(&controls, f64::from(step) / f64::from(steps)),
            after: each,
        })
        .collect()
}

/// An anchor: a random point of the line from `a` to `b`, moved off it
/// perpendicularly by up to `spread` (negative for the other side).
fn anchor(entropy: &mut Entropy, a: Point, b: Point, spread: f64) -> Point {
    let along = unit(entropy);
    let middle = Point {
        x: a.x + (b.x - a.x) * along,
        y: a.y + (b.y - a.y) * along,
    };
    let (dx, dy) = (middle.x - a.x, middle.y - a.y);
    let length = dx.hypot(dy);
    let (px, py) = if length > 0.0 {
        (-dy / length * spread, dx / length * spread)
    } else {
        (0.0, spread)
    };
    let off = unit(entropy);
    Point {
        x: middle.x + px * off,
        y: middle.y + py * off,
    }
}

/// The cubic Bézier curve of `controls` at `t`.
fn bezier(controls: &[Point; 4], t: f64) -> Point {
    let inv = 1.0 - t;
    let (w0, w1, w2, w3) = (
        inv * inv * inv,
        3.0 * inv * inv * t,
        3.0 * inv * t * t,
        t * t * t,
    );
    Point {
        x: w0 * controls[0].x + w1 * controls[1].x + w2 * controls[2].x + w3 * controls[3].x,
        y: w0 * controls[0].y + w1 * controls[1].y + w2 * controls[2].y + w3 * controls[3].y,
    }
}

/// The curve's length, from 32 straight pieces of it.
fn curve_length(controls: &[Point; 4]) -> f64 {
    let pieces = 32_u32;
    let mut length = 0.0;
    let mut previous = controls[0];
    for piece in 1..=pieces {
        let point = bezier(controls, f64::from(piece) / f64::from(pieces));
        length += previous.distance(point);
        previous = point;
    }
    length
}
