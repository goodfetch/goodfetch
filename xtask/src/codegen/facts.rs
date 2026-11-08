//! The facts tests read in more than one crate or language: the Secret
//! canary kinds (ADR 0067), the browser builds the Products pin (ADRs 0020
//! and 0024) with the Playwright revision that runs Collie's, and the
//! `tracing` targets the exercise audit reads (ADR 0072). Each has one
//! source in Rust; `xtask codegen` reads it there and writes it for each
//! reader that cannot depend on that source: test support, which takes no
//! Goodfetch crate (ADR 0062), `xtask`, which builds no engine crate
//! (ADR 0072), and the TypeScript and Python helpers.
//!
//! A pin is read from the `pins.json` its crate embeds, a constant from its
//! Rust source by name, and the kinds from `goodfetch-test-env
//! --print-secret-kinds`, since they are an enum's.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

/// Test support's generated folder, which its `facts` module includes.
pub(super) const TEST_SUPPORT_FOLDER: &str = "crates/goodfetch-test-support/generated";

/// `xtask`'s generated folder, which its `test_facts` module includes.
pub(super) const XTASK_FOLDER: &str = "xtask/generated";

/// Where one fact is kept.
enum Source {
    /// A string in a JSON file, at a JSON pointer.
    Json {
        path: &'static str,
        pointer: &'static str,
    },
    /// A `const` string of that name in a Rust file.
    Constant {
        path: &'static str,
        name: &'static str,
    },
}

impl Source {
    fn path(&self) -> &'static str {
        match self {
            Self::Json { path, .. } | Self::Constant { path, .. } => path,
        }
    }

    /// The fact's value, read at `root`.
    fn read(&self, root: &Path) -> Result<String> {
        let path = self.path();
        let text =
            std::fs::read_to_string(root.join(path)).with_context(|| format!("reading {path}"))?;
        match self {
            Self::Json { pointer, .. } => {
                let json: Value =
                    serde_json::from_str(&text).with_context(|| format!("parsing {path}"))?;
                json.pointer(pointer)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .with_context(|| format!("{path} holds no string at {pointer}"))
            }
            Self::Constant { name, .. } => {
                let file = syn::parse_file(&text).with_context(|| format!("parsing {path}"))?;
                file.items
                    .iter()
                    .find_map(|item| match item {
                        syn::Item::Const(constant) if constant.ident == name => {
                            match &*constant.expr {
                                syn::Expr::Lit(syn::ExprLit {
                                    lit: syn::Lit::Str(value),
                                    ..
                                }) => Some(value.value()),
                                _ => None,
                            }
                        }
                        _ => None,
                    })
                    .with_context(|| format!("{path} has no string constant `{name}`"))
            }
        }
    }
}

/// One fact: its name in every generated file, what it is, and its source.
struct Fact {
    name: &'static str,
    doc: &'static str,
    source: Source,
}

const CHROME_FOR_TESTING_BUILD: Fact = Fact {
    name: "CHROME_FOR_TESTING_BUILD",
    doc: "The Chrome for Testing build Collie pins.",
    source: Source::Json {
        path: "crates/goodfetch-collie/src/pins.json",
        pointer: "/chrome_for_testing/build",
    },
};

const LIGHTPANDA_BUILD: Fact = Fact {
    name: "LIGHTPANDA_BUILD",
    doc: "The Lightpanda build Foxhound pins.",
    source: Source::Json {
        path: "crates/foxhound/goodfetch-fetch-tier/src/light_browser/pins.json",
        pointer: "/lightpanda/build",
    },
};

const PLAYWRIGHT_REVISION: Fact = Fact {
    name: "PLAYWRIGHT_REVISION",
    doc: "Playwright's revision of the Chrome for Testing build Collie pins: its folder's name.",
    source: Source::Constant {
        path: "crates/goodfetch-test-support/src/browsers.rs",
        name: "PLAYWRIGHT_REVISION",
    },
};

const DISPATCH_TARGET: Fact = Fact {
    name: "DISPATCH_TARGET",
    doc: "The `tracing` target of `rpc`'s dispatch records.",
    source: Source::Constant {
        path: "crates/goodfetch-engine/src/rpc.rs",
        name: "DISPATCH_TARGET",
    },
};

const OUTCOME_TARGET: Fact = Fact {
    name: "OUTCOME_TARGET",
    doc: "The `tracing` target of the engine's outcome records: Error codes no reply carries.",
    source: Source::Constant {
        path: "crates/goodfetch-engine/src/runs/outcome.rs",
        name: "OUTCOME_TARGET",
    },
};

const COMMAND_TARGET: Fact = Fact {
    name: "COMMAND_TARGET",
    doc: "The `tracing` target of test support's command records.",
    source: Source::Constant {
        path: "crates/goodfetch-test-support/src/exercise.rs",
        name: "COMMAND_TARGET",
    },
};

/// The facts test support reads from crates it cannot depend on.
const TEST_SUPPORT: [&Fact; 4] = [
    &CHROME_FOR_TESTING_BUILD,
    &LIGHTPANDA_BUILD,
    &DISPATCH_TARGET,
    &OUTCOME_TARGET,
];

/// The facts `xtask` reads from crates it cannot depend on.
const XTASK: [&Fact; 5] = [
    &CHROME_FOR_TESTING_BUILD,
    &PLAYWRIGHT_REVISION,
    &DISPATCH_TARGET,
    &OUTCOME_TARGET,
    &COMMAND_TARGET,
];

/// Every file the facts are written to, by path relative to the root.
pub(super) fn files(cargo: &OsStr, root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let secret_kinds = super::test_support::secret_kinds(cargo, root)?;
    if secret_kinds.is_empty() {
        bail!("goodfetch-test-env printed no Secret canary kind");
    }
    Ok(vec![
        (
            format!("{TEST_SUPPORT_FOLDER}/facts.rs"),
            rust_constants(root, &TEST_SUPPORT, "pub")?.into_bytes(),
        ),
        (
            format!("{XTASK_FOLDER}/test_facts.rs"),
            rust_constants(root, &XTASK, "pub(crate)")?.into_bytes(),
        ),
        (
            format!("{}/secret-kinds.ts", super::helpers::TYPESCRIPT_FOLDER),
            typescript_kinds(&secret_kinds).into_bytes(),
        ),
        (
            format!("{}/secret_kinds.py", super::helpers::PYTHON_FOLDER),
            python_kinds(&secret_kinds).into_bytes(),
        ),
    ])
}

/// What every file says first, after its comment marker.
const NOTICE: &str =
    "Generated by `xtask codegen` from each fact's source in Rust; never edit by hand.";

/// The Rust constants of `facts`, each `visibility`, with its source named.
fn rust_constants(root: &Path, facts: &[&Fact], visibility: &str) -> Result<String> {
    let mut text = format!("// {NOTICE}\n");
    for fact in facts {
        let value = fact.source.read(root)?;
        write!(
            text,
            "\n/// {}\n///\n/// From `{}`.\n{visibility} const {}: &str = {value:?};\n",
            fact.doc,
            fact.source.path(),
            fact.name,
        )
        .expect("invariant: writing to a String succeeds");
    }
    Ok(text)
}

/// The kinds, each as a string literal on its own line after `indent`.
fn listed(kinds: &[String], indent: &str) -> String {
    kinds.iter().fold(String::new(), |mut text, kind| {
        writeln!(text, "{indent}{kind:?},").expect("invariant: writing to a String succeeds");
        text
    })
}

fn typescript_kinds(kinds: &[String]) -> String {
    format!(
        "// {NOTICE}\n\n\
         /** The kinds of secret a canary stands for, as Rust's `SecretKind::ALL` lists them (ADR 0067). */\n\
         export const SECRET_KINDS = [\n{}] as const;\n\n\
         /** A kind of secret a canary stands for. */\n\
         export type SecretKind = (typeof SECRET_KINDS)[number];\n",
        listed(kinds, "  "),
    )
}

fn python_kinds(kinds: &[String]) -> String {
    let listed = listed(kinds, "    ");
    format!(
        "\"\"\"The Secret canary kinds (ADR 0067), as Rust's `SecretKind::ALL` lists them.\n\n\
         {NOTICE}\n\"\"\"\n\n\
         from typing import Literal\n\n\
         SecretKind = Literal[\n{listed}]\n\
         \"\"\"A kind of secret a canary stands for.\"\"\"\n\n\
         SECRET_KINDS: tuple[SecretKind, ...] = (\n{listed})\n\
         \"\"\"Every kind, in Rust's order.\"\"\"\n",
    )
}
