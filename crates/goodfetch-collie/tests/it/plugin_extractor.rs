//! An Extractor Plugin chosen by `content.extractor` (spec #596, ticket
//! 823): `goodfetch-scripted-plugin` installed in the Plugin store of the
//! test's Goodfetch home as an Extractor, which Collie starts confined
//! through its own executable as a page first reads with it, and whose
//! answer `page.formats` answers as the page's main content.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::fs;
use std::path::Path;

use goodfetch_test_support::{ProvingGround, TestEnv, Tool, browser_cache, tools};
use serde_json::{Value, json};

use crate::page::{opened, started};
use crate::plugin_host::hello;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 823;

/// Installs `goodfetch-scripted-plugin` in the Plugin store of `home` as
/// the Extractor `scripted`, which answers `extract.content` with
/// `answer`.
fn install(home: &Path, answer: &Value) {
    let bundle = home.join("plugins").join("scripted").join("1.0.0");
    fs::create_dir_all(&bundle).unwrap();
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedPlugin)
        .unwrap()
        .to_owned();
    fs::copy(scripted, bundle.join("plugin")).unwrap();
    let script = json!({
        "hello": hello(&json!([{ "kind": "extract", "capabilities": [] }])),
        "answers": { "extract.content": answer },
    });
    fs::write(bundle.join("script.json"), script.to_string()).unwrap();
    let manifest = json!({
        "manifest_version": "0.1",
        "name": "scripted",
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
    });
    fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
}

/// A page whose `content.extractor` names the installed Extractor reads
/// its main content with it; a page that names none reads it with the
/// built-in; a page naming an Extractor no one installed is refused.
///
/// Covers: story:7-87, setting:content.extractor, operation:page.formats, error:setting_invalid
#[test]
fn a_page_reads_its_main_content_with_the_extractor_plugin_it_names() {
    let env = TestEnv::new();
    install(
        env.home(),
        &json!({ "result": {
            "title": "Kettle",
            "text": "Kettle\n\nBoils water.",
            "markdown": "# Kettle\n\nBoils water.",
            "metadata": {},
        } }),
    );
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "plugins");
    let product = proving_ground.url("extraction.test", "/markdown/mixed/product");
    let extractor = |name: &str| json!({ "content": { "extractor": name } });

    let by_plugin = opened(&engine.ask(
        2,
        "run.new_page",
        &json!({ "run_id": "plugins", "settings": extractor("scripted") }),
    ));
    let built_in = opened(&engine.ask(3, "run.new_page", &json!({ "run_id": "plugins" })));
    let absent = engine.ask(
        4,
        "run.new_page",
        &json!({ "run_id": "plugins", "settings": extractor("absent") }),
    );
    for (id, page) in [(5, &by_plugin), (6, &built_in)] {
        engine.ask(
            id,
            "page.goto",
            &json!({ "page_id": page.page_id, "url": product }),
        );
    }
    let formats = json!({ "markdown": true, "text": true, "html": { "kind": "cleaned" } });
    let plugins = engine.ask(
        7,
        "page.formats",
        &json!({ "page_id": by_plugin.page_id, "formats": formats }),
    );
    let built_ins = engine.ask(
        8,
        "page.formats",
        &json!({ "page_id": built_in.page_id, "formats": { "markdown": true } }),
    );
    engine.ask(9, "run.finish", &json!({ "run_id": "plugins" }));
    let (rest, exited) = engine.finish();

    let cleaned = plugins["result"]["html"].as_str().unwrap_or_default();
    assert_eq!(
        (
            &plugins["result"]["main_content"],
            &plugins["result"]["markdown"],
            &plugins["result"]["text"],
            cleaned.contains("Desk Lamp"),
            built_ins["result"]["markdown"]
                .as_str()
                .is_some_and(|markdown| markdown.starts_with("## Desk Lamp")),
            &absent["error"]["data"]["code"],
            &absent["error"]["data"]["details"]["key"],
            (rest, exited),
        ),
        (
            &json!("found"),
            &json!("# Kettle\n\nBoils water."),
            &json!("Kettle\n\nBoils water."),
            true,
            true,
            &json!("setting_invalid"),
            &json!("content.extractor"),
            (vec![], true)
        ),
        "{plugins}\n{built_ins}\n{absent}"
    );
}
