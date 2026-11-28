//! Human-like input (ADR 0035): the paths, keystrokes, wheel notches and
//! press-and-hold motions a `stealth` Session's own gestures are sent with
//! when `stealth.human_input` is on. A pointer path is one of Cursory's
//! recorded movements reshaped by Goodfetch's own step to end exactly on
//! the target ([`Recordings`]), or, when the recordings are not in the
//! Browser cache, a curve of the Rust port of ghost-cursor. Typing,
//! scrolling and press-and-hold use Goodfetch's own model. Every random
//! choice comes from the page's own `Entropy`, so a seed gives one path; a
//! path never changes what a gesture targets or the value the guard checks,
//! and fits the time the guard gives it, with at most [`EVENTS_PER_GESTURE`]
//! moves. Human-like input applies only to the engine's own actions; a
//! library attached to a Lent page keeps its own input (ADR 0089).

mod ghost_cursor;
mod recordings;
#[cfg(test)]
mod tests;
mod typing;

use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::browser_cache::HeldArtifact;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::Home;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::RunWarning;
use goodfetch_protocol::run::warning::HUMAN_INPUT_FALLBACK;
use serde_json::json;
use tokio::sync::Mutex;

use crate::browser_run::BrowserRun;
use crate::trace;

pub(crate) use self::recordings::Recordings;
#[cfg(test)]
pub(crate) use self::recordings::RecordingsError;
pub use self::recordings::{CURSORY_NAME, cursory_pin};
pub(crate) use self::typing::{KeyKind, Keystroke};

/// The Provenance action of a `stealth` Session's human-like input choice.
const HUMAN_INPUT_ACTION: &str = "human_input";

/// The Provenance scope stating human-like input applies only to the
/// engine's own actions, never to a library attached to a Lent page
/// (ADR 0089).
pub(crate) const ENGINE_ACTIONS_ONLY: &str = "engine_actions_only";

/// The most input events one gesture's path, scroll or press-and-hold sends:
/// the bound of human-like input's own action kind on `C3.commands` (ADR
/// 0140, #1018's Shared paths decision), until B2's first measure of it
/// sets the figure.
pub(crate) const EVENTS_PER_GESTURE: usize = 100;

/// How far a page's first path starts from its target, in CSS pixels: the
/// pointer was somewhere on the page before the first gesture.
const FIRST_START: (f64, f64) = (150.0, 450.0);

/// The pixels one wheel notch scrolls, as a mouse's does.
const NOTCH: f64 = 100.0;

/// The pause between two notches, in milliseconds.
const NOTCH_PAUSE: (f64, f64) = (30.0, 90.0);

/// How long between two subtle hand tremors while holding a button down,
/// in milliseconds.
const TREMOR_INTERVAL: (f64, f64) = (80.0, 180.0);

/// How far a hand tremor drifts from the press point, in CSS pixels.
const TREMOR_RADIUS: f64 = 1.5;

/// How many approach moves a press-and-hold reserves out of
/// [`EVENTS_PER_GESTURE`] so tremor moves still fit under the bound.
const HOLD_APPROACH_CAP: usize = 60;

/// A point of the page's viewport, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl Point {
    fn distance(self, to: Self) -> f64 {
        (to.x - self.x).hypot(to.y - self.y)
    }
}

/// The page's viewport, which every point of a path stays in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Viewport {
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl Viewport {
    fn clamp(self, point: Point) -> Point {
        Point {
            x: point.x.clamp(0.0, (self.width - 1.0).max(0.0)),
            y: point.y.clamp(0.0, (self.height - 1.0).max(0.0)),
        }
    }
}

/// One move of a path: where the pointer goes, once `after` has passed
/// since the move before it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Move {
    pub(crate) to: Point,
    pub(crate) after: Duration,
}

/// One notch of a scroll: the wheel's delta, once `after` has passed since
/// the notch before it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Notch {
    pub(crate) delta: (f64, f64),
    pub(crate) after: Duration,
}

/// One press-and-hold gesture: approach moves onto the target, subtle
/// micro-tremor moves while the button stays down, and the remaining wait
/// before release. Total moves (`approach.len() + hold_moves.len()`) stay
/// within [`EVENTS_PER_GESTURE`] and total duration within `budget`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HoldPlan {
    pub(crate) approach: Vec<Move>,
    pub(crate) hold_moves: Vec<Move>,
    pub(crate) release_after: Duration,
}

/// Where a Run's pointer paths take their shapes from.
#[derive(Debug)]
pub(crate) enum Shapes {
    /// Cursory's recordings, from the Browser cache.
    Recorded(Recordings),
    /// The ghost-cursor port, when the recordings are not there.
    GhostCursor,
}

impl Shapes {
    /// The source name recorded in Provenance.
    pub(crate) const fn source_name(&self) -> &'static str {
        match self {
            Self::Recorded(_) => "cursory",
            Self::GhostCursor => "ghost_cursor",
        }
    }

    /// Whether these shapes fell back to the ghost-cursor port.
    pub(crate) const fn is_fallback(&self) -> bool {
        matches!(self, Self::GhostCursor)
    }
}

/// A `stealth` Session's human-like input plan: the shapes its pages share
/// and the Browser cache hold that keeps Cursory's recordings on disk
/// while the Session lives.
#[derive(Clone)]
pub(crate) struct Plan {
    shapes: Arc<Shapes>,
    _held: Option<Arc<HeldArtifact>>,
}

impl Plan {
    /// Opens a human-like input plan for `run`, loading Cursory's
    /// recordings from the Browser cache under `home` or falling back to
    /// the ghost-cursor port when absent or offline.
    pub(crate) async fn open(home: &Home, run: &RunIdentity, pinned_by: &PinnedBy) -> Self {
        match recordings::load(home, run, pinned_by).await {
            Some(loaded) => Self {
                shapes: Arc::new(Shapes::Recorded(loaded.recordings)),
                _held: loaded.held.map(Arc::new),
            },
            None => Self {
                shapes: Arc::new(Shapes::GhostCursor),
                _held: None,
            },
        }
    }

    /// Records the Session's human-like input choice in Provenance, and
    /// emits `human_input_fallback` in the Run report when Cursory's
    /// recordings were absent and the ghost-cursor port is used.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may lose the Provenance or warning write not yet
    /// completed.
    pub(crate) async fn chosen(&self, run: &BrowserRun, session: &SessionId) {
        trace::provenance(
            &run.trace,
            json!({
                "action": HUMAN_INPUT_ACTION,
                "session_id": session,
                "source": self.shapes.source_name(),
                "fell_back": self.shapes.is_fallback(),
                "scope": ENGINE_ACTIONS_ONLY,
            }),
        )
        .await;
        if self.shapes.is_fallback() {
            let warning = RunWarning {
                code: HUMAN_INPUT_FALLBACK.to_owned(),
                message: "Cursory's mouse-movement recordings were not in the Browser cache: human-like input fell back to the ghost-cursor port.".to_owned(),
                sessions: vec![session.0.clone()],
                folder: None,
            };
            if let Err(error) = run.warnings.warn(warning).await {
                tracing::warn!(%error, "the warning of a Session's human-like input fallback was not kept");
            }
        }
    }

    /// Creates a page's human-like input handle seeded by `entropy`,
    /// sharing this Session's shapes.
    pub(crate) fn for_page(&self, entropy: Entropy) -> PageHumanInput {
        PageHumanInput::new(entropy, Arc::clone(&self.shapes))
    }
}

/// Shared handle to one page's [`HumanInput`], locked only long enough to
/// compute pure movement/typing/scroll plans, never across an `.await`.
#[derive(Clone, Debug)]
pub(crate) struct PageHumanInput(Arc<Mutex<HumanInput>>);

impl PageHumanInput {
    pub(crate) fn new(entropy: Entropy, shapes: Arc<Shapes>) -> Self {
        Self(Arc::new(Mutex::new(HumanInput::new(entropy, shapes))))
    }

    pub(crate) async fn path_to(
        &self,
        to: Point,
        viewport: Viewport,
        budget: Duration,
    ) -> Vec<Move> {
        self.0.lock().await.path_to(to, viewport, budget)
    }

    pub(crate) async fn wheel(&self, delta: (f64, f64), budget: Duration) -> Vec<Notch> {
        self.0.lock().await.wheel(delta, budget)
    }

    pub(crate) async fn typing(&self, text: &str, budget: Duration) -> Vec<Keystroke> {
        self.0.lock().await.typing(text, budget)
    }

    pub(crate) async fn press_and_hold(
        &self,
        to: Point,
        hold: Duration,
        viewport: Viewport,
        budget: Duration,
    ) -> HoldPlan {
        self.0
            .lock()
            .await
            .press_and_hold(to, hold, viewport, budget)
    }
}

/// The human-like input of one page: its entropy, where its paths take
/// their shapes from, and where its pointer is.
#[derive(Debug)]
pub(crate) struct HumanInput {
    entropy: Entropy,
    shapes: Arc<Shapes>,
    at: Option<Point>,
}

impl HumanInput {
    pub(crate) fn new(entropy: Entropy, shapes: Arc<Shapes>) -> Self {
        Self {
            entropy,
            shapes,
            at: None,
        }
    }

    /// Where the pointer is, once a path moved it.
    #[cfg(test)]
    pub(crate) fn at(&self) -> Option<Point> {
        self.at
    }

    /// The moves from the pointer to `to` within `viewport`, whose delays
    /// add up to `budget` at most, the last one exactly on `to`; the
    /// pointer is then at `to`.
    pub(crate) fn path_to(&mut self, to: Point, viewport: Viewport, budget: Duration) -> Vec<Move> {
        self.path_to_capped(to, viewport, budget, EVENTS_PER_GESTURE)
    }

    fn path_to_capped(
        &mut self,
        to: Point,
        viewport: Viewport,
        budget: Duration,
        cap: usize,
    ) -> Vec<Move> {
        let from = self.at.unwrap_or_else(|| self.first_start(to, viewport));
        let moves = if from.distance(to) < 2.0 {
            vec![Move {
                to,
                after: Duration::ZERO,
            }]
        } else {
            match &*self.shapes {
                Shapes::Recorded(recordings) => recordings.path(&mut self.entropy, from, to),
                Shapes::GhostCursor => ghost_cursor::path(&mut self.entropy, from, to),
            }
        };
        self.at = Some(to);
        fitted(moves, to, viewport, budget, cap)
    }

    /// The notches that scroll the page by `delta`, whose delays add up to
    /// `budget` at most.
    pub(crate) fn wheel(&mut self, delta: (f64, f64), budget: Duration) -> Vec<Notch> {
        let longest = delta.0.abs().max(delta.1.abs());
        let count = whole((longest / NOTCH).ceil()).clamp(1, bound());
        let share = f64::from(count);
        let mut notches: Vec<Notch> = (0..count)
            .map(|_| Notch {
                delta: (delta.0 / share, delta.1 / share),
                after: millis(between(&mut self.entropy, NOTCH_PAUSE)),
            })
            .collect();
        scale_delays(&mut notches, |notch| &mut notch.after, budget);
        notches
    }

    /// The keystrokes that type `text` with human pacing and occasional
    /// corrected neighbouring-key slips, their delays adding up to at most
    /// `budget`.
    pub(crate) fn typing(&mut self, text: &str, budget: Duration) -> Vec<Keystroke> {
        typing::plan(&mut self.entropy, text, budget)
    }

    /// A press-and-hold on `to` held for `hold` inside `viewport`, its
    /// approach moves and micro-tremor moves together staying within
    /// [`EVENTS_PER_GESTURE`] and its total duration within `budget`.
    pub(crate) fn press_and_hold(
        &mut self,
        to: Point,
        hold: Duration,
        viewport: Viewport,
        budget: Duration,
    ) -> HoldPlan {
        let hold_duration = hold.min(budget);
        let approach_budget = budget.saturating_sub(hold_duration);
        let approach = self.path_to_capped(to, viewport, approach_budget, HOLD_APPROACH_CAP);
        let approach_time: Duration = approach.iter().map(|step| step.after).sum();
        let remaining_hold = hold_duration.min(budget.saturating_sub(approach_time));
        let tremor_cap = EVENTS_PER_GESTURE.saturating_sub(approach.len());
        let mut hold_moves = Vec::new();
        let mut elapsed = Duration::ZERO;
        while hold_moves.len() < tremor_cap {
            let step = millis(between(&mut self.entropy, TREMOR_INTERVAL));
            if step.is_zero() || elapsed + step >= remaining_hold {
                break;
            }
            elapsed += step;
            let angle = unit(&mut self.entropy) * std::f64::consts::TAU;
            let radius = unit(&mut self.entropy) * TREMOR_RADIUS;
            let point = viewport.clamp(Point {
                x: to.x + angle.cos() * radius,
                y: to.y + angle.sin() * radius,
            });
            hold_moves.push(Move {
                to: point,
                after: step,
            });
        }
        let release_after = remaining_hold.saturating_sub(elapsed);
        self.at = Some(to);
        HoldPlan {
            approach,
            hold_moves,
            release_after,
        }
    }

    /// Where a page's first path starts: a point of the viewport some way
    /// from `to`, in a random direction.
    fn first_start(&mut self, to: Point, viewport: Viewport) -> Point {
        let angle = unit(&mut self.entropy) * std::f64::consts::TAU;
        let length = between(&mut self.entropy, FIRST_START);
        viewport.clamp(Point {
            x: to.x + angle.cos() * length,
            y: to.y + angle.sin() * length,
        })
    }
}

/// `moves` within `viewport`, thinned to `cap` with the last kept, ending
/// exactly on `to`, their delays scaled down to fit `budget`.
fn fitted(
    mut moves: Vec<Move>,
    to: Point,
    viewport: Viewport,
    budget: Duration,
    cap: usize,
) -> Vec<Move> {
    let cap = cap.max(1);
    if moves.len() > cap {
        let mut kept = Vec::with_capacity(cap);
        let mut pending = Duration::ZERO;
        let count = moves.len();
        for (index, step) in moves.into_iter().enumerate() {
            pending += step.after;
            let slot = (index + 1) * cap / count;
            if slot > kept.len() {
                kept.push(Move {
                    to: step.to,
                    after: pending,
                });
                pending = Duration::ZERO;
            }
        }
        moves = kept;
    }
    for step in &mut moves {
        step.to = viewport.clamp(step.to);
    }
    match moves.last_mut() {
        Some(last) => last.to = to,
        None => moves.push(Move {
            to,
            after: Duration::ZERO,
        }),
    }
    scale_delays(&mut moves, |step| &mut step.after, budget);
    moves
}

/// Scales the delays of `items` down proportionally with integer floor
/// division when their sum exceeds `budget`, so the total never exceeds
/// `budget` even by one nanosecond.
pub(super) fn scale_delays<T>(
    items: &mut [T],
    mut delay: impl FnMut(&mut T) -> &mut Duration,
    budget: Duration,
) {
    let total: u128 = items.iter_mut().map(|item| delay(item).as_nanos()).sum();
    let budget_ns = budget.as_nanos();
    if total > budget_ns && total > 0 {
        for item in items {
            let slot = delay(item);
            let scaled = slot.as_nanos().saturating_mul(budget_ns) / total;
            *slot = Duration::from_nanos(u64::try_from(scaled).unwrap_or(u64::MAX));
        }
    }
}

/// [`EVENTS_PER_GESTURE`] as a count of notches.
fn bound() -> u32 {
    u32::try_from(EVENTS_PER_GESTURE).expect("invariant: the bound fits a u32")
}

/// A uniform draw in `[0, 1)`, from 52 bits of `entropy`.
fn unit(entropy: &mut Entropy) -> f64 {
    f64::from_bits(0x3FF0_0000_0000_0000 | (entropy.next_u64() >> 12)) - 1.0
}

/// A uniform draw in `[low, high)`.
fn between(entropy: &mut Entropy, (low, high): (f64, f64)) -> f64 {
    low + unit(entropy) * (high - low)
}

/// A uniform draw of an index below `count`, which is not zero.
fn below(entropy: &mut Entropy, count: usize) -> usize {
    let count = u64::try_from(count).expect("invariant: a count fits a u64");
    usize::try_from(entropy.next_u64() % count).expect("invariant: below a usize count")
}

/// `value` milliseconds, none when it is not positive.
fn millis(value: f64) -> Duration {
    Duration::try_from_secs_f64(value / 1000.0).unwrap_or(Duration::ZERO)
}

/// `value`, a non-negative whole number, as a count; 0 for anything else,
/// `u32::MAX` past it.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped into u32's range first, and a whole number has no fraction to lose"
)]
fn whole(value: f64) -> u32 {
    if value.is_nan() {
        return 0;
    }
    value.clamp(0.0, f64::from(u32::MAX)) as u32
}
