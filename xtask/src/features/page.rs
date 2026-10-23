//! The Feature report's HTML page (ADR 0072), written beside its JSON: for
//! each Product, its counts by area and by due milestone, then its untested
//! Features with their cause and declaring tests, its excepted Features by
//! evidence kind, and its planned Features. One self-contained file, with no
//! script and nothing fetched.

use std::fmt::Write as _;

use super::report::{Cause, Entry, Group, Report, State, Summary, View};

const STYLE: &str = "\
:root{color-scheme:light dark;--fg:#1d1d1f;--bg:#fff;--muted:#6e6e73;--line:#d2d2d7;--bad:#b3261e;--ok:#1b6e3a}
@media (prefers-color-scheme:dark){:root{--fg:#f5f5f7;--bg:#1d1d1f;--muted:#a1a1a6;--line:#424245;--bad:#f2b8b5;--ok:#8fd19e}}
body{font:15px/1.5 system-ui,sans-serif;color:var(--fg);background:var(--bg);margin:0 auto;max-width:72rem;padding:1rem}
h1,h2,h3{line-height:1.2}table{border-collapse:collapse;width:100%;margin:.5rem 0 1.5rem;display:block;overflow-x:auto}
th,td{border-bottom:1px solid var(--line);padding:.3rem .6rem;text-align:left;vertical-align:top}
th{color:var(--muted);font-weight:600}td.n{text-align:right;font-variant-numeric:tabular-nums}
code{font-size:.9em;overflow-wrap:anywhere}.failed,.quarantined{color:var(--bad)}.passed{color:var(--ok)}.muted{color:var(--muted)}";

/// The page of `report`.
pub(crate) fn render(report: &Report) -> String {
    let mut page = String::new();
    let milestone = milestone(Some(report.milestone));
    let _ = write!(
        page,
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Feature report</title>\n<style>{STYLE}</style>\n</head>\n<body>\n\
         <h1>Feature report at the {milestone} gate</h1>\n<p><strong>{}</strong></p>\n<p class=\"muted\">{}</p>\n",
        escape(&report.gate_line()),
        escape(&source_line(report))
    );
    for view in &report.views {
        render_view(&mut page, report, view);
    }
    page.push_str("</body>\n</html>\n");
    page
}

fn source_line(report: &Report) -> String {
    let commit = report.release_commit.as_deref().map_or_else(
        || "no Release run of main in the results store".to_owned(),
        |commit| format!("main's latest Release run, of {commit}"),
    );
    let listings = if report.listings {
        "every runner's declarations"
    } else {
        "no runner listing, so no Vitest, Playwright or pytest declaration and no proved Journey variant"
    };
    format!("Read from {commit}, with {listings}.")
}

fn render_view(page: &mut String, report: &Report, view: &View) {
    let product = view.product.as_str();
    let _ = write!(
        page,
        "<h2>{product}</h2>\n<p>{} tested, {} untested, {} excepted, {} planned.</p>\n",
        view.summary.tested,
        view.summary.untested_count(),
        view.summary.excepted_count(),
        view.summary.planned
    );
    page.push_str("<h3>By area</h3>\n");
    summaries(page, "Area", &view.areas, |area| {
        area.map_or_else(|| "none".to_owned(), |area| area.to_string())
    });
    page.push_str("<h3>By due milestone</h3>\n");
    summaries(page, "Due", &view.milestones, |due| milestone(*due));
    let carried: Vec<&Entry> = report
        .features
        .iter()
        .filter(|entry| entry.products.contains(&view.product))
        .collect();
    untested(page, &carried);
    excepted(page, &carried);
    planned(page, &carried);
}

fn milestone(due: Option<goodfetch_protocol::journey::Milestone>) -> String {
    due.map_or_else(|| "none".to_owned(), |due| format!("{due:?}"))
}

fn summaries<K>(
    page: &mut String,
    heading: &str,
    groups: &[Group<K>],
    name: impl Fn(&Option<K>) -> String,
) {
    let _ = write!(page, "<table>\n<tr><th>{heading}</th><th>Tested</th>");
    for cause in Cause::ALL {
        let _ = write!(page, "<th>{}</th>", cause.as_str());
    }
    page.push_str("<th>Excepted</th><th>Planned</th></tr>\n");
    for group in groups {
        let Summary {
            tested,
            untested,
            planned,
            ..
        } = &group.summary;
        let _ = write!(
            page,
            "<tr><td>{}</td><td class=\"n\">{tested}</td>",
            escape(&name(&group.key))
        );
        for cause in Cause::ALL {
            let _ = write!(
                page,
                "<td class=\"n\">{}</td>",
                untested.get(cause.as_str()).copied().unwrap_or_default()
            );
        }
        let _ = writeln!(
            page,
            "<td class=\"n\">{}</td><td class=\"n\">{planned}</td></tr>",
            group.summary.excepted_count()
        );
    }
    page.push_str("</table>\n");
}

fn untested(page: &mut String, carried: &[&Entry]) {
    page.push_str("<h3>Untested</h3>\n");
    let entries: Vec<&&Entry> = carried
        .iter()
        .filter(|entry| entry.status == State::Untested)
        .collect();
    if entries.is_empty() {
        page.push_str("<p>None.</p>\n");
        return;
    }
    page.push_str("<table>\n<tr><th>Feature</th><th>Cause</th><th>Declaring tests</th></tr>\n");
    for entry in entries {
        let cause = entry.cause.map_or("", Cause::as_str);
        let tests = if entry.tests.is_empty() {
            "<span class=\"muted\">none</span>".to_owned()
        } else {
            entry
                .tests
                .iter()
                .map(|test| {
                    format!(
                        "<code>{}</code> <span class=\"muted\">({}, {})</span> <span class=\"{outcome}\">{outcome}</span>",
                        escape(&test.test),
                        test.source,
                        test.level,
                        outcome = test.outcome.as_str()
                    )
                })
                .collect::<Vec<_>>()
                .join("<br>")
        };
        let _ = writeln!(
            page,
            "<tr><td><code>{}</code></td><td>{cause}</td><td>{tests}</td></tr>",
            escape(&entry.id)
        );
    }
    page.push_str("</table>\n");
}

fn excepted(page: &mut String, carried: &[&Entry]) {
    page.push_str("<h3>Excepted</h3>\n");
    let mut entries: Vec<&&Entry> = carried
        .iter()
        .filter(|entry| entry.status == State::Excepted)
        .collect();
    if entries.is_empty() {
        page.push_str("<p>None.</p>\n");
        return;
    }
    entries.sort_by_key(|entry| entry.evidence);
    page.push_str("<table>\n<tr><th>Evidence</th><th>Feature</th><th>Reason</th></tr>\n");
    for entry in entries {
        let link = entry.link.as_deref().unwrap_or_default();
        let _ = writeln!(
            page,
            "<tr><td>{}</td><td><code>{}</code></td><td>{} <a href=\"{}\">{}</a></td></tr>",
            entry.evidence.unwrap_or_default(),
            escape(&entry.id),
            escape(entry.reason.as_deref().unwrap_or_default()),
            escape(link),
            escape(link)
        );
    }
    page.push_str("</table>\n");
}

fn planned(page: &mut String, carried: &[&Entry]) {
    let entries: Vec<&&Entry> = carried
        .iter()
        .filter(|entry| entry.status == State::Planned)
        .collect();
    let _ = writeln!(page, "<h3>Planned ({})</h3>", entries.len());
    if entries.is_empty() {
        page.push_str("<p>None.</p>\n");
        return;
    }
    page.push_str("<table>\n<tr><th>Feature</th><th>Due</th></tr>\n");
    for entry in entries {
        let _ = writeln!(
            page,
            "<tr><td><code>{}</code></td><td>{}</td></tr>",
            escape(&entry.id),
            milestone(entry.due)
        );
    }
    page.push_str("</table>\n");
}

/// `text` with the characters HTML gives a meaning replaced.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}
