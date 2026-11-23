//! The Browser cache through its interface, on real files in the test's
//! Goodfetch home (spec #213's Seam 4, Quick suite): where it is, the digest
//! every launch checks, a tampered copy refused and removed, the missing
//! build's Error codes, the platform rules, and the clean-up's three
//! conditions on readings of the test clock's start (ADR 0098). Downloads
//! bind a socket, so they are tested in `tests/it`.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests lay out cache entries by hand, as a prepared GOODFETCH_BROWSERS_PATH copy holds them"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::tasks::{BlockingPolicy, BlockingThreads, TaskOwner};

use goodfetch_protocol::browser_cache::{
    ArtifactId, ArtifactKind, ChromeChannel, DownloadProgress, MissingBrowser, PinnedBy,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::json;

use crate::clock::Clock;
use crate::pinned_download::{Digest, Fetching};

use super::{
    Arch, BrowserCache, BrowserChoice, BrowserSource, BuildPlatform, CHROME_FOR_TESTING,
    CHROME_HEADLESS_SHELL, CacheError, CacheLocation, CacheMode, CleanUp, InstalledExecutable,
    MaxUnusedAge, Os, PinnedArtifact, Platform, PlatformNotice, browser_source, content_digest,
    installed_browser, kept_artifacts,
};

/// The digest of [`build_files`]'s tree, worked out by hand from the rule
/// in `artifact`'s header.
const BUILD_DIGEST: &str =
    "sha256:1b084060d413018b4448804bb41c6928bf1c95956cd86fe640535bffed7b0327";

const DAY: Duration = Duration::from_hours(24);

fn build_files(folder: &Path) {
    fs::create_dir_all(folder.join("resources")).unwrap();
    fs::write(folder.join("chrome"), "#!chrome\n").unwrap();
    fs::write(folder.join("resources/locale.pak"), "é\n").unwrap();
}

fn now(env: &TestEnv) -> jiff::Timestamp {
    jiff::Timestamp::try_from(env.start_time()).unwrap()
}

fn days_before(env: &TestEnv, days: u64) -> jiff::Timestamp {
    jiff::Timestamp::try_from(env.start_time() - DAY * u32::try_from(days).unwrap()).unwrap()
}

fn pinned_by() -> PinnedBy {
    PinnedBy {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
    }
}

fn artifact(kind: ArtifactKind, name: &str, build: &str) -> ArtifactId {
    ArtifactId {
        kind,
        name: name.to_owned(),
        build: build.to_owned(),
    }
}

fn browser(build: &str) -> ArtifactId {
    artifact(ArtifactKind::Browser, CHROME_FOR_TESTING, build)
}

fn pin(id: ArtifactId) -> PinnedArtifact {
    PinnedArtifact {
        id,
        platform: None,
        url: "https://builds.goodfetch.test/chrome.zip".to_owned(),
        archive_digest: Some(Digest::of(b"no archive is downloaded here")),
        application_settings: std::collections::BTreeMap::new(),
        digest: BUILD_DIGEST.parse().unwrap(),
    }
}

fn inline() -> BlockingThreads {
    TaskOwner::root(BlockingPolicy::Inline).blocking_threads()
}

fn cache_in(env: &TestEnv) -> BrowserCache {
    BrowserCache::new(&CacheLocation::new(env.home(), None), inline())
}

/// Lays out `id`'s entry as `browser install` leaves it, used at `used_at`.
fn lay_out(cache: &BrowserCache, id: &ArtifactId, used_at: jiff::Timestamp) -> PathBuf {
    let folder = cache
        .root()
        .join(id.kind.as_str())
        .join(&id.name)
        .join(&id.build);
    build_files(&folder.join("files"));
    fs::write(folder.join("lock"), "").unwrap();
    let entry = json!({
        "format_version": "0.1",
        "artifact": { "kind": id.kind.as_str(), "name": id.name, "build": id.build },
        "digest": BUILD_DIGEST,
        "installed_at": used_at.to_string(),
        "used_at": used_at.to_string(),
    });
    fs::write(folder.join("entry.json"), entry.to_string()).unwrap();
    folder
}

fn offline() -> Fetching {
    Fetching {
        offline: Some(SettingsLayer::UserConfiguration),
        ..Fetching::new(Clock::system())
    }
}

fn no_progress() -> impl FnMut(DownloadProgress) + Send {
    |progress| panic!("nothing downloads here, yet it reported {progress:?}")
}

fn clean_up_at(env: &TestEnv, kept: BTreeSet<ArtifactId>, dry_run: bool) -> CleanUp {
    CleanUp {
        kept,
        now: now(env),
        max_unused_age: MaxUnusedAge::DEFAULT,
        dry_run,
    }
}

#[cfg(unix)]
fn make_read_only(folder: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(folder, fs::Permissions::from_mode(0o555)).unwrap();
}

#[cfg(unix)]
fn make_writable(folder: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();
}

#[rstest]
#[case::in_goodfetch_home(None, "home/browsers")]
#[case::at_goodfetch_browsers_path(Some("prepared cache"), "prepared cache")]
fn the_cache_sits_in_goodfetch_home_unless_goodfetch_browsers_path_names_a_copy(
    #[case] browsers_path: Option<&str>,
    #[case] expected: &str,
) {
    let env = TestEnv::new();
    let browsers_path = browsers_path.map(|path| env.folder().join(path));

    let location = CacheLocation::new(env.home(), browsers_path);

    assert_eq!(location.root(), env.folder().join(expected));
}

#[test]
fn a_trees_digest_covers_each_files_path_and_bytes() {
    let env = TestEnv::new();
    build_files(&env.folder().join("tree"));

    let digest = content_digest(&env.folder().join("tree")).unwrap();

    assert_eq!(digest.to_string(), BUILD_DIGEST);
}

/// Covers: row:SDK-08
#[test]
fn a_build_with_its_pinned_digest_is_held_for_the_launch() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let id = browser("141.0.7390.54-linux64");
    let folder = lay_out(&cache, &id, days_before(&env, 3));

    let prepared = goodfetch_test_support::runtime()
        .block_on(cache.prepare(
            &pin(id),
            &pinned_by(),
            &offline(),
            now(&env),
            &mut no_progress(),
        ))
        .unwrap();

    assert!(!prepared.downloaded);
    assert_eq!(prepared.artifact.files(), folder.join("files"));
}

/// Covers: row:SDK-08, error:artifact_unavailable_offline
#[test]
fn a_tampered_build_is_refused_and_removed() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let id = browser("141.0.7390.54-linux64");
    let folder = lay_out(&cache, &id, now(&env));
    fs::write(folder.join("files/chrome"), "#!tampered\n").unwrap();

    let refused = goodfetch_test_support::runtime()
        .block_on(cache.prepare(
            &pin(id),
            &pinned_by(),
            &offline(),
            now(&env),
            &mut no_progress(),
        ))
        .err()
        .unwrap();

    assert_eq!(refused.code(), "artifact_unavailable_offline");
    assert!(!folder.exists());
}

/// A build whose files cannot be read now, as under a permission change
/// or a process out of file handles, is the cache's failure: it is kept,
/// not taken for a tampered build and removed.
///
/// Covers: row:SDK-08
#[cfg(unix)]
#[test]
fn a_build_that_cannot_be_read_is_kept_not_removed_as_tampered() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let id = browser("141.0.7390.54-linux64");
    let folder = lay_out(&cache, &id, now(&env));
    let resources = folder.join("files/resources");
    fs::set_permissions(&resources, fs::Permissions::from_mode(0o000)).unwrap();

    let found = goodfetch_test_support::runtime().block_on(cache.prepare(
        &pin(id),
        &pinned_by(),
        &offline(),
        now(&env),
        &mut no_progress(),
    ));
    if resources.exists() {
        fs::set_permissions(&resources, fs::Permissions::from_mode(0o755)).unwrap();
    }

    assert_eq!(
        (
            found.err().map(|error| error.code()),
            folder.join("files/chrome").exists()
        ),
        (Some("internal_error"), true)
    );
}

/// Covers: error:artifact_unavailable_offline
#[test]
fn a_missing_build_under_offline_names_the_build_its_pin_and_the_setting() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let id = browser("141.0.7390.54-linux64");

    let refused = goodfetch_test_support::runtime()
        .block_on(cache.prepare(
            &pin(id.clone()),
            &pinned_by(),
            &offline(),
            now(&env),
            &mut no_progress(),
        ))
        .err()
        .unwrap();

    let CacheError::Offline { details, .. } = refused else {
        panic!("expected artifact_unavailable_offline, got {refused:?}");
    };
    assert_eq!(details.artifact, id);
    assert_eq!(details.pinned_by, pinned_by());
    assert_eq!(details.limit.key, "egress.offline");
    assert_eq!(details.limit.layer, SettingsLayer::UserConfiguration);
    assert_eq!(
        details.fix,
        "Run `goodfetch-collie browser install` beforehand where the Browser cache is writable, or lift `egress.offline` where it was set"
    );
}

/// A missing headless shell's fix names it, since a bare `browser install`
/// fetches only the builds a Run launches by default (#3224).
/// Covers: error:artifact_unavailable_offline, setting:browser.headless_shell
#[test]
fn a_missing_headless_shell_under_offline_names_its_own_install() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let id = artifact(ArtifactKind::Browser, CHROME_HEADLESS_SHELL, "156.0.8078.4");

    let refused = goodfetch_test_support::runtime()
        .block_on(cache.prepare(
            &pin(id),
            &pinned_by(),
            &offline(),
            now(&env),
            &mut no_progress(),
        ))
        .err()
        .unwrap();

    let CacheError::Offline { details, .. } = refused else {
        panic!("expected artifact_unavailable_offline, got {refused:?}");
    };
    assert_eq!(
        details.fix,
        "Run `goodfetch-collie browser install chrome-headless-shell` beforehand where the Browser cache is writable, or lift `egress.offline` where it was set"
    );
}

/// A pin whose archive digest is not known yet is never downloaded, even
/// into a cache the engine can write: only a cache that holds it serves it.
/// Covers: error:browser_not_installed
#[test]
fn a_missing_build_whose_pin_names_no_archive_digest_is_not_installed() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let id = browser("156.0.8078.4");
    let pinned = PinnedArtifact {
        archive_digest: None,
        application_settings: std::collections::BTreeMap::new(),
        ..pin(id.clone())
    };

    let refused = goodfetch_test_support::runtime()
        .block_on(cache.prepare(
            &pinned,
            &pinned_by(),
            &Fetching::new(Clock::system()),
            now(&env),
            &mut no_progress(),
        ))
        .err()
        .unwrap();

    let CacheError::BrowserNotInstalled { details, .. } = refused else {
        panic!("expected browser_not_installed, got {refused:?}");
    };
    assert_eq!(
        (
            details.missing,
            fs::read_dir(cache.root()).map_or(0, Iterator::count),
        ),
        (
            MissingBrowser::Pinned {
                artifact: id,
                pinned_by: pinned_by(),
                cache: cache.root().display().to_string(),
            },
            0
        )
    );
}

/// Covers: error:browser_not_installed
#[cfg(unix)]
#[test]
fn a_missing_build_in_a_cache_the_engine_cannot_write_is_not_installed() {
    let env = TestEnv::new();
    let root = env.folder().join("read-only cache");
    fs::create_dir_all(&root).unwrap();
    make_read_only(&root);
    let cache = BrowserCache::new(
        &CacheLocation::new(env.home(), Some(root.clone())),
        inline(),
    );
    let id = browser("141.0.7390.54-linux64");

    let refused = goodfetch_test_support::runtime()
        .block_on(cache.prepare(
            &pin(id.clone()),
            &pinned_by(),
            &Fetching::new(Clock::system()),
            now(&env),
            &mut no_progress(),
        ))
        .err()
        .unwrap();
    make_writable(&root);

    let CacheError::BrowserNotInstalled { details, .. } = refused else {
        panic!("expected browser_not_installed, got {refused:?}");
    };
    assert_eq!(
        details.missing,
        MissingBrowser::Pinned {
            artifact: id,
            pinned_by: pinned_by(),
            cache: root.display().to_string(),
        }
    );
    assert_eq!(
        details.fix,
        "Run `goodfetch-collie browser install` where the Browser cache is writable, such as in the image's build, or point GOODFETCH_BROWSERS_PATH at a cache that holds it"
    );
}

/// Every path under `root`, `root` itself included, with its kind, length
/// and modification time, symbolic links unfollowed.
fn tree(root: &Path) -> BTreeMap<PathBuf, (bool, u64, SystemTime)> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(next) = pending.pop() {
        let metadata = fs::symlink_metadata(&next).unwrap();
        if metadata.is_dir() {
            for entry in fs::read_dir(&next).unwrap() {
                pending.push(entry.unwrap().path());
            }
        }
        entries.insert(
            next.strip_prefix(root).unwrap().to_owned(),
            (
                metadata.is_dir(),
                metadata.len(),
                metadata.modified().unwrap(),
            ),
        );
    }
    entries
}

/// The use `folder`'s entry records.
fn used_at(folder: &Path) -> jiff::Timestamp {
    let entry: serde_json::Value =
        serde_json::from_slice(&fs::read(folder.join("entry.json")).unwrap()).unwrap();
    entry["used_at"].as_str().unwrap().parse().unwrap()
}

/// A launch records its build's use in a cache it can write, and none in
/// [`CacheMode::ReadOnly`].
///
/// Covers: setting:browser_cache.read_only
#[rstest]
#[case::read_write(CacheMode::ReadWrite, 0)]
#[case::read_only(CacheMode::ReadOnly, 3)]
fn a_held_build_s_use_is_recorded_unless_the_cache_is_read_only(
    #[case] mode: CacheMode,
    #[case] days_since_use: u64,
) {
    let env = TestEnv::new();
    let cache = cache_in(&env).in_mode(mode);
    let id = browser("141.0.7390.54-linux64");
    let folder = lay_out(&cache, &id, days_before(&env, 3));

    let prepared = goodfetch_test_support::runtime()
        .block_on(cache.prepare(
            &pin(id),
            &pinned_by(),
            &offline(),
            now(&env),
            &mut no_progress(),
        ))
        .unwrap();

    assert_eq!(prepared.artifact.files(), folder.join("files"));
    assert_eq!(used_at(&folder), days_before(&env, days_since_use));
}

/// Covers: setting:browser_cache.read_only
#[test]
fn a_read_only_cache_that_holds_the_pinned_build_is_left_as_it_was() {
    let env = TestEnv::new();
    let cache = cache_in(&env).in_mode(CacheMode::ReadOnly);
    let id = browser("141.0.7390.54-linux64");
    lay_out(&cache, &id, days_before(&env, 3));
    let before = tree(cache.root());

    let prepared = goodfetch_test_support::runtime()
        .block_on(cache.prepare(
            &pin(id),
            &pinned_by(),
            &offline(),
            now(&env),
            &mut no_progress(),
        ))
        .unwrap();
    drop(prepared);

    assert_eq!(tree(cache.root()), before);
}

/// Covers: error:browser_not_installed
#[test]
fn a_chrome_channel_that_is_not_installed_names_where_it_was_looked_for() {
    let env = TestEnv::new();
    let candidates = vec![env.folder().join("Google Chrome Beta")];

    let refused = goodfetch_test_support::runtime()
        .block_on(installed_browser(
            &inline(),
            &BrowserChoice::Channel(ChromeChannel::Beta),
            &candidates,
            Product::Collie,
        ))
        .err()
        .unwrap();

    let CacheError::BrowserNotInstalled { details, .. } = refused else {
        panic!("expected browser_not_installed, got {refused:?}");
    };
    assert_eq!(
        details.missing,
        MissingBrowser::Channel {
            channel: ChromeChannel::Beta,
            looked_in: vec![candidates[0].display().to_string()],
        }
    );
}

#[test]
fn a_browser_named_by_its_path_is_used_when_it_is_there() {
    let env = TestEnv::new();
    let path = env.folder().join("my chrome");
    fs::write(&path, "#!chrome\n").unwrap();

    let found = goodfetch_test_support::runtime()
        .block_on(installed_browser(
            &inline(),
            &BrowserChoice::Path(path.clone()),
            std::slice::from_ref(&path),
            Product::Collie,
        ))
        .unwrap();

    assert_eq!(found, path);
}

fn pinned(platform: BuildPlatform, notice: Option<PlatformNotice>) -> BrowserSource {
    BrowserSource::Pinned {
        name: CHROME_FOR_TESTING,
        platform,
        notice,
    }
}

fn distribution() -> BrowserSource {
    BrowserSource::Installed {
        candidates: vec![
            "/usr/bin/chromium".into(),
            "/usr/bin/chromium-browser".into(),
            "/usr/lib/chromium/chromium".into(),
        ],
        notice: Some(PlatformNotice::DistributionChromium),
    }
}

#[rstest]
#[case::linux_x64(Os::LinuxGnu, Arch::X64, pinned(BuildPlatform::LinuxX64, None))]
#[case::mac_arm64(Os::Mac, Arch::Arm64, pinned(BuildPlatform::MacArm64, None))]
#[case::mac_x64(Os::Mac, Arch::X64, pinned(BuildPlatform::MacX64, None))]
#[case::windows_x64(Os::Windows, Arch::X64, pinned(BuildPlatform::WindowsX64, None))]
#[case::windows_arm64_under_emulation(
    Os::Windows,
    Arch::Arm64,
    pinned(BuildPlatform::WindowsX64, Some(PlatformNotice::WindowsArm64Emulation))
)]
#[case::musl_x64(Os::LinuxMusl, Arch::X64, distribution())]
#[case::musl_arm64(Os::LinuxMusl, Arch::Arm64, distribution())]
#[case::linux_arm64(Os::LinuxGnu, Arch::Arm64, distribution())]
fn chrome_for_testing_is_the_default_where_it_has_a_build(
    #[case] os: Os,
    #[case] arch: Arch,
    #[case] expected: BrowserSource,
) {
    let source = browser_source(
        Platform { os, arch },
        &BrowserChoice::Default,
        &BTreeMap::new(),
    );

    assert_eq!(source, expected);
}

#[rstest]
#[case::linux_beta(Os::LinuxGnu, ChromeChannel::Beta, vec!["/opt/google/chrome-beta/chrome".into()])]
#[case::mac_stable(
    Os::Mac,
    ChromeChannel::Stable,
    vec!["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into()]
)]
#[case::windows_canary(
    Os::Windows,
    ChromeChannel::Canary,
    vec![
        ["C:\\Users\\é\\AppData\\Local", "Google", "Chrome SxS", "Application", "chrome.exe"]
            .iter()
            .collect()
    ]
)]
fn an_installed_chrome_is_named_by_its_channel(
    #[case] os: Os,
    #[case] channel: ChromeChannel,
    #[case] expected: Vec<PathBuf>,
) {
    let environment = BTreeMap::from([(
        OsString::from("LOCALAPPDATA"),
        OsString::from("C:\\Users\\é\\AppData\\Local"),
    )]);

    let source = browser_source(
        Platform {
            os,
            arch: Arch::X64,
        },
        &BrowserChoice::Channel(channel),
        &environment,
    );

    let BrowserSource::Installed { candidates, notice } = source else {
        panic!("a channel is an installed browser, got {source:?}");
    };
    assert_eq!(candidates, expected);
    assert_eq!(notice, None);
}

/// Covers: story:3-140, setting:browser_cache.max_unused_age
#[test]
fn an_entry_counts_as_installed_while_at_its_path_or_seen_within_the_age() {
    let env = TestEnv::new();
    let entry = |still_at_path, seen_days_ago, build: &str| InstalledExecutable {
        still_at_path,
        last_seen: SystemTime::from(days_before(&env, seen_days_ago)),
        pins: vec![browser(build)],
    };
    let installed = [
        entry(true, 400, "at-its-path"),
        entry(false, 29, "gone-but-seen-within-the-age"),
        entry(false, 31, "gone-and-unseen"),
    ];
    let declared = [artifact(
        ArtifactKind::Browser,
        "camoufox",
        "declared-by-a-plugin",
    )];

    let kept = kept_artifacts(&installed, &declared, now(&env), MaxUnusedAge::default());

    assert_eq!(
        kept,
        BTreeSet::from([
            browser("at-its-path"),
            browser("gone-but-seen-within-the-age"),
            artifact(ArtifactKind::Browser, "camoufox", "declared-by-a-plugin"),
        ])
    );
}

/// Covers: setting:browser_cache.max_unused_age
#[test]
fn a_null_max_unused_age_turns_the_automatic_run_off_and_counts_with_thirty_days() {
    let null = MaxUnusedAge::new(None);

    assert_eq!(null.automatic(), None);
    assert_eq!(null.counting(), Duration::from_hours(30 * 24));
}

/// Covers: story:3-140
#[test]
fn the_clean_up_deletes_only_what_nothing_pins_uses_or_holds() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let pinned_by_counting_entry = browser("pinned-by-a-counting-entry");
    let declared_by_plugin = artifact(ArtifactKind::Browser, "camoufox", "declared-by-a-plugin");
    let used_within_the_age = browser("used-29-days-ago");
    let held = browser("held-by-a-running-browser");
    let old_driver = artifact(ArtifactKind::Driver, "playwright", "unpinned-and-unused");
    let old_browser = browser("unpinned-and-unused");
    for id in [
        &pinned_by_counting_entry,
        &declared_by_plugin,
        &held,
        &old_driver,
        &old_browser,
    ] {
        lay_out(&cache, id, days_before(&env, 31));
    }
    lay_out(&cache, &used_within_the_age, days_before(&env, 29));
    let runtime = goodfetch_test_support::runtime();
    let running = runtime
        .block_on(cache.prepare(
            &pin(held.clone()),
            &pinned_by(),
            &offline(),
            days_before(&env, 31),
            &mut no_progress(),
        ))
        .unwrap();
    let kept = kept_artifacts(
        &[InstalledExecutable {
            still_at_path: false,
            last_seen: SystemTime::from(days_before(&env, 2)),
            pins: vec![pinned_by_counting_entry.clone()],
        }],
        std::slice::from_ref(&declared_by_plugin),
        now(&env),
        MaxUnusedAge::default(),
    );

    let report = runtime
        .block_on(cache.clean_up(&clean_up_at(&env, kept, false)))
        .unwrap();

    assert_eq!(report.deleted, [old_browser.clone(), old_driver.clone()]);
    assert_eq!(report.not_cleaned, None);
    for id in [
        &pinned_by_counting_entry,
        &declared_by_plugin,
        &used_within_the_age,
        &held,
    ] {
        assert!(entry_exists(&cache, id), "{id:?} was deleted");
    }
    for id in [&old_browser, &old_driver] {
        assert!(!entry_exists(&cache, id), "{id:?} was kept");
    }
    drop(running);
}

fn entry_exists(cache: &BrowserCache, id: &ArtifactId) -> bool {
    cache
        .root()
        .join(id.kind.as_str())
        .join(&id.name)
        .join(&id.build)
        .join("entry.json")
        .exists()
}

/// Covers: story:3-140, format:browser-cache
#[rstest]
#[case::newer_minor(json!({ "format_version": "0.2", "artifact": { "kind": "browser", "name": "chrome-for-testing", "build": "b" }, "digest": BUILD_DIGEST, "installed_at": "2025-01-01T00:00:00Z", "used_at": "2025-01-01T00:00:00Z" }))]
#[case::newer_minor_with_a_field_it_adds(json!({ "format_version": "0.2", "artifact": { "kind": "browser", "name": "chrome-for-testing", "build": "b" }, "digest": BUILD_DIGEST, "installed_at": "2025-01-01T00:00:00Z", "used_at": "2025-01-01T00:00:00Z", "declared_by": "a Plugin" }))]
#[case::newer_major(json!({ "format_version": "1.0", "artifact": { "kind": "browser", "name": "chrome-for-testing", "build": "b" }, "digest": BUILD_DIGEST, "installed_at": "2025-01-01T00:00:00Z", "used_at": "2025-01-01T00:00:00Z" }))]
#[case::unknown_kind_named_in_it(json!({ "format_version": "0.1", "artifact": { "kind": "lightpanda", "name": "chrome-for-testing", "build": "b" }, "digest": BUILD_DIGEST, "installed_at": "2025-01-01T00:00:00Z", "used_at": "2025-01-01T00:00:00Z" }))]
#[case::not_json(json!("garbled"))]
fn the_clean_up_never_deletes_an_entry_it_cannot_read(#[case] entry: serde_json::Value) {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let id = browser("b");
    let folder = lay_out(&cache, &id, days_before(&env, 400));
    let text = match &entry {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    fs::write(folder.join("entry.json"), text).unwrap();

    let report = goodfetch_test_support::runtime()
        .block_on(cache.clean_up(&clean_up_at(&env, BTreeSet::new(), false)))
        .unwrap();

    assert_eq!(report.deleted, []);
    assert!(folder.join("entry.json").exists());
}

/// Covers: story:3-140
#[test]
fn the_clean_up_never_enters_a_folder_of_a_kind_it_does_not_know() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let unknown = cache
        .root()
        .join("lightpanda")
        .join("lightpanda")
        .join("old");
    build_files(&unknown);

    let report = goodfetch_test_support::runtime()
        .block_on(cache.clean_up(&clean_up_at(&env, BTreeSet::new(), false)))
        .unwrap();

    assert_eq!(report.deleted, []);
    assert!(unknown.join("chrome").exists());
}

/// Covers: story:3-140
#[test]
fn a_dry_run_lists_what_it_would_delete_and_deletes_nothing() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let old = browser("unpinned-and-unused");
    lay_out(&cache, &old, days_before(&env, 31));

    let report = goodfetch_test_support::runtime()
        .block_on(cache.clean_up(&clean_up_at(&env, BTreeSet::new(), true)))
        .unwrap();

    assert_eq!(report.deleted, std::slice::from_ref(&old));
    assert!(entry_exists(&cache, &old));
}

/// Each entry the clean-up deletes, or would delete, comes with the bytes
/// its folder takes, which `browser clean` reports as freed (spec #608).
///
/// Covers: story:11-296
#[rstest]
#[case::deleting(false)]
#[case::dry_run(true)]
fn the_clean_up_reports_the_bytes_each_entry_it_deletes_takes(#[case] dry_run: bool) {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let old = browser("unpinned-and-unused");
    lay_out(&cache, &old, days_before(&env, 31));

    let report = goodfetch_test_support::runtime()
        .block_on(cache.clean_up(&clean_up_at(&env, BTreeSet::new(), dry_run)))
        .unwrap();

    // 12 bytes of build files (`#!chrome\n` and `é\n`), an empty lock and a
    // 266-byte entry.json.
    assert_eq!(
        (report.deleted.clone(), report.bytes_of(&old)),
        (vec![old.clone()], Some(278))
    );
}

/// Covers: story:3-140
#[cfg(unix)]
#[test]
fn a_cache_the_engine_cannot_write_is_never_cleaned() {
    use goodfetch_protocol::browser_cache::NotCleaned;

    let env = TestEnv::new();
    let root = env.folder().join("read-only cache");
    let cache = BrowserCache::new(
        &CacheLocation::new(env.home(), Some(root.clone())),
        inline(),
    );
    let old = browser("unpinned-and-unused");
    lay_out(&cache, &old, days_before(&env, 31));
    make_read_only(&root);

    let report = goodfetch_test_support::runtime()
        .block_on(cache.clean_up(&clean_up_at(&env, BTreeSet::new(), false)))
        .unwrap();
    make_writable(&root);

    assert_eq!(report.deleted, []);
    assert_eq!(report.not_cleaned, Some(NotCleaned::CacheNotWritable));
    assert!(entry_exists(&cache, &old));
}

/// Covers: story:3-140
#[test]
fn a_deletion_cut_off_after_its_rename_finishes_on_the_next_run() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let cut_off = cache.root().join(".deleting").join("entry-cut-off");
    build_files(&cut_off.join("files"));

    goodfetch_test_support::runtime()
        .block_on(cache.clean_up(&clean_up_at(&env, BTreeSet::new(), false)))
        .unwrap();

    assert!(!cut_off.exists());
}

/// Covers: story:3-140, setting:browser_cache.max_unused_age
#[test]
fn the_automatic_clean_up_runs_at_most_once_a_day() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let runtime = goodfetch_test_support::runtime();
    let first = runtime
        .block_on(cache.clean_up_daily(
            BTreeSet::new(),
            days_before(&env, 1),
            MaxUnusedAge::default(),
        ))
        .unwrap();
    let old = browser("unpinned-and-unused");
    lay_out(&cache, &old, days_before(&env, 40));
    let an_hour_short_of_a_day = days_before(&env, 1) + jiff::SignedDuration::from_hours(23);

    let within_the_day = runtime
        .block_on(cache.clean_up_daily(
            BTreeSet::new(),
            an_hour_short_of_a_day,
            MaxUnusedAge::default(),
        ))
        .unwrap();
    let a_day_later = runtime
        .block_on(cache.clean_up_daily(BTreeSet::new(), now(&env), MaxUnusedAge::default()))
        .unwrap();

    assert!(first.is_some());
    assert_eq!(within_the_day, None);
    assert_eq!(a_day_later.map(|report| report.deleted), Some(vec![old]));
}

/// Covers: setting:browser_cache.max_unused_age
#[test]
fn a_null_max_unused_age_never_runs_the_automatic_clean_up() {
    let env = TestEnv::new();
    let cache = cache_in(&env);
    let old = browser("unpinned-and-unused");
    lay_out(&cache, &old, days_before(&env, 400));

    let report = goodfetch_test_support::runtime()
        .block_on(cache.clean_up_daily(BTreeSet::new(), now(&env), MaxUnusedAge::new(None)))
        .unwrap();

    assert_eq!(report, None);
    assert!(entry_exists(&cache, &old));
}

#[rstest]
#[case::windows_arm64_runs_x64_builds(Os::Windows, Arch::Arm64, Some(BuildPlatform::WindowsX64))]
#[case::linux_arm64(Os::LinuxGnu, Arch::Arm64, Some(BuildPlatform::LinuxArm64))]
#[case::musl_has_no_builds(Os::LinuxMusl, Arch::X64, None)]
fn browsers_install_fetches_the_builds_of_the_platform_a_host_runs(
    #[case] os: Os,
    #[case] arch: Arch,
    #[case] expected: Option<BuildPlatform>,
) {
    let platform = Platform { os, arch };

    assert_eq!(platform.build_platform(), expected);
}

const GLIBC_LDD: &[u8] = b"#! /bin/bash\n# This file is part of the GNU C Library.\n";
const ALPINE_LDD: &[u8] = b"#!/bin/sh\nexec /lib/ld-musl-x86_64.so.1 --list \"$@\"\n";

/// A static musl executable runs on any Linux, so the host's C library,
/// not the build's, decides whether Chrome for Testing runs there. The
/// host's `ldd` names it, as Node's `detect-libc` reads it (glibc's loader
/// does not: Alpine's `gcompat` installs it). An `ldd` that is missing,
/// unreadable or names neither leaves the musl build's own answer, musl.
#[rstest]
#[case::glibc_host(Some(GLIBC_LDD), Os::LinuxGnu)]
#[case::alpine_host(Some(ALPINE_LDD), Os::LinuxMusl)]
#[case::missing_or_unreadable_ldd(None, Os::LinuxMusl)]
#[case::ldd_naming_neither(Some(b"#!/bin/sh\n".as_slice()), Os::LinuxMusl)]
fn a_musl_executable_reads_the_c_library_of_the_linux_host_it_runs_on(
    #[case] ldd: Option<&[u8]>,
    #[case] expected: Os,
) {
    assert_eq!(super::platform::linux_os(ldd), expected);
}

/// Each platform Chrome for Testing builds has its name, which reads back as
/// it, and its executable's place in the folder its archive unpacks to;
/// Linux on arm64, which it builds nothing for, has neither.
#[rstest]
#[case::linux(
    BuildPlatform::LinuxX64,
    Some(("linux64", "chrome-linux64/chrome", "chrome-headless-shell-linux64/chrome-headless-shell"))
)]
#[case::mac_arm(
    BuildPlatform::MacArm64,
    Some((
        "mac-arm64",
        "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        "chrome-headless-shell-mac-arm64/chrome-headless-shell",
    ))
)]
#[case::mac_intel(
    BuildPlatform::MacX64,
    Some((
        "mac-x64",
        "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        "chrome-headless-shell-mac-x64/chrome-headless-shell",
    ))
)]
#[case::windows(
    BuildPlatform::WindowsX64,
    Some(("win64", "chrome-win64/chrome.exe", "chrome-headless-shell-win64/chrome-headless-shell.exe"))
)]
#[case::linux_arm(BuildPlatform::LinuxArm64, None)]
fn each_chrome_for_testing_platform_has_its_name_and_its_executables(
    #[case] platform: BuildPlatform,
    #[case] expected: Option<(&str, &str, &str)>,
) {
    let named = platform.chrome_for_testing_name();
    let executables = platform
        .chrome_for_testing_executable()
        .zip(platform.chrome_headless_shell_executable());

    assert_eq!(
        (
            named
                .zip(executables)
                .map(|(name, (full, shell))| (name, full, shell)),
            named.and_then(BuildPlatform::of_chrome_for_testing_name),
        ),
        (expected, expected.map(|_| platform))
    );
}
