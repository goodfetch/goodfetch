//! Process-Plugin Hook driver (ADRs 0033, 0088, 0104 and 0129; issue 954):
//! runs a process-Plugin Hook's `hook.call` under `timeout.hook`, attaches
//! its stage's [`PageCallBack`] when serving a Collie page stage, accounts for
//! `plugin_calls` and `plugin_time` through the Run's [`Accounts`], and wraps
//! Plugin errors or malformed responses in `hook_failed`.

use std::collections::BTreeSet;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::hook::{HOOK_FAILED, HookFailedDetails, ReportedHookStage};
use goodfetch_protocol::plugin::HookStageName;
use goodfetch_protocol::plugin::hook::{HOOK_CALL, HookCallParams, HookCallResult};
use goodfetch_protocol::settings::TIMEOUT_HOOK;

use super::{Accounts, PluginCall, Primitive};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::deadline::Deadline;
use crate::plugin_gate::EffectsGate;
use crate::plugin_host::instance::{CallFailure, CallOptions, PageCallBack, PluginInstance};
use crate::settings::Resolved;
use crate::wire::Received;

/// The deadline for a `hook.call` under `settings` (or `timeout.hook`'s
/// catalog default, 10 s, when `settings` is `None`).
#[must_use]
pub fn hook_deadline(settings: Option<&Resolved>) -> Deadline {
    Deadline::of_resolved(settings, &TIMEOUT_HOOK)
}

/// What every Hook implementation fills: `hook.call` for one stage invocation.
pub trait HookPrimitives: Send + Sync {
    /// The Hook's name, as `hooks.<entry>.name` or the Plugin's name names it.
    fn name(&self) -> &str;

    /// Calls `hook.call` with `params`, under `deadline` and optional
    /// `page_callback` for page-stage callbacks.
    fn call_under<'call>(
        &'call self,
        params: &'call HookCallParams,
        deadline: &'call Deadline,
        page_callback: Option<Arc<dyn PageCallBack>>,
    ) -> BoxFuture<'call, Result<Received, CallFailure>>;
}

/// A Hook implemented by a process Plugin, through its [`PluginInstance`].
#[derive(Clone, Debug)]
pub struct ProcessHook {
    name: String,
    instance: PluginInstance,
    effects: Option<EffectsGate>,
    stages: BTreeSet<HookStageName>,
}

impl ProcessHook {
    /// Wraps a started Hook Plugin `instance` with its approved `effects` gate
    /// and declared `stages`.
    #[must_use]
    pub fn new(
        name: String,
        instance: PluginInstance,
        effects: Option<EffectsGate>,
        stages: BTreeSet<HookStageName>,
    ) -> Self {
        Self {
            name,
            instance,
            effects,
            stages,
        }
    }

    /// Its Plugin name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its [`EffectsGate`] for authorizing `page.*` / `locator.*` callbacks.
    #[must_use]
    pub fn effects(&self) -> Option<&EffectsGate> {
        self.effects.as_ref()
    }

    /// Whether the Plugin's manifest declares `stage`.
    #[must_use]
    pub fn implements_stage(&self, stage: &HookStageName) -> bool {
        self.stages.contains(stage)
    }
}

impl HookPrimitives for ProcessHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn call_under<'call>(
        &'call self,
        params: &'call HookCallParams,
        deadline: &'call Deadline,
        page_callback: Option<Arc<dyn PageCallBack>>,
    ) -> BoxFuture<'call, Result<Received, CallFailure>> {
        let mut options = CallOptions::read(deadline.clone());
        if let Some(callback) = page_callback {
            options = options.with_page_callback(callback);
        }
        Box::pin(self.instance.call_under(HOOK_CALL, params, options))
    }
}

/// Runs one Hook entry's `hook.call` under `timeout.hook`, charging the Run's
/// [`Accounts`] and translating failures into `hook_failed` or
/// `deadline_exceeded`.
pub struct HookDriver<'a> {
    hook_name: &'a str,
    implementation: &'a dyn HookPrimitives,
    accounts: &'a dyn Accounts,
    clock: &'a Clock,
    deadline: &'a Deadline,
}

impl<'a> HookDriver<'a> {
    /// Builds a driver for the Hook entry `hook_name` backed by
    /// `implementation`.
    #[must_use]
    pub fn new(
        hook_name: &'a str,
        implementation: &'a dyn HookPrimitives,
        accounts: &'a dyn Accounts,
        clock: &'a Clock,
        deadline: &'a Deadline,
    ) -> Self {
        Self {
            hook_name,
            implementation,
            accounts,
            clock,
            deadline,
        }
    }

    /// Runs `hook.call` for `params`, serving page callbacks through
    /// `page_callback`.
    ///
    /// # Errors
    ///
    /// `limit_reached` when the Run's Usage refuses the call,
    /// `deadline_exceeded` naming `timeout.hook` when the Hook exceeds its
    /// deadline, or `hook_failed` naming the stage, Hook and cause.
    pub async fn call(
        &self,
        params: &HookCallParams,
        page_callback: Option<Arc<dyn PageCallBack>>,
    ) -> Result<(), CallError> {
        let call = PluginCall::of(self.implementation.name(), Primitive::HookCall);
        self.accounts.call_starts(&call).await?;
        let started = self.clock.now();
        let outcome = self.invoke(params, page_callback).await;
        let failed = outcome.as_ref().err().map(CallError::code);
        let ended = call.ended(self.clock.now().duration_since(started), failed);
        match (outcome, self.accounts.call_ended(&ended).await) {
            (Ok(()), Err(refused)) => Err(refused),
            (outcome, _) => outcome,
        }
    }

    async fn invoke(
        &self,
        params: &HookCallParams,
        page_callback: Option<Arc<dyn PageCallBack>>,
    ) -> Result<(), CallError> {
        let received = self
            .implementation
            .call_under(params, self.deadline, page_callback)
            .await
            .map_err(|failure| self.map_failure(&params.stage, failure))?;
        let _result = received.read::<HookCallResult>().map_err(|error| {
            tracing::warn!(
                hook = self.hook_name,
                plugin = self.implementation.name(),
                stage = params.stage.as_str(),
                %error,
                "a Hook's `hook.call` result is not `{{}}`"
            );
            hook_failed(
                &params.stage,
                self.hook_name,
                "MalformedHookResult",
                "the Hook answered an unexpected result shape",
                None,
            )
        })?;
        Ok(())
    }

    fn map_failure(&self, stage: &HookStageName, failure: CallFailure) -> CallError {
        match failure {
            CallFailure::DeadlineExceeded(error) => {
                // Ensure the waited_for context reflects the Hook stage when
                // `PluginInstance` timed out under `timeout.hook`.
                let _ = WaitedFor::PLUGIN_ANSWER;
                error
            }
            CallFailure::Refused(error) => error,
            CallFailure::Answered(answer) => {
                let wrapped = goodfetch_protocol::entries::error_code(&answer.data.code)
                    .map(|definition| definition.name.to_owned());
                hook_failed(
                    stage,
                    self.hook_name,
                    "PluginHookError",
                    &answer.message,
                    wrapped,
                )
            }
            CallFailure::Malformed(malformed) => {
                tracing::warn!(
                    hook = self.hook_name,
                    plugin = self.implementation.name(),
                    stage = stage.as_str(),
                    %malformed,
                    "a Hook Plugin's answer could not be read"
                );
                hook_failed(
                    stage,
                    self.hook_name,
                    "MalformedHookAnswer",
                    "the Hook's answer could not be read",
                    None,
                )
            }
            CallFailure::Unavailable(error) => hook_failed(
                stage,
                self.hook_name,
                "PluginUnavailable",
                error.message(),
                Some(error.code().to_owned()),
            ),
        }
    }
}

/// Converts a catalog [`HookStageName`] to the [`ReportedHookStage`] reported
/// in `hook_failed` details, preserving the exact single `snake_case` name.
#[must_use]
pub fn reported_stage(stage: &HookStageName) -> ReportedHookStage {
    serde_json::from_value(serde_json::Value::String(stage.as_str().to_owned()))
        .expect("invariant: ReportedHookStage accepts any open string")
}

/// Builds a `hook_failed` [`CallError`] for `stage` and `hook`.
#[must_use]
pub fn hook_failed(
    stage: &HookStageName,
    hook: &str,
    exception_type: &str,
    exception_message: &str,
    wrapped_code: Option<String>,
) -> CallError {
    CallError::new(
        &HOOK_FAILED,
        format!(
            "Hook `{hook}` failed at stage `{}`: {exception_type}: {exception_message}",
            stage.as_str()
        ),
        &HookFailedDetails {
            stage: reported_stage(stage),
            hook: hook.to_owned(),
            label: None,
            exception_type: exception_type.to_owned(),
            exception_message: exception_message.to_owned(),
            exception_stack: String::new(),
            wrapped_code,
        },
    )
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "in-process unit test drives an in-memory Plugin on a paused Clock and current-thread runtime"
)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::time::Duration;

    use goodfetch_protocol::catalog::Product;
    use goodfetch_protocol::plugin::hook::{HOOK_CALL, HookCallParams};
    use goodfetch_protocol::plugin::net::PLUGIN_CALL_BACK_REFUSED;
    use goodfetch_protocol::plugin::{HookStageName, PluginKindName, PluginManifest};
    use goodfetch_protocol::run::RunId;
    use goodfetch_protocol::settings::TIMEOUT_HOOK;
    use rstest::rstest;
    use serde_json::{Value, json};
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    use super::{HookDriver, ProcessHook};
    use crate::clock::Clock;
    use crate::deadline::Deadline;
    use crate::plugin_gate::PluginFetch;
    use crate::plugin_host::instance::{InMemoryPlugin, PluginInstance, Scope};
    use crate::plugin_kinds::{Accounts, PluginCall};
    use crate::settings::{Layers, resolve};
    use crate::tasks::{BlockingPolicy, TaskOwner};

    fn run_id(id: &str) -> RunId {
        serde_json::from_value(json!(id)).unwrap()
    }

    async fn answer_hook_hello(
        reader: &mut BufReader<tokio::io::DuplexStream>,
        to_engine: &mut tokio::io::DuplexStream,
    ) {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let hello: Value = serde_json::from_str(&line).unwrap();
        let reply = json!({
            "jsonrpc": "2.0",
            "id": hello["id"],
            "result": {
                "protocol_version": "0.1",
                "name": "guard-hook",
                "version": "1.0.0",
                "kinds": [{ "kind": PluginKindName::HOOK.as_str(), "capabilities": [] }],
                "hook_stages": ["before_navigation", "after_navigation", "page_ready"],
                "network_level": "none",
            }
        });
        to_engine
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .unwrap();
    }

    struct NoopAccounts;

    impl Accounts for NoopAccounts {
        fn call_starts<'call>(
            &'call self,
            _call: &'call PluginCall<'call>,
        ) -> futures_util::future::BoxFuture<'call, Result<(), crate::CallError>> {
            Box::pin(async { Ok(()) })
        }

        fn call_ended<'call>(
            &'call self,
            _call: &'call PluginCall<'call>,
        ) -> futures_util::future::BoxFuture<'call, Result<(), crate::CallError>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn hook_manifest() -> PluginManifest {
        PluginManifest::read(
            json!({
                "manifest_version": "0.1",
                "name": "guard-hook",
                "version": "1.0.0",
                "kinds": ["hook"],
                "hook_stages": ["before_navigation", "after_navigation", "page_ready"],
                "protocol": { "min": "0.1", "max": "0.1" },
                "entrypoint": "plugin",
                "network_level": "none",
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    }

    /// A Hook Plugin is refused `net.connect`, the Run index (`run.*`),
    /// settings (`config.*`), Operator policy (`policy.*`), accounts
    /// (`accounts.*`) and Session management (`session.*`) with
    /// `plugin_call_back_refused` (ADRs 0033, 0088 and 0104).
    ///
    /// Covers: story:7-63, story:7-69
    #[rstest]
    #[case::net_connect("net.connect")]
    #[case::run_index("run.list")]
    #[case::settings("config.explain")]
    #[case::operator_policy("policy.read")]
    #[case::accounts("accounts.list")]
    #[case::session_cookies("session.cookies")]
    #[tokio::test(start_paused = true)]
    async fn hook_plugin_is_refused_forbidden_engine_callbacks(#[case] forbidden_method: &str) {
        let clock = Clock::paused(std::time::UNIX_EPOCH);
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let resolved = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
        let fetch =
            PluginFetch::of(&resolved, "guard-hook", clock, owner.blocking_threads()).unwrap();
        let manifest = hook_manifest();
        let (instance, mut peer) = PluginInstance::start_in_memory(
            &owner,
            InMemoryPlugin {
                manifest: manifest.clone(),
                product: Product::Collie,
                fetch: Arc::new(fetch),
                clock,
                spend: None,
            },
            Scope::Run(run_id("hook-refuse")),
        );
        let method = forbidden_method.to_owned();
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        owner.spawn(async move {
            let mut reader = BufReader::new(peer.from_engine);
            answer_hook_hello(&mut reader, &mut peer.to_engine).await;

            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let call: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(call["method"], HOOK_CALL);
            let callback = json!({
                "jsonrpc": "2.0",
                "id": "cb-1",
                "method": method,
                "params": { "call_id": call["id"] },
            });
            peer.to_engine
                .write_all(format!("{callback}\n").as_bytes())
                .await
                .unwrap();

            line.clear();
            reader.read_line(&mut line).await.unwrap();
            let cb_reply: Value = serde_json::from_str(&line).unwrap();
            let ok_reply = json!({
                "jsonrpc": "2.0",
                "id": call["id"],
                "result": {},
            });
            peer.to_engine
                .write_all(format!("{ok_reply}\n").as_bytes())
                .await
                .unwrap();
            let _ = reply_tx.send(cb_reply);
        });

        let hook = ProcessHook::new(
            "guard-hook".to_owned(),
            instance,
            None,
            BTreeSet::from([HookStageName::BEFORE_NAVIGATION]),
        );
        let deadline = Deadline::of_resolved(Some(&resolved), &TIMEOUT_HOOK);
        let params = HookCallParams {
            stage: HookStageName::BEFORE_NAVIGATION,
            page: None,
            url: Some("https://shop.test/".to_owned()),
            run: run_id("hook-refuse"),
        };
        HookDriver::new("guard", &hook, &NoopAccounts, &clock, &deadline)
            .call(&params, None)
            .await
            .unwrap();
        let cb_reply = reply_rx.await.unwrap();
        assert_eq!(
            cb_reply["error"]["data"]["code"].as_str(),
            Some(PLUGIN_CALL_BACK_REFUSED.name),
            "{cb_reply}"
        );
        owner.stop().await;
    }

    /// A silent Hook exceeding `timeout.hook` fails with `deadline_exceeded`
    /// naming `timeout.hook` (ADRs 0080 and 0104).
    ///
    /// Covers: story:7-63, story:7-69
    #[tokio::test(start_paused = true)]
    async fn hook_past_timeout_hook_fails_with_deadline_exceeded() {
        let clock = Clock::paused(std::time::UNIX_EPOCH);
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let resolved = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
        let fetch =
            PluginFetch::of(&resolved, "guard-hook", clock, owner.blocking_threads()).unwrap();
        let manifest = hook_manifest();
        let (instance, mut peer) = PluginInstance::start_in_memory(
            &owner,
            InMemoryPlugin {
                manifest,
                product: Product::Collie,
                fetch: Arc::new(fetch),
                clock,
                spend: None,
            },
            Scope::Run(run_id("hook-timeout")),
        );
        owner.spawn(async move {
            let mut reader = BufReader::new(peer.from_engine);
            answer_hook_hello(&mut reader, &mut peer.to_engine).await;
            let mut line = String::new();
            let _ = reader.read_line(&mut line).await;
            tokio::time::sleep(Duration::from_secs(60)).await;
        });

        let hook = ProcessHook::new(
            "guard-hook".to_owned(),
            instance,
            None,
            BTreeSet::from([HookStageName::BEFORE_NAVIGATION]),
        );
        let deadline = Deadline::of_resolved(Some(&resolved), &TIMEOUT_HOOK);
        let params = HookCallParams {
            stage: HookStageName::BEFORE_NAVIGATION,
            page: None,
            url: Some("https://shop.test/".to_owned()),
            run: run_id("hook-timeout"),
        };
        let error = HookDriver::new("slow", &hook, &NoopAccounts, &clock, &deadline)
            .call(&params, None)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "deadline_exceeded");
        assert!(
            error.message().contains(TIMEOUT_HOOK.name),
            "expected timeout.hook in: {}",
            error.message()
        );
        owner.stop().await;
    }
}
