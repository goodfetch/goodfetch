//! Linked Rust engine `plugin-shim` integration tests (ADRs 0033, 0051, 0090
//! and 0115; issue 1077):
//! - a real linked Collie engine finds `plugin-shim` through
//!   `GOODFETCH_COLLIE_ENGINE_PATH` and confines a `proxy_tcp` Plugin the same
//!   as an Attached engine;
//! - under `egress.offline` without `GOODFETCH_COLLIE_ENGINE_PATH`, the linked
//!   engine refuses with `artifact_unavailable_offline` naming the
//!   `engine_executable` artifact and Collie's version;
//! - without `GOODFETCH_COLLIE_ENGINE_PATH`, downloading the real
//!   `goodfetch-collie` executable from a local HTTPS stand-in verifies its
//!   digest into the shared Browser cache and confines a `proxy_tcp` Plugin
//!   through its loopback proxy relay identically (allowed destination 200,
//!   forbidden destination 403).
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests set up Plugin bundles, echo sockets and stand-in files outside async workers"
)]
#![expect(
    clippy::disallowed_types,
    reason = "tests bind local echo listeners for proxy_tcp confinement verification (ADR 0062)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::path::Path;
use std::sync::Arc;

use goodfetch_collie::{Collie, ConnectOptions, EngineChoice, ErrorCode};
use goodfetch_confinement::launch::Launch;
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::Layout;
use goodfetch_engine::browser_cache::{
    BrowserCache, CacheLocation, PinnedArtifact, content_digest,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::pinned_download::{Digest, Fetching};
use goodfetch_engine::plugin_gate::PluginFetch;
use goodfetch_engine::plugin_host::instance::{self, PluginInstance, Scope};
use goodfetch_engine::plugin_shim::{PluginShim, default_pin, host_platform};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::tls::Trust;
use goodfetch_protocol::browser_cache::{
    ArtifactId, ArtifactKind, ArtifactUnavailableOfflineDetails, Pinner,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::run::{PluginConfinement, PluginForm};
use goodfetch_protocol::settings::COLLIE_ENGINE_PATH_VARIABLE;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    ProvingGround, TestEnv, Tool, browser_cache, real_time_runtime, tools,
};
use serde_json::{Value, json};

use crate::COLLIE;
use crate::clean_up_rules::stand_in::StandIn;
use crate::page::{opened, started};
use crate::plugin_host::hello;

const SEED: u64 = 1077;

fn has_user_namespaces() -> bool {
    matches!(
        goodfetch_confinement::linux::host::read().user_namespace,
        goodfetch_confinement::host::UserNamespace::Created
    )
}

/// Installs a `proxy_tcp` Extractor Plugin in `home`'s Plugin store and allows
/// it in Operator policy. When `extract.content` runs, `goodfetch-scripted-plugin`
/// expands `${HTTP_PROXY}` into `text` so the test proves the loopback proxy
/// relay was wired inside the isolated network namespace.
fn install_proxy_tcp_extractor(home: &Path, endpoint_url: &str) {
    let policy = HomeLayout::of(home).operator_policy();
    fs::create_dir_all(policy.parent().unwrap()).unwrap();
    fs::write(
        &policy,
        format!(
            "version = \"0.1\"\n\n[settings.plugins.scripted]\nallow = true\n\n[settings.plugins.scripted.settings]\napi = \"{endpoint_url}\"\n"
        ),
    )
    .unwrap();
    let bundle = home.join("plugins").join("scripted").join("1.0.0");
    fs::create_dir_all(&bundle).unwrap();
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedPlugin)
        .unwrap()
        .to_owned();
    fs::copy(scripted, bundle.join("plugin")).unwrap();
    let mut hello_result = hello(&json!([{ "kind": "extract", "capabilities": [] }]));
    hello_result["network_level"] = json!("proxy_tcp");
    let script = json!({
        "hello": hello_result,
        "answers": {
            "extract.content": {
                "with_environment": true,
                "result": {
                    "title": "Proxied",
                    "text": "${HTTP_PROXY}",
                    "markdown": "${HTTP_PROXY}",
                    "metadata": {},
                }
            }
        }
    });
    fs::write(bundle.join("script.json"), script.to_string()).unwrap();
    let manifest = json!({
        "manifest_version": "0.1",
        "name": "scripted",
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "proxy_tcp",
        "env": { "API_URL": "api" },
        "settings_schema": {
            "type": "object",
            "properties": {
                "api": { "type": "string", "format": "uri", "x-goodfetch-endpoint": true }
            }
        }
    });
    fs::write(bundle.join("goodfetch-plugin.json"), manifest.to_string()).unwrap();
}

/// A linked Rust engine finds `plugin-shim` through
/// `GOODFETCH_COLLIE_ENGINE_PATH` and confines a `proxy_tcp` Plugin the same
/// as an Attached engine: both deliver a per-instance `HTTP_PROXY` loopback
/// relay inside an isolated network namespace and report identical `RunReport`
/// Plugin confinement (`form: process`, `network_level: proxy_tcp`,
/// `confinement: full`).
///
/// Covers: story:7-107
#[test]
fn linked_engine_finds_shim_via_env_var_and_confines_proxy_tcp_like_attached() {
    if !has_user_namespaces() {
        return;
    }

    let endpoint = "http://127.0.0.1:18080/";

    // 1. Attached engine baseline.
    let attached_env = TestEnv::new();
    install_proxy_tcp_extractor(attached_env.home(), endpoint);
    let attached_pg = ProvingGround::start(&attached_env, SEED).unwrap();
    let attached_cache = browser_cache(&attached_env).unwrap();
    let mut attached = started(
        &attached_env,
        &attached_cache,
        &attached_pg,
        "attached-shim",
    );
    let attached_url = attached_pg.url("extraction.test", "/markdown/mixed/product");
    let attached_page = opened(&attached.ask(
        2,
        "run.new_page",
        &json!({ "run_id": "attached-shim", "settings": { "content": { "extractor": "scripted" } } }),
    ));
    attached.ask(
        3,
        "page.goto",
        &json!({ "page_id": attached_page.page_id, "url": attached_url }),
    );
    let attached_formats = attached.ask(
        4,
        "page.formats",
        &json!({ "page_id": attached_page.page_id, "formats": { "text": true } }),
    );
    let attached_text = attached_formats["result"]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    let attached_report = attached.ask(5, "run.finish", &json!({ "run_id": "attached-shim" }));
    drop(attached);

    // 2. Linked Rust engine finding `plugin-shim` via GOODFETCH_COLLIE_ENGINE_PATH.
    let linked_env = TestEnv::new();
    install_proxy_tcp_extractor(linked_env.home(), endpoint);
    let linked_pg = ProvingGround::start(&linked_env, SEED).unwrap();
    let linked_cache = browser_cache(&linked_env).unwrap();
    linked_env.user_configuration(&linked_pg.description().reaching());
    let mut env_map = linked_env.env_os();
    env_map.insert(
        "GOODFETCH_BROWSERS_PATH".into(),
        linked_cache.into_os_string(),
    );
    env_map.insert(COLLIE_ENGINE_PATH_VARIABLE.into(), COLLIE.into());
    let linked_url = linked_pg.url("extraction.test", "/markdown/mixed/product");

    let (linked_text, linked_report) = real_time_runtime().block_on(async {
        let collie = Collie::connect(
            ConnectOptions::default()
                .engine(EngineChoice::Linked)
                .environment(env_map),
        )
        .await
        .unwrap();
        let run = collie.start().await.unwrap();
        let page = run
            .call("run.new_page")
            .params(json!({
                "run_id": run.id(),
                "settings": { "content": { "extractor": "scripted" } }
            }))
            .await
            .unwrap();
        let page_id = page["page_id"].as_str().unwrap().to_owned();
        run.call("page.goto")
            .params(json!({ "page_id": page_id, "url": linked_url }))
            .await
            .unwrap();
        let formats = run
            .call("page.formats")
            .params(json!({ "page_id": page_id, "formats": { "text": true } }))
            .await
            .unwrap();
        let text = formats["text"].as_str().unwrap().to_owned();
        let report = run.finish().await.unwrap();
        collie.close().await.unwrap();
        (text, report)
    });

    assert!(
        attached_text.starts_with("http://") && attached_text.contains("@127.0.0.1:"),
        "attached proxy_tcp Plugin should receive loopback HTTP_PROXY: {attached_text}"
    );
    assert!(
        linked_text.starts_with("http://") && linked_text.contains("@127.0.0.1:"),
        "linked proxy_tcp Plugin should receive loopback HTTP_PROXY: {linked_text}"
    );
    let linked_plugins = linked_report.plugins.as_deref().unwrap_or(&[]);
    assert_eq!(linked_plugins.len(), 1, "{linked_plugins:?}");
    let linked_plugin = &linked_plugins[0];
    assert_eq!(linked_plugin.name, "scripted");
    assert_eq!(linked_plugin.form, PluginForm::Process);
    assert_eq!(linked_plugin.network_level, "proxy_tcp");
    assert_eq!(linked_plugin.confinement, PluginConfinement::Full);

    let attached_plugins = attached_report["result"]["plugins"].as_array().unwrap();
    assert_eq!(attached_plugins.len(), 1, "{attached_plugins:?}");
    assert_eq!(attached_plugins[0]["form"], json!("process"));
    assert_eq!(attached_plugins[0]["network_level"], json!("proxy_tcp"));
    assert_eq!(attached_plugins[0]["confinement"], json!("full"));
}

/// Under `egress.offline`, a linked Collie engine without
/// `GOODFETCH_COLLIE_ENGINE_PATH` and with an empty shim cache fails when
/// starting a process Plugin with `artifact_unavailable_offline`, naming the
/// `engine_executable` artifact and Collie's version.
///
/// Covers: story:7-107, error:artifact_unavailable_offline
#[test]
fn linked_engine_offline_without_env_var_fails_with_artifact_unavailable_offline() {
    if !has_user_namespaces() {
        return;
    }

    let env = TestEnv::new();
    install_proxy_tcp_extractor(env.home(), "http://127.0.0.1:18080/");
    let pg = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    env.user_configuration(&pg.description().reaching());
    let mut env_map = env.env_os();
    env_map.insert("GOODFETCH_BROWSERS_PATH".into(), cache.into_os_string());
    env_map.remove(std::ffi::OsStr::new(COLLIE_ENGINE_PATH_VARIABLE));
    let url = pg.url("extraction.test", "/markdown/mixed/product");

    let error = real_time_runtime().block_on(async {
        let collie = Collie::connect(
            ConnectOptions::default()
                .engine(EngineChoice::Linked)
                .environment(env_map),
        )
        .await
        .unwrap();
        let run = collie
            .start()
            .settings(
                goodfetch_collie::settings::Settings::new()
                    .set(goodfetch_collie::settings::egress::OFFLINE, true),
            )
            .await
            .unwrap();
        let page = run
            .call("run.new_page")
            .params(json!({
                "run_id": run.id(),
                "settings": { "content": { "extractor": "scripted" } }
            }))
            .await
            .unwrap();
        let page_id = page["page_id"].as_str().unwrap().to_owned();
        run.call("page.goto")
            .params(json!({ "page_id": page_id, "url": url }))
            .await
            .unwrap();
        let failed = run
            .call("page.formats")
            .params(json!({ "page_id": page_id, "formats": { "text": true } }))
            .await
            .expect_err("offline linked engine without shim must refuse");
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        failed
    });

    assert_eq!(
        error.code(),
        ErrorCode::ARTIFACT_UNAVAILABLE_OFFLINE,
        "{error:?}"
    );
    let details: ArtifactUnavailableOfflineDetails = error.details().unwrap();
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap();
    assert_eq!(details.artifact, default_pin(Product::Collie, &version).id);
    assert_eq!(
        details.pinned_by,
        vec![Pinner::Product {
            name: Product::Collie,
            version,
        }]
    );
    assert!(
        details.fix.contains(COLLIE_ENGINE_PATH_VARIABLE),
        "fix must name {COLLIE_ENGINE_PATH_VARIABLE}: {}",
        details.fix
    );
}

fn serve_collie_shim(
    host: &str,
    path: &str,
    version: &ProductVersion,
) -> (StandIn, PinnedArtifact, Fetching) {
    let stand_in = StandIn::start(&[host]);
    let collie_bytes = fs::read(COLLIE).unwrap();
    stand_in.serve(path, collie_bytes.clone());

    let digest_dir = tempfile::tempdir().unwrap();
    fs::write(digest_dir.path().join("goodfetch-collie"), &collie_bytes).unwrap();
    let tree = content_digest(digest_dir.path()).unwrap();

    let pin = PinnedArtifact {
        id: ArtifactId {
            kind: ArtifactKind::EngineExecutable,
            name: Product::Collie.executable_name().to_owned(),
            build: format!("{version}-{}", host_platform()),
        },
        platform: None,
        url: format!("https://{}{path}", stand_in.origin(host)),
        archive_digest: Some(Digest::of(&collie_bytes)),
        digest: tree,
        application_settings: BTreeMap::new(),
    };

    let ca_dir = tempfile::tempdir().unwrap();
    let ca_file = ca_dir.path().join("ca.pem");
    stand_in.write_authority(&ca_file);
    let ca_pem = fs::read(&ca_file).unwrap();
    let ca_certs: Vec<_> =
        <rustls::pki_types::CertificateDer<'static> as rustls::pki_types::pem::PemObject>::pem_slice_iter(
            &ca_pem,
        )
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let fetching = Fetching {
        resolve: HostMapping::default().with(host, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]),
        trust: Trust::default_roots()
            .with_trusted_cas(host, ca_certs)
            .unwrap(),
        ..Fetching::new(Clock::system())
    };
    (stand_in, pin, fetching)
}

fn build_proxy_tcp_launch() -> (tempfile::TempDir, PluginManifest, Launch) {
    let plugin_root = tempfile::tempdir().unwrap();
    let layout = Layout::under(&fs::canonicalize(plugin_root.path()).unwrap());
    for folder in [&layout.bundle, &layout.instance] {
        fs::create_dir_all(folder).unwrap();
    }
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedPlugin)
        .unwrap()
        .to_owned();
    fs::copy(scripted, layout.bundle.join("plugin")).unwrap();
    let mut hello_result = hello(&json!([{ "kind": "proxy", "capabilities": [] }]));
    hello_result["network_level"] = json!("proxy_tcp");
    let played = layout.bundle.join("script.json");
    fs::write(&played, json!({ "hello": hello_result }).to_string()).unwrap();
    let manifest: PluginManifest = serde_json::from_value(json!({
        "manifest_version": "0.1",
        "name": "scripted",
        "version": "1.0.0",
        "kinds": ["proxy"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "proxy_tcp",
        "env": { "API_URL": "api" },
        "settings_schema": {
            "type": "object",
            "properties": {
                "api": { "type": "string", "format": "uri", "x-goodfetch-endpoint": true }
            }
        }
    }))
    .unwrap();
    let instance_spec = Instance {
        bundle: layout.bundle.clone(),
        runtime: None,
        folders: vec![layout.instance.clone()],
        env: BTreeMap::new(),
    };
    let policy = Policy::of(&manifest, &instance_spec, &layout.places).unwrap();
    let launch = Launch {
        policy,
        program: layout.bundle.join("plugin"),
        args: vec![played.to_string_lossy().into_owned()],
        folder: layout.instance.clone(),
        proxy_relay: None,
        reduced_confinement: false,
    };
    (plugin_root, manifest, launch)
}

/// Downloading the real `goodfetch-collie` executable from a local HTTPS
/// server into the shared Browser cache verifies its SHA-256 digest and runs
/// a real confined `proxy_tcp` Plugin through the downloaded `plugin-shim`:
/// allowed destinations connect with 200 and forbidden destinations are
/// refused with 403.
///
/// Covers: story:7-107
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloaded_shim_confines_proxy_tcp_plugin_and_enforces_destinations() {
    if !has_user_namespaces() {
        return;
    }

    let host = "shim-builds.goodfetch.test";
    let path = "/releases/v0.1.0/goodfetch-collie";
    let version = ProductVersion::parse(env!("CARGO_PKG_VERSION")).unwrap();
    let (stand_in, pin, fetching) = serve_collie_shim(host, path, &version);

    let env = TestEnv::new();
    let root = TaskOwner::root(BlockingPolicy::Inline);
    let cache = BrowserCache::new(
        &CacheLocation::new(env.home(), None),
        root.blocking_threads(),
    );
    let shim = PluginShim::linked(Product::Collie, version, Some(pin));
    let resolved = shim
        .resolve(
            &cache,
            Some(&fetching),
            Clock::system().timestamp(),
            &root.blocking_threads(),
        )
        .await
        .unwrap();
    assert!(resolved.launcher.starts_with(cache.root()));
    assert_eq!(stand_in.asked(), vec![path.to_owned()]);

    let allowed = TcpListener::bind("127.0.0.1:0").unwrap();
    let allowed_addr = allowed.local_addr().unwrap();
    let forbidden = TcpListener::bind("127.0.0.1:0").unwrap();
    let forbidden_addr = forbidden.local_addr().unwrap();
    let echo_thread = std::thread::spawn(move || {
        if let Ok((mut sock, _)) = allowed.accept() {
            let mut buf = [0_u8; 32];
            if let Ok(n) = sock.read(&mut buf) {
                let _ = sock.write_all(&buf[..n]);
            }
        }
    });

    let (_plugin_root, manifest, launch) = build_proxy_tcp_launch();
    let user = json!({
        "plugins": { "scripted": { "settings": { "api": format!("http://{allowed_addr}/") } } }
    });
    let layers = Layers {
        user_configuration: Values::read(
            goodfetch_protocol::settings::SettingsLayer::UserConfiguration,
            user.as_object().unwrap(),
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };
    let settings = resolve(&layers, Product::Collie, Product::Collie).unwrap();
    let fetch = PluginFetch::of(
        &settings,
        "scripted",
        Clock::system(),
        root.blocking_threads(),
    )
    .unwrap();
    let inst = PluginInstance::start(
        &root,
        instance::Plugin {
            launcher: resolved.launcher.clone(),
            launcher_env: Vec::new(),
            launch,
            manifest,
            product: Product::Collie,
            fetch: Arc::new(fetch),
            clock: Clock::system(),
            spend: None,
        },
        Scope::Task(None),
    );

    let ok: Value = inst
        .call(
            "scripted.proxy_connect",
            &json!({ "target": allowed_addr.to_string(), "payload": "downloaded-shim" }),
        )
        .await
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(ok["status"], json!(200), "{ok}");
    assert_eq!(ok["response"], json!("downloaded-shim"), "{ok}");

    let denied: Value = inst
        .call(
            "scripted.proxy_connect",
            &json!({ "target": forbidden_addr.to_string(), "payload": "blocked" }),
        )
        .await
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(denied["status"], json!(403), "{denied}");

    inst.stop().await;
    drop(resolved);
    root.stop().await;
    let _ = echo_thread.join();
}
