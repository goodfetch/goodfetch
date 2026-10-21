//! The TLC job's own logic (ADR 0060): `xtask tlc fetch` keeps the pinned
//! pair of TLA+ tools only when each download's SHA-256 is its pin, so a
//! changed download fails the job before the deny. Its downloads come from a
//! local stand-in of their release host. `xtask affected` selects the job,
//! and the models a change reaches through their own files or the modules
//! they trace.

use rstest::rstest;

use crate::affected::{commit_all, repository, value};
use crate::github::GitHub;
use crate::workspace::{Workspace, finding};

/// The SHA-256 of `TLA2TOOLS` and of `COMMUNITY_MODULES`, from `shasum -a 256`.
const TLA2TOOLS: &str = "stand-in tla2tools.jar";
const TLA2TOOLS_SHA256: &str = "e0829182c2c599608f165e38fa0416c094fb7ad4bbb5949eab3df602123158f9";
const COMMUNITY_MODULES: &str = "stand-in CommunityModules-deps.jar";
const COMMUNITY_MODULES_SHA256: &str =
    "81debe735be5448aee4addccfa418e815a9a57baecde7040b8856af26b27848a";

const TOOLS_FILE: &str = "test-support/models/tools.toml";

fn release_host() -> GitHub {
    GitHub::serving(&[
        ("/tla2tools.jar", TLA2TOOLS),
        ("/CommunityModules-deps.jar", COMMUNITY_MODULES),
    ])
}

/// A workspace whose tools file pins both stand-in downloads, the first at
/// `tla2tools_sha256`.
fn pinned(host: &GitHub, tla2tools_sha256: &str) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    let url = host.url();
    workspace.write(
        TOOLS_FILE,
        format!(
            "[[tool]]\nname = \"tla2tools.jar\"\nurl = \"{url}/tla2tools.jar\"\nsha256 = \"{tla2tools_sha256}\"\n\n\
             [[tool]]\nname = \"CommunityModules-deps.jar\"\nurl = \"{url}/CommunityModules-deps.jar\"\nsha256 = \"{COMMUNITY_MODULES_SHA256}\"\n"
        )
        .as_bytes(),
    );
    workspace
}

#[test]
fn fetch_keeps_each_download_whose_digest_is_its_pin() {
    let host = release_host();
    let workspace = pinned(&host, TLA2TOOLS_SHA256);

    let outcome = workspace.xtask(&["tlc", "fetch"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        (
            workspace.read("target/tla-tools/tla2tools.jar"),
            workspace.read("target/tla-tools/CommunityModules-deps.jar"),
        ),
        (
            Some(TLA2TOOLS.as_bytes().to_vec()),
            Some(COMMUNITY_MODULES.as_bytes().to_vec()),
        )
    );
}

#[test]
fn fetch_fails_on_a_download_whose_digest_differs_from_its_pin() {
    let host = release_host();
    let workspace = pinned(&host, COMMUNITY_MODULES_SHA256);

    let outcome = workspace.xtask(&["tlc", "fetch"]);

    assert_eq!(
        (
            outcome.code,
            outcome.findings(),
            workspace.read("target/tla-tools/tla2tools.jar")
        ),
        (1, vec![finding("digest-mismatch", "tla2tools.jar")], None)
    );
}

#[test]
fn fetch_downloads_nothing_when_each_kept_file_has_its_pinned_digest() {
    let host = release_host();
    let workspace = pinned(&host, TLA2TOOLS_SHA256);
    assert_eq!(workspace.xtask(&["tlc", "fetch"]).code, 0);
    let downloads = host.requests().len();

    let outcome = workspace.xtask(&["tlc", "fetch"]);

    assert_eq!(
        (outcome.code, host.requests().len()),
        (0, downloads),
        "{}",
        outcome.stderr
    );
}

#[test]
fn fetch_replaces_a_kept_file_whose_digest_differs_from_its_pin() {
    let host = release_host();
    let workspace = pinned(&host, TLA2TOOLS_SHA256);
    workspace.write("target/tla-tools/tla2tools.jar", b"a changed jar");

    let outcome = workspace.xtask(&["tlc", "fetch"]);

    assert_eq!(
        (
            outcome.code,
            workspace.read("target/tla-tools/tla2tools.jar")
        ),
        (0, Some(TLA2TOOLS.as_bytes().to_vec())),
        "{}",
        outcome.stderr
    );
}

/// The release of the repository that keeps a copy of the tools, as GitHub's
/// API lists it: asset 7 is `tla2tools.jar`.
const RELEASE: &str = "/repos/goodfetch/goodfetch/releases/tags/tla-tools-1";
const RELEASE_LISTING: &str =
    r#"{"tag_name":"tla-tools-1","assets":[{"id":7,"name":"tla2tools.jar"}]}"#;
const ASSET: &str = "/repos/goodfetch/goodfetch/releases/assets/7";

/// A workspace whose tools file pins `tla2tools.jar` as `asset` of the
/// repository's `tla-tools-1` release, at its stand-in's SHA-256.
fn pinned_to_release(asset: &str) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        TOOLS_FILE,
        format!(
            "[[tool]]\nname = \"tla2tools.jar\"\n\
             release = {{ repository = \"goodfetch/goodfetch\", tag = \"tla-tools-1\", asset = \"{asset}\" }}\n\
             sha256 = \"{TLA2TOOLS_SHA256}\"\n"
        )
        .as_bytes(),
    );
    workspace
}

#[test]
fn fetch_downloads_a_release_asset_through_the_api_with_the_token() {
    let github = GitHub::serving(&[(RELEASE, RELEASE_LISTING), (ASSET, TLA2TOOLS)]);
    let workspace = pinned_to_release("tla2tools.jar");

    let outcome = workspace.xtask_with_env(
        &["tlc", "fetch", "--api-url", github.url()],
        &[("GITHUB_TOKEN", "a-read-token".as_ref())],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let asset = github
        .requests()
        .into_iter()
        .find(|request| request.path == ASSET)
        .map(|request| (request.authorization, request.accept));
    assert_eq!(
        (workspace.read("target/tla-tools/tla2tools.jar"), asset),
        (
            Some(TLA2TOOLS.as_bytes().to_vec()),
            Some((
                Some("Bearer a-read-token".to_owned()),
                Some("application/octet-stream".to_owned())
            ))
        )
    );
}

#[rstest]
#[case::a_release_without_the_asset(&[(RELEASE, RELEASE_LISTING), (ASSET, TLA2TOOLS)], "tla2tools-renamed.jar")]
#[case::no_such_release(&[], "tla2tools.jar")]
fn fetch_fails_on_a_release_asset_it_cannot_find(
    #[case] routes: &[(&str, &str)],
    #[case] asset: &str,
) {
    let github = GitHub::serving(routes);
    let workspace = pinned_to_release(asset);

    let outcome = workspace.xtask(&["tlc", "fetch", "--api-url", github.url()]);

    assert_eq!(
        (
            outcome.code,
            outcome.findings(),
            workspace.read("target/tla-tools/tla2tools.jar")
        ),
        (1, vec![finding("missing-asset", "tla2tools.jar")], None)
    );
}

/// `xtask affected`'s `jobs` and `models` for one change of the `affected`
/// fixture, whose `fencing` model traces the engine's `lease.rs` and the
/// frontier's `claim/` folder, and whose `sample` model traces nothing.
fn selection_for(path: &str) -> (String, String) {
    let workspace = repository();
    workspace.write(path, b"\\* Changed.\n");
    commit_all(&workspace, "Change one file");

    let outcome = workspace.xtask(&["affected", "--base", "main", "--head", "change"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    (value(&outcome, "jobs"), value(&outcome, "models"))
}

#[rstest]
#[case::the_model_itself("test-support/models/fencing/Fencing.tla", r#"["tlc"]"#, "fencing")]
#[case::a_traced_file(
    "crates/goodfetch-engine/src/lease.rs",
    r#"["pre-merge","sinks","perf","tlc"]"#,
    "fencing"
)]
#[case::a_file_in_a_traced_folder(
    "crates/foxhound/goodfetch-frontier/src/claim/slice.rs",
    r#"["pre-merge","sinks","tlc"]"#,
    "fencing"
)]
#[case::the_pinned_tools("test-support/models/tools.toml", r#"["tlc"]"#, "fencing sample")]
fn change_selects_the_models_it_reaches(
    #[case] path: &str,
    #[case] jobs: &str,
    #[case] models: &str,
) {
    assert_eq!(selection_for(path), (jobs.to_owned(), models.to_owned()));
}

#[test]
fn change_reaching_no_model_selects_none() {
    assert_eq!(
        selection_for("crates/goodfetch-engine/src/lib.rs"),
        (r#"["pre-merge","sinks","perf"]"#.to_owned(), String::new())
    );
}
