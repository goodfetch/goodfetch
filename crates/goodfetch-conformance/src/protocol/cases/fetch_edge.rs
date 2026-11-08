//! The page suite's fetch edge and response decoding cases, Collie's lines
//! (ADRs 0118 and 0119, ticket #699): a redirect loop raises
//! `too_many_redirects`; a main document cut by its transfer raises
//! `response_truncated` for a `load` wait and answers its response at
//! `commit`; a body whose listed coding fails to decode fails the
//! navigation, while an unknown coding passes through; a 3xx's
//! `Retry-After` is not waited for; a navigation to a file emits
//! `page.download` and answers no response, the page keeping its document;
//! and each document's response names the encoding it was decoded with.
//!
//! The fetch edge and content-coding pages are reached over plain
//! HTTP/1.1, since the browser does not trust the Proving ground's
//! authority until the engine's TLS relay does (ADR 0042).

use goodfetch_protocol::connection::CANCEL;
use goodfetch_protocol::events::{CREDIT, EVENTS, EVENTS_SUBSCRIBE};
use goodfetch_protocol::page::{
    PAGE_DOWNLOAD, PAGE_GOTO, PAGE_RELOAD, RESPONSE_TRUNCATED, TOO_MANY_REDIRECTS,
};
use serde_json::{Value, json};

use super::navigation::{answered, call, failed_with, open, open_page, response, url_on};
use super::{Checks, Site, finish_run};
use crate::protocol::connection::{Connection, Gone};

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    RedirectLoopIsTooManyRedirects,
    RetryAfterOnARedirectIsNotWaited,
    CutDocumentsAreTruncated,
    ContentCodingsDecodeAsChrome,
    FileNavigationIsADownload,
    ResponsesNameTheirDecodedEncoding,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-redirect-loop-raises-too-many-redirects" => Self::RedirectLoopIsTooManyRedirects,
            "a-redirect-is-followed-without-waiting-its-retry-after" => {
                Self::RetryAfterOnARedirectIsNotWaited
            }
            "a-cut-document-raises-response-truncated-unless-the-navigation-returned-at-commit" => {
                Self::CutDocumentsAreTruncated
            }
            "a-coding-that-fails-to-decode-fails-the-navigation-and-an-unknown-one-passes" => {
                Self::ContentCodingsDecodeAsChrome
            }
            "a-goto-to-a-file-emits-download-and-answers-no-response" => {
                Self::FileNavigationIsADownload
            }
            "each-document-response-names-its-decoded-encoding" => {
                Self::ResponsesNameTheirDecodedEncoding
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::RedirectLoopIsTooManyRedirects => redirect_loop(connection, site, checks),
            Self::RetryAfterOnARedirectIsNotWaited => {
                retry_after_on_a_redirect(connection, site, checks)
            }
            Self::CutDocumentsAreTruncated => cut_documents(connection, site, checks),
            Self::ContentCodingsDecodeAsChrome => content_codings(connection, site, checks),
            Self::FileNavigationIsADownload => file_navigation(connection, site, checks),
            Self::ResponsesNameTheirDecodedEncoding => decoded_encodings(connection, site, checks),
        }
    }
}

/// The browser's redirect count (ADR 0118): a navigation's chain holds at
/// most this many requests, its first included. A fact of the pinned
/// Chromium, which this case pins by its own figure.
const BROWSER_REDIRECTS: u32 = 20;

/// How many redirects the pinned Chromium follows before it fails a
/// navigation.
const FOLLOWED: u32 = BROWSER_REDIRECTS - 1;

/// The network error of a body whose listed coding does not decode.
const DECODING_FAILED: &str = "net::ERR_CONTENT_DECODING_FAILED";

/// How many events a subscription takes in these cases.
const CREDITS: u32 = 256;

/// A redirect loop fails the navigation at the browser's count with
/// `too_many_redirects`, naming the first URL, the hops followed and the
/// browser's count (pitfall U12).
fn redirect_loop(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-redirect-loop";
    let looping = site.start_url("redirects/19").map(ToOwned::to_owned);
    let Some(page) = open(connection, site, run_id, &[&looping], checks)? else {
        return finish_if(connection, run_id, &[&looping]);
    };
    let looping = looping.unwrap_or_default();
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &page,
        &json!({ "url": looping }),
    )?;
    let data = &answer["error"]["data"];
    checks.check(
        format!("a redirect loop fails with `{}`", TOO_MANY_REDIRECTS.name),
        data["code"] == json!(TOO_MANY_REDIRECTS.name),
    );
    checks.check(
        "a retry is not said to succeed",
        data["retryable"] == json!(false),
    );
    checks.check("it names the first URL", data["url"] == json!(looping));
    checks.check(
        format!("it names the {FOLLOWED} hops followed"),
        data["hops"].as_array().map(Vec::len) == usize::try_from(FOLLOWED).ok(),
    );
    checks.check(
        "it names the browser's count as the limit",
        data["limit"] == json!({ "kind": "browser", "count": BROWSER_REDIRECTS }),
    );
    finish_run(connection, run_id)
}

/// A redirect carrying `Retry-After` is followed at once: the navigation
/// answers its target, though the site asked to wait an hour, far past
/// `timeout.navigation`, so no measured time decides it (ADR 0118).
fn retry_after_on_a_redirect(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-retry-after-on-a-redirect";
    let redirect = site.named_url("fetch-edge.test", "/retry-after/302/seconds/long");
    let target = site.named_url("fetch-edge.test", "/target");
    let Some(page) = open_page(connection, site, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &page,
        &json!({ "url": redirect }),
    )?;
    answered(
        &answer,
        "`page.goto` to a 302 whose `Retry-After` asks for an hour",
        &target,
        200,
        checks,
    );
    finish_run(connection, run_id)
}

/// A main document cut by a length mismatch, a chunked stream ending early
/// or a compressed stream cut before its `Content-Length` raises
/// `response_truncated` for a `load` wait, naming the bytes received and
/// expected; one that returned at `commit` answers the response, leaving
/// the cut to the body's read (pitfall H15).
fn cut_documents(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-cut-documents";
    let cuts = [
        (
            "a body shorter than its `Content-Length`",
            "/cut/content-length",
            true,
        ),
        (
            "a chunked body without its last chunk",
            "/cut/chunked",
            false,
        ),
        (
            "a gzip body cut before its `Content-Length`",
            "/cut/gzip-content-length",
            true,
        ),
    ]
    .map(|(what, path, declared)| (what, site.named_url("fetch-edge.test", path), declared));
    let Some(page) = open_page(connection, site, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    for (what, url, declared) in &cuts {
        let loaded = call(connection, PAGE_GOTO.name, &page, &json!({ "url": url }))?;
        let data = &loaded["error"]["data"];
        checks.check(
            format!(
                "{what} fails a `load` wait with `{}`",
                RESPONSE_TRUNCATED.name
            ),
            data["code"] == json!(RESPONSE_TRUNCATED.name),
        );
        checks.check(
            format!("{what}: a retry may succeed"),
            data["retryable"] == json!(true),
        );
        checks.check(
            format!("{what}: it names the URL"),
            data["url"] == json!(url),
        );
        let received = data["received"].as_u64();
        let expected = data["expected"].as_u64();
        if *declared {
            checks.check(
                format!("{what}: fewer bytes arrived than its `Content-Length` declared"),
                matches!((received, expected), (Some(received), Some(expected)) if received < expected),
            );
        } else {
            checks.check(
                format!("{what}: it names the bytes received and no length expected"),
                received.is_some() && data["expected"].is_null(),
            );
        }
        let committed = call(
            connection,
            PAGE_GOTO.name,
            &page,
            &json!({ "url": url, "wait_until": "commit" }),
        )?;
        answered(&committed, &format!("{what} at `commit`"), url, 200, checks);
    }
    finish_run(connection, run_id)
}

/// A body whose listed content coding fails to decode fails the navigation
/// with the browser's `net::ERR_CONTENT_DECODING_FAILED`, a plain body
/// labelled gzip and an invalid gzip header alike, while an unknown coding
/// passes the body through, as in Chrome (ADR 0119; pitfalls H13, H25).
fn content_codings(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-content-codings";
    let plain_as_gzip = site.start_url("content-coding/1").map(plain_http);
    let invalid_header = site.start_url("content-coding/2").map(plain_http);
    let unknown = site.start_url("content-coding/5").map(plain_http);
    let Some(page) = open(
        connection,
        site,
        run_id,
        &[&plain_as_gzip, &invalid_header, &unknown],
        checks,
    )?
    else {
        return finish_if(
            connection,
            run_id,
            &[&plain_as_gzip, &invalid_header, &unknown],
        );
    };
    for (what, url) in [
        ("a plain body labelled gzip", &plain_as_gzip),
        ("a body whose gzip header is invalid", &invalid_header),
    ] {
        let answer = call(connection, PAGE_GOTO.name, &page, &json!({ "url": url }))?;
        failed_with(&answer, what, DECODING_FAILED, checks);
    }
    let unknown = unknown.unwrap_or_default();
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &page,
        &json!({ "url": unknown }),
    )?;
    answered(&answer, "a body labelled `utf-8`", &unknown, 200, checks);
    finish_run(connection, run_id)
}

/// A program's own `goto` to a PDF emits `page.download`, answers no
/// response, and leaves the page on its document, which a reload loads
/// again (ADR 0119).
fn file_navigation(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-file-navigation";
    let document = site.page_url.clone();
    let file = url_on(site, "files/2", "/files/report.pdf");
    let Some(page) = open(connection, site, run_id, &[&document, &file], checks)? else {
        return finish_if(connection, run_id, &[&document, &file]);
    };
    let (document, file) = (document.unwrap_or_default(), file.unwrap_or_default());
    call(
        connection,
        PAGE_GOTO.name,
        &page,
        &json!({ "url": document }),
    )?;
    let subscription = connection.request(EVENTS_SUBSCRIBE.name, &json!({}))?;
    connection.notify(CREDIT, &json!({ "id": subscription, "credits": CREDITS }))?;
    let goto_id = connection.request(PAGE_GOTO.name, &json!({ "page_id": page, "url": file }))?;
    let goto = connection.browser_answer(goto_id)?;
    connection.notify(CANCEL, &json!({ "id": subscription }))?;
    let ended = connection.answer(subscription)?;
    checks.check(
        "`page.goto` to a file answers no response",
        goto.message.get("result") == Some(&json!({ "response": null })),
    );
    let download = goto
        .others
        .iter()
        .chain(&ended.others)
        .filter(|message| message["method"] == json!(EVENTS))
        .filter_map(|message| message["params"]["events"].as_array())
        .flatten()
        .find(|event| event["event"] == json!(PAGE_DOWNLOAD.name))
        .map(|event| event["data"].clone());
    checks.check(
        format!("it emits `{}`", PAGE_DOWNLOAD.name),
        download.is_some(),
    );
    let download = download.unwrap_or(Value::Null);
    checks.check(
        "the download names the page, the file's URL and its suggested name",
        download == json!({ "page_id": page, "url": file, "suggested_filename": "report.pdf" }),
    );
    let reload = call(connection, PAGE_RELOAD.name, &page, &json!({}))?;
    answered(
        &reload,
        "`page.reload` after the download",
        &document,
        200,
        checks,
    );
    finish_run(connection, run_id)
}

/// Each document's response names the encoding the browser decoded it
/// with: a `Shift_JIS` page and a UTF-8 page (ADR 0119).
fn decoded_encodings(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-decoded-encodings";
    let shift_jis = url_on(site, "encodings/1", "/pages/shift-jis");
    let utf8 = site.page_url.clone();
    let Some(page) = open(connection, site, run_id, &[&shift_jis, &utf8], checks)? else {
        return finish_if(connection, run_id, &[&shift_jis, &utf8]);
    };
    for (what, url, encoding) in [
        ("a Shift_JIS page", &shift_jis, "Shift_JIS"),
        ("a UTF-8 page", &utf8, "UTF-8"),
    ] {
        let answer = call(connection, PAGE_GOTO.name, &page, &json!({ "url": url }))?;
        if let Some(response) = response(&answer, &format!("`page.goto` to {what}"), checks) {
            checks.check(
                format!("{what}'s response names {encoding} as its decoded encoding"),
                response.encoding.as_deref() == Some(encoding),
            );
        }
    }
    finish_run(connection, run_id)
}

/// Finishes the Run `run_id` when the case started it, which it does once
/// the Proving ground serves every one of `urls`.
fn finish_if(
    connection: &mut Connection,
    run_id: &str,
    urls: &[&Option<String>],
) -> Result<(), Gone> {
    if urls.iter().all(|url| url.is_some()) {
        finish_run(connection, run_id)?;
    }
    Ok(())
}

/// A raw host's task URL over plain HTTP/1.1, which the browser reaches
/// without trusting the Proving ground's authority.
fn plain_http(url: &str) -> String {
    url.replacen("https://", "http://", 1)
}
