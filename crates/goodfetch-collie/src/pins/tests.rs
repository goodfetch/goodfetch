#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_engine::browser_cache::{BuildPlatform, CHROME_FOR_TESTING, CHROME_HEADLESS_SHELL};

use super::pinned_artifacts;

/// Each platform Chrome for Testing builds for pins both its builds of the
/// one release, the full browser and `chrome-headless-shell`, each with
/// its own archive and digests (#3224).
///
/// Covers: setting:browser.headless_shell
#[test]
fn each_platform_pins_the_full_build_and_the_headless_shell_of_one_release() {
    let pins = pinned_artifacts();

    let linux = |name: &str| {
        pins.iter()
            .find(|pin| pin.id.name == name && pin.platform == Some(BuildPlatform::LinuxX64))
            .map(|pin| pin.url.clone())
    };
    let shells = pins
        .iter()
        .filter(|pin| pin.id.name == CHROME_HEADLESS_SHELL)
        .count();
    let builds: Vec<&str> = pins.iter().map(|pin| pin.id.build.as_str()).collect();
    let build = builds[0];
    assert_eq!(
        (
            linux(CHROME_FOR_TESTING),
            linux(CHROME_HEADLESS_SHELL),
            shells,
            builds.windows(2).all(|pair| pair[0] == pair[1]),
            pins.iter()
                .filter(|pin| pin.id.name == CHROME_HEADLESS_SHELL)
                .all(|pin| pin.archive_digest.is_some()),
        ),
        (
            Some(format!(
                "https://storage.googleapis.com/chrome-for-testing-public/{build}/linux64/chrome-linux64.zip"
            )),
            Some(format!(
                "https://storage.googleapis.com/chrome-for-testing-public/{build}/linux64/chrome-headless-shell-linux64.zip"
            )),
            BuildPlatform::CHROME_FOR_TESTING.len(),
            true,
            true,
        )
    );
}
