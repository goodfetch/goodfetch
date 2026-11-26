//! The release plan's last steps: each Product's formula in the Homebrew
//! tap and its `.sh` and `.ps1` installers (ADRs 0032, 0090 and 0094),
//! which install its executable from its archive on the GitHub Release.
//!
//! - **Only a stable version** moves them (ADR 0030).
//! - **A formula** is `Formula/<executable>.rb` in the tap's repository,
//!   read through GitHub's contents interface; **an installer** is
//!   `<site>/install/<product>.sh` or `.ps1`, read from the site.
//! - **Either is out** once it names the version's archives,
//!   `/releases/download/v<version>/`.
//! - **Either is `held`** while the GitHub Release is a draft, whose
//!   archives no one else can download yet.
//! - **One not there yet** is planned as `publish`: the release job writes
//!   it, with no first publish by hand.

use anyhow::{Context as _, Result};
use serde::Deserialize;

use super::{Arguments, PackageState, Step, fetch};
use crate::github::Api;
use crate::products::Product;

/// The tap's repository, which `brew tap goodfetch/tap` clones (ADR 0090).
pub(super) const TAP: &str = "goodfetch/homebrew-tap";
/// The tap's name, as `brew install goodfetch/tap/<formula>` writes it.
const TAP_NAME: &str = "goodfetch/tap";
/// The site the installers are served from, under `/install/` (ADR 0090).
pub(super) const SITE: &str = "https://goodfetch.dev";
/// Each Product's installers: `curl | sh`'s and PowerShell's.
const INSTALLERS: [&str; 2] = ["sh", "ps1"];

/// Each Product's formula in the plan's tap, then each Product's installers
/// on its site, at the stable `semver`; `released` once the GitHub Release
/// is published.
pub(super) fn steps(
    api: &Api<'_>,
    arguments: &Arguments,
    semver: &str,
    released: bool,
) -> Result<Vec<Step>> {
    let (tap, site) = (&arguments.tap, &arguments.site);
    let archives = format!("/releases/download/v{semver}/");
    let step = |registry: &'static str, name: String, text: Option<String>| Step::Package {
        registry,
        name,
        version: semver.to_owned(),
        state: match text {
            Some(text) if text.contains(&archives) => PackageState::Published,
            _ if released => PackageState::Publish,
            _ => PackageState::Held,
        },
    };
    let mut steps = Vec::new();
    for product in Product::ALL {
        let formula = product.names().executable;
        let text = api
            .find::<Contents>(&format!("/repos/{tap}/contents/Formula/{formula}.rb"))?
            .map(Contents::text)
            .transpose()?;
        steps.push(step("homebrew", format!("{TAP_NAME}/{formula}"), text));
    }
    for product in Product::ALL {
        for extension in INSTALLERS {
            let path = format!("install/{}.{extension}", product.key());
            let text = fetch(
                &format!("{}/{path}", site.trim_end_matches('/')),
                |response| response.body_mut().read_to_string(),
            )?;
            steps.push(step("installer", path, text));
        }
    }
    Ok(steps)
}

/// A file as GitHub's contents interface answers it.
#[derive(Deserialize)]
struct Contents {
    /// The file's bytes in base64, broken into lines.
    content: String,
}

impl Contents {
    fn text(self) -> Result<String> {
        let encoded: String = self.content.split_whitespace().collect();
        let bytes = data_encoding::BASE64
            .decode(encoded.as_bytes())
            .context("decoding a formula from GitHub's contents answer")?;
        String::from_utf8(bytes).context("reading a formula that is not UTF-8")
    }
}
