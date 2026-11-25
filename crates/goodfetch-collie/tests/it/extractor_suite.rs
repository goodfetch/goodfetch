//! The Extractor's kind suite (`conformance/extract/cases.toml`, spec
//! #596, ticket 823): run by the harness (#171) against
//! `goodfetch-scripted-plugin` scripted as a toy Extractor, over the plugin
//! protocol, and in process against each built-in main-content step behind
//! the Extractor's driver, so both pass the same cases (ADR 0129).

use std::collections::BTreeMap;
use std::fs;
use std::pin::Pin;

use goodfetch_conformance::{PluginSuite, PluginTarget, ProvingGround, run_plugin_suite};
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::framing::Received;
use goodfetch_engine::plugin_host::instance::CallFailure;
use goodfetch_engine::plugin_kinds::extractor::{ExtractPrimitives, ExtractorDriver};
use goodfetch_engine::plugin_kinds::{Accounts, PluginCall};
use goodfetch_extractor::{BuiltIn, Chosen, Context, Document, Serialization, formats};
use goodfetch_protocol::page::{Asked, FormatsRequest, MarkdownFormat};
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::plugin::extractor::{ExtractContentParams, ExtractContentResult};
use goodfetch_protocol::result_document::{CaseCheck, ConformanceDocument, Outcome};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_PLUGIN};
use goodfetch_test_support::{ProvingGround as LocalCopy, TestEnv, Tool, real_time_runtime, tools};
use serde::Deserialize as _;
use serde_json::{Value, json};
use url::Url;

/// The sentence of the suite's article page, which its main content holds.
const SENTENCE: &str = "The kettle boils a litre of water in three minutes";

/// The deadline the built-ins' calls wait under.
const PLUGIN_MILLIS: u64 = 30_000;

/// The Extractor suite run against `goodfetch-scripted-plugin` declaring
/// the Extractor's `content` and answering `extract.content` with
/// `answer`, and the local copy of the Proving ground.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the scripted Plugin's script before the harness starts it"
)]
fn run_against_scripted(answer: &Value) -> ConformanceDocument {
    let env = TestEnv::new();
    let script = env.folder().join("script.json");
    let hello = json!({
        "protocol_version": "0.1",
        "name": "scripted",
        "version": "1.0.0",
        "kinds": [{ "kind": "extract", "capabilities": ["content"] }],
        "network_level": "none",
    });
    fs::write(
        &script,
        json!({ "hello": hello, "answers": { "extract.content": answer } }).to_string(),
    )
    .unwrap();
    let manifest = PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": "scripted",
            "version": "1.0.0",
            "kinds": ["extract"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let target = PluginTarget {
        manifest,
        executable: tools()
            .unwrap()
            .path(Tool::ScriptedPlugin)
            .unwrap()
            .to_owned(),
        arguments: vec![script.display().to_string()],
        environment: BTreeMap::new(),
        folder: env.folder().to_path_buf(),
    };
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    run_plugin_suite(&PluginSuite::extract(), &target, &proving_ground).unwrap()
}

/// Each capability's name and outcome.
fn outcomes(document: &ConformanceDocument) -> Vec<(&str, Outcome)> {
    document
        .capabilities()
        .iter()
        .map(|capability| (capability.name(), capability.outcome()))
        .collect()
}

/// A toy Extractor that answers the article page's main content in the
/// kind's shape is proved in `content`; one whose result holds a field the
/// kind does not have fails it.
///
/// Covers: story:7-87
#[test]
fn the_extractor_suite_proves_a_conforming_toy_extractor_and_fails_a_broken_one() {
    let content = json!({
        "title": "Kettle",
        "text": format!("Kettle\n\n{SENTENCE}."),
        "markdown": format!("# Kettle\n\n{SENTENCE}."),
        "metadata": { "author": "Ada" },
    });
    let mut broken = content.clone();
    broken["summary"] = json!("A kettle.");

    let conforming = run_against_scripted(&json!({ "result": content }));
    let broken = run_against_scripted(&json!({ "result": broken }));

    assert_eq!(
        (outcomes(&conforming), outcomes(&broken)),
        (
            vec![("content", Outcome::Proved)],
            vec![("content", Outcome::Failed)]
        ),
    );
}

/// The Run a built-in's calls are accounted to, which refuses none.
struct Unlimited;

impl Accounts for Unlimited {
    fn call_starts<'call>(
        &'call self,
        _call: &'call PluginCall<'call>,
    ) -> Pin<Box<dyn Future<Output = Result<(), CallError>> + Send + 'call>> {
        Box::pin(async { Ok(()) })
    }

    fn call_ended<'call>(
        &'call self,
        _call: &'call PluginCall<'call>,
    ) -> Pin<Box<dyn Future<Output = Result<(), CallError>> + Send + 'call>> {
        Box::pin(async { Ok(()) })
    }
}

/// A built-in main-content step as an implementation of the Extractor: the
/// page's main content as Markdown and as text, read as `page.formats`
/// reads them with the step, with no title and no metadata, which the
/// built-ins read into `page.formats`' own fields.
struct BuiltInExtractor(BuiltIn);

impl ExtractPrimitives for BuiltInExtractor {
    fn name(&self) -> &str {
        match self.0 {
            BuiltIn::DomSmoothie => "dom_smoothie",
            BuiltIn::Trafilatura => "trafilatura",
        }
    }

    fn content<'call>(
        &'call self,
        params: &'call ExtractContentParams,
    ) -> Pin<Box<dyn Future<Output = Result<Received, CallFailure>> + Send + 'call>> {
        let chosen = Chosen {
            built_in: self.0,
            ..Chosen::default()
        };
        let read = formats(
            &Document {
                html: &params.html,
                url: &params.url,
                base_url: &params.url,
                serialization: Serialization::Served,
            },
            &FormatsRequest {
                markdown: Some(MarkdownFormat::Asked(Asked)),
                text: Some(Asked),
                ..FormatsRequest::default()
            },
            &Context {
                extractor: chosen.extractor(),
                ..Context::default()
            },
        );
        let result = ExtractContentResult {
            title: None,
            text: read.text.unwrap_or_default(),
            markdown: read.markdown.unwrap_or_default(),
            metadata: BTreeMap::new(),
        };
        let json = serde_json::to_string(&result).unwrap();
        Box::pin(async move { Ok(Received::of_json(json)) })
    }
}

/// What the driver of `extractor` answers for `method` with `params`, as
/// a process Plugin's response would hold it.
fn driven(extractor: &BuiltInExtractor, method: &str, params: &Value) -> Value {
    assert_eq!(method, "extract.content");
    let params = ExtractContentParams::deserialize(params).unwrap();
    let url = Url::parse(&params.url).unwrap();
    let clock = Clock::system();
    let deadline = Deadline::given(&TIMEOUT_PLUGIN, PLUGIN_MILLIS, SettingsLayer::Default);
    real_time_runtime().block_on(async {
        match ExtractorDriver::new(extractor, &Unlimited, &clock, &deadline)
            .content(&url, params.html)
            .await
        {
            Ok(result) => json!({ "result": result }),
            Err(failure) => json!({ "error": { "code": failure.error().code() } }),
        }
    })
}

/// Each built-in main-content step, behind the Extractor's driver, passes
/// every case of the suite a Plugin passes, params from a newer engine
/// included.
///
/// Covers: story:7-87
#[test]
fn each_built_in_extractor_passes_the_extractor_suite_in_process() {
    for built_in in [BuiltIn::DomSmoothie, BuiltIn::Trafilatura] {
        let extractor = BuiltInExtractor(built_in);

        let failed: Vec<(String, Vec<CaseCheck>)> = PluginSuite::extract()
            .checks_in_process(|method, params| driven(&extractor, method, params))
            .into_iter()
            .filter(|(_, checks)| checks.is_empty() || checks.iter().any(|check| !check.passed))
            .collect();

        assert_eq!(failed, vec![], "{built_in:?}");
    }
}
