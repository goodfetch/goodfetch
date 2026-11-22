//! The guided build a refresh leaves in its work folder, tied by a record to
//! the merged profile and hot order it was built from, so that
//! `xtask pgo refresh --order-only` maps the touched pages of that build and
//! no other (ADR 0057's note of 2026-11-14).

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{ORDER, PROFILE};

/// The record's name in the work folder.
const RECORD: &str = "guided.record.json";

/// The SHA-256 of each input of the guided build, and of its Foxhound.
#[derive(Serialize, Deserialize)]
struct Record {
    profile: String,
    order: String,
    foxhound: String,
}

/// The files a record covers, each with the name a refusal gives it.
fn covered(work: &Path, foxhound: &Path) -> [(PathBuf, &'static str); 3] {
    let guided = work.join("guided");
    [
        (guided.join(PROFILE), "the merged profile"),
        (guided.join(ORDER), "the hot order"),
        (foxhound.to_path_buf(), "Foxhound's executable"),
    ]
}

/// Records that `foxhound` was built from the merged profile and hot order
/// in `<work>/guided`; refused for a build outside `work`, in a target
/// folder that the checkout's other release builds, guided by the
/// committed files, write their Foxhound to.
pub(crate) fn record(work: &Path, foxhound: &Path) -> Result<()> {
    if !foxhound.starts_with(work) {
        bail!(
            "the guided build of {} is outside the work folder {}, where another release build would overwrite it before `xtask pgo refresh --order-only` maps it",
            foxhound.display(),
            work.display()
        );
    }
    let [profile, order, foxhound] = covered(work, foxhound).map(|(path, _)| sha256(&path));
    let record = Record {
        profile: profile?,
        order: order?,
        foxhound: foxhound?,
    };
    let path = work.join(RECORD);
    std::fs::write(&path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("writing {}", path.display()))
}

/// The hot order in `<work>/guided`, the profile's functions hottest first,
/// once the record shows `foxhound` was built from it and that folder's
/// merged profile as they are now; refused, naming what changed, otherwise.
pub(crate) fn recorded_hot(work: &Path, foxhound: &Path) -> Result<Vec<String>> {
    let path = work.join(RECORD);
    if !path.is_file() {
        bail!(
            "no guided build is recorded in {}: run `xtask pgo refresh` without --order-only",
            work.display()
        );
    }
    let record: Record = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    let recorded = [&record.profile, &record.order, &record.foxhound];
    for ((file, name), recorded) in covered(work, foxhound).iter().zip(recorded) {
        if sha256(file)? != *recorded {
            bail!(
                "{name} changed since the guided build was recorded ({}): run `xtask pgo refresh` without --order-only",
                file.display()
            );
        }
    }
    let order = work.join("guided").join(ORDER);
    Ok(std::fs::read_to_string(&order)
        .with_context(|| format!("reading {}", order.display()))?
        .lines()
        .map(str::to_owned)
        .collect())
}

/// The SHA-256 of the file at `path`, in hexadecimal.
fn sha256(path: &Path) -> Result<String> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("reading {}", path.display()))?;
        if read == 0 {
            return Ok(format!("{:x}", digest.finalize()));
        }
        digest.update(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests;
