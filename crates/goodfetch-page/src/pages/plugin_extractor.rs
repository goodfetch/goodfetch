//! A browser page's formats read with an Extractor Plugin (spec #596): the
//! Clean-up rules apply as for a built-in (ADR 0047), in the same one read
//! of the page, which then serializes the cleaned page instead of finding
//! its main content; the Plugin `content.extractor` names reads that HTML
//! under `timeout.plugin` and answers the main content's Markdown and text,
//! which the call answers as page content (ADR 0014).
//!
//! A Plugin answers no main HTML, so `html` of kind `main` is refused, and
//! no "fit": `markdown` of content `fit` holds its Markdown as it answered.

use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::plugin_kinds::Failure;
use goodfetch_engine::plugin_kinds::extractor::ExtractorDriver;
use goodfetch_engine::plugin_kinds::run_plugins::StartError;
use goodfetch_engine::run::accounts::RunAccounts;
use goodfetch_engine::secrets::secret_error;
use goodfetch_engine::settings::{Resolved, RunSettings, Values};
use goodfetch_engine::{CallError, named};
use goodfetch_extractor::BuiltIn;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{
    Asked, CONTENT_EXTRACTOR, FormatsRequest, HtmlFormat, HtmlKind, HtmlOptions, MainContent,
    MarkdownContent, MarkdownFormat, PageFormats,
};
use goodfetch_protocol::plugin::extractor::ExtractContentResult;
use goodfetch_protocol::settings::TIMEOUT_PLUGIN;
use url::Url;

use crate::browser_run::BrowserRun;
use crate::extractor;

/// How a page whose formats are `asked` is read: the Extractor Plugin that
/// reads its main content, when `asked` names it and the nearest of the
/// Run's settings `run`, the Session's or the crawl request's Label
/// `scopes` and the page's settings `page` names one
/// ([`extractor::plugin`]); the request the page's one read takes; and
/// whether that read also keeps the page's raw HTML, which only a Plugin's
/// answer to a request for it needs.
///
/// # Errors
///
/// [`cleaned_request`]'s.
pub(super) fn chosen(
    asked: FormatsRequest,
    run: Option<&Resolved>,
    scopes: (&Values, Option<BuiltIn>),
    page: &Values,
) -> Result<(Option<String>, FormatsRequest, bool), CallError> {
    let plugin = asked
        .reads_main_content()
        .then(|| extractor::plugin(run, scopes, page))
        .flatten();
    let Some(name) = plugin else {
        return Ok((None, asked, false));
    };
    let raw = asked.html.map(HtmlFormat::kind) == Some(HtmlKind::Raw);
    let read = cleaned_request(asked, &name)?;
    Ok((Some(name), read, raw))
}

/// The request the page's one read takes when the Plugin `name` reads its
/// main content: `asked` without the formats the Plugin answers, the
/// Markdown of the main content or "fit" and `text`, and with the cleaned
/// page's HTML, which the Plugin reads.
///
/// # Errors
///
/// `invalid_params` when `asked` names `html` of kind `main`, which a
/// Plugin does not answer.
pub(super) fn cleaned_request(
    asked: FormatsRequest,
    name: &str,
) -> Result<FormatsRequest, CallError> {
    if asked.html.map(HtmlFormat::kind) == Some(HtmlKind::Main) {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "The Extractor Plugin `{name}` that {} names answers no main HTML: ask for `html` of kind `cleaned` or `raw`, or name a built-in step.",
                named::setting(CONTENT_EXTRACTOR.name),
            ),
        ));
    }
    Ok(FormatsRequest {
        markdown: asked
            .markdown
            .filter(|format| format.content() == MarkdownContent::Full),
        html: Some(HtmlFormat::Options(HtmlOptions {
            kind: HtmlKind::Cleaned,
        })),
        text: None,
        ..asked
    })
}

/// The formats `asked` names: `read`, what the page's one read answered,
/// as it is when no `plugin` reads the main content ([`chosen`]). With
/// the Plugin `plugin` of `run`, `read` answered [`cleaned_request`], its
/// `html` the cleaned page, and the Plugin reads the main content from it;
/// `raw` is the page's raw HTML when `asked` names it. The Plugin starts on
/// the Run's first call that names it, and each call is charged to the
/// Run's Usage and written in its Run trace.
///
/// # Errors
///
/// `extractor_unavailable` when no installed Extractor Plugin has the name
/// or it cannot start, `setting_invalid` naming the field of its settings
/// its schema refuses, and the call's failure, as the Extractor's driver
/// reads it, when the Plugin fails it or the Run's Usage refuses it.
pub(super) async fn answered(
    run: &BrowserRun,
    plugin: Option<&str>,
    asked: FormatsRequest,
    mut read: PageFormats,
    raw: Option<String>,
) -> Result<PageFormats, CallError> {
    let Some(name) = plugin else {
        return Ok(read);
    };
    let cleaned = read.html.take().unwrap_or_default();
    let html = match asked.html.map(HtmlFormat::kind) {
        Some(HtmlKind::Cleaned) => Some(cleaned.clone()),
        Some(HtmlKind::Raw) => raw,
        Some(HtmlKind::Main) | None => None,
    };
    let url = Url::parse(&read.url).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the page's URL does not parse: {error}"),
        )
    })?;
    let settings = run.identity.settings().map(RunSettings::resolved);
    let extractor = run
        .plugins
        .extractor(
            name,
            settings,
            run.keys.secrets(),
            run.engine.sandbox_probe(),
        )
        .await
        .map_err(|error| match error {
            StartError::Refused(error) => error,
            StartError::Secret(error) => secret_error(&error),
        })?;
    let clock = run.identity.clock();
    let deadline = Deadline::of_resolved(settings, &TIMEOUT_PLUGIN);
    let accounts = RunAccounts {
        identity: &run.identity,
        charging: &run.charging,
        trace: &run.trace,
    };
    let ExtractContentResult { text, markdown, .. } =
        ExtractorDriver::new(&extractor, &accounts, &clock, &deadline)
            .content(&url, cleaned)
            .await
            .map_err(Failure::into_error)?;
    let (form, confined, confinement) = if extractor.is_sdk_handler() {
        ("sdk_handler", false, "unconfined")
    } else {
        ("process", true, "confined")
    };
    crate::trace::provenance(
        &run.trace,
        serde_json::json!({
            "action": "extract_content",
            "extractor": name,
            "form": form,
            "confined": confined,
            "confinement": confinement,
            "url": url.as_str(),
        }),
    )
    .await;
    let found = !text.trim().is_empty() || !markdown.trim().is_empty();
    Ok(PageFormats {
        main_content: Some(if found {
            MainContent::Found
        } else {
            MainContent::None
        }),
        markdown: match asked.markdown.map(MarkdownFormat::content) {
            Some(MarkdownContent::Full) => read.markdown,
            Some(MarkdownContent::Main | MarkdownContent::Fit) => Some(markdown),
            None => None,
        },
        html,
        text: asked.text.map(|Asked| text),
        ..read
    })
}

#[cfg(test)]
mod tests;
