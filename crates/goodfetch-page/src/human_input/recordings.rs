//! Cursory's recorded mouse movements (LGPL-3.0-or-later, Vinyzu's
//! `cursory`, `cursory/trajectories.json.gz`), which a Run reads from the
//! Browser cache and never carries compiled in, and Goodfetch's own step
//! that reshapes one to a gesture's ends: among the recordings closest to
//! the move in length and direction, one is drawn, turned and scaled so
//! its ends land on the move's, its inner points shifted by up to a pixel,
//! and its timing stretched with the square root of the scale. Cursory
//! publishes the method's outline only, so the step is Goodfetch's design.
#![expect(
    clippy::disallowed_methods,
    reason = "recordings are read from the Browser cache on the tasks module's blocking policy, never on an async worker"
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use goodfetch_engine::browser_cache::{
    BrowserCache, CacheLocation, CacheMode, HeldArtifact, PinnedArtifact,
};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::Home;
use goodfetch_engine::pinned_download::{Digest, Fetching};
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::settings::{RunSettings, get_or_default};
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, PinnedBy, READ_ONLY};
use serde::Deserialize;

use super::{Move, Point, below, between, millis};

/// The artifact name of Cursory's recordings in the Browser cache.
pub(crate) const CURSORY_NAME: &str = "cursory";

/// The pinned commit of Cursory's recordings.
pub(crate) const CURSORY_COMMIT: &str = "51a40db323fec9c294af099112e2df08d8cf5086";

/// The file name of Cursory's gzip-compressed trajectories inside its
/// cache entry and for a user-supplied replacement.
pub(crate) const CURSORY_FILE: &str = "trajectories.json.gz";

/// The pinned HTTPS URL for Cursory's recordings (ADR 0043).
const CURSORY_URL: &str = "https://raw.githubusercontent.com/Vinyzu/cursory/51a40db323fec9c294af099112e2df08d8cf5086/cursory/trajectories.json.gz";

/// The SHA-256 digest of `trajectories.json.gz` at [`CURSORY_COMMIT`].
const CURSORY_ARCHIVE_DIGEST: &str =
    "sha256:1bf3af168719a580e2c5d6fb439f894f9bfff72fc90147149cc531dee80b6203";

/// The Browser cache tree digest of `files/trajectories.json.gz` at
/// [`CURSORY_COMMIT`].
const CURSORY_TREE_DIGEST: &str =
    "sha256:836e32551610dd57af158a11cae59b5cd9d8f9d34f861d9e3d8d593a74082ae0";

/// How many of the closest recordings a path is drawn among.
const CLOSEST: usize = 5;

/// The fewest pixels between a recording's ends for it to give a shape.
const SHORTEST: f64 = 1.0;

/// How far an inner point of a reshaped recording may shift, in pixels.
const JITTER: f64 = 1.0;

/// The most bytes read from a recordings file in the Browser cache.
const MAX_RECORDINGS_BYTES: u64 = 32 * 1024 * 1024;

/// The pinned Browser cache artifact for Cursory's recorded movements
/// (ADRs 0035, 0043 and 0098).
pub(crate) fn cursory_pin() -> PinnedArtifact {
    let parse = |text: &str| -> Digest {
        text.parse()
            .expect("invariant: Cursory's pinned digest is a valid sha256 digest")
    };
    PinnedArtifact {
        id: ArtifactId {
            kind: ArtifactKind::MouseMovements,
            name: CURSORY_NAME.to_owned(),
            build: CURSORY_COMMIT.to_owned(),
        },
        platform: None,
        url: CURSORY_URL.to_owned(),
        archive_digest: Some(parse(CURSORY_ARCHIVE_DIGEST)),
        digest: parse(CURSORY_TREE_DIGEST),
        application_settings: BTreeMap::new(),
    }
}

/// Where a user places a replaceable LGPL recordings file in the Browser
/// cache: `<cache_root>/mouse_movements/cursory/trajectories.json.gz`.
pub(crate) fn replacement_path(cache_root: &Path) -> PathBuf {
    cache_root
        .join(ArtifactKind::MouseMovements.as_str())
        .join(CURSORY_NAME)
        .join(CURSORY_FILE)
}

/// Cursory's recordings loaded from the Browser cache, together with the
/// cache hold that keeps clean-up from removing them while a Session runs
/// (`None` for a user-supplied replacement file).
pub(crate) struct Loaded {
    pub(crate) recordings: Recordings,
    pub(crate) held: Option<HeldArtifact>,
}

/// Loads Cursory's recordings from the Browser cache at `home` for `run`:
/// first checking a user-supplied replacement at [`replacement_path`],
/// then preparing the pinned artifact in [`BrowserCache`] (reading the
/// cached copy or downloading it at runtime when online and writable).
/// Returns `None` when absent offline or unreadable, so the caller falls
/// back to the ghost-cursor port with `human_input_fallback`.
pub(crate) async fn load(home: &Home, run: &RunIdentity, pinned_by: &PinnedBy) -> Option<Loaded> {
    let location = CacheLocation::new(home.root(), home.browsers_path().map(Path::to_path_buf));
    let blocking = run.owner().blocking_threads();
    let replacement = replacement_path(location.root());
    if let Some(recordings) = read_file(&blocking, replacement).await {
        return Some(Loaded {
            recordings,
            held: None,
        });
    }
    let read_only = get_or_default(run.settings().map(RunSettings::resolved), &READ_ONLY);
    let cache = BrowserCache::new(&location, blocking.clone()).in_mode(CacheMode::of(read_only));
    let fetching = match run.settings() {
        Some(settings) => Fetching::of(settings.resolved(), run.clock(), run.owner()).await,
        None => Fetching::new(run.clock()),
    };
    let pin = cursory_pin();
    let prepared = match cache
        .prepare(
            &pin,
            pinned_by,
            &fetching,
            run.clock().timestamp(),
            &mut |_| {},
        )
        .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::debug!(
                %error,
                "Cursory's recordings are not available in the Browser cache; falling back to ghost-cursor"
            );
            return None;
        }
    };
    let file = prepared.artifact.files().join(CURSORY_FILE);
    let recordings = read_file(&blocking, file).await?;
    Some(Loaded {
        recordings,
        held: Some(prepared.artifact),
    })
}

/// Reads and parses a recordings file on `blocking`, returning `None` if
/// absent, oversized or unreadable.
async fn read_file(
    blocking: &goodfetch_engine::tasks::BlockingThreads,
    path: PathBuf,
) -> Option<Recordings> {
    blocking
        .run(move || {
            let meta = std::fs::metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > MAX_RECORDINGS_BYTES {
                return None;
            }
            let bytes = std::fs::read(&path).ok()?;
            match Recordings::parse(&bytes) {
                Ok(recordings) => Some(recordings),
                Err(error) => {
                    tracing::warn!(
                        path = %path.display(),
                        %error,
                        "mouse-movement recordings file could not be parsed; falling back to ghost-cursor"
                    );
                    None
                }
            }
        })
        .await
}

/// Cursory's recordings, each with its ends apart.
#[derive(Debug)]
pub(crate) struct Recordings(Vec<Recording>);

/// One recorded movement: its points, and when each was sampled, in
/// milliseconds since the first.
#[derive(Debug)]
struct Recording {
    points: Vec<Point>,
    timing: Vec<f64>,
    length: f64,
    angle: f64,
}

/// Why the recordings could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RecordingsError {
    #[error("the recordings are not gzip-compressed JSON: {0}")]
    Unreadable(#[from] std::io::Error),
    #[error("the recordings are not in Cursory's format: {0}")]
    Format(#[from] serde_json::Error),
    #[error("the recordings hold no movement")]
    Empty,
}

/// A recording as Cursory's file holds it; `start`, `end`, `dx`, `dy` and
/// `length` are left out, since the points give them.
#[derive(Deserialize)]
struct Recorded {
    points: Vec<(f64, f64)>,
    timing: Vec<f64>,
}

impl Recordings {
    /// The recordings of Cursory's file, `compressed`: those with as many
    /// samples as points, two at least, and ends apart.
    pub(crate) fn parse(compressed: &[u8]) -> Result<Self, RecordingsError> {
        let mut text = Vec::new();
        GzDecoder::new(compressed).read_to_end(&mut text)?;
        let recorded: Vec<Recorded> = serde_json::from_slice(&text)?;
        let recordings: Vec<Recording> = recorded.into_iter().filter_map(Recording::of).collect();
        if recordings.is_empty() {
            return Err(RecordingsError::Empty);
        }
        Ok(Self(recordings))
    }

    /// The moves from `from` to `to` of one of the closest recordings,
    /// reshaped, the last one on `to`.
    pub(super) fn path(&self, entropy: &mut Entropy, from: Point, to: Point) -> Vec<Move> {
        let length = from.distance(to);
        let angle = (to.y - from.y).atan2(to.x - from.x);
        let mut costs: Vec<(f64, usize)> = self
            .0
            .iter()
            .enumerate()
            .map(|(index, recording)| (recording.cost(length, angle), index))
            .collect();
        let closest = CLOSEST.min(costs.len());
        costs.select_nth_unstable_by(closest - 1, |a, b| a.0.total_cmp(&b.0));
        costs[..closest].sort_by(|a, b| a.0.total_cmp(&b.0));
        let recording = &self.0[costs[below(entropy, closest)].1];
        recording.reshaped(entropy, from, (length, angle))
    }
}

impl Recording {
    fn of(recorded: Recorded) -> Option<Self> {
        if recorded.points.len() < 2 || recorded.points.len() != recorded.timing.len() {
            return None;
        }
        let points: Vec<Point> = recorded
            .points
            .into_iter()
            .map(|(x, y)| Point { x, y })
            .collect();
        let (first, last) = (points[0], points[points.len() - 1]);
        let length = first.distance(last);
        let start = recorded.timing[0];
        let timing: Vec<f64> = recorded.timing.iter().map(|at| at - start).collect();
        let ordered = timing.windows(2).all(|pair| pair[1] >= pair[0]);
        (length >= SHORTEST && ordered && timing.iter().all(|at| at.is_finite())).then(|| Self {
            points,
            timing,
            length,
            angle: (last.y - first.y).atan2(last.x - first.x),
        })
    }

    /// How far this recording is from a move of `length` toward `angle`:
    /// the log of their ratio of lengths plus their angle apart.
    fn cost(&self, length: f64, angle: f64) -> f64 {
        let turn = (angle - self.angle).rem_euclid(std::f64::consts::TAU);
        let apart = turn.min(std::f64::consts::TAU - turn);
        (length / self.length).ln().abs() + apart
    }

    /// The recording turned and scaled so its first point is `from` and its
    /// last is `length` away toward `angle`, its inner points jittered.
    fn reshaped(
        &self,
        entropy: &mut Entropy,
        from: Point,
        (length, angle): (f64, f64),
    ) -> Vec<Move> {
        let scale = length / self.length;
        let (sin, cos) = (angle - self.angle).sin_cos();
        let origin = self.points[0];
        let last = self.points.len() - 1;
        let stretch = scale.sqrt();
        self.points
            .iter()
            .zip(&self.timing)
            .enumerate()
            .skip(1)
            .map(|(index, (point, at))| {
                let (dx, dy) = ((point.x - origin.x) * scale, (point.y - origin.y) * scale);
                let (jx, jy) = if index == last {
                    (0.0, 0.0)
                } else {
                    (
                        between(entropy, (-JITTER, JITTER)),
                        between(entropy, (-JITTER, JITTER)),
                    )
                };
                let before = self.timing[index - 1];
                Move {
                    to: Point {
                        x: from.x + dx * cos - dy * sin + jx,
                        y: from.y + dx * sin + dy * cos + jy,
                    },
                    after: millis((at - before) * stretch),
                }
            })
            .collect()
    }
}
