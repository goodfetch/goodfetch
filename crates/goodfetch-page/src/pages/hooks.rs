//! Process-Plugin Hooks around Collie main-frame navigations (ADRs 0033, 0088
//! and 0104; issue 954): runs `hooks` entries for `before_navigation`,
//! `after_navigation` and `page_ready` in resolved layer order (Operator policy
//! first, run layer last), with `{ page, url, run }` context and zero work when
//! `hooks` is empty so `C4.detection_reads` and `C4.compilations` stay at their
//! register figures.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use goodfetch_engine::CallError;
use goodfetch_engine::plugin_gate::EffectsGate;
use goodfetch_engine::plugin_host::instance::PageCallBack;
use goodfetch_engine::plugin_kinds::hook::{HookDriver, hook_deadline, hook_failed};
use goodfetch_engine::plugin_kinds::run_plugins::StartError;
use goodfetch_engine::run::accounts::RunAccounts;
use goodfetch_engine::secrets::secret_error;
use goodfetch_engine::settings::{Resolved, RunSettings};
use goodfetch_protocol::connection::Form;
use goodfetch_protocol::effect::UncertainReason;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::functions::{EvaluateParams, World};
use goodfetch_protocol::plugin::HookStageName;
use goodfetch_protocol::plugin::hook::{HOOKS, HookCallParams, HookEntry};
use goodfetch_protocol::plugin::net::PLUGIN_CALL_BACK_REFUSED;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer,
};
use serde_json::{Value, json};
use url::Url;

use crate::cdp::CdpPage;
use crate::page_functions;
use crate::page_reads;
use crate::sessions::{OpenPage, PageEvent};
use crate::trace;

/// Fast-path check: whether `resolved` declares at least one Hook entry.
/// When `false`, navigation performs zero Hook allocations, lookups or calls.
#[must_use]
pub(super) fn has_any(resolved: Option<&Resolved>) -> bool {
    resolved
        .and_then(|resolved| resolved.effective(&HOOKS))
        .and_then(|effective| effective.value.as_array())
        .is_some_and(|entries| !entries.is_empty())
}

/// Returns the resolved `hooks` entries scoped to `stage`, in layer order.
fn entries_for_stage(resolved: Option<&Resolved>, stage: &HookStageName) -> Vec<HookEntry> {
    let Some(entries) = resolved
        .and_then(|resolved| resolved.effective(&HOOKS))
        .and_then(|effective| effective.value.as_array())
    else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|raw| HookEntry::read(raw).ok())
        .filter(|entry| entry.stage == *stage)
        .collect()
}

/// Runs every settings-named Hook registered for `stage` around a main-frame
/// navigation of `page_id` in layer order (`hook_dispatch`, ADR 0104).
///
/// # Errors
///
/// Fails at the first Hook that refuses to start, times out past
/// `timeout.hook` (`deadline_exceeded`), or raises (`hook_failed`).
pub(super) async fn dispatch(
    page_id: &PageId,
    page: &OpenPage,
    stage: HookStageName,
    url: Option<&str>,
) -> Result<(), CallError> {
    let settings = page.run.identity.settings().map(RunSettings::resolved);
    let entries = entries_for_stage(settings, &stage);
    if entries.is_empty() {
        return Ok(());
    }
    let clock = page.run.identity.clock();
    let deadline = hook_deadline(settings);
    let accounts = RunAccounts {
        identity: &page.run.identity,
        charging: &page.run.charging,
        trace: &page.run.trace,
    };
    let current_url = url
        .and_then(|raw| Url::parse(raw).ok())
        .unwrap_or_else(|| Url::parse("about:blank").expect("invariant: valid about:blank URL"));
    for entry in entries {
        let Some(plugin_name) = entry.plugin.as_deref() else {
            return Err(CallError::new(
                &CAPABILITY_UNAVAILABLE,
                format!(
                    "Hook `{}` at stage `{}` uses a Script Hook, which this engine does not build yet (`script_hooks`)",
                    entry.name,
                    stage.as_str()
                ),
                &CapabilityUnavailableDetails {
                    key: HOOKS.name.to_owned(),
                    layer: SettingsLayer::Run,
                    kind: "script_hooks".to_owned(),
                    fix: "name a process Plugin with `plugin` instead of `script`".to_owned(),
                },
            ));
        };
        let hook = page
            .run
            .plugins
            .hook(
                plugin_name,
                &entry.name,
                &stage,
                settings,
                page.run.keys.secrets(),
                page.run.engine.sandbox_probe(),
            )
            .await
            .map_err(|error| match error {
                StartError::Refused(error) => error,
                StartError::Secret(error) => secret_error(&error),
            })?;
        let callback: Arc<dyn PageCallBack> = Arc::new(HookPageCallBack {
            page_id: page_id.clone(),
            page: page.clone(),
            stage,
            current_url: current_url.clone(),
            effects: hook.effects().cloned(),
            logged_in: page.page_check.holds_login,
        });
        let params = HookCallParams {
            stage,
            page: Some(page_id.clone()),
            url: url.map(str::to_owned),
            run: page.run.identity.id().clone(),
        };
        let outcome = HookDriver::new(&entry.name, &hook, &accounts, &clock, &deadline)
            .call(&params, Some(callback))
            .await;
        trace::provenance(
            &page.run.trace,
            json!({
                "action": "hook_call",
                "hook": entry.name,
                "plugin": plugin_name,
                "stage": stage.as_str(),
                "page_id": page_id.0,
                "outcome": if outcome.is_ok() { "ok" } else { "failed" },
            }),
        )
        .await;
        outcome.map_err(|error| {
            if error.code() == "deadline_exceeded"
                || error.code() == "limit_reached"
                || error.code() == "hook_failed"
            {
                error
            } else {
                hook_failed(
                    &stage,
                    &entry.name,
                    "HookError",
                    error.message(),
                    Some(error.code().to_owned()),
                )
            }
        })?;
    }
    Ok(())
}

/// Executes `page.*` / `locator.*` callbacks from one in-flight Hook call on
/// its active page, holding every call to page scope, logged-in Session
/// request protection, and `EffectsGate`.
struct HookPageCallBack {
    page_id: PageId,
    page: OpenPage,
    stage: HookStageName,
    current_url: Url,
    effects: Option<EffectsGate>,
    logged_in: bool,
}

impl std::fmt::Debug for HookPageCallBack {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HookPageCallBack")
            .field("page_id", &self.page_id.0)
            .field("stage", &self.stage.as_str())
            .finish_non_exhaustive()
    }
}

impl PageCallBack for HookPageCallBack {
    fn call<'a>(
        &'a self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<Value, CallError>> {
        Box::pin(async move {
            if let Some(asked_page) = params.get("page_id").and_then(Value::as_str)
                && asked_page != self.page_id.0
            {
                return Err(call_back_refused(&format!(
                    "a Hook on page `{}` cannot act on another page `{asked_page}`",
                    self.page_id.0
                )));
            }
            if self.logged_in && is_request_mutating(method) {
                return Err(call_back_refused(&format!(
                    "a Hook cannot call `{method}` to change a logged-in Session's requests"
                )));
            }
            let effect_entry = if let Some(gate) = &self.effects {
                if is_hook_safe_page_op(method) {
                    None
                } else {
                    gate.authorize_page_call(
                        method,
                        Some(self.stage),
                        self.logged_in,
                        &self.current_url,
                        None,
                    )
                    .await?
                }
            } else {
                None
            };
            let result = self.execute(method, params).await;
            if let Some(entry) = effect_entry {
                match &result {
                    Ok(_) => {
                        let _ = entry.done().await;
                    }
                    Err(_) => {
                        entry.uncertain(UncertainReason::FailedAfterSend).await;
                    }
                }
            }
            result
        })
    }
}

impl HookPageCallBack {
    async fn execute(&self, method: &str, params: Value) -> Result<Value, CallError> {
        match method {
            "page.set_cookie" => self.set_cookie(params).await,
            "page.block_url" => self.block_url(&params),
            "page.evaluate" => self.evaluate(params).await,
            "page.body" => {
                let document = page_reads::document(&CdpPage::of(&self.page.driver)).await?;
                Ok(json!({ "url": document.url, "html": &*document.html }))
            }
            other => Err(call_back_refused(&format!(
                "the Hook stage `{}` does not serve `{other}`",
                self.stage.as_str()
            ))),
        }
    }

    async fn set_cookie(&self, params: Value) -> Result<Value, CallError> {
        let name = required_str(&params, "name", "page.set_cookie")?;
        let value = required_str(&params, "value", "page.set_cookie")?;
        let url = params
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                (self.current_url.scheme() == "http" || self.current_url.scheme() == "https")
                    .then(|| self.current_url.to_string())
            });
        let domain = params
            .get("domain")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.page
            .driver
            .set_cookie(name, value, url, domain, path, "page.set_cookie")
            .await?;
        Ok(json!({}))
    }

    fn block_url(&self, params: &Value) -> Result<Value, CallError> {
        let pattern = required_str(params, "url", "page.block_url")?;
        self.page.owner.interception().block_url(&pattern)?;
        Ok(json!({}))
    }

    async fn evaluate(&self, mut params: Value) -> Result<Value, CallError> {
        if let Some(object) = params.as_object_mut() {
            object
                .entry("page_id".to_owned())
                .or_insert_with(|| json!(self.page_id.0));
        }
        let parsed: EvaluateParams = serde_json::from_value(params).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("the params of `page.evaluate` cannot be read: {error}"),
            )
        })?;
        let expression = page_functions::expression(&parsed.function, parsed.arg.as_ref());
        let returns = self
            .page
            .life
            .call(
                PageEvent::Read,
                page_reads::evaluate(
                    &CdpPage::of(&self.page.driver),
                    &expression,
                    World::Isolated,
                    "page.evaluate",
                ),
            )
            .await?;
        let evaluated = page_functions::answered(returns, (Form::Rust, "page.evaluate"))?;
        serde_json::to_value(evaluated).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("`page.evaluate` result could not encode: {error}"),
            )
        })
    }
}

/// Whether `method` is a built-in page-hook action (`page.set_cookie` or
/// `page.block_url`) scoped to non-logged-in Sessions without requiring
/// `page_script` or `effects` grants.
fn is_hook_safe_page_op(method: &str) -> bool {
    matches!(method, "page.set_cookie" | "page.block_url")
}

/// Whether `method` alters outgoing browser requests or cookies, which a Hook
/// must never do on a logged-in Session (ADR 0088, issue 954).
fn is_request_mutating(method: &str) -> bool {
    matches!(
        method,
        "page.set_cookie" | "page.block_url" | "page.route" | "page.add_init_script"
    )
}

fn required_str(params: &Value, field: &str, method: &str) -> Result<String, CallError> {
    params
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("`{method}` requires a non-empty string `{field}`"),
            )
        })
}

fn call_back_refused(message: &str) -> CallError {
    CallError::new(&PLUGIN_CALL_BACK_REFUSED, message, &())
}
