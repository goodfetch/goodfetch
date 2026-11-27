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
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::plugin_gate::PluginFetch;
use goodfetch_engine::plugin_host::instance::{
    self, CallFailure, CallOptions, PluginInstance, Scope, UncertainCall, UncertainReporter,
};
use goodfetch_engine::plugin_host::process::{Pipes, PluginProcess};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, OwnerKind, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::effect::UncertainReason;
use goodfetch_protocol::plugin::{COOKIE, COOKIE_VARIABLE, PluginManifest};
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
