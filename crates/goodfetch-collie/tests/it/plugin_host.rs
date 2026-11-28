//! A process Plugin started as the engine's Plugin host starts it (ADR
//! 0033): through Collie's executable in its hidden `plugin-shim` mode,
//! confined, with `goodfetch-scripted-plugin` as the Plugin, alone or as
//! an instance of a Run or of a task.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests lay out a Plugin's folders and start processes outside any async worker"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a Plugin's `net.fetch` reaches stand-in endpoints on loopback sockets (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use goodfetch_confinement::launch::Launch;
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::Layout;
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::{Deadline, Enclosing};
use goodfetch_engine::home::Home;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded};
use goodfetch_engine::plugin_gate::PluginFetch;
use goodfetch_engine::plugin_host::instance::{
    self, CallFailure, CallOptions, InstanceHooks, PluginInstance, QuarantineEvent,
    QuarantineNotifier, Scope, UncertainCall, UncertainReporter,
};
use goodfetch_engine::plugin_host::process::{Pipes, PluginProcess};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, OwnerKind, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{CALL_CANCELLED, EnclosingCall};
use goodfetch_protocol::effect::UncertainReason;
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::plugin::{
    COOKIE, COOKIE_VARIABLE, PLUGIN_QUARANTINED_AUDIT, PluginManifest,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_PLUGIN};
use goodfetch_test_support::{Tool, capturing, tools};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::ChildStdout;
use tracing::Level;

use crate::COLLIE;

/// A scripted Plugin's bundle and private temporary folder under a fresh
/// root, its approved manifest and the launch that starts it.
pub(crate) struct Plugin {
    _root: TempDir,
    layout: Layout,
    manifest: PluginManifest,
    launch: Launch,
}

impl Plugin {
    /// A Plugin whose manifest declares `API_URL`, given with a variable it
    /// does not declare, and whose `hello` answers nothing.
    fn new() -> Self {
        Self::scripted(&json!({ "hello": {} }))
    }

    /// A Plugin that answers its approved manifest's `hello`, the Proxy
    /// kind's.
    fn approved() -> Self {
        Self::scripted(
            &json!({ "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])) }),
        )
    }

    /// The Plugin as [`new`](Self::new) lays it out, which plays `script`.
    fn scripted(script: &Value) -> Self {
        Self::of_kind("proxy", script)
    }

    /// The Plugin of `kind` as [`new`](Self::new) lays it out, which plays
    /// `script`.
    pub(crate) fn of_kind(kind: &str, script: &Value) -> Self {
        Self::of_kind_with_concurrency(kind, 4, script)
    }

    /// The Plugin of `kind` with `manifest.limits.concurrency` set to
    /// `concurrency`, which plays `script`.
    fn of_kind_with_concurrency(kind: &str, concurrency: u32, script: &Value) -> Self {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::under(&fs::canonicalize(root.path()).unwrap());
        for folder in [&layout.bundle, &layout.instance] {
            fs::create_dir_all(folder).unwrap();
        }
        let scripted = tools()
            .unwrap()
            .path(Tool::ScriptedPlugin)
            .unwrap()
            .to_owned();
        fs::copy(scripted, layout.bundle.join("plugin")).unwrap();
        let played = layout.bundle.join("script.json");
        fs::write(&played, script.to_string()).unwrap();
        let manifest = json!({
            "manifest_version": "0.1",
            "name": "scripted",
            "version": "1.0.0",
            "kinds": [kind],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
            "env": { "API_URL": "api_url" },
            "limits": { "concurrency": concurrency },
        });
        let manifest = PluginManifest::read(manifest.to_string().as_bytes()).unwrap();
        let instance = Instance {
            bundle: layout.bundle.clone(),
            runtime: None,
            folders: vec![layout.instance.clone()],
            env: BTreeMap::from([
                ("API_URL".to_owned(), "https://api.test".to_owned()),
                ("GITHUB_TOKEN".to_owned(), "canary".to_owned()),
            ]),
        };
        let policy = Policy::of(&manifest, &instance, &layout.places).unwrap();
        let launch = Launch {
            policy,
            program: layout.bundle.join("plugin"),
            args: vec![played.to_string_lossy().into_owned()],
            folder: layout.instance.clone(),
            proxy_relay: None,
            proxy_loopback: None,
            windows_broker: None,
            reduced_confinement: false,
        };
        Self {
            _root: root,
            layout,
            manifest,
            launch,
        }
    }

    /// What starts its instance in Collie, its `net.fetch` under the
    /// built-in settings.
    pub(crate) fn instance(&self) -> instance::Plugin {
        self.instance_with(&json!({}))
    }

    /// What starts its instance in Collie, its `net.fetch` under the
    /// settings user configuration sets to `user`, a settings object.
    fn instance_with(&self, user: &Value) -> instance::Plugin {
        let Value::Object(user) = user else {
            panic!("settings are an object");
        };
        let layers = Layers {
            user_configuration: Values::read(
                SettingsLayer::UserConfiguration,
                user,
                Product::Collie,
            )
            .unwrap(),
            ..Layers::default()
        };
        let settings = resolve(&layers, Product::Collie, Product::Collie).unwrap();
        let blocking = TaskOwner::root(BlockingPolicy::Threads).blocking_threads();
        let fetch = PluginFetch::of(&settings, "scripted", Clock::system(), blocking).unwrap();
        instance::Plugin {
            launcher: COLLIE.into(),
            launcher_env: Vec::new(),
            launch: self.launch.clone(),
            manifest: self.manifest.clone(),
            product: Product::Collie,
            fetch: Arc::new(fetch),
            clock: Clock::system(),
            spend: None,
        }
    }
}

/// The scripted Plugin's answer to `hello` with `kinds`, its approved
/// manifest's otherwise.
pub(crate) fn hello(kinds: &Value) -> Value {
    json!({
        "protocol_version": "0.1",
        "name": "scripted",
        "version": "1.0.0",
        "kinds": kinds,
        "network_level": "none",
    })
}

/// The JSON value `instance` answers `method` with.
async fn answer(instance: &PluginInstance, method: &str) -> Value {
    instance
        .call(method, &json!({}))
        .await
        .unwrap()
        .read()
        .unwrap()
}

/// The Error code of a call that the instance did not carry out.
fn unavailable(failure: Result<impl std::fmt::Debug, CallFailure>) -> &'static str {
    match failure {
        Err(CallFailure::Unavailable(error)) => error.code(),
        other => panic!("{other:?} is no call the instance did not carry out"),
    }
}

/// The Run the tests' instances serve.
fn checkout() -> RunId {
    RunId::parse("checkout").unwrap()
}

/// The started Plugin's side of the plugin protocol, one line a message.
struct Connection {
    stdin: tokio::process::ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next: u64,
}

impl Connection {
    fn new(pipes: Pipes) -> Self {
        Self {
            stdin: pipes.stdin,
            lines: BufReader::new(pipes.stdout).lines(),
            next: 1,
        }
    }

    /// The Plugin's answer to `method` with `params`.
    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut line = serde_json::to_vec(&request).unwrap();
        line.push(b'\n');
        self.stdin.write_all(&line).await.unwrap();
        self.stdin.flush().await.unwrap();
        let answer = self.lines.next_line().await.unwrap().unwrap();
        let answer: Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(answer["id"], id, "{answer}");
        answer
    }
}

/// The Plugin sees the variables its manifest declares and the cookie,
/// nothing the engine holds and no loader variable, works from its
/// private temporary folder and holds no handle beyond stdio.
///
/// Covers: story:7-30
#[tokio::test]
async fn a_started_plugin_holds_its_declared_variables_alone() {
    let plugin = Plugin::new();
    let (process, pipes) = PluginProcess::start(Path::new(COLLIE), &plugin.launch).unwrap();
    let mut connection = Connection::new(pipes);

    let mut environment = connection.call("scripted.environment", json!({})).await;

    let mut environment = environment["result"].take();
    let id = environment.as_object_mut().unwrap().remove("process");
    assert_eq!(
        (environment, id.is_some_and(|id| id.is_u64())),
        (
            json!({
                "variables": { "API_URL": "https://api.test", COOKIE_VARIABLE: COOKIE },
                "folder": plugin.layout.instance,
                "handles": [],
            }),
            true
        )
    );
    process
        .stop(&TaskOwner::root(BlockingPolicy::Threads))
        .await
        .unwrap();
}

/// What the Plugin writes in its private temporary folder is gone once it
/// stops.
///
/// Covers: story:7-30
#[tokio::test]
async fn a_stopped_plugins_temporary_folder_is_gone() {
    let plugin = Plugin::new();
    let (process, pipes) = PluginProcess::start(Path::new(COLLIE), &plugin.launch).unwrap();
    let mut connection = Connection::new(pipes);
    let written = plugin.layout.instance.join("written");
    let answer = connection
        .call(
            "scripted.write",
            json!({ "path": written, "text": "canary" }),
        )
        .await;
    assert_eq!(answer["result"], json!({}), "{answer}");
    assert!(written.exists());

    process
        .stop(&TaskOwner::root(BlockingPolicy::Threads))
        .await
        .unwrap();

    assert!(!plugin.layout.instance.exists());
}

/// A Plugin started without the plugin protocol's cookie, as a program
/// run by hand, refuses to run and says nothing on the protocol's pipe.
///
/// Covers: story:7-31
#[test]
fn a_plugin_started_without_the_cookie_refuses_to_run() {
    let plugin = Plugin::new();

    let output = Command::new(&plugin.launch.program)
        .args(&plugin.launch.args)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!((output.status.code(), output.stdout), (Some(1), Vec::new()));
}

/// An instance started on a Run's owner says `hello` for that Run before
/// the call made while it started, and is gone once the Run's owner stops,
/// as every end of a Run stops it: its process, its private temporary
/// folder and the calls made after.
///
/// Covers: story:7-28
#[tokio::test]
async fn a_runs_instance_is_gone_once_its_run_ends() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let run = root.child_for(OwnerKind::Run);
    let plugin = Plugin::approved();
    let instance = PluginInstance::start(&run, plugin.instance(), Scope::Run(checkout()));

    let calls = answer(&instance, "scripted.calls").await;
    run.stop().await;

    assert_eq!(
        (
            calls.as_array().map(Vec::len),
            calls[0]["method"].clone(),
            calls[0]["params"]["run_id"].clone(),
            plugin.layout.instance.exists(),
            unavailable(instance.call("scripted.calls", &json!({})).await),
        ),
        (
            Some(1),
            json!("hello"),
            json!("checkout"),
            false,
            "call_cancelled"
        )
    );
}

/// An instance started on a task's own owner, outside the Run's life,
/// runs a process of its own, says `hello` for no Run, and is killed when
/// its task ends while the Run's instance still answers.
///
/// Covers: story:7-29
#[tokio::test]
async fn a_tasks_instance_shares_no_process_with_a_run_and_ends_with_its_task() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let (run, task) = (root.child_for(OwnerKind::Run), root.child());
    let (for_run, for_task) = (Plugin::approved(), Plugin::approved());
    let run_instance = PluginInstance::start(&run, for_run.instance(), Scope::Run(checkout()));
    let task_instance = PluginInstance::start(&task, for_task.instance(), Scope::Task(None));
    let run_process = answer(&run_instance, "scripted.environment").await["process"].take();
    let task_process = answer(&task_instance, "scripted.environment").await["process"].take();
    let task_hello = answer(&task_instance, "scripted.calls").await[0]["params"].take();

    task.stop().await;

    assert_eq!(
        (
            run_process == task_process,
            task_hello.get("run_id"),
            for_task.layout.instance.exists(),
            unavailable(task_instance.call("scripted.calls", &json!({})).await),
            answer(&run_instance, "scripted.calls").await[0]["method"].clone(),
        ),
        (false, None, false, "call_cancelled", json!("hello"))
    );
    run.stop().await;
}

/// An instance whose `hello` declares another kind than its approved
/// manifest is stopped with `plugin_manifest_mismatch`, which the call made
/// while it started fails with.
///
/// Covers: story:7-31, error:plugin_manifest_mismatch
#[tokio::test]
async fn an_instance_whose_hello_differs_from_its_manifest_is_stopped() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let plugin = Plugin::scripted(
        &json!({ "hello": hello(&json!([{ "kind": "sink", "capabilities": [] }])) }),
    );
    let instance = PluginInstance::start(&root, plugin.instance(), Scope::Task(None));

    let failure = instance.call("scripted.calls", &json!({})).await;

    assert_eq!(
        (
            unavailable(failure),
            instance.ended().map(|error| error.code()),
            plugin.layout.instance.exists(),
        ),
        (
            "plugin_manifest_mismatch",
            Some("plugin_manifest_mismatch"),
            false
        )
    );
    root.stop().await;
}

/// A second answer to `hello` and an answer to no call are logged at
/// `warn` as malformed answers and change nothing: the instance still
/// answers, and no transition is refused.
///
/// Covers: story:7-28
#[tokio::test]
async fn a_plugins_stray_answers_are_malformed_answers_not_events() {
    let (subscriber, log) = capturing(&[("goodfetch_engine::plugin_host", Level::WARN)]);
    let _logging = tracing::subscriber::set_default(subscriber);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let mut script = json!({ "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])) });
    script["after_hello"] = json!([
        { "jsonrpc": "2.0", "id": 0, "result": script["hello"] },
        { "jsonrpc": "2.0", "id": 99, "result": {} },
    ]);
    let plugin = Plugin::scripted(&script);
    let instance = PluginInstance::start(&root, plugin.instance(), Scope::Task(None));

    let calls = answer(&instance, "scripted.calls").await;

    let written = String::from_utf8(log.written()).unwrap();
    let levels: Vec<&str> = written
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .collect();
    assert_eq!(
        (
            calls[0]["method"].clone(),
            levels,
            instance.ended().is_none()
        ),
        (json!("hello"), vec!["WARN", "WARN"], true),
        "{written}"
    );
    instance.stop().await;
}

/// A `net.fetch` of `url`, as the scripted Plugin's answer calls back with
/// it, naming `call_id` when given, the served call's id otherwise.
fn fetching(url: &str, call_id: Option<u64>) -> Value {
    let mut params = json!({ "method": "GET", "url": url });
    if let Some(call_id) = call_id {
        params["call_id"] = json!(call_id);
    }
    json!({ "call_back": { "method": "net.fetch", "params": params } })
}

/// A loopback server, a task of `owner`, that answers its first request
/// `200` with `hello`, and the address it listens on.
async fn answering_once(owner: &TaskOwner) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    owner.spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await
            && !line.is_empty()
        {}
        let _ = writer
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello")
            .await;
    });
    address
}

/// A Plugin calls back only while it serves a call, naming that call's
/// id: a `net.fetch` it sends before any call, or naming an id it does not
/// serve, is refused with `plugin_call_back_refused`, and reaches nothing.
///
/// Covers: story:7-53, error:plugin_call_back_refused
#[tokio::test]
async fn a_call_back_outside_the_call_it_names_is_refused() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let mut script = json!({
        "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
        "answers": { "kettle.fetch": fetching("https://example.com/", Some(42)) },
    });
    script["after_hello"] = json!([{
        "jsonrpc": "2.0",
        "id": "before-any-call",
        "method": "net.fetch",
        "params": { "call_id": 1000, "method": "GET", "url": "https://example.com/" },
    }]);
    let plugin = Plugin::scripted(&script);
    let instance = PluginInstance::start(&root, plugin.instance(), Scope::Task(None));

    let naming_another = answer(&instance, "kettle.fetch").await;
    let calls = answer(&instance, "scripted.calls").await;

    let before_any_call = calls
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["id"] == "before-any-call")
        .unwrap_or_else(|| panic!("{calls} holds no answer to the call back before any call"));
    assert_eq!(
        (
            before_any_call["error"]["data"]["code"].clone(),
            naming_another["error"]["data"]["code"].clone(),
        ),
        (
            json!("plugin_call_back_refused"),
            json!("plugin_call_back_refused")
        ),
        "{calls}"
    );
    instance.stop().await;
    root.stop().await;
}

/// A call back during the call it names is sent: to one of the Plugin's
/// endpoints, which user configuration set, on loopback, it gets the whole
/// response; to a loopback address that is none of them, checked as a
/// page's, it is refused with `egress_refused` before any connection.
///
/// Covers: story:7-53, story:7-57, story:7-58, error:egress_refused
#[tokio::test]
async fn a_call_back_during_its_call_reaches_an_endpoint_and_no_other_loopback_address() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let endpoint = answering_once(&root).await;
    let elsewhere = TcpListener::bind("127.0.0.1:0").unwrap();
    elsewhere.set_nonblocking(true).unwrap();
    let elsewhere_url = format!("http://{}/", elsewhere.local_addr().unwrap());
    let plugin = Plugin::scripted(&json!({
        "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
        "answers": {
            "kettle.endpoint": fetching(&format!("{endpoint}/status"), None),
            "kettle.elsewhere": fetching(&elsewhere_url, None),
        },
    }));
    let user = json!({ "plugins": { "scripted": { "settings": { "api": endpoint } } } });
    let instance = PluginInstance::start(&root, plugin.instance_with(&user), Scope::Task(None));

    let reached = answer(&instance, "kettle.endpoint").await;
    let refused = answer(&instance, "kettle.elsewhere").await;

    assert_eq!(
        (
            reached["result"]["status"].clone(),
            reached["result"]["body"].clone(),
            refused["error"]["data"]["code"].clone(),
            elsewhere.accept().is_err(),
        ),
        (json!(200), json!("aGVsbG8="), json!("egress_refused"), true),
        "{reached} {refused}"
    );
    instance.stop().await;
    root.stop().await;
}

/// Records every uncertain Effect call reported by a confined Plugin
/// instance.
#[derive(Debug, Default)]
struct RecordedUncertain(Mutex<Vec<UncertainCall>>);

impl UncertainReporter for RecordedUncertain {
    fn report(&self, call: UncertainCall) {
        self.0.lock().unwrap().push(call);
    }
}

/// Over Collie's executable with a confined scripted Plugin, calls beyond
/// `manifest.limits.concurrency` wait in order with nothing dropped, and a
/// Plugin at its concurrency limit can still call back `net.fetch` without
/// deadlocking.
///
/// Covers: story:7-32
#[tokio::test]
async fn calls_beyond_concurrency_wait_in_order_and_call_back_at_limit_over_collie() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let endpoint = answering_once(&root).await;
    let plugin = Plugin::of_kind_with_concurrency(
        "proxy",
        1,
        &json!({
            "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
            "answers": {
                "work.callback": fetching(&format!("{endpoint}/status"), None),
                "work.second": { "result": { "order": 2 } },
                "work.third": { "result": { "order": 3 } },
            },
        }),
    );
    let user = json!({ "plugins": { "scripted": { "settings": { "api": endpoint } } } });
    let instance = PluginInstance::start(&root, plugin.instance_with(&user), Scope::Task(None));

    // Fire three calls against concurrency=1; call 1 calls back `net.fetch`
    // while calls 2 and 3 wait for a credit.
    let (r1, r2, r3) = tokio::join!(
        answer(&instance, "work.callback"),
        answer(&instance, "work.second"),
        answer(&instance, "work.third"),
    );
    let calls = answer(&instance, "scripted.calls").await;
    let methods: Vec<&str> = calls
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|message| message["method"].as_str())
        .collect();
    assert_eq!(
        (r1["result"]["status"].clone(), r2, r3, methods,),
        (
            json!(200),
            json!({ "order": 2 }),
            json!({ "order": 3 }),
            vec!["hello", "work.callback", "work.second", "work.third"],
        )
    );
    instance.stop().await;

    // Also prove concurrency=2 holds call 3 until call 2 releases call 1.
    let gated = Plugin::of_kind_with_concurrency(
        "proxy",
        2,
        &json!({
            "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
            "answers": {
                "work.first": { "hold": "g", "then": { "result": { "order": 1 } } },
                "work.second": { "release": "g", "then": { "result": { "order": 2 } } },
                "work.third": { "result": { "order": 3 } },
            },
        }),
    );
    let gated_instance = PluginInstance::start(&root, gated.instance(), Scope::Task(None));
    let (g1, g2, g3) = tokio::join!(
        answer(&gated_instance, "work.first"),
        answer(&gated_instance, "work.second"),
        answer(&gated_instance, "work.third"),
    );
    let gated_calls = answer(&gated_instance, "scripted.calls").await;
    let gated_methods: Vec<&str> = gated_calls
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|message| message["method"].as_str())
        .collect();
    assert_eq!(
        (g1, g2, g3, gated_methods),
        (
            json!({ "order": 1 }),
            json!({ "order": 2 }),
            json!({ "order": 3 }),
            vec!["hello", "work.first", "work.second", "work.third"],
        )
    );
    gated_instance.stop().await;
    root.stop().await;
}

/// Over Collie's executable with a confined scripted Plugin, an Effect-class
/// call past `timeout.plugin` is abandoned with `$/cancel`, fails with
/// `deadline_exceeded` naming `timeout.plugin`, is reported uncertain at
/// once, releases its credit, and its late reply triggered by `$/cancel` is
/// refused as `late_result`.
///
/// Covers: story:7-33
#[tokio::test]
async fn a_call_past_its_deadline_cancels_refuses_late_result_and_reports_uncertain_over_collie() {
    let (subscriber, log) = capturing(&[("goodfetch_engine::plugin_host", Level::WARN)]);
    let _logging = tracing::subscriber::set_default(subscriber);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let plugin = Plugin::of_kind_with_concurrency(
        "proxy",
        1,
        &json!({
            "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
            "answers": {
                "work.effect": { "answer_on_cancel": { "result": { "late": true } } },
                "work.after": { "result": { "ok": true } },
            },
        }),
    );
    let instance = PluginInstance::start(&root, plugin.instance(), Scope::Task(None));
    // Ensure `hello` completed so the short deadline measures only `work.effect`.
    let _ = answer(&instance, "work.after").await;

    let reporter = Arc::new(RecordedUncertain::default());
    let failure = instance
        .call_under(
            "work.effect",
            &json!({}),
            CallOptions::effect(
                Deadline::given(&TIMEOUT_PLUGIN, 200, SettingsLayer::Default),
                Arc::clone(&reporter) as _,
            ),
        )
        .await;
    let Err(CallFailure::DeadlineExceeded(error)) = failure else {
        panic!("expected DeadlineExceeded, got {failure:?}");
    };
    let uncertain = reporter.0.lock().unwrap().clone();
    assert_eq!(uncertain.len(), 1, "{uncertain:?}");
    assert_eq!(uncertain[0].method, "work.effect");
    assert_eq!(uncertain[0].reason, UncertainReason::Interrupted);

    // Follow-up call proves credit release and drains the Plugin's late reply.
    let after = answer(&instance, "work.after").await;
    let calls = answer(&instance, "scripted.calls").await;
    let cancel = calls
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["method"] == json!("$/cancel"))
        .unwrap_or_else(|| panic!("{calls} holds no $/cancel notification"));
    let written = String::from_utf8(log.written()).unwrap();

    assert_eq!(
        (
            error.code(),
            error.message().contains("timeout.plugin"),
            after,
            cancel["params"]["deadline"].clone(),
            written.contains("WARN") && written.contains("late_result"),
        ),
        (
            "deadline_exceeded",
            true,
            json!({ "ok": true }),
            json!(true),
            true
        ),
        "{written}\n{calls}"
    );
    instance.stop().await;
    root.stop().await;
}

#[cfg(target_os = "linux")]
fn proxy_tcp_plugin() -> Plugin {
    let root = tempfile::tempdir().unwrap();
    let layout = Layout::under(&fs::canonicalize(root.path()).unwrap());
    for folder in [&layout.bundle, &layout.instance] {
        fs::create_dir_all(folder).unwrap();
    }
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedPlugin)
        .unwrap()
        .to_owned();
    fs::copy(scripted, layout.bundle.join("plugin")).unwrap();
    let script = json!({
        "hello": {
            "protocol_version": "0.1",
            "name": "scripted",
            "version": "1.0.0",
            "kinds": [{ "kind": "proxy", "capabilities": [] }],
            "network_level": "proxy_tcp",
        }
    });
    let played = layout.bundle.join("script.json");
    fs::write(&played, script.to_string()).unwrap();
    let manifest = PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": "scripted",
            "version": "1.0.0",
            "kinds": ["proxy"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "proxy_tcp",
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let instance = Instance {
        bundle: layout.bundle.clone(),
        runtime: None,
        folders: vec![layout.instance.clone()],
        env: BTreeMap::new(),
    };
    let policy = Policy::of(&manifest, &instance, &layout.places).unwrap();
    let launch = Launch {
        policy,
        program: layout.bundle.join("plugin"),
        args: vec![played.to_string_lossy().into_owned()],
        folder: layout.instance.clone(),
        proxy_relay: None,
        proxy_loopback: None,
        windows_broker: None,
        reduced_confinement: false,
    };
    Plugin {
        _root: root,
        layout,
        manifest,
        launch,
    }
}

/// A real confined Linux Plugin at `proxy_tcp` (ADR 0033, issue 833) runs
/// inside an isolated network namespace with `plugin-shim`'s loopback relay:
/// it reaches its configured endpoint through `HTTP_PROXY` with its own
/// per-instance credentials, is refused any other destination, and a second
/// instance receives distinct credentials.
///
/// Covers: story:7-55, story:7-61
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_confined_proxy_tcp_plugin_reaches_allowed_destination_through_shim_relay_with_distinct_credentials()
 {
    use std::io::{Read as _, Write as _};

    let host = goodfetch_confinement::linux::host::read();
    if !matches!(
        host.user_namespace,
        goodfetch_confinement::host::UserNamespace::Created
    ) {
        return;
    }

    let allowed = TcpListener::bind("127.0.0.1:0").unwrap();
    let allowed_addr = allowed.local_addr().unwrap();
    let forbidden = TcpListener::bind("127.0.0.1:0").unwrap();
    let forbidden_addr = forbidden.local_addr().unwrap();

    let echo_thread = std::thread::spawn(move || {
        for _ in 0..2 {
            if let Ok((mut sock, _)) = allowed.accept() {
                let mut buf = [0_u8; 32];
                if let Ok(n) = sock.read(&mut buf) {
                    let _ = sock.write_all(&buf[..n]);
                }
            }
        }
    });

    let endpoint_url = format!("http://{allowed_addr}/");
    let user = json!({
        "plugins": { "scripted": { "settings": { "api": endpoint_url } } }
    });
    let plugin1 = proxy_tcp_plugin();
    let plugin2 = proxy_tcp_plugin();
    let root = TaskOwner::root(BlockingPolicy::Inline);
    let inst1 = PluginInstance::start(&root, plugin1.instance_with(&user), Scope::Task(None));
    let inst2 = PluginInstance::start(&root, plugin2.instance_with(&user), Scope::Task(None));

    let ok1: Value = inst1
        .call(
            "scripted.proxy_connect",
            &json!({ "target": allowed_addr.to_string(), "payload": "via-shim-1" }),
        )
        .await
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(ok1["status"], json!(200), "{ok1}");
    assert_eq!(ok1["response"], json!("via-shim-1"), "{ok1}");

    let denied: Value = inst1
        .call(
            "scripted.proxy_connect",
            &json!({ "target": forbidden_addr.to_string(), "payload": "nope" }),
        )
        .await
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(denied["status"], json!(403), "{denied}");

    let ok2: Value = inst2
        .call(
            "scripted.proxy_connect",
            &json!({ "target": allowed_addr.to_string(), "payload": "via-shim-2" }),
        )
        .await
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(ok2["status"], json!(200), "{ok2}");
    assert_eq!(ok2["response"], json!("via-shim-2"), "{ok2}");

    assert_ne!(
        ok1["http_proxy"], ok2["http_proxy"],
        "each confined proxy_tcp instance must receive distinct credentials"
    );

    inst1.stop().await;
    inst2.stop().await;
    root.stop().await;
    let _ = echo_thread.join();
}

#[derive(Debug, Default)]
struct RecordedQuarantines(Mutex<Vec<QuarantineEvent>>);

impl QuarantineNotifier for RecordedQuarantines {
    fn notify(
        &self,
        event: QuarantineEvent,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        self.0.lock().unwrap().push(event);
        Box::pin(async {})
    }
}

#[derive(Debug)]
struct ActiveLease(Enclosing);

impl LeaseCheck for ActiveLease {
    fn check(&self) -> Result<(), LeaseEnded> {
        Ok(())
    }

    fn withheld(&self, ended: LeaseEnded, method: &str) -> goodfetch_engine::CallError {
        goodfetch_engine::CallError::new(
            &CALL_CANCELLED,
            format!("lease {ended}: `{method}` withheld"),
            &(),
        )
    }

    fn enclosing(&self) -> Option<Enclosing> {
        Some(self.0.clone())
    }
}

fn plugin_with_limits(limits: &Value, script: &Value) -> Plugin {
    let root = tempfile::tempdir().unwrap();
    let layout = Layout::under(&fs::canonicalize(root.path()).unwrap());
    for folder in [&layout.bundle, &layout.instance] {
        fs::create_dir_all(folder).unwrap();
    }
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedPlugin)
        .unwrap()
        .to_owned();
    fs::copy(scripted, layout.bundle.join("plugin")).unwrap();
    let played = layout.bundle.join("script.json");
    fs::write(&played, script.to_string()).unwrap();
    let manifest = json!({
        "manifest_version": "0.1",
        "name": "scripted",
        "version": "1.0.0",
        "kinds": ["proxy"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
        "limits": limits,
    });
    let manifest = PluginManifest::read(manifest.to_string().as_bytes()).unwrap();
    let instance = Instance {
        bundle: layout.bundle.clone(),
        runtime: None,
        folders: vec![layout.instance.clone()],
        env: BTreeMap::new(),
    };
    let policy = Policy::of(&manifest, &instance, &layout.places).unwrap();
    let launch = Launch {
        policy,
        program: layout.bundle.join("plugin"),
        args: vec![played.to_string_lossy().into_owned()],
        folder: layout.instance.clone(),
        proxy_relay: None,
        proxy_loopback: None,
        windows_broker: None,
        reduced_confinement: false,
    };
    Plugin {
        _root: root,
        layout,
        manifest,
        launch,
    }
}

/// Over Collie's executable with a confined scripted Plugin on Linux:
/// - a crash fails an in-flight read with retryable `plugin_crashed`, retries
///   reads under their lease after backoff + `hello_ok`, and leaves an
///   unchecked Effect uncertain;
/// - three crashes within 10 minutes quarantine the Plugin, record
///   `plugin_quarantined` in the Audit log, fail later calls with
///   `plugin_quarantined`, and send one Notification.
///
/// Covers: story:7-34, story:7-35
#[cfg(target_os = "linux")]
#[tokio::test]
async fn confined_crashing_plugin_retries_under_lease_and_quarantines_with_audit_and_notification()
{
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let clock = Clock::system();
    let home_dir = tempfile::tempdir().unwrap();
    let home = Home::new(home_dir.path().to_path_buf());
    let audit = AuditLog::of(&home, None, Arc::new(root.child()));
    let notifications = Arc::new(RecordedQuarantines::default());
    let run_id = RunId::parse("quarantine-collie-test").unwrap();

    // 1. Crash once, retry read under lease -> succeeds on restarted process.
    let retry_plugin = plugin_with_limits(
        &json!({ "concurrency": 2 }),
        &json!({
            "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
            "answers": {
                "work.flaky": {
                    "crash_count_file": "flaky.count",
                    "fail_times": 1,
                    "then": { "result": { "recovered": true } },
                },
                "work.crash_effect": { "crash": true },
            },
        }),
    );
    let inst = PluginInstance::start(&root, retry_plugin.instance(), Scope::Run(run_id.clone()));
    let lease_end = clock.now() + std::time::Duration::from_secs(20);
    let lease: Arc<dyn LeaseCheck> = Arc::new(ActiveLease(Enclosing::new(
        &Deadline::given(&TIMEOUT_PLUGIN, 20_000, SettingsLayer::Default),
        EnclosingCall {
            operation: "work.item".to_owned(),
            call_id: RequestId::Number(1),
        },
        move || lease_end,
    )));
    let recovered: Value = inst
        .call_retrying(
            "work.flaky",
            &json!({}),
            CallOptions::read(Deadline::declared(&TIMEOUT_PLUGIN)).with_lease(lease),
        )
        .await
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(recovered, json!({ "recovered": true }));

    // An unchecked Effect on crash is reported uncertain and not retried blindly.
    let reporter = Arc::new(RecordedUncertain::default());
    let effect_err = inst
        .call_retrying(
            "work.crash_effect",
            &json!({}),
            CallOptions::effect(
                Deadline::declared(&TIMEOUT_PLUGIN),
                Arc::clone(&reporter) as _,
            ),
        )
        .await
        .unwrap_err();
    assert!(matches!(effect_err, CallFailure::Unavailable(ref e) if e.code() == "plugin_crashed"));
    assert_eq!(reporter.0.lock().unwrap().len(), 1);
    inst.stop().await;

    // 2. Three crashes -> quarantine, Audit log entry, and exactly 1 Notification.
    let crash_plugin = plugin_with_limits(
        &json!({ "concurrency": 2 }),
        &json!({
            "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
            "answers": { "work.boom": { "crash": true } },
        }),
    );
    let q_inst = PluginInstance::start_with(
        &root,
        crash_plugin.instance(),
        Scope::Run(run_id.clone()),
        InstanceHooks {
            audit: Some(audit.clone()),
            quarantine_notifier: Some(Arc::clone(&notifications) as _),
            ..InstanceHooks::default()
        },
    );
    for _ in 0..3 {
        let err = q_inst.call("work.boom", &json!({})).await.unwrap_err();
        assert!(matches!(err, CallFailure::Unavailable(ref e) if e.code() == "plugin_crashed"));
    }
    let after = q_inst.call("work.boom", &json!({})).await.unwrap_err();
    assert!(matches!(after, CallFailure::Unavailable(ref e) if e.code() == "plugin_quarantined"));
    assert_eq!(notifications.0.lock().unwrap().len(), 1);

    let exported = audit.export().unwrap();
    assert!(
        exported
            .iter()
            .any(|e| e.entry.kind == PLUGIN_QUARANTINED_AUDIT.name
                && e.entry.details["plugin"] == json!("scripted")),
        "missing `plugin_quarantined` Audit log entry: {exported:?}"
    );
    q_inst.stop().await;
    root.stop().await;
}

/// Real Linux confined Plugins exceeding `memory`, `cpu_time` and `open_files`
/// are killed with non-retryable `plugin_limit_exceeded` under the crash rules.
///
/// Covers: story:7-37, error:plugin_limit_exceeded
#[cfg(target_os = "linux")]
#[tokio::test]
async fn confined_linux_plugin_resource_limits_kill_with_plugin_limit_exceeded() {
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let cases = [
        (
            "memory",
            json!({ "memory": 32 * 1024 * 1024_u64, "cpu_time": 30, "open_files": 64 }),
        ),
        (
            "cpu_time",
            json!({ "memory": 128 * 1024 * 1024_u64, "cpu_time": 1, "open_files": 64 }),
        ),
        (
            "open_files",
            json!({ "memory": 128 * 1024 * 1024_u64, "cpu_time": 30, "open_files": 16 }),
        ),
    ];

    for (resource, limits) in cases {
        let plugin = plugin_with_limits(
            &limits,
            &json!({
                "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
                "answers": { "work.exceed": { "exceed": resource } },
            }),
        );
        let inst = PluginInstance::start(&root, plugin.instance(), Scope::Task(None));
        let failure = inst.call("work.exceed", &json!({})).await.unwrap_err();
        let CallFailure::Unavailable(error) = failure else {
            panic!("expected Unavailable(plugin_limit_exceeded) for `{resource}`, got {failure:?}");
        };
        assert_eq!(
            error.code(),
            "plugin_limit_exceeded",
            "resource={resource}: {error:?}"
        );
        assert!(
            !error.retryable(),
            "resource={resource} must not be retryable"
        );
        assert_eq!(
            error.details().map(|d| d["resource"].clone()),
            Some(json!(resource)),
            "resource={resource}: {error:?}"
        );
        inst.stop().await;
    }

    root.stop().await;
}

#[cfg(target_os = "linux")]
fn write_stand_in_proc(scratch: &Path) -> std::path::PathBuf {
    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;
    let proc_dir = scratch.join("proc");
    let cgroup_dir = scratch.join("cgroup");
    let write_file = |path: &Path, text: &str| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let rollup = |pss: u64| format!("Rss: {} kB\nPss: {} kB\n", pss / 1024 * 2, pss / 1024);
    write_file(
        &proc_dir.join("meminfo"),
        &format!(
            "MemTotal: {} kB\nMemAvailable: {} kB\n",
            8 * GIB / 1024,
            6 * GIB / 1024
        ),
    );
    write_file(
        &proc_dir.join("pressure/memory"),
        "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
    );
    write_file(&proc_dir.join("self/smaps_rollup"), &rollup(64 * MIB));
    write_file(
        &proc_dir.join("self/stat"),
        "1 (engine) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0\n",
    );
    write_file(&proc_dir.join("self/cgroup"), "0::/pod.scope\n");
    write_file(
        &proc_dir.join("self/mountinfo"),
        &format!(
            "48 22 0:30 / {} rw,relatime - cgroup2 cgroup2 rw\n",
            cgroup_dir.display()
        ),
    );
    write_file(&cgroup_dir.join("memory.max"), "max\n");
    write_file(
        &cgroup_dir.join("pod.scope/memory.max"),
        &format!("{}\n", 4 * GIB),
    );
    proc_dir
}

#[cfg(target_os = "linux")]
fn first_child_pid() -> u32 {
    fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|e| fs::read_to_string(e.path().join("children")).ok())
        .flat_map(|s| {
            s.split_whitespace()
                .filter_map(|p| p.parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .next()
        .expect("expected live confined Plugin child PID")
}

#[cfg(target_os = "linux")]
fn write_proc_footprint(proc_dir: &Path, pid: u32, pss_bytes: u64) {
    let path = proc_dir.join(format!("{pid}/smaps_rollup"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            "Rss: {} kB\nPss: {} kB\n",
            pss_bytes / 1024 * 2,
            pss_bytes / 1024
        ),
    )
    .unwrap();
}

/// Governor footprint sampling enforces a Plugin's manifest `memory` limit:
/// a footprint under the limit leaves the instance untouched across samples,
/// and a footprint past the limit kills it with non-retryable
/// `plugin_limit_exceeded` on the governor's sample without any per-instance
/// sampling task or timer (ADRs 0033 and 0040, issue 867).
///
/// Covers: story:7-22, error:plugin_limit_exceeded
#[cfg(target_os = "linux")]
#[tokio::test]
async fn governor_footprint_sampling_enforces_memory_limit_and_leaves_under_limit_untouched() {
    use goodfetch_engine::governor::{Governor, System};
    use goodfetch_engine::metrics::Metrics;
    use goodfetch_engine::settings::{Settings, SettingsFiles};
    use goodfetch_protocol::connection::ProductVersion;

    const MIB: u64 = 1 << 20;
    let memory_limit = 100 * MIB;
    let scratch = tempfile::tempdir().unwrap();
    let proc_dir = write_stand_in_proc(scratch.path());

    let home = Home::new(scratch.path().join("home"));
    let files = SettingsFiles::of(&home, &ProductVersion::parse("0.0.0").unwrap());
    fs::create_dir_all(files.user_configuration().parent().unwrap()).unwrap();
    fs::write(
        files.user_configuration(),
        "version = \"0.1\"\n\n[settings]\ngovernor.sample_interval = 25\n",
    )
    .unwrap();

    let root = TaskOwner::root(BlockingPolicy::Threads);
    let settings = Settings::new(files, Product::Collie, root.child());
    let governor = Governor::start(
        System::linux_files(proc_dir.clone()),
        settings,
        Clock::system(),
        &root,
        Metrics::none().counted_work().clone(),
    );
    let mut info = governor.system_info();
    while info.borrow_and_update().is_none() {
        info.changed().await.unwrap();
    }

    let gate = scratch.path().join("hold.gate");
    let plugin = plugin_with_limits(
        &json!({ "memory": memory_limit, "cpu_time": 30, "open_files": 64 }),
        &json!({
            "hello": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
            "answers": {
                "work.ping": { "result": { "ok": true } },
                "work.hold": { "hold": gate.to_string_lossy() },
            },
        }),
    );
    let inst = PluginInstance::start_with(
        &root,
        plugin.instance(),
        Scope::Task(None),
        InstanceHooks {
            governor: Some(governor.clone()),
            ..InstanceHooks::default()
        },
    );

    let ping: Value = inst
        .call("work.ping", &json!({}))
        .await
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(ping, json!({ "ok": true }));

    let child_pid = first_child_pid();
    let children_path = proc_dir.join("self/task/1/children");
    fs::create_dir_all(children_path.parent().unwrap()).unwrap();
    fs::write(&children_path, format!("{child_pid}\n")).unwrap();
    write_proc_footprint(&proc_dir, child_pid, 50 * MIB);
    info.changed().await.unwrap();
    info.changed().await.unwrap();
    assert_eq!(
        inst.call("work.ping", &json!({}))
            .await
            .unwrap()
            .read::<Value>()
            .unwrap(),
        json!({ "ok": true })
    );

    let held_inst = inst.clone();
    let held_call = tokio::spawn(async move { held_inst.call("work.hold", &json!({})).await });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    write_proc_footprint(&proc_dir, child_pid, 150 * MIB);

    let CallFailure::Unavailable(error) = held_call.await.unwrap().unwrap_err() else {
        panic!("expected Unavailable(plugin_limit_exceeded)");
    };
    assert_eq!(error.code(), "plugin_limit_exceeded");
    assert!(!error.retryable());
    assert_eq!(
        error.details().map(|d| d["resource"].clone()),
        Some(json!("memory"))
    );
    assert_eq!(
        error.details().map(|d| d["limit"].clone()),
        Some(json!(memory_limit))
    );

    inst.stop().await;
    root.stop().await;
}

/// When a confined launcher reports `BLOCKED_BY_SMART_APP_CONTROL_STATUS`
/// (Windows Smart App Control blocking a Plugin executable or cached
/// interpreter), `PluginInstance` fails immediately without crash-retrying,
/// returning non-retryable `executable_blocked_by_smart_app_control` naming
/// the blocked file path (ADR 0039, issue 873).
///
/// Covers: story:7-105, error:executable_blocked_by_smart_app_control
#[cfg(unix)]
#[tokio::test]
async fn smart_app_control_block_from_launcher_fails_with_named_file_and_is_not_retryable() {
    use std::os::unix::fs::PermissionsExt as _;

    use goodfetch_confinement::launch::BLOCKED_BY_SMART_APP_CONTROL_STATUS;

    let scratch = tempfile::tempdir().unwrap();
    let stub_launcher = scratch.path().join("sac-launcher.sh");
    fs::write(
        &stub_launcher,
        format!("#!/bin/sh\nexit {BLOCKED_BY_SMART_APP_CONTROL_STATUS}\n"),
    )
    .unwrap();
    fs::set_permissions(&stub_launcher, fs::Permissions::from_mode(0o755)).unwrap();

    let plugin = Plugin::approved();
    let mut start = plugin.instance();
    start.launcher = stub_launcher;
    let expected_file = start.launch.program.to_string_lossy().into_owned();

    let root = TaskOwner::root(BlockingPolicy::Threads);
    let inst = PluginInstance::start(&root, start, Scope::Task(None));
    let CallFailure::Unavailable(error) = inst.call("work.ping", &json!({})).await.unwrap_err()
    else {
        panic!("expected Unavailable(executable_blocked_by_smart_app_control)");
    };
    assert_eq!(error.code(), "executable_blocked_by_smart_app_control");
    assert!(!error.retryable());
    assert_eq!(
        error.details().map(|d| d["file"].clone()),
        Some(json!(expected_file))
    );

    inst.stop().await;
    root.stop().await;
}

/// When a confined launcher exits with any Windows Job Object limit status
/// (`memory`, `cpu_time`, `processes`, `open_files`), `PluginInstance` fails
/// with non-retryable `plugin_limit_exceeded` naming that resource (ADR 0033,
/// issue 873).
///
/// Covers: story:7-105, error:plugin_limit_exceeded
#[cfg(unix)]
#[tokio::test]
async fn job_object_limit_exit_statuses_end_in_plugin_limit_exceeded() {
    use std::os::unix::fs::PermissionsExt as _;

    use goodfetch_confinement::launch::LaunchError;
    use goodfetch_protocol::plugin::PluginResourceLimit;

    let scratch = tempfile::tempdir().unwrap();
    let root = TaskOwner::root(BlockingPolicy::Threads);
    for (limit, expected_name) in [
        (PluginResourceLimit::Memory, "memory"),
        (PluginResourceLimit::CpuTime, "cpu_time"),
        (PluginResourceLimit::Processes, "processes"),
        (PluginResourceLimit::OpenFiles, "open_files"),
    ] {
        let code = LaunchError::LimitExceeded(limit).status();
        let hello_reply = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "result": hello(&json!([{ "kind": "proxy", "capabilities": [] }])),
        });
        let stub = scratch.path().join(format!("limit-{expected_name}.sh"));
        fs::write(
            &stub,
            format!(
                "#!/bin/sh\nread -r _hello\nprintf '%s\\n' '{hello_reply}'\nread -r _call\nexit {code}\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();

        let plugin = Plugin::approved();
        let mut start = plugin.instance();
        start.launcher = stub;
        let inst = PluginInstance::start(&root, start, Scope::Task(None));
        let CallFailure::Unavailable(error) = inst.call("work.ping", &json!({})).await.unwrap_err()
        else {
            panic!("expected Unavailable(plugin_limit_exceeded) for {expected_name}");
        };
        assert_eq!(error.code(), "plugin_limit_exceeded", "{expected_name}");
        assert!(!error.retryable(), "{expected_name}");
        assert_eq!(
            error.details().map(|d| d["resource"].clone()),
            Some(json!(expected_name)),
            "{expected_name}"
        );
        inst.stop().await;
    }
    root.stop().await;
}
