//! Every writer the Products' crates hold, listed with how it meets the
//! `Mask` (spec #564): each place their code opens a file, writes one
//! whole, gives the `tracing` subscriber a writer, takes stdout or stderr,
//! creates a SQLite table or writes a Connection's lines is found in the
//! source and must be in [`WRITERS`], so a new writer cannot skip the
//! `Mask` unseen. Tests, benchmarks, examples, fuzz targets, build scripts,
//! generated code and the test-only crates write no Product output and are
//! left out.
//! Covers: story:4-56
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the workspace's source files, outside any async worker"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What finds a writer in a line of source.
const WRITING: [&str; 9] = [
    "OpenOptions::new()",
    "fs::write(",
    "File::create(",
    "File::create_new(",
    ".with_writer(",
    "io::stdout()",
    "io::stderr()",
    "CREATE TABLE",
    "fn write_lines(",
];

/// Each file holding writers, how many [`WRITING`] finds in it, and how
/// they meet the `Mask`: what they write is masked, where it says, or
/// holds no resolved value, for the reason it says.
const WRITERS: &[(&str, usize, &str)] = &[
    (
        "crates/foxhound/goodfetch-crawl/src/code_free.rs",
        1,
        "holds no value: a code-free crawl's extraction, the formats it asks for and which links it follows, as the program gave them",
    ),
    (
        "crates/foxhound/goodfetch-crawl/src/labels.rs",
        2,
        "holds no value: the Labels a crawl declares, their Payloads' schemas and their `fetch.tier`, and whether the Run and each Label keep URL fragments",
    ),
    (
        "crates/foxhound/goodfetch-crawl/src/tier_choice.rs",
        1,
        "holds no value: each Label's tier on each domain key, its samples' path segments and whether they matched",
    ),
    (
        "crates/foxhound/goodfetch-crawl/src/work_items.rs",
        6,
        "masked: a handler's failure is masked as `Scope::Kept` where the delivery receives it, a refused Record's `record_invalid` too; work items and outcomes hold URLs and the program's own data, and the reserved positions only a number; the request and Record counts hold an outcome's kind, its skip reason, Error code or Block cause (a code, never its message) and numbers",
    ),
    (
        "crates/foxhound/goodfetch-dataset/src/lib.rs",
        3,
        "masked: the Records a crawl commits, their Provenance and the rejection table's rejected Records with the check's reason and issues are written with every value the Run resolved as its `%name%` (`Scope::Kept`, in `records.rs`, #972); the Datasets a crawl declares (names, schemas, keys) are the program's own declarations",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/bounds.rs",
        5,
        "holds no value: the Frontier's counts and bounds",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/identity.rs",
        2,
        "holds no value: request identities: URL fingerprints",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/payload.rs",
        3,
        "holds no value: the program's own Payloads and the Labels' schemas",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/politeness.rs",
        3,
        "holds no value: per-domain Politeness state, the Crawl-delay each robots.txt asks for by domain key and location, its origin without credentials, and each domain key's pacing (its latency's mean and its token bucket), numbers only",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/queues.rs",
        1,
        "holds no value: per-domain queues of URLs",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/robots_txt.rs",
        1,
        "holds no value: robots.txt rules a site published",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/rules.rs",
        10,
        "holds no value: the crawl's scope and enqueue rules, from its inputs",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/scope.rs",
        2,
        "holds no value: the crawl's scope, from its inputs",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/seeds.rs",
        1,
        "holds no value: the crawl's seeds, from its inputs",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/sitemap.rs",
        3,
        "holds no value: a sitemap seed's sitemaps (URLs, the seed's rules), their entries' counts by reason, and each it could not read with the engine's own error code, status and message, no handler's",
    ),
    (
        "crates/foxhound/goodfetch-frontier/src/trap_links.rs",
        1,
        "holds no value: the IDs of the work items added through a link suspected a trap",
    ),
    (
        "crates/foxhound/goodfetch-session-pool/src/store.rs",
        3,
        "holds no value: Session pool entries: identities, counts and local-address positions, and the address blocks blocked on a domain, settings by reference",
    ),
    (
        "crates/goodfetch-cli/src/output.rs",
        4,
        "masked: the CLI prints what the engine's replies hold, masked by the RPC layer as `Scope::Returned`, and its own messages",
    ),
    (
        "crates/goodfetch-collie/src/executable/logs.rs",
        5,
        "masked: every log writer is `Masked`, `Scope::KeptOutput`",
    ),
    (
        "crates/goodfetch-collie/src/executable/service.rs",
        1,
        "holds no value: the service manager's definitions and the Daemon's token record, no Run's value",
    ),
    (
        "crates/goodfetch-engine/src/age_notice.rs",
        1,
        "holds no value: the date a notice was shown",
    ),
    (
        "crates/goodfetch-engine/src/attached/files.rs",
        2,
        "holds no value: the private endpoint's socket and its own token, no Run's value",
    ),
    (
        "crates/goodfetch-engine/src/audit.rs",
        2,
        "holds no value: each entry names a Secret reference and its placeholder, never a value (ADR 0016)",
    ),
    (
        "crates/goodfetch-engine/src/browser_cache/store.rs",
        4,
        "holds no value: pinned browser builds and their markers",
    ),
    (
        "crates/goodfetch-engine/src/checkpoint.rs",
        1,
        "masked: a copy of the run file, whose texts were masked as `Scope::Kept` when written",
    ),
    (
        "crates/goodfetch-engine/src/config.rs",
        1,
        "holds no value: the Operator policy's lock file, opened empty and only locked; the settings files themselves are written through `durable::replace`, holding a person's values and Secret references, never a Secret's value",
    ),
    (
        "crates/goodfetch-engine/src/crash_report.rs",
        1,
        "masked: each report is masked as `Scope::Kept`",
    ),
    (
        "crates/goodfetch-engine/src/daemon/files.rs",
        4,
        "holds no value: the Daemon's runtime file, token, lock and pick, no Run's value",
    ),
    (
        "crates/goodfetch-engine/src/installed.rs",
        1,
        "holds no value: installed browser builds",
    ),
    (
        "crates/goodfetch-engine/src/key_value_store.rs",
        1,
        "holds no value: the program's own values, which it set from masked reads",
    ),
    (
        "crates/goodfetch-engine/src/known_certificates.rs",
        3,
        "holds no value: hosts and key hashes",
    ),
    (
        "crates/goodfetch-engine/src/ledger.rs",
        3,
        "holds no value: Usage counts",
    ),
    (
        "crates/goodfetch-engine/src/lowest_minor.rs",
        1,
        "holds no value: the lowest-minor marker",
    ),
    (
        "crates/goodfetch-engine/src/people/store.rs",
        2,
        "holds no value: Console sign-ins by verifier, no Run's value",
    ),
    (
        "crates/goodfetch-engine/src/pinned_download.rs",
        1,
        "holds no value: a pinned download's bytes",
    ),
    (
        "crates/goodfetch-engine/src/politeness.rs",
        4,
        "holds no value: per-domain Politeness state",
    ),
    (
        "crates/goodfetch-engine/src/run_files.rs",
        2,
        "holds no value: the program's own files, content-addressed",
    ),
    (
        "crates/goodfetch-engine/src/run_index.rs",
        4,
        "masked: its progress message is masked as `Scope::Kept` where the program sets it; consent walls and refetch stops hold domains and causes",
    ),
    (
        "crates/goodfetch-engine/src/run_key.rs",
        1,
        "holds no value: the Run's data key, wrapped",
    ),
    (
        "crates/goodfetch-engine/src/run_state.rs",
        7,
        "masked: warnings, the progress message and the Run report, its error included, are masked as `Scope::Kept` in `runs` before they are written; settings are kept by reference",
    ),
    (
        "crates/goodfetch-engine/src/run_state/install.rs",
        4,
        "holds no value: the per-install layout's copies of run files, masked when written",
    ),
    (
        "crates/goodfetch-engine/src/run_trace.rs",
        1,
        "masked: the request and Provenance logs pass through the Run's `Mask`, `Scope::KeptOutput`",
    ),
    (
        "crates/goodfetch-engine/src/rpc/connection.rs",
        1,
        "masked: every line a Connection sends is masked as `Scope::Returned`",
    ),
    (
        "crates/goodfetch-engine/src/runs/folder.rs",
        2,
        "holds no value: the run folder's lock and identifier files",
    ),
    (
        "crates/goodfetch-engine/src/runs/import.rs",
        1,
        "holds no value: a run file another engine wrote, masked there",
    ),
    (
        "crates/goodfetch-engine/src/runs/writer.rs",
        2,
        "holds no value: the writer's lock file",
    ),
    (
        "crates/goodfetch-engine/src/transport.rs",
        1,
        "masked: the Attached engine's stdout carries only the lines `rpc` masked",
    ),
    (
        "crates/goodfetch-page/src/clean_up_versions.rs",
        1,
        "holds no value: downloaded Clean-up rules",
    ),
    (
        "crates/goodfetch-page/src/launch.rs",
        1,
        "holds no value: the browser profile's starting preferences",
    ),
    (
        "crates/goodfetch-page/src/proxy/limits.rs",
        1,
        "holds no value: proxy limits by endpoint and username, never a password",
    ),
    (
        "crates/goodfetch-page/src/sessions.rs",
        1,
        "holds no value: Sessions' settings, Secret settings by reference",
    ),
    (
        "crates/goodfetch-page/src/trace.rs",
        1,
        "holds no value: the CDP command log: each command's method alone; its Provenance log is `run_trace`'s",
    ),
];

/// The workspace's folder.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the engine's crate sits two folders under the workspace")
        .to_owned()
}

/// Whether `path`, a Rust file relative to the workspace, is Product code.
fn is_product_code(path: &str) -> bool {
    let left_out = [
        "/tests/",
        "/benches/",
        "/examples/",
        "/fuzz/",
        "/build/",
        "/generated/",
        "crates/goodfetch-test-support/",
        "crates/goodfetch-conformance/",
        "crates/goodfetch-stub-gateway/",
    ];
    !path.ends_with("tests.rs")
        && !path.ends_with("/build.rs")
        && !left_out.iter().any(|part| path.contains(part))
}

fn rust_files(folder: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(folder).expect("the source folder reads") {
        let path = entry.expect("a source entry reads").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                rust_files(&path, files);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// Each Product file holding writers, with how many [`WRITING`] finds in
/// its lines that are not comments.
fn found() -> BTreeMap<String, usize> {
    let root = workspace();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut found = BTreeMap::new();
    for file in files {
        let path = file
            .strip_prefix(&root)
            .expect("a source file is in the workspace")
            .to_string_lossy()
            .replace('\\', "/");
        if !is_product_code(&path) {
            continue;
        }
        let source = std::fs::read_to_string(&file).expect("a source file reads");
        let count: usize = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .map(|line| {
                WRITING
                    .iter()
                    .filter(|writing| line.contains(*writing))
                    .count()
            })
            .sum();
        if count > 0 {
            found.insert(path, count);
        }
    }
    found
}

#[test]
fn every_writer_of_the_products_is_listed_with_how_it_meets_the_mask() {
    let listed: BTreeMap<String, usize> = WRITERS
        .iter()
        .map(|(path, count, _)| ((*path).to_owned(), *count))
        .collect();

    let found = found();

    let unlisted: Vec<String> = found
        .iter()
        .filter(|(path, count)| listed.get(*path) != Some(count))
        .map(|(path, count)| format!("{path}: {count}"))
        .collect();
    let gone: Vec<&String> = listed
        .keys()
        .filter(|path| !found.contains_key(*path))
        .collect();
    assert_eq!(
        (unlisted, gone),
        (Vec::<String>::new(), Vec::<&String>::new()),
        "list each new writer, or each one whose count changed, in WRITERS with how it meets the Mask"
    );
}
