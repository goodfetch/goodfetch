//! The Rust install docs (story 11-280; ADRs 0094 and 0124): the crate's
//! README and the docs' Install page, `docs/install.md`, each list the
//! tools a build from source needs, which `source_build` removes one at a
//! time from a clean container, with the Debian line that installs exactly
//! the clean container's packages, and each says that `cargo install`
//! builds an unsigned executable from source and gives the way to the
//! release's signed archive: the crate's `cargo binstall` metadata and the
//! archive's attestation on its GitHub Release.

use rstest::rstest;

const README: &str = include_str!("../../README.md");
const PAGE: &str = include_str!("../../../../docs/install.md");
const MANIFEST: &str = include_str!("../../Cargo.toml");
const WORKSPACE_MANIFEST: &str = include_str!("../../../../Cargo.toml");
pub(crate) const CONTAINERFILE: &str = include_str!("../../../../ci/source-build/Containerfile");

/// A tool a build from source needs: its name as the docs list it, the
/// Debian package their `apt install` line names for it, and the `dpkg`
/// package pattern that removes it from the clean container (a
/// metapackage's removal would leave the tool there).
pub(crate) struct Prerequisite {
    pub(crate) name: &'static str,
    pub(crate) package: &'static str,
    #[cfg_attr(
        not(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")),
        expect(
            dead_code,
            reason = "only `source_build` removes a tool, on Linux x86-64 glibc"
        )
    )]
    pub(crate) installed_as: &'static str,
}

/// Every tool a build from source needs, in the docs' order.
pub(crate) const PREREQUISITES: [Prerequisite; 5] = [
    Prerequisite {
        name: "CMake",
        package: "cmake",
        installed_as: "cmake",
    },
    Prerequisite {
        name: "make",
        package: "make",
        installed_as: "make",
    },
    Prerequisite {
        name: "a C and C++ compiler",
        package: "g++",
        installed_as: "g++*",
    },
    Prerequisite {
        name: "libclang",
        package: "libclang-dev",
        installed_as: "libclang*",
    },
    Prerequisite {
        name: "git",
        package: "git",
        installed_as: "git",
    },
];

/// A document and the headings of its two sections: the tools, and the
/// executable.
struct Doc {
    name: &'static str,
    text: &'static str,
    tools: &'static str,
    executable: &'static str,
}

const README_DOC: Doc = Doc {
    name: "README.md",
    text: README,
    tools: "## Prerequisites",
    executable: "## The executable",
};

const PAGE_DOC: Doc = Doc {
    name: "docs/install.md",
    text: PAGE,
    tools: "### What a build from source needs",
    executable: "### The executable",
};

/// The section under `heading`, up to the next heading of its level or
/// above.
fn section<'a>(doc: &'a Doc, heading: &str) -> &'a str {
    let level = heading.find(' ').expect("a heading has a space");
    let start = doc
        .text
        .find(&format!("\n{heading}\n"))
        .unwrap_or_else(|| panic!("{} has no {heading:?}", doc.name))
        + 1;
    let body = &doc.text[start + heading.len()..];
    let end = body
        .match_indices("\n#")
        .find(|(at, _)| {
            let hashes = body[at + 1..].chars().take_while(|c| *c == '#').count();
            hashes <= level && body[at + 1 + hashes..].starts_with(' ')
        })
        .map_or(body.len(), |(at, _)| at);
    &body[..end]
}

/// The section's bullet items.
fn bullets(section: &str) -> Vec<&str> {
    section
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .collect()
}

/// The lines of the section's `sh` blocks.
fn shell_lines(section: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut inside = false;
    for line in section.lines() {
        match (inside, line) {
            (false, "```sh") => inside = true,
            (true, "```") => inside = false,
            (true, line) => lines.push(line),
            (false, _) => {}
        }
    }
    lines
}

/// The section's prose, outside its code blocks, on one line.
fn prose(section: &str) -> String {
    let mut words = Vec::new();
    let mut inside = false;
    for line in section.lines() {
        if line.starts_with("```") {
            inside = !inside;
        } else if !inside {
            words.extend(line.split_whitespace());
        }
    }
    words.join(" ")
}

/// The packages the clean container's `apt-get install` installs.
fn container_packages() -> Vec<&'static str> {
    let line = CONTAINERFILE
        .lines()
        .find_map(|line| line.split_once("apt-get install -y --no-install-recommends "))
        .expect("the Containerfile installs its packages with apt-get")
        .1;
    line.trim_end_matches('\\').split_whitespace().collect()
}

fn manifest(text: &str) -> toml::Table {
    text.parse().expect("the manifest is TOML")
}

/// The docs list exactly the tools the clean container holds, and their
/// Debian line installs exactly its packages, so `source_build`'s builds
/// prove the docs' list.
///
/// Covers: story:11-280
#[rstest]
#[case::readme(README_DOC)]
#[case::page(PAGE_DOC)]
fn the_docs_list_the_tools_a_build_from_source_needs_and_the_line_that_installs_them(
    #[case] doc: Doc,
) {
    let tools = section(&doc, doc.tools);
    let names: Vec<&str> = PREREQUISITES.iter().map(|tool| tool.name).collect();
    assert_eq!(bullets(tools), names, "{}", doc.name);

    let packages: Vec<&str> = PREREQUISITES.iter().map(|tool| tool.package).collect();
    let install = format!("apt install {}", packages.join(" "));
    assert_eq!(shell_lines(tools), [install.as_str()], "{}", doc.name);
    assert_eq!(container_packages(), packages);
}

/// The docs say that `cargo install` builds an unsigned executable from
/// source, compiling V8 with its sandbox on for about 30 minutes, with the
/// feature the crate's executable requires, and give the signed archive's
/// way: `cargo binstall`, which the crate's metadata points at the GitHub
/// Release, and `gh attestation verify` of the archive it installs.
///
/// Covers: story:11-280
#[rstest]
#[case::readme(README_DOC)]
#[case::page(PAGE_DOC)]
fn the_docs_say_the_source_build_is_unsigned_and_give_the_signed_archive_s_way(#[case] doc: Doc) {
    let crate_manifest = manifest(MANIFEST);
    let name = crate_manifest["package"]["name"]
        .as_str()
        .expect("the crate has a name");
    let bin = crate_manifest["bin"]
        .as_array()
        .and_then(|bins| bins.iter().find(|bin| bin["name"].as_str() == Some(name)))
        .expect("the crate has its executable");
    let features: Vec<&str> = bin["required-features"]
        .as_array()
        .expect("the executable requires features")
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    let binstall = crate_manifest["package"]["metadata"]["binstall"]["pkg-url"]
        .as_str()
        .expect("the crate has `cargo binstall` metadata");
    let repository = manifest(WORKSPACE_MANIFEST)["workspace"]["package"]["repository"]
        .as_str()
        .expect("the workspace names its repository")
        .trim_start_matches("https://github.com/")
        .to_owned();
    let archive = binstall
        .rsplit('/')
        .next()
        .expect("the URL names a file")
        .replace("{ name }", name)
        .replace("{ target }", "<target>");
    assert_eq!(
        binstall, "{ repo }/releases/download/v{ version }/{ name }-{ target }.tar.xz",
        "the docs' download line follows this URL",
    );

    let executable = section(&doc, doc.executable);
    assert_eq!(
        shell_lines(executable),
        [
            format!(
                "cargo install {name} --locked --features {}",
                features.join(",")
            ),
            format!("cargo binstall {name}"),
            format!("gh release download v<version> --repo {repository} --pattern '{archive}'"),
            format!("gh attestation verify {archive} --repo {repository}"),
        ],
        "{}",
        doc.name,
    );
    let prose = prose(executable);
    for statement in [
        "compiles V8 from source with its sandbox on",
        "often takes 30 minutes",
        "The executable it writes is unsigned",
        "The release's own archive is signed",
        "cargo-binstall checks only minisign signatures",
    ] {
        assert!(
            prose.contains(statement),
            "{} does not say {statement:?}",
            doc.name
        );
    }
}
