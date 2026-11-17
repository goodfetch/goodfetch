//! `xtask docs-samples extract`: every runnable block of each Product's
//! rendering of the docs pages and of the package READMEs, written to a file
//! for its tool under `target/docs-samples/` with `manifest.json` (ADR 0068,
//! as ADR 0097 amends it). A page both Products carry runs in Collie's
//! rendering and is checked statically in Foxhound's; no file holds the
//! online Proving ground's origin; a code block that carries the other
//! Product's names, or a page the rendering cannot read, fails and writes
//! nothing.

use rstest::rstest;
use serde_json::{Value, json};

use crate::workspace::{Finding, Workspace, finding};

const OUTPUT: &str = "target/docs-samples";

/// The manifest's entries for `page`'s `rendering`, each with the `fields`
/// named.
fn entries(workspace: &Workspace, page: &str, rendering: &str, fields: &[&str]) -> Vec<Value> {
    let manifest: Value =
        serde_json::from_str(&workspace.read_text(&format!("{OUTPUT}/manifest.json"))).unwrap();
    manifest["samples"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|sample| sample["page"] == page && sample["rendering"] == rendering)
        .map(|sample| {
            fields
                .iter()
                .map(|field| ((*field).to_owned(), sample[*field].clone()))
                .collect::<serde_json::Map<_, _>>()
                .into()
        })
        .collect()
}

fn sample(relative: &str) -> String {
    format!("{OUTPUT}/{relative}")
}

/// A workspace from the clean fixture with `case` copied to `page`, and
/// `xtask docs-samples extract` run in it.
fn extracted_with(case: &str, page: &str) -> (Workspace, crate::workspace::Outcome) {
    let workspace = Workspace::from_fixture("docs-samples");
    workspace.add_fixture_file(&format!("docs-samples-cases/{case}"), page);
    let outcome = workspace.xtask(&["docs-samples", "extract"]);
    (workspace, outcome)
}

#[test]
fn collie_rendering_of_a_page_both_products_carry_runs_every_sample() {
    let workspace = Workspace::from_fixture("docs-samples");

    let outcome = workspace.xtask(&["docs-samples", "extract"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0);
    let fields = [
        "line", "language", "markers", "runner", "product", "check", "file", "origin",
    ];
    assert_eq!(
        entries(&workspace, "docs/guide.mdx", "collie", &fields),
        vec![
            json!({"line": 7, "language": "ts", "markers": [], "runner": "vitest", "product": "collie", "check": "run", "file": "typescript/collie/docs/guide/line_7.ts", "origin": [11]}),
            json!({"line": 14, "language": "tsx", "markers": [], "runner": "vitest", "product": "collie", "check": "run", "file": "typescript/collie/docs/guide/line_14.tsx", "origin": [15]}),
            json!({"line": 18, "language": "python", "markers": [], "runner": "pytest", "product": "collie", "check": "run", "file": "python/collie/docs/guide/line_18.py", "origin": [22]}),
            json!({"line": 25, "language": "rust", "markers": [], "runner": "rustdoc", "product": "collie", "check": "run", "file": "rust/collie/docs/guide.md", "origin": [26]}),
            json!({"line": 30, "language": "console", "markers": [], "runner": "trycmd", "product": "collie", "check": "run", "file": "trycmd/collie/docs/guide/line_30.trycmd", "origin": [31]}),
            json!({"line": 35, "language": "json", "markers": ["mcp-request"], "runner": "rmcp", "product": "collie", "check": "run", "file": "mcp/collie/docs/guide/line_35.json", "origin": [40]}),
        ]
    );
}

#[test]
fn foxhound_rendering_of_a_page_both_products_carry_is_checked_statically() {
    let workspace = Workspace::from_fixture("docs-samples");

    let outcome = workspace.xtask(&["docs-samples", "extract"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
    assert_eq!(
        entries(
            &workspace,
            "docs/guide.mdx",
            "foxhound",
            &["line", "product", "check", "file"]
        ),
        vec![
            json!({"line": 7, "product": "foxhound", "check": "static", "file": "typescript/foxhound/docs/guide/line_7.ts"}),
            json!({"line": 14, "product": "foxhound", "check": "static", "file": "typescript/foxhound/docs/guide/line_14.tsx"}),
            json!({"line": 18, "product": "foxhound", "check": "static", "file": "python/foxhound/docs/guide/line_18.py"}),
            json!({"line": 25, "product": "foxhound", "check": "static", "file": "rust/foxhound/docs/guide.md"}),
            json!({"line": 30, "product": "foxhound", "check": "listed", "file": null}),
            json!({"line": 35, "product": "foxhound", "check": "listed", "file": null}),
        ]
    );
}

/// Each file writes its rendering's names, holds the placeholder where the
/// page writes the online origin, and drops Twoslash's cut line; a `rust`
/// literal of the origin reads it from `GOODFETCH_DOCS_ORIGIN`.
#[rstest]
#[case::typescript_module(
    "typescript/collie/docs/guide/line_7.ts",
    "import { Collie } from \"@goodfetch/collie\";\nconst engine = await Collie.connect();\nawait engine.page.goto(\"__GOODFETCH_DOCS_ORIGIN__/catalog\");\n"
)]
#[case::foxhound_typescript_module(
    "typescript/foxhound/docs/guide/line_7.ts",
    "import { Foxhound } from \"@goodfetch/foxhound\";\nconst engine = await Foxhound.connect();\nawait engine.page.goto(\"__GOODFETCH_DOCS_ORIGIN__/catalog\");\n"
)]
#[case::python_module(
    "python/collie/docs/guide/line_18.py",
    "from goodfetch_collie import Collie\n\nengine = Collie.connect()\nengine.page.goto(\"__GOODFETCH_DOCS_ORIGIN__/catalog\")\n"
)]
#[case::rust_page_that_runs(
    "rust/collie/docs/guide.md",
    "```rust\n# macro_rules! goodfetch_docs_origin { ($path:literal) => { concat!(env!(\"GOODFETCH_DOCS_ORIGIN\"), $path) }; }\nlet catalog = goodfetch_docs_origin!(\"/catalog\");\nassert!(catalog.ends_with(\"/catalog\"));\n```\n"
)]
#[case::rust_page_checked_statically(
    "rust/foxhound/docs/guide.md",
    "```rust no_run\n# macro_rules! goodfetch_docs_origin { ($path:literal) => { concat!(env!(\"GOODFETCH_DOCS_ORIGIN\"), $path) }; }\nlet catalog = goodfetch_docs_origin!(\"/catalog\");\nassert!(catalog.ends_with(\"/catalog\"));\n```\n"
)]
#[case::console_session(
    "trycmd/collie/docs/guide/line_30.trycmd",
    "```console\n$ goodfetch-collie fetch __GOODFETCH_DOCS_ORIGIN__/catalog\n...\n```\n"
)]
fn extracted_file_holds_the_rendered_sample(#[case] file: &str, #[case] expected: &str) {
    let workspace = Workspace::from_fixture("docs-samples");

    let outcome = workspace.xtask(&["docs-samples", "extract"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
    assert_eq!(workspace.read_text(&sample(file)), expected);
}

#[test]
fn mcp_pair_is_one_json_document() {
    let workspace = Workspace::from_fixture("docs-samples");

    let outcome = workspace.xtask(&["docs-samples", "extract"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
    let pair: Value =
        serde_json::from_str(&workspace.read_text(&sample("mcp/collie/docs/guide/line_35.json")))
            .unwrap();
    assert_eq!(
        pair,
        json!({
            "request": {"method": "tools/call", "params": {"name": "fetch", "arguments": {"url": "__GOODFETCH_DOCS_ORIGIN__/catalog"}}},
            "answer": {"result": {"content": []}},
        })
    );
}

#[test]
fn no_extracted_file_holds_the_online_origin() {
    let workspace = Workspace::from_fixture("docs-samples");

    let outcome = workspace.xtask(&["docs-samples", "extract"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
    let mut holding = Vec::new();
    let mut folders = vec![workspace.path().join(OUTPUT)];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if std::fs::read_to_string(&path)
                .unwrap()
                .contains("proving-ground.goodfetch.dev")
            {
                holding.push(path);
            }
        }
    }
    assert_eq!(holding, Vec::<std::path::PathBuf>::new());
}

/// The macro's hidden line follows the crate's attributes, which a doctest
/// takes only first.
#[test]
fn rust_origin_macro_follows_the_crate_attributes() {
    let (workspace, outcome) = extracted_with("rust-attributes.md", "docs/rust-attributes.md");

    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
    assert_eq!(
        workspace.read_text(&sample("rust/collie/docs/rust-attributes.md")),
        "```rust\n#![deny(warnings)]\n# macro_rules! goodfetch_docs_origin { ($path:literal) => { concat!(env!(\"GOODFETCH_DOCS_ORIGIN\"), $path) }; }\nlet catalog = goodfetch_docs_origin!(\"/catalog\");\n```\n"
    );
}

/// A `no_run` block is checked statically where it would run; a `console`
/// block or an MCP pair, which nothing checks statically, is only listed,
/// the pair whichever of its blocks is marked.
#[test]
fn no_run_block_does_not_run() {
    let (workspace, outcome) = extracted_with("no-run.md", "docs/no-run.md");

    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
    assert_eq!(
        entries(
            &workspace,
            "docs/no-run.md",
            "collie",
            &["line", "check", "file"]
        ),
        vec![
            json!({"line": 3, "check": "static", "file": "rust/collie/docs/no-run.md"}),
            json!({"line": 7, "check": "listed", "file": null}),
            json!({"line": 11, "check": "listed", "file": null}),
        ]
    );
    assert_eq!(
        workspace.read_text(&sample("rust/collie/docs/no-run.md")),
        "```rust no_run\nlet answer = 42;\n```\n"
    );
}

#[test]
fn page_foxhound_alone_carries_runs_on_foxhound() {
    let (workspace, outcome) = extracted_with("foxhound-only.md", "docs/foxhound-only.md");

    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
    assert_eq!(
        entries(
            &workspace,
            "docs/foxhound-only.md",
            "foxhound",
            &["line", "product", "check", "file"]
        ),
        vec![
            json!({"line": 5, "product": "foxhound", "check": "run", "file": "python/foxhound/docs/foxhound-only/line_5.py"}),
        ]
    );
    assert_eq!(
        entries(&workspace, "docs/foxhound-only.md", "collie", &["line"]),
        Vec::<Value>::new()
    );
    assert_eq!(
        workspace.read_text(&sample("python/foxhound/docs/foxhound-only/line_5.py")),
        "from goodfetch_foxhound import Foxhound\n"
    );
}

#[rstest]
#[case::python_package(
    "sdks/python/collie-client/README.md",
    json!({"rendering": "collie", "product": "collie", "check": "run", "file": "python/collie/sdks/python/collie-client/README/line_3.py"})
)]
#[case::npm_package(
    "sdks/typescript/foxhound/README.md",
    json!({"rendering": "foxhound", "product": "foxhound", "check": "run", "file": "typescript/foxhound/sdks/typescript/foxhound/README/line_3.ts"})
)]
fn readme_runs_on_its_package_product(#[case] readme: &str, #[case] expected: Value) {
    let workspace = Workspace::from_fixture("docs-samples");

    let outcome = workspace.xtask(&["docs-samples", "extract"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
    let rendering = expected["rendering"].as_str().unwrap();
    assert_eq!(
        entries(
            &workspace,
            readme,
            rendering,
            &["rendering", "product", "check", "file"]
        ),
        vec![expected]
    );
}

/// The README of a folder with no package manifest, or of a package whose
/// path names no Product, is not read.
#[rstest]
#[case::no_package_manifest("sdks/typescript/collie/README.md", None)]
#[case::no_product_named(
    "sdks/typescript/shared/README.md",
    Some("sdks/typescript/shared/package.json")
)]
fn readme_not_published_with_a_product_is_not_read(
    #[case] readme: &str,
    #[case] manifest: Option<&str>,
) {
    let workspace = Workspace::from_fixture("docs-samples");
    workspace.add_fixture_file("docs-samples/sdks/typescript/foxhound/README.md", readme);
    if let Some(manifest) = manifest {
        workspace.write(manifest, b"{ \"name\": \"@goodfetch/shared\" }\n");
    }

    let outcome = workspace.xtask(&["docs-samples", "extract"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stdout);
    for rendering in ["collie", "foxhound"] {
        assert_eq!(
            entries(&workspace, readme, rendering, &["line"]),
            Vec::<Value>::new()
        );
    }
}

/// The case page fails the names check wherever it is read.
#[rstest]
#[case::decisions("docs/adr/0001-install.md")]
#[case::agents_configuration("docs/agents/install.md")]
fn unpublished_docs_folder_is_not_read(#[case] page: &str) {
    let (_workspace, outcome) = extracted_with("foxhound-package.md", page);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::collie_executable_in_foxhound_rendering(
    "collie-executable.md",
    "foxhound:docs/collie-executable.md:4"
)]
#[case::foxhound_package_in_collie_rendering(
    "foxhound-package.md",
    "collie:docs/foxhound-package.md:4"
)]
fn code_block_carrying_the_other_product_name_fails(#[case] case: &str, #[case] subject: &str) {
    let (workspace, outcome) = extracted_with(case, &format!("docs/{case}"));

    assert_eq!(outcome.findings(), vec![finding("docs-names", subject)]);
    assert_eq!(outcome.code, 1);
    assert_eq!(workspace.read(&format!("{OUTPUT}/manifest.json")), None);
}

/// `goodfetch_collie_client` is not Collie's `goodfetch_collie`.
#[test]
fn longer_name_holding_a_product_name_passes() {
    let (_workspace, outcome) = extracted_with("longer-name.md", "docs/longer-name.md");

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// A part marked as naming Collie keeps Collie's names in both renderings,
/// and is checked statically on Collie in each.
#[test]
fn part_naming_a_product_is_checked_statically_on_it() {
    let (workspace, outcome) = extracted_with("names-part.mdx", "docs/names-part.mdx");

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    for rendering in ["collie", "foxhound"] {
        assert_eq!(
            entries(
                &workspace,
                "docs/names-part.mdx",
                rendering,
                &["line", "product", "check"]
            ),
            vec![json!({"line": 5, "product": "collie", "check": "static"})]
        );
        assert_eq!(
            workspace.read_text(&sample(&format!(
                "typescript/{rendering}/docs/names-part/line_5.ts"
            ))),
            "import { Collie } from \"@goodfetch/collie\";\n"
        );
    }
}

/// A page the rendering cannot read fails once, at the line a writer
/// fixes, and leaves no sample behind, not even an earlier run's.
#[rstest]
#[case::unclosed_part("unclosed-part.md", "docs/unclosed-part.md:3")]
#[case::unknown_product("unknown-product.md", "docs/unknown-product.md:3")]
#[case::unclosed_front_matter("unclosed-front-matter.md", "docs/unclosed-front-matter.md:1")]
#[case::request_without_answer("request-without-answer.md", "docs/request-without-answer.md:3")]
fn page_the_rendering_cannot_read_fails(#[case] case: &str, #[case] subject: &str) {
    let workspace = Workspace::from_fixture("docs-samples");
    workspace.add_fixture_file(
        &format!("docs-samples-cases/{case}"),
        &format!("docs/{case}"),
    );
    workspace.write(&format!("{OUTPUT}/manifest.json"), b"{\"samples\": []}\n");

    let outcome = workspace.xtask(&["docs-samples", "extract"]);

    let expected: Vec<Finding> = vec![finding("docs-samples", subject)];
    assert_eq!(outcome.findings(), expected);
    assert_eq!(outcome.code, 1);
    assert_eq!(workspace.read(&format!("{OUTPUT}/manifest.json")), None);
}
