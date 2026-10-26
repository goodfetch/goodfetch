//! The `screen:` Features: one per Console route, named by the route as the
//! Console's router writes it (`/runs/$run`). They come from the route list
//! each package of the Console commits as `routes.json` at its root, which
//! its build writes and checks for drift (ADR 0072), so the Quick suite
//! never builds the Console. Foxhound's screens' package is Foxhound's alone
//! (ADR 0087); every other package's routes are both Products'. A Console
//! with no route list generates nothing.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};

use super::entry::{Carriers, Entry};
use super::files::{Kind, RouteList, Status};

/// The Console's folder, whose packages sit one level below it.
const CONSOLE: &str = "console";
/// The folder of Foxhound's Console package, `@goodfetch/console-foxhound`,
/// which Collie's Console build does not depend on (ADR 0087).
const FOXHOUND_SCREENS: &str = "foxhound";
/// A package's route list.
const ROUTE_LIST: &str = "routes.json";

pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let folder = root.join(CONSOLE);
    let Ok(packages) = std::fs::read_dir(&folder) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for package in packages {
        let package = package.with_context(|| format!("reading {}", folder.display()))?;
        let path = package.path().join(ROUTE_LIST);
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let list: RouteList = serde_json::from_str(&text)
            .with_context(|| format!("{} breaks its schema", path.display()))?;
        let carriers = if package.file_name() == FOXHOUND_SCREENS {
            Carriers::Foxhound
        } else {
            Carriers::Both
        };
        entries.extend(list.routes.into_iter().map(|route| Entry {
            kind: Kind::Screen,
            name: route,
            area: None,
            milestone: None,
            status: Status::Active,
            carriers,
        }));
    }
    Ok(entries)
}
