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
use goodfetch_engine::plugin_kinds::extractor::ExtractorDriver;
use goodfetch_engine::runs::{Run, plugins};
use goodfetch_engine::settings::RunSettings;
use goodfetch_engine::{CallError, named};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{
    Asked, CONTENT_EXTRACTOR, FormatsRequest, HtmlFormat, HtmlKind, HtmlOptions, MainContent,
    MarkdownContent, MarkdownFormat, PageFormats,
};
use goodfetch_protocol::plugin::extractor::ExtractContentResult;
use goodfetch_protocol::settings::TIMEOUT_PLUGIN;
use url::Url;

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

/// The formats `asked` names: `read`, what the page's one read answered to
/// [`cleaned_request`], its `html` the cleaned page, with the main content
/// the Plugin `name` of `run` reads from it, and `raw`, the page's raw HTML
/// when `asked` names it. The Plugin starts on the Run's first call that
/// names it.
///
/// # Errors
///
/// `extractor_unavailable` when no installed Extractor Plugin has the name
/// or it cannot start, and the call's failure, as the Extractor's driver
/// reads it, when the Plugin fails it.
pub(super) async fn answered(
    run: &Run,
    name: &str,
    asked: FormatsRequest,
    mut read: PageFormats,
    raw: Option<String>,
) -> Result<PageFormats, CallError> {
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
    let extractor = plugins(run).extractor(name).await?;
    let clock = run.clock();
    let deadline =
        Deadline::of_resolved(run.settings().map(RunSettings::resolved), &TIMEOUT_PLUGIN);
    let ExtractContentResult { text, markdown, .. } =
        ExtractorDriver::new(&extractor, &clock, &deadline)
            .content(&url, cleaned)
            .await
            .map_err(|failure| failure.into_error())?;
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
