//! `xtask sdk-exports`: an SDK's manifest lists every public hand-written
//! member its packages export, once however many entries export it, and no
//! member they do not export or the generator writes (ADRs 0069, 0099). The
//! fixture's TypeScript package is read by the repository's TypeScript, its
//! Rust crate and its Python packages from their source.
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
#[case::member_no_entry_exports("Collie", "begin", "[\"begin\"]", "opener")]
#[case::member_the_generator_writes("(module)", "Page", "[\"page\"]", "type")]
fn check_refuses_a_line_naming_no_hand_written_export(
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
        vec![finding(
            "unused-line",
            &format!("{MANIFEST} {receiver}.{member}")
        )]
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
/// crate's manifest lists, and nothing only a test build compiles.
#[rstest]
#[case::member_the_crate_lacks("Foxhound", "abort", "[\"abort\"]", "closer")]
#[case::hidden_member("Foxhound", "hidden", "[\"hidden\"]", "read")]
#[case::member_the_generator_writes("Foxhound", "start", "[\"start\"]", "opener")]
#[case::generated_type("(crate)", "Handle", "[\"handle\"]", "type")]
#[case::generated_type_behind_a_path("blocking", "Waited", "[\"waited\"]", "type")]
#[case::other_crates_type("(crate)", "Run", "[\"run\"]", "type")]
#[case::item_of_a_test_module("tests", "helper", "[\"helper\"]", "read")]
#[case::item_of_a_module_only_some_tests_build("slow_tests", "helper", "[\"helper\"]", "read")]
#[case::member_of_a_test_impl("Foxhound", "fake", "[\"fake\"]", "opener")]
fn check_refuses_a_rust_line_naming_no_public_hand_written_item(
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
            "unused-line",
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
#[case::member_no_entry_exports("goodfetch_collie", "Collie", "begin", "[\"begin\"]", "opener")]
#[case::name_left_out_of_all("goodfetch_collie", "(module)", "helper", "[\"helper\"]", "read")]
#[case::name_of_a_module_with_no_all(
    "goodfetch_collie.collie",
    "(module)",
    "open_over",
    "[\"open\", \"over\"]",
    "opener"
)]
#[case::private_member("goodfetch_collie", "Collie", "_flush", "[\"flush\"]", "closer")]
#[case::field_of_a_shape_of_data(
    "goodfetch_collie",
    "StartOptions",
    "timeout",
    "[\"timeout\"]",
    "read"
)]
#[case::generated_type("goodfetch_collie", "(module)", "Page", "[\"page\"]", "type")]
#[case::member_the_generator_writes("goodfetch_collie", "Collie", "start", "[\"start\"]", "opener")]
fn check_refuses_a_python_line_naming_no_public_hand_written_member(
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
            "unused-line",
            &format!("{PYTHON_MANIFEST} {receiver}.{member}")
        )],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}
