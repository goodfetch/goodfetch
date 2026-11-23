//! The API breakage check of the TypeScript and Python SDK packages (ADRs
//! 0030, 0069 and 0105): API Extractor on each TypeScript package's entry
//! points and Griffe on each Python package's modules, testing entry points
//! and modules included, against the commit `compat/api.toml` names as the
//! previous release. A break fails unless `compat/api.toml` announces it for
//! the next major as `typescript <entry> <name>` or `python <module>
//! <name>`; an addition passes, and so does any change to an Experimental
//! member. The tests run the repository's installed TypeScript, API
//! Extractor and Griffe on a workspace holding Collie's packages alone,
//! which the check reads as the SDKs whose other packages have no API yet.
//! Covers: story:1-30

use crate::workspace::{Workspace, finding};

const API: &str = "compat/api.toml";

const INDEX: &str = "sdks/typescript/collie/src/index.ts";
const TESTING: &str = "sdks/typescript/collie/src/testing.ts";
const PY_INIT: &str = "sdks/python/collie/src/goodfetch_collie/__init__.py";
const PY_TESTING: &str = "sdks/python/collie/src/goodfetch_collie/testing.py";
/// A generated file of each package, which the packages re-export.
const TS_SETTINGS: &str = "sdks/typescript/collie/src/generated/settings.ts";
const PY_SETTINGS: &str = "sdks/python/collie/src/goodfetch_collie/generated/settings.py";
/// A generated module whose Experimental objects carry only the docstring
/// line ADR 0099 gives Python, as the Plugins' handlers do.
const PY_PLUGINS: &str = "sdks/python/collie/src/goodfetch_collie/generated/plugins.py";

/// Collie's TypeScript and Python packages, each with a testing entry,
/// committed as the previous release `compat/api.toml` names, with
/// `announced` as its announced breaks.
fn released(announced: &str) -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.link_installed_runners();
    workspace.write(".gitignore", b"/node_modules\n/.venv\n");
    workspace.write(
        "sdks/typescript/collie/package.json",
        br#"{
  "name": "@goodfetch/collie",
  "type": "module",
  "exports": {
    ".": "./src/index.ts",
    "./generated": "./src/generated/index.ts",
    "./testing": "./src/testing.ts"
  }
}
"#,
    );
    workspace.write(
        TS_SETTINGS,
        b"/** A page's settings. */\nexport interface PageSettings {\n  /** How long a wait may take. */\n  readonly timeout?: number;\n}\n",
    );
    workspace.write(
        "sdks/typescript/collie/src/generated/index.ts",
        b"export type { PageSettings } from \"./settings.ts\";\n",
    );
    workspace.write(
        "sdks/python/collie/src/goodfetch_collie/generated/__init__.py",
        b"\"\"\"The layer `xtask codegen` writes.\"\"\"\n",
    );
    workspace.write(
        PY_SETTINGS,
        b"\"\"\"The settings.\"\"\"\n\nfrom __future__ import annotations\n\n\nclass PageSettings:\n    \"\"\"A page's settings.\"\"\"\n\n    def with_timeout(self, timeout: float) -> PageSettings:\n        \"\"\"These settings with `timeout`.\"\"\"\n        return self\n",
    );
    workspace.write(
        PY_PLUGINS,
        b"\"\"\"The Plugins' handlers.\"\"\"\n\n\nclass SampleStoreHandler:\n    \"\"\"Keeps values by key.\n\n    Experimental: outside semver until it is promoted (ADR 0030).\n    \"\"\"\n\n    def put(self, key: str) -> None:\n        \"\"\"Keeps a value under `key`.\"\"\"\n\n\ndef sample_store() -> SampleStoreHandler:\n    \"\"\"The sample store.\n\n    Experimental: outside semver until it is promoted (ADR 0030).\n    \"\"\"\n    return SampleStoreHandler()\n",
    );
    workspace.write(
        "sdks/typescript/collie/tsconfig.json",
        br#"{
  "compilerOptions": {
    "target": "es2024",
    "module": "nodenext",
    "moduleResolution": "nodenext",
    "strict": true,
    "declaration": true,
    "isolatedDeclarations": true,
    "allowImportingTsExtensions": true,
    "noEmit": true,
    "types": []
  },
  "include": ["src"]
}
"#,
    );
    workspace.write(
        INDEX,
        b"/** How a page opens. */\nexport interface OpenOptions {\n  /** The page's address. */\n  readonly url: string;\n  /** How long it may take. */\n  readonly timeout?: number;\n}\n\n/** Opens a page. */\nexport function open(options: OpenOptions): string {\n  return options.url;\n}\n\n/**\n * Previews a page.\n *\n * @experimental\n */\nexport function preview(url: string): string {\n  return url;\n}\n\nexport type { PageSettings } from \"./generated/settings.ts\";\n",
    );
    workspace.write(
        TESTING,
        b"/** Starts a test engine. */\nexport function startTestEngine(): number {\n  return 0;\n}\n\n/** Connects over a transport the test holds. */\nexport function connectOver(transport: string): string {\n  return transport;\n}\n",
    );
    workspace.write(
        PY_INIT,
        b"\"\"\"Collie.\"\"\"\n\nfrom goodfetch_collie.generated.settings import PageSettings\n\n\ndef open_page(url: str) -> str:\n    \"\"\"Opens a page.\"\"\"\n    return url\n\n\ndef preview(url: str) -> str:\n    \"\"\"Previews a page.\"\"\"\n    warn_experimental(\"preview\")\n    return url\n\n\ndef warn_experimental(name: str) -> None:\n    \"\"\"Warns once that `name` is Experimental.\"\"\"\n",
    );
    workspace.write(
        PY_TESTING,
        b"\"\"\"Collie's testing kit.\"\"\"\n\n\ndef start_test_engine() -> int:\n    \"\"\"Starts a test engine.\"\"\"\n    return 0\n\n\ndef connect_over(transport: str) -> str:\n    \"\"\"Connects over a transport the test holds.\"\"\"\n    return transport\n",
    );
    let previous = workspace.commit_all();
    workspace.write(
        API,
        format!("previous = \"{previous}\"\n{announced}").as_bytes(),
    );
    workspace
}

#[test]
fn an_export_removed_from_a_typescript_testing_entry_is_an_unannounced_break() {
    let workspace = released("");
    workspace.edit(
        TESTING,
        "/** Connects over a transport the test holds. */\nexport function connectOver(transport: string): string {\n  return transport;\n}\n",
        "",
    );

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unannounced-break",
            "typescript @goodfetch/collie/testing connectOver"
        )],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_property_a_typescript_interface_now_requires_is_a_break() {
    let workspace = released("");
    workspace.edit(
        INDEX,
        "  readonly timeout?: number;\n",
        "  readonly timeout?: number;\n  /** The page's referrer. */\n  readonly referrer: string;\n",
    );

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unannounced-break",
            "typescript @goodfetch/collie OpenOptions"
        )],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_changed_typescript_signature_is_a_break() {
    let workspace = released("");
    workspace.edit(
        TESTING,
        "export function startTestEngine(): number {",
        "export function startTestEngine(): string {",
    );
    workspace.edit(TESTING, "  return 0;\n", "  return \"\";\n");

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unannounced-break",
            "typescript @goodfetch/collie/testing startTestEngine"
        )],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn typescript_additions_pass_as_notes() {
    let workspace = released("");
    // A new export, a new optional parameter and a new optional property.
    workspace.append(
        TESTING,
        "\n/** Stops a test engine. */\nexport function stopTestEngine(): void {}\n",
    );
    workspace.edit(
        TESTING,
        "export function connectOver(transport: string): string {",
        "export function connectOver(transport: string, label?: string): string {",
    );
    workspace.edit(
        INDEX,
        "  readonly timeout?: number;\n",
        "  readonly timeout?: number;\n  /** The page's referrer. */\n  readonly referrer?: string;\n",
    );

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    for subject in [
        "typescript @goodfetch/collie/testing stopTestEngine",
        "typescript @goodfetch/collie/testing connectOver",
        "typescript @goodfetch/collie OpenOptions",
    ] {
        assert!(
            outcome.notes().contains(&finding("addition", subject)),
            "{subject}\n{}",
            outcome.stdout
        );
    }
    assert_eq!(outcome.code, 0);
}

#[test]
fn an_experimental_member_changes_and_goes_freely() {
    let workspace = released("");
    workspace.edit(
        INDEX,
        "export function preview(url: string): string {",
        "export function preview(url: URL): string {",
    );
    workspace.edit(INDEX, "  return url;\n}\n", "  return url.href;\n}\n");
    workspace.edit(
        PY_INIT,
        "def preview(url: str) -> str:\n    \"\"\"Previews a page.\"\"\"\n    warn_experimental(\"preview\")\n    return url\n",
        "",
    );

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn a_python_object_whose_docstring_marks_it_experimental_changes_freely() {
    // The docstring line marks a class and what it holds; the generated
    // Plugins' handlers call no `warn_experimental(`.
    let workspace = released("");
    workspace.edit(
        PY_PLUGINS,
        "def put(self, key: str) -> None:",
        "def put(self, key: str, value: bytes) -> None:",
    );
    workspace.edit(
        PY_PLUGINS,
        "\n\ndef sample_store() -> SampleStoreHandler:\n    \"\"\"The sample store.\n\n    Experimental: outside semver until it is promoted (ADR 0030).\n    \"\"\"\n    return SampleStoreHandler()\n",
        "",
    );

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn a_rename_made_in_the_generated_layer_is_an_unannounced_break() {
    // The generated layer's declarations are public API a program imports,
    // through the packages' re-exports and their `generated` entries.
    let workspace = released("");
    workspace.edit(
        TS_SETTINGS,
        "export interface PageSettings {",
        "export interface PageOptions {",
    );
    workspace.edit(
        "sdks/typescript/collie/src/generated/index.ts",
        "export type { PageSettings }",
        "export type { PageOptions }",
    );
    workspace.edit(
        INDEX,
        "export type { PageSettings }",
        "export type { PageOptions }",
    );
    workspace.edit(PY_SETTINGS, "def with_timeout(", "def timed(");

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![
            finding(
                "unannounced-break",
                "python goodfetch_collie.generated.settings PageSettings"
            ),
            finding(
                "unannounced-break",
                "typescript @goodfetch/collie PageSettings"
            ),
            finding(
                "unannounced-break",
                "typescript @goodfetch/collie/generated PageSettings"
            ),
        ],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_function_removed_from_a_python_testing_module_is_an_unannounced_break() {
    let workspace = released("");
    workspace.edit(
        PY_TESTING,
        "\n\ndef connect_over(transport: str) -> str:\n    \"\"\"Connects over a transport the test holds.\"\"\"\n    return transport\n",
        "",
    );

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unannounced-break",
            "python goodfetch_collie.testing connect_over"
        )],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_new_python_function_and_a_new_optional_parameter_pass() {
    let workspace = released("");
    workspace.append(
        PY_TESTING,
        "\n\ndef stop_test_engine() -> None:\n    \"\"\"Stops a test engine.\"\"\"\n",
    );
    workspace.edit(
        PY_TESTING,
        "def connect_over(transport: str) -> str:",
        "def connect_over(transport: str, label: str | None = None) -> str:",
    );

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn an_announced_break_passes_as_a_note() {
    let workspace = released(
        "\n[[announced]]\nentry = \"python goodfetch_collie open_page\"\nreason = \"`open_page` becomes `open`.\"\n",
    );
    workspace.edit(PY_INIT, "def open_page(url: str)", "def open(url: str)");

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome.notes().contains(&finding(
            "announced-break",
            "python goodfetch_collie open_page"
        )),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 0);
}

#[test]
fn an_announcement_of_an_sdk_break_that_no_longer_breaks_fails() {
    let workspace = released(
        "\n[[announced]]\nentry = \"typescript @goodfetch/collie open\"\nreason = \"`open` goes.\"\n\n[[announced]]\nentry = \"rust goodfetch-collie function_missing\"\nreason = \"Left to semver-checks.\"\n",
    );

    let outcome = workspace.xtask(&["sdk-api"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "announced-without-break",
            "typescript @goodfetch/collie open"
        )],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert_eq!(outcome.code, 1);
}
