//! Compiles the embedded Clean-up rules (ADRs 0047, 0048 and 0057): each
//! Filter list under `rules/`, with Brave's unbreak list, into one `adblock`
//! engine serialized with `Engine::serialize` from the binary's own
//! `adblock` version and compressed with zstd, beside the lines of each
//! that are `$generichide` exceptions (`filter_set::generic_hide_lines`),
//! which an engine keeps no text of; the never-block list and the
//! stand-ins' alias table into Rust tables; and the Consent prompt runner's
//! bundle (`consent`). Everything it reads is in the crate, so it downloads
//! nothing (STYLE.md).
#![expect(
    clippy::disallowed_methods,
    reason = "a build script is synchronous and reads its own environment and files"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use adblock::Engine;
use serde::Deserialize;

#[path = "build/consent.rs"]
mod consent;
#[path = "src/filter_set.rs"]
mod filter_set;

/// The embedded part's manifest, which names each list's file under
/// `rules/`, as a version of the Clean-up rules does.
const MANIFEST: &str = "rules/manifest.json";

const NEVER_BLOCK: &str = "rules/never-block.toml";

const ALIASES: &str = "stand-ins/aliases.toml";

/// zstd's level for the embedded engines: the strongest that still
/// compresses them in about a second.
const LEVEL: i32 = 19;

fn main() {
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("invariant: Cargo sets CARGO_MANIFEST_DIR"),
    );
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("invariant: Cargo sets OUT_DIR"));
    println!("cargo::rerun-if-changed=rules");
    println!("cargo::rerun-if-changed=stand-ins");
    println!("cargo::rerun-if-changed=consent");
    println!("cargo::rerun-if-changed=src/filter_set.rs");

    let lists = embedded_lists(&manifest);
    let unbreak = read(&manifest, &format!("rules/{}", lists.unbreak));
    let mut embedded = String::new();
    writeln!(
        embedded,
        "/// The unbreak list's `$generichide` exceptions.\n\
         pub(crate) const UNBREAK_GENERIC_HIDE: &str = {:?};",
        filter_set::generic_hide_lines(&unbreak)
    )
    .expect("invariant: writing to a String cannot fail");
    for (name, file) in lists.engines() {
        let text = read(&manifest, &format!("rules/{file}"));
        writeln!(
            embedded,
            "/// The `{name}` list's `$generichide` exceptions.\n\
             pub(crate) const {}_GENERIC_HIDE: &str = {:?};",
            name.to_ascii_uppercase(),
            filter_set::generic_hide_lines(&text)
        )
        .expect("invariant: writing to a String cannot fail");
        let set = filter_set::of(text, unbreak.clone());
        let serialized = Engine::new_with_filter_set(set).serialize();
        let compressed = zstd::bulk::compress(&serialized, LEVEL)
            .expect("invariant: zstd compresses a buffer in memory");
        let file = format!("{name}.dat.zst");
        std::fs::write(out.join(&file), &compressed)
            .expect("invariant: a build script can write its OUT_DIR");
        let size = separated(serialized.len());
        writeln!(
            embedded,
            "/// The `{name}` engine, serialized and compressed.\n\
             pub(crate) const {}: Compressed = Compressed {{ bytes: include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{file}\")), size: {size} }};",
            name.to_ascii_uppercase(),
        )
        .expect("invariant: writing to a String cannot fail");
    }
    writeln!(
        embedded,
        "/// The digest of the embedded part's files, by the Browser cache's tree rule.\n\
         const EMBEDDED_DIGEST_OF_BUILD: &str = {:?};",
        embedded_digest(&manifest, &lists)
    )
    .expect("invariant: writing to a String cannot fail");
    write(&out, "embedded.rs", &embedded);
    write(
        &out,
        "never_block.rs",
        &never_block(&read(&manifest, NEVER_BLOCK)),
    );
    write(
        &out,
        "stand_ins.rs",
        &stand_ins(&manifest, &read(&manifest, ALIASES)),
    );
    consent::bundle(&manifest, &out);
}

/// The `lists` of the embedded part's manifest: each Filter list's file
/// under `rules/`, by the `filter_lists.*` setting that turns it on, and
/// Brave's unbreak list's.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lists {
    ads: String,
    trackers: String,
    annoyances: String,
    unbreak: String,
}

impl Lists {
    /// Each list compiled to its own engine, by the name of the setting
    /// that turns it on, with its file: the unbreak list is compiled into
    /// each.
    fn engines(&self) -> [(&'static str, &str); 3] {
        [
            ("ads", &self.ads),
            ("trackers", &self.trackers),
            ("annoyances", &self.annoyances),
        ]
    }
}

/// The lists the embedded part's manifest names.
fn embedded_lists(manifest: &Path) -> Lists {
    #[derive(Deserialize)]
    struct Manifest {
        lists: Lists,
    }
    let parsed: Manifest = serde_json::from_str(&read(manifest, MANIFEST))
        .unwrap_or_else(|error| panic!("invariant: {MANIFEST} is well formed: {error}"));
    parsed.lists
}

/// The digest of the embedded part's files, its manifest and the `lists`
/// it names, `sha256:<hex>`, by the Browser cache's tree rule (ADR 0024):
/// SHA-256 over, for each file in the order of its path, the path, a NUL,
/// `f`, a NUL, the SHA-256 of its bytes and a newline.
fn embedded_digest(manifest: &Path, lists: &Lists) -> String {
    use sha2::{Digest as _, Sha256};
    let mut files = vec![
        lists.ads.as_str(),
        lists.trackers.as_str(),
        lists.annoyances.as_str(),
        lists.unbreak.as_str(),
        "manifest.json",
    ];
    files.sort_unstable();
    let mut tree = Sha256::new();
    for name in files {
        let bytes = std::fs::read(manifest.join("rules").join(name))
            .unwrap_or_else(|error| panic!("invariant: the crate carries rules/{name}: {error}"));
        tree.update(name.as_bytes());
        tree.update([0, b'f', 0]);
        tree.update(Sha256::digest(&bytes));
        tree.update(b"\n");
    }
    let mut digest = String::from("sha256:");
    for byte in tree.finalize() {
        write!(digest, "{byte:02x}").expect("invariant: writing to a String cannot fail");
    }
    digest
}

/// `size` as a Rust literal with `_` between each group of three digits.
fn separated(size: usize) -> String {
    let digits = size.to_string();
    let mut literal = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            literal.push('_');
        }
        literal.push(digit);
    }
    literal
}

fn read(manifest: &Path, path: &str) -> String {
    std::fs::read_to_string(manifest.join(path))
        .unwrap_or_else(|error| panic!("invariant: the crate carries {path}: {error}"))
}

fn write(out: &Path, name: &str, text: &str) {
    std::fs::write(out.join(name), text).expect("invariant: a build script can write its OUT_DIR");
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NeverBlock {
    entry: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    vendor: String,
    kind: Kind,
    rule: String,
    example: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Challenge,
    AntiBot,
    AntiFraud,
    Payment,
    Consent,
}

/// The never-block list as a Rust table of `Entry` values.
fn never_block(text: &str) -> String {
    let list: NeverBlock = toml::from_str(text)
        .unwrap_or_else(|error| panic!("invariant: {NEVER_BLOCK} is well formed: {error}"));
    let mut table = String::from("pub(crate) const ENTRIES: &[Entry] = &[\n");
    for entry in &list.entry {
        writeln!(
            table,
            "    Entry {{ vendor: {:?}, kind: Kind::{:?}, rule: {:?}, example: {:?} }},",
            entry.vendor, entry.kind, entry.rule, entry.example
        )
        .expect("invariant: writing to a String cannot fail");
    }
    table.push_str("];\n");
    table
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Aliases {
    stand_in: Vec<StandIn>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StandIn {
    names: Vec<String>,
    mime: String,
    file: Option<String>,
    body: Option<String>,
    base64: Option<String>,
}

/// The alias table as a Rust table of `StandIn` values, each file's bytes
/// included.
fn stand_ins(manifest: &Path, text: &str) -> String {
    let aliases: Aliases = toml::from_str(text)
        .unwrap_or_else(|error| panic!("invariant: {ALIASES} is well formed: {error}"));
    let mut table = String::from("pub(crate) const STAND_INS: &[StandIn] = &[\n");
    for stand_in in &aliases.stand_in {
        let body = match (&stand_in.file, &stand_in.body, &stand_in.base64) {
            (Some(file), None, None) => {
                let path = manifest.join("stand-ins").join(file);
                assert!(
                    path.is_file(),
                    "invariant: {ALIASES} names {file}, which the crate carries"
                );
                format!(
                    "Body::Bytes(include_bytes!({:?}))",
                    path.display().to_string()
                )
            }
            (None, Some(body), None) => format!("Body::Bytes({body:?}.as_bytes())"),
            (None, None, Some(base64)) => format!("Body::Base64({base64:?})"),
            _ => panic!(
                "invariant: each stand-in of {ALIASES} has one of `file`, `body` and `base64`"
            ),
        };
        writeln!(
            table,
            "    StandIn {{ names: &{:?}, mime: {:?}, body: {body} }},",
            stand_in.names, stand_in.mime
        )
        .expect("invariant: writing to a String cannot fail");
    }
    table.push_str("];\n");
    table
}
