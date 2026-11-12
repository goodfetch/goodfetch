//! The four runners' listings of the workspace's tests (ADR 0063):
//! `listings` writes them into one folder, and `read` takes each Journey test
//! with the variant it carries from nextest's, Vitest's and pytest's. No
//! surface's variants run on Playwright; its listing is written for the
//! Feature check, which reads the same folder.
//!
//! A test carries its variant where its runner selects by it:
//! - nextest: in its path, `journeys::j001::cli`, whose last segment is the
//!   surface with `_` for `-`;
//! - Vitest: as two tags, the Journey's ID and the surface;
//! - pytest: as the `journey` marker's `id` and `surface` keywords, which
//!   `-m` matches.
//!
//! A test is a Journey test when it names a Journey: a `journeys::j<digits>`
//! path, a `J-<digits>` tag, or a `journey` marker.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::runners::{Runner, listing};

use super::variants::{self, Carried, JourneyTest};

/// Each runner's listing file in the folder.
fn file(runner: Runner) -> &'static str {
    match runner {
        Runner::Nextest => "nextest.json",
        Runner::Vitest => "vitest.json",
        Runner::Playwright => "playwright.json",
        Runner::Pytest => "pytest.json",
    }
}

/// The script that lists Vitest's tests with their tags, which `vitest list`
/// leaves out; it runs from the workspace's folder.
const VITEST_LISTING: &str = include_str!("vitest-listing.mjs");

/// Writes the four listings of the workspace at `root` into `out`.
pub(super) fn write(
    root: &Path,
    out: &Path,
    cargo: &OsStr,
    cargo_profile: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    for runner in [Runner::Nextest, Runner::Playwright] {
        let output = listing::output(runner, root, cargo, cargo_profile, None, None)?;
        let path = out.join(file(runner));
        std::fs::write(&path, output).with_context(|| format!("writing {}", path.display()))?;
    }
    let vitest = out.join(file(Runner::Vitest));
    listing::run(
        Command::new("node")
            .args(["--input-type=module", "--eval", VITEST_LISTING])
            .arg(&vitest)
            .current_dir(root),
        "the Vitest listing",
        &[0],
    )?;
    let pytest = out.join(file(Runner::Pytest));
    // No test collected writes the listing too.
    listing::run(
        listing::pytest_collection(root)?.arg(format!("--marker-listing={}", pytest.display())),
        listing::PYTEST_COLLECTION,
        &[0, 5],
    )?;
    Ok(())
}

/// Every Journey test of nextest's, Vitest's and pytest's listings in
/// `folder`.
pub(super) fn read(root: &Path, folder: &Path) -> Result<Vec<JourneyTest>> {
    type Parse<'a> = &'a dyn Fn(&[u8]) -> Result<Vec<JourneyTest>>;
    let listings: [(Runner, Parse<'_>); 3] = [
        (Runner::Nextest, &|bytes| nextest(root, bytes)),
        (Runner::Vitest, &|bytes| vitest(root, bytes)),
        (Runner::Pytest, &pytest),
    ];
    let mut tests = Vec::new();
    for (runner, parse) in listings {
        let path = folder.join(file(runner));
        let bytes = std::fs::read(&path).with_context(|| {
            format!(
                "reading {}; `xtask journeys listings` writes it",
                path.display()
            )
        })?;
        tests.extend(parse(&bytes).with_context(|| format!("reading {}", path.display()))?);
    }
    Ok(tests)
}

fn nextest(root: &Path, bytes: &[u8]) -> Result<Vec<JourneyTest>> {
    Ok(listing::parse(Runner::Nextest, root, bytes)?
        .into_iter()
        .filter_map(|listed| {
            Some(JourneyTest {
                carried: nextest_variant(listed.test.nextest_name())?,
                subject: format!("nextest {}", listed.test),
            })
        })
        .collect())
}

/// The variant a nextest test's path carries, if it names a Journey.
fn nextest_variant(name: &str) -> Option<Carried> {
    let segments: Vec<&str> = name.split("::").collect();
    let at = segments.windows(2).position(|pair| {
        pair[0] == "journeys"
            && pair[1].strip_prefix('j').is_some_and(|digits| {
                !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
            })
    })?;
    let module = segments[at + 1];
    Some(match &segments[at + 2..] {
        [surface] => Carried::Variant {
            journey: format!("J-{}", &module[1..]),
            surface: surface.replace('_', "-"),
        },
        _ => Carried::Malformed(format!(
            "its path does not end in `journeys::{module}::<surface>`, the surface's name with `_` for `-`"
        )),
    })
}

#[derive(Deserialize)]
struct VitestTest {
    name: String,
    file: String,
    tags: Vec<String>,
}

fn vitest(root: &Path, bytes: &[u8]) -> Result<Vec<JourneyTest>> {
    let listed: Vec<VitestTest> = serde_json::from_slice(bytes)?;
    let roots = [
        root.to_path_buf(),
        std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()),
    ];
    Ok(listed
        .into_iter()
        .filter_map(|test| {
            let file = Path::new(&test.file);
            let file = roots
                .iter()
                .find_map(|root| file.strip_prefix(root).ok())
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            Some(JourneyTest {
                carried: tagged(&test.tags)?,
                subject: format!("vitest {file} > {}", test.name),
            })
        })
        .collect())
}

/// The variant a test's tags carry, if a tag names a Journey.
fn tagged(tags: &[String]) -> Option<Carried> {
    let journeys: Vec<&String> = tags
        .iter()
        .filter(|tag| {
            tag.strip_prefix("J-")
                .is_some_and(|digits| digits.bytes().next().is_some_and(|b| b.is_ascii_digit()))
        })
        .collect();
    if journeys.is_empty() {
        return None;
    }
    let surfaces: Vec<&String> = tags
        .iter()
        .filter(|tag| variants::is_surface(tag))
        .collect();
    Some(match (journeys.as_slice(), surfaces.as_slice()) {
        ([journey], [surface]) => Carried::Variant {
            journey: (*journey).clone(),
            surface: (*surface).clone(),
        },
        _ => Carried::Malformed(format!(
            "it has {} Journey tag(s) and {} surface tag(s), not one of each",
            journeys.len(),
            surfaces.len()
        )),
    })
}

#[derive(Deserialize)]
struct PytestListing {
    tests: Vec<PytestTest>,
}

#[derive(Deserialize)]
struct PytestTest {
    id: String,
    journey: Vec<serde_json::Value>,
}

fn pytest(bytes: &[u8]) -> Result<Vec<JourneyTest>> {
    let listing: PytestListing = serde_json::from_slice(bytes)?;
    Ok(listing
        .tests
        .into_iter()
        .filter(|test| !test.journey.is_empty())
        .map(|test| {
            let carried = match test.journey.as_slice() {
                [marker] => match (
                    marker.get("id").and_then(serde_json::Value::as_str),
                    marker.get("surface").and_then(serde_json::Value::as_str),
                    marker.as_object().map(serde_json::Map::len),
                ) {
                    (Some(journey), Some(surface), Some(2)) => Carried::Variant {
                        journey: journey.to_owned(),
                        surface: surface.to_owned(),
                    },
                    _ => Carried::Malformed(
                        "its `journey` marker is not `journey(id=\"J-NNN\", surface=\"<surface>\")`"
                            .to_owned(),
                    ),
                },
                markers => Carried::Malformed(format!(
                    "it has {} `journey` markers, not one",
                    markers.len()
                )),
            };
            JourneyTest {
                subject: format!("pytest {}", test.id),
                carried,
            }
        })
        .collect())
}
