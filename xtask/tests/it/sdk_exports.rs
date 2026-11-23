//! `xtask sdk-exports`: an SDK's manifest lists every public hand-written
//! member its packages export, once however many entries export it, and no
//! member they do not export or the generator writes (ADRs 0069, 0099). A
//! listed member carries its entry's doc text and markers, and every entry
//! of the catalog reaches a public member in each SDK or is listed as
//! unreached there with its reason. The fixture's TypeScript package is read
//! by the repository's TypeScript, its Rust crate and its Python packages
//! from their source.
//! Covers: story:11-186

use rstest::rstest;

use crate::workspace::{Workspace, finding};

const MANIFEST: &str = "sdks/typescript/manifest.toml";
const COLLIE: &str = "sdks/typescript/collie/src/collie.ts";
const RUST_MANIFEST: &str = "crates/foxhound/goodfetch-foxhound/manifest.toml";
const RUST_SOURCE: &str = "crates/foxhound/goodfetch-foxhound/src";
const PYTHON_MANIFEST: &str = "sdks/python/manifest.toml";
const PYTHON_ENTRY: &str = "sdks/python/collie/src/goodfetch_collie/__init__.py";
const PYTHON_COLLIE: &str = "sdks/python/collie/src/goodfetch_collie/collie.py";
const PYTHON_HANDLE: &str = "sdks/python/collie-client/src/goodfetch_collie_client/handle.py";
const RUST_FOXHOUND: &str = "crates/foxhound/goodfetch-foxhound/src/foxhound.rs";
const CATALOG: &str = "crates/goodfetch-protocol/generated/catalog.json";
/// The description of `run.start`, the entry a member of each SDK serves.
const DESCRIPTION: &str = "Starts a Run, and answers once it is started.";
/// The Experimental line of a Rust or Python member's doc.
const EXPERIMENTAL: &str = "Experimental: outside semver until it is promoted (ADR 0030).";
/// `run.start`'s deprecation in the catalog of the test.
const DEPRECATION: &str = "{ \"since\": \"0.1.0\", \"replacement\": null, \"earliest_removal\": \"1.0.0\", \"note\": \"Gone.\" }";
/// The line of `crawl.start`, which no SDK of the test reaches.
const UNREACHED: &str = "[[unreached]]\nentries = [\"crawl.start\"]\nreason = \"The test's SDK has no Foxhound package.\"\n";
/// The finding on a line naming no member its SDK exports.
const UNUSED: &str = "unused-line";
/// The finding on a line naming a member the generator writes.
const GENERATED: &str = "generated";

fn workspace() -> Workspace {
    let workspace = Workspace::from_fixture("sdk-exports");
    workspace.link_installed_runners();
    workspace
}

#[test]
fn check_passes_when_the_manifest_lists_each_hand_written_export() {
    let workspace = workspace();

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// `Subscription`'s `[Symbol.dispose]` is `Disposable`'s, which no line has
/// to list and one may.
#[test]
fn check_passes_a_line_naming_a_member_the_language_library_declares() {
    let workspace = workspace();
    workspace.append(
        MANIFEST,
        "\n[[member]]\npackage = \"@goodfetch/collie\"\nreceiver = \"Subscription\"\nmember = \"[Symbol.dispose]\"\nentry = \"none\"\nreason = \"The scoped form of `close()`.\"\nwords = [\"close\"]\nform = \"closer\"\n",
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::function_of_an_entry(
    "sdks/typescript/collie/src/index.ts",
    "export { Collie }",
    "export function openOver(): void {}\nexport { Collie }",
    "(module)",
    "openOver"
)]
#[case::method_of_an_exported_class(
    COLLIE,
    "  close(): void {",
    "  abort(): void {}\n\n  close(): void {",
    "Collie",
    "abort"
)]
#[case::public_constructor(
    COLLIE,
    "private constructor() {}",
    "constructor() {}",
    "Collie",
    "constructor"
)]
fn check_refuses_a_hand_written_export_no_line_lists(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] receiver: &str,
    #[case] member: &str,
) {
    let workspace = workspace();
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unlisted",
            &format!("{MANIFEST} {receiver}.{member}")
        )],
        "{}",
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains(&format!(
            "package = \"@goodfetch/collie\", receiver = \"{receiver}\", member = \"{member}\""
        )),
        "the finding names the line to add: {}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::member_no_entry_exports(UNUSED, "Collie", "begin", "[\"begin\"]", "opener")]
#[case::member_the_generator_writes(GENERATED, "(module)", "Page", "[\"page\"]", "type")]
fn check_refuses_a_line_naming_no_hand_written_export(
    #[case] kind: &str,
    #[case] receiver: &str,
    #[case] member: &str,
    #[case] words: &str,
    #[case] form: &str,
) {
    let workspace = workspace();
    workspace.append(
        MANIFEST,
        &format!(
            "\n[[member]]\npackage = \"@goodfetch/collie\"\nreceiver = \"{receiver}\"\nmember = \"{member}\"\nentry = \"none\"\nreason = \"A member of the test.\"\nwords = {words}\nform = \"{form}\"\n"
        ),
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(kind, &format!("{MANIFEST} {receiver}.{member}"))]
    );
    assert_eq!(outcome.code, 1);
}

/// A public item of the crate: one it declares at its root, in a public
/// module, or in a private one its root re-exports, and each public method
/// of an inherent `impl` of such a type, wherever the `impl` sits.
#[rstest]
#[case::function_at_the_root(
    "lib.rs",
    "pub use foxhound::{Foxhound, PAID_FETCHES};",
    "pub use foxhound::{Foxhound, PAID_FETCHES};\n\n/// Seeds.\npub fn seed() {}",
    "(crate)",
    "seed"
)]
#[case::item_of_a_public_module(
    "blocking.rs",
    "/// The connection without async.",
    "/// Spawns.\npub fn spawn() {}\n\n/// The connection without async.",
    "blocking",
    "spawn"
)]
#[case::method_of_a_re_exported_type(
    "foxhound.rs",
    "    /// Closes it.",
    "    /// Aborts.\n    pub fn abort(&self) {}\n\n    /// Closes it.",
    "Foxhound",
    "abort"
)]
#[case::method_of_a_type_in_a_public_module(
    "blocking.rs",
    "    /// Opens a connection.",
    "    /// Closes it.\n    pub fn close(&self) {}\n\n    /// Opens a connection.",
    "blocking::Foxhound",
    "close"
)]
#[case::function_of_a_public_trait(
    "blocking.rs",
    "    fn wait(&self);",
    "    fn wait(&self);\n\n    /// Blocks for a while.\n    fn wait_for(&self);",
    "blocking::Wait",
    "wait_for"
)]
#[case::method_of_an_impl_that_names_its_type_through_a_glob(
    "foxhound.rs",
    "impl Clone for Foxhound {",
    "mod split {\n    use super::*;\n\n    impl Foxhound {\n        /// Reconnects.\n        pub fn reconnect(&self) {}\n    }\n}\n\nimpl Clone for Foxhound {",
    "Foxhound",
    "reconnect"
)]
#[case::item_re_exported_through_an_imported_module(
    "blocking.rs",
    "pub use goodfetch_collie_client::blocking::Run;",
    "pub use goodfetch_collie_client::blocking::Run;\nuse crate::hidden;\npub use hidden::LIMIT;",
    "blocking",
    "LIMIT"
)]
#[case::item_re_exported_through_super_twice(
    "blocking.rs",
    "/// What blocks until it is done.",
    "/// Deeper.\npub mod deeper {\n    /// Deepest.\n    pub mod deepest {\n        pub use super::super::super::hidden::LIMIT;\n    }\n}\n\n/// What blocks until it is done.",
    "blocking::deeper::deepest",
    "LIMIT"
)]
#[case::item_of_a_re_exported_module(
    "lib.rs",
    "mod hidden;",
    "mod hidden;\nmod inner {\n    pub fn spawn() {}\n}\npub use inner as api;",
    "api",
    "spawn"
)]
#[case::item_a_glob_chain_re_exports(
    "lib.rs",
    "mod hidden;",
    "mod hidden;\nmod a {\n    pub struct Globbed;\n}\nmod b {\n    pub use super::a::*;\n}\npub use b::Globbed;",
    "(crate)",
    "Globbed"
)]
#[case::re_export_a_glob_re_exports(
    "lib.rs",
    "mod hidden;",
    "mod hidden;\nmod deep {\n    pub struct Deep;\n}\nmod b {\n    pub use super::deep::Deep;\n}\npub use b::*;",
    "(crate)",
    "Deep"
)]
#[case::item_of_a_path_module_in_an_inline_module(
    "blocking.rs",
    "/// What blocks until it is done.",
    "/// The outer module.\npub mod outer {\n    #[path = \"far.rs\"]\n    pub mod far;\n}\n\n/// What blocks until it is done.",
    "blocking::outer::far",
    "reach"
)]
fn check_refuses_a_public_rust_item_no_line_lists(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] receiver: &str,
    #[case] member: &str,
) {
    let workspace = workspace();
    workspace.edit(&format!("{RUST_SOURCE}/{file}"), from, to);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unlisted",
            &format!("{RUST_MANIFEST} {receiver}.{member}")
        )],
        "{}",
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains(&format!(
            "crate = \"goodfetch-foxhound\", receiver = \"{receiver}\", member = \"{member}\""
        )),
        "the finding names the line to add: {}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// A crate's public item is only what a dependent crate reaches by name:
/// not a `#[doc(hidden)]` method, not a member the generator writes, even on
/// a hand-written type, not another crate's type it re-exports, which that
/// crate's manifest lists, not a struct's public field, which is plain data
/// (ADR 0099), and nothing only a test build compiles.
#[rstest]
#[case::member_the_crate_lacks(UNUSED, "Foxhound", "abort", "[\"abort\"]", "closer")]
#[case::field_of_a_struct(UNUSED, "Foxhound", "pid", "[\"pid\"]", "read")]
#[case::hidden_member(UNUSED, "Foxhound", "hidden", "[\"hidden\"]", "read")]
#[case::member_the_generator_writes(GENERATED, "Foxhound", "start", "[\"start\"]", "opener")]
#[case::generated_type(GENERATED, "(crate)", "Handle", "[\"handle\"]", "type")]
#[case::generated_type_behind_a_path(GENERATED, "blocking", "Waited", "[\"waited\"]", "type")]
#[case::other_crates_type(UNUSED, "(crate)", "Run", "[\"run\"]", "type")]
#[case::item_of_a_test_module(UNUSED, "tests", "helper", "[\"helper\"]", "read")]
#[case::item_of_a_module_only_some_tests_build(
    UNUSED,
    "slow_tests",
    "helper",
    "[\"helper\"]",
    "read"
)]
#[case::member_of_a_test_impl(UNUSED, "Foxhound", "fake", "[\"fake\"]", "opener")]
fn check_refuses_a_rust_line_naming_no_public_hand_written_item(
    #[case] kind: &str,
    #[case] receiver: &str,
    #[case] member: &str,
    #[case] words: &str,
    #[case] form: &str,
) {
    let workspace = workspace();
    workspace.append(
        RUST_MANIFEST,
        &format!(
            "\n[[member]]\ncrate = \"goodfetch-foxhound\"\nreceiver = \"{receiver}\"\nmember = \"{member}\"\nentry = \"none\"\nreason = \"A member of the test.\"\nwords = {words}\nform = \"{form}\"\n"
        ),
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            kind,
            &format!("{RUST_MANIFEST} {receiver}.{member}")
        )],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// A dunder is the language's own protocol, as `__enter__` is, so a line
/// may name it and none has to.
#[test]
fn check_passes_a_line_naming_a_python_dunder() {
    let workspace = workspace();
    workspace.append(
        PYTHON_MANIFEST,
        "\n[[member]]\npackage = \"goodfetch_collie\"\nreceiver = \"Collie\"\nmember = \"__enter__\"\nentry = \"none\"\nreason = \"The scoped form.\"\nwords = [\"close\"]\nform = \"closer\"\n",
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// A public member of a Python entry: a name of a module's `__all__`, and
/// each public member of a class it names, its own or one a base the SDK
/// declares gives it, method or field.
#[rstest]
#[case::name_in_all(
    PYTHON_ENTRY,
    "__all__ = [\"Collie\",",
    "__all__ = [\"helper\", \"Collie\",",
    "(module)",
    "helper"
)]
#[case::method_of_an_exported_class(
    PYTHON_COLLIE,
    "    def close(self) -> None:",
    "    async def abort(self) -> None:\n        \"\"\"Aborts.\"\"\"\n\n    def close(self) -> None:",
    "Collie",
    "abort"
)]
#[case::method_of_a_base_the_sdk_declares(
    PYTHON_HANDLE,
    "    def close(self) -> None:",
    "    def flush(self) -> None:\n        \"\"\"Flushes.\"\"\"\n\n    def close(self) -> None:",
    "Subscription",
    "flush"
)]
#[case::field_of_a_class_with_methods(
    PYTHON_COLLIE,
    "    @classmethod",
    "    timeout: float = 0.0\n\n    @classmethod",
    "Collie",
    "timeout"
)]
#[case::field_its_constructor_sets(
    PYTHON_COLLIE,
    "    @classmethod",
    "    def __init__(self) -> None:\n        self.engine = None\n\n    @classmethod",
    "Collie",
    "engine"
)]
fn check_refuses_a_public_python_member_no_line_lists(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] receiver: &str,
    #[case] member: &str,
) {
    let workspace = workspace();
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unlisted",
            &format!("{PYTHON_MANIFEST} {receiver}.{member}")
        )],
        "{}",
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains(&format!(
            "package = \"goodfetch_collie\", receiver = \"{receiver}\", member = \"{member}\""
        )),
        "the finding names the line to add: {}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// A Python entry exports only its `__all__`: not a name it leaves out, not
/// a module with no `__all__`, not a private member, not a field of a shape
/// of data, and nothing the generator writes, even on a hand-written class.
#[rstest]
#[case::member_no_entry_exports(
    UNUSED,
    "goodfetch_collie",
    "Collie",
    "begin",
    "[\"begin\"]",
    "opener"
)]
#[case::name_left_out_of_all(
    UNUSED,
    "goodfetch_collie",
    "(module)",
    "helper",
    "[\"helper\"]",
    "read"
)]
#[case::name_of_a_module_with_no_all(
    UNUSED,
    "goodfetch_collie.collie",
    "(module)",
    "open_over",
    "[\"open\", \"over\"]",
    "opener"
)]
#[case::private_member(
    UNUSED,
    "goodfetch_collie",
    "Collie",
    "_flush",
    "[\"flush\"]",
    "closer"
)]
#[case::field_of_a_shape_of_data(
    UNUSED,
    "goodfetch_collie",
    "StartOptions",
    "timeout",
    "[\"timeout\"]",
    "read"
)]
#[case::generated_type(
    GENERATED,
    "goodfetch_collie",
    "(module)",
    "Page",
    "[\"page\"]",
    "type"
)]
#[case::member_the_generator_writes(
    GENERATED,
    "goodfetch_collie",
    "Collie",
    "start",
    "[\"start\"]",
    "opener"
)]
fn check_refuses_a_python_line_naming_no_public_hand_written_member(
    #[case] kind: &str,
    #[case] package: &str,
    #[case] receiver: &str,
    #[case] member: &str,
    #[case] words: &str,
    #[case] form: &str,
) {
    let workspace = workspace();
    workspace.append(
        PYTHON_MANIFEST,
        &format!(
            "\n[[member]]\npackage = \"{package}\"\nreceiver = \"{receiver}\"\nmember = \"{member}\"\nentry = \"none\"\nreason = \"A member of the test.\"\nwords = {words}\nform = \"{form}\"\n"
        ),
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            kind,
            &format!("{PYTHON_MANIFEST} {receiver}.{member}")
        )],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// An `__all__` the reader cannot read whole, or naming what its module
/// neither defines nor imports, fails the run rather than dropping a name.
#[rstest]
#[case::name_nothing_declares("[\"Collie\", \"Colie\"]", "Colie")]
#[case::name_built_at_run_time("[\"Collie\", *EXTRA]", "__all__")]
#[case::all_extended("[\"Collie\"]\n__all__ += [\"Page\"]", "__all__")]
fn check_fails_on_a_python_all_it_cannot_read(#[case] all: &str, #[case] named: &str) {
    let workspace = workspace();
    workspace.edit(
        PYTHON_ENTRY,
        "__all__ = [\"Collie\", \"Page\", \"StartOptions\", \"Subscription\"]",
        &format!("__all__ = {all}"),
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.stdout, "");
    assert!(
        outcome.stderr.contains(&format!("`{named}`")),
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 2);
}

/// A member serving an entry carries the entry's description as its doc
/// text, however its lines wrap it, as the generator writes a generated
/// member's (ADR 0099).
#[rstest]
#[case::typescript(COLLIE, "\n   * ")]
#[case::rust(RUST_FOXHOUND, "\n    /// ")]
#[case::python(PYTHON_COLLIE, "\n        ")]
fn check_passes_a_doc_text_wrapped_on_other_lines(#[case] file: &str, #[case] wrap: &str) {
    let workspace = workspace();
    workspace.edit(
        file,
        DESCRIPTION,
        &DESCRIPTION.replace("answers ", &format!("answers{wrap}")),
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::typescript(COLLIE, MANIFEST, "Collie.start")]
#[case::rust(RUST_FOXHOUND, RUST_MANIFEST, "Foxhound.launch")]
#[case::python(PYTHON_COLLIE, PYTHON_MANIFEST, "Collie.launch")]
fn check_refuses_a_member_lacking_its_doc_text(
    #[case] file: &str,
    #[case] manifest: &str,
    #[case] member: &str,
) {
    let workspace = workspace();
    workspace.edit(file, DESCRIPTION, "Starts a Run.");

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("missing-doc", &format!("{manifest} {member}"))],
        "{}",
        outcome.stdout
    );
    assert!(outcome.stdout.contains(DESCRIPTION), "{}", outcome.stdout);
    assert_eq!(outcome.code, 1);
}

/// An Experimental entry's member carries TypeScript's tag `@experimental`,
/// or the Experimental line of a Rust or Python doc (ADR 0099).
#[rstest]
#[case::typescript(COLLIE, "   *\n   * @experimental\n".to_owned(), MANIFEST, "Collie.start")]
#[case::rust(
    RUST_FOXHOUND,
    format!("    ///\n    /// {EXPERIMENTAL}\n"),
    RUST_MANIFEST,
    "Foxhound.launch"
)]
#[case::python(
    PYTHON_COLLIE,
    format!("\n        {EXPERIMENTAL}\n"),
    PYTHON_MANIFEST,
    "Collie.launch"
)]
fn check_refuses_a_member_lacking_its_experimental_marker(
    #[case] file: &str,
    #[case] marker: String,
    #[case] manifest: &str,
    #[case] member: &str,
) {
    let workspace = workspace();
    workspace.edit(file, &marker, "");

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("missing-marker", &format!("{manifest} {member}"))],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// Deprecates `run.start` in the catalog of the test.
fn deprecate_run_start(workspace: &Workspace) {
    let from = format!(
        "{DESCRIPTION}\",\n      \"stability\": \"experimental\",\n      \"deprecated\": null"
    );
    let to = format!(
        "{DESCRIPTION}\",\n      \"stability\": \"experimental\",\n      \"deprecated\": {DEPRECATION}"
    );
    workspace.edit(CATALOG, &from, &to);
}

/// A deprecated entry's member carries the language's own marker:
/// TypeScript's tag `@deprecated`, Rust's `#[deprecated]`, PEP 702's
/// `@deprecated`.
#[test]
fn check_refuses_members_lacking_their_deprecated_marker() {
    let workspace = workspace();
    deprecate_run_start(&workspace);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![
            finding("missing-marker", &format!("{MANIFEST} Collie.start")),
            finding(
                "missing-marker",
                &format!("{RUST_MANIFEST} Foxhound.launch")
            ),
            finding(
                "missing-marker",
                &format!("{PYTHON_MANIFEST} Collie.launch")
            ),
        ],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn check_passes_members_carrying_their_deprecated_marker() {
    let workspace = workspace();
    deprecate_run_start(&workspace);
    workspace.edit(
        COLLIE,
        "   * @experimental\n",
        "   * @experimental\n   * @deprecated Gone.\n",
    );
    workspace.edit(
        RUST_FOXHOUND,
        "    pub async fn launch",
        "    #[deprecated(note = \"Gone.\")]\n    pub async fn launch",
    );
    workspace.edit(
        PYTHON_COLLIE,
        "    def launch",
        "    @deprecated(\"Gone.\")\n    def launch",
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// An entry reaches an SDK through a member the generator writes on a
/// handle, as `run.abort` does, or a line serving it; one that reaches
/// neither needs an `[[unreached]]` line with its reason in one of the
/// SDK's manifests.
#[rstest]
#[case::python(PYTHON_MANIFEST, PYTHON_MANIFEST)]
#[case::rust(RUST_MANIFEST, "crates/goodfetch-collie-client/manifest.toml")]
fn check_refuses_an_entry_reaching_no_member_of_an_sdk(
    #[case] manifest: &str,
    #[case] subject: &str,
) {
    let workspace = workspace();
    let text = workspace.read_text(manifest);
    let line = text
        .find("[[unreached]]")
        .expect("the manifest lists crawl.start as unreached");
    workspace.write(manifest, &text.as_bytes()[..line]);

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unreached", &format!("{subject} crawl.start"))],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// `_meta` is the runtime's alone (ADR 0132): no public type or method of
/// any SDK names it, listed or not. A hand-written name fails when `meta`
/// is one of its words, whatever its case or prefix; a generated one when
/// it is the type the protocol gives `_meta`.
#[rstest]
#[case::typescript_method(
    MANIFEST,
    COLLIE,
    "  close(): void {",
    "  withMeta(): void {}\n\n  close(): void {",
    "Collie",
    "withMeta"
)]
#[case::typescript_type(
    MANIFEST,
    "sdks/typescript/collie/src/index.ts",
    "export { Collie }",
    "export interface RequestMeta {\n  traceparent: string;\n}\nexport { Collie }",
    "(module)",
    "RequestMeta"
)]
#[case::rust_method(
    RUST_MANIFEST,
    RUST_FOXHOUND,
    "    /// Closes it.",
    "    /// Its request's meta.\n    pub fn set_meta(&self) {}\n\n    /// Closes it.",
    "Foxhound",
    "set_meta"
)]
#[case::python_method(
    PYTHON_MANIFEST,
    PYTHON_COLLIE,
    "    def close(self) -> None:",
    "    def meta(self) -> None:\n        \"\"\"Its request's meta.\"\"\"\n\n    def close(self) -> None:",
    "Collie",
    "meta"
)]
fn check_refuses_a_public_member_naming_meta(
    #[case] manifest: &str,
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] receiver: &str,
    #[case] member: &str,
) {
    let workspace = workspace();
    workspace.edit(file, from, to);

    let outcome = workspace.xtask(&["sdk-exports"]);

    let subject = format!("{manifest} {receiver}.{member}");
    assert_eq!(
        outcome.findings(),
        vec![
            finding("unlisted", &subject),
            finding("names-meta", &subject)
        ],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// The generated type of `_meta`, `RequestMeta`, which an SDK's runtime
/// imports and no entry exports.
#[test]
fn check_refuses_an_entry_exporting_the_generated_type_of_meta() {
    let workspace = workspace();
    workspace.append(
        "sdks/typescript/collie/src/generated/page.ts",
        "export interface RequestMeta {\n  traceparent?: string;\n}\n",
    );
    workspace.append(
        "sdks/typescript/collie/src/index.ts",
        "export type { RequestMeta } from \"./generated/page.ts\";\n",
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "names-meta",
            &format!("{MANIFEST} (module).RequestMeta")
        )],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

/// A name holding `meta` inside a word, such as `metadata`, names no
/// `_meta`, nor does a generated name other than `_meta`'s type, such as
/// that of an HTML `<meta>` tag.
#[test]
fn check_passes_a_name_holding_meta_otherwise() {
    let workspace = workspace();
    workspace.edit(
        COLLIE,
        "  close(): void {",
        "  metadata(): void {}\n\n  close(): void {",
    );
    workspace.append(
        "sdks/typescript/collie/src/generated/page.ts",
        "export interface MetaTag {\n  name: string;\n}\n",
    );
    workspace.append(
        "sdks/typescript/collie/src/index.ts",
        "export type { MetaTag } from \"./generated/page.ts\";\n",
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("unlisted", &format!("{MANIFEST} Collie.metadata"))],
        "{}",
        outcome.stdout
    );
}

/// An `[[unreached]]` line names only entries the SDK must reach and does
/// not: never one a member serves, one the generator writes a member for,
/// one only a person calls, or no entry at all.
#[rstest]
#[case::entry_a_line_serves("reached-unreached", "run.start")]
#[case::entry_a_generated_member_serves("reached-unreached", "run.abort")]
#[case::person_only_entry("person-only", "usage.raise")]
#[case::unknown_entry("unknown-entry", "run.begin")]
fn check_refuses_an_unreached_line_naming_no_unreached_entry(
    #[case] kind: &str,
    #[case] entry: &str,
) {
    let workspace = workspace();
    workspace.append(
        MANIFEST,
        &UNREACHED.replace("\"crawl.start\"", &format!("\"{entry}\"")),
    );

    let outcome = workspace.xtask(&["sdk-exports"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(kind, &format!("{MANIFEST} {entry}"))],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}
