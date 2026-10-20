//! The `responsive` family: a layout (`/layout`) whose plan changes at
//! viewport widths of 600 and 1,024 CSS pixels and whose density changes at
//! device pixel ratios of 1.5 and 2.5, each by media queries alone, and
//! whose dot is an image of three densities (`/dot-1x.svg` to
//! `/dot-3x.svg`) the browser picks from by its device pixel ratio. Each
//! task names a viewport; the oracle reads the layout the tool returns, and
//! an image of another density, if the Proving ground saw one requested,
//! refutes it.

use std::fmt::Write as _;

use hyper::StatusCode;

use super::document;
use crate::exchange::Reply;
use crate::generator;
use crate::observation::Seen;
use crate::oracle::Finding;
use crate::task::{self, Entry, Expected, Kind, Record, Task};

pub(super) const HOST: &str = "responsive.test";
pub(super) const FAMILY: &str = "responsive";
pub(super) const TASKS: u32 = 3;

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 24;

/// The breakpoints, narrowest first, each with the media query that shows
/// its plan.
const BREAKPOINTS: [(&str, &str); 3] = [
    ("narrow", "(width < 600px)"),
    ("medium", "(600px <= width < 1024px)"),
    ("wide", "(width >= 1024px)"),
];

/// The densities, each with the media query that shows it.
const DENSITIES: [(&str, &str); 3] = [
    ("1x", "(resolution < 1.5dppx)"),
    ("2x", "(1.5dppx <= resolution < 2.5dppx)"),
    ("3x", "(resolution >= 2.5dppx)"),
];

/// Each task's breakpoint and density, by index, which its viewport gives
/// (390 by 844 at 3, 820 by 1,180 at 2, 1,440 by 900 at 1).
const TASK_LAYOUTS: [(usize, usize); 3] = [(0, 2), (1, 1), (2, 0)];

/// The plan shown at breakpoint `index` for `seed`.
fn plan(seed: u64, index: usize) -> String {
    let value = generator::below(seed, &[FAMILY_PLACE, index as u64], 10_000);
    format!("N-{value:04}")
}

/// The path of the dot image of density `name`.
fn dot(name: &str) -> String {
    format!("/dot-{name}.svg")
}

/// A `GET` of `path` on one of the family's task hosts.
pub(super) fn page(seed: u64, path: &str) -> Option<Reply> {
    if path == "/layout" {
        return Some(Reply::html(layout(seed)));
    }
    let density = DENSITIES.iter().position(|(name, _)| dot(name) == path)?;
    let radius = 4 * (density + 1);
    Some(Reply {
        status: StatusCode::OK,
        content_type: "image/svg+xml",
        headers: Vec::new(),
        body: format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" viewBox=\"0 0 32 32\"><circle cx=\"16\" cy=\"16\" r=\"{radius}\" fill=\"#047857\"/></svg>\n"
        )
        .into(),
    })
}

fn layout(seed: u64) -> String {
    let mut style = String::from("<style>\n.plan, .density { display: none; }\n");
    let mut main = String::from("<h1>Layout</h1>\n");
    for (index, (name, query)) in BREAKPOINTS.iter().enumerate() {
        writeln!(
            style,
            "@media {query} {{ .plan.{name} {{ display: block; }} }}"
        )
        .expect("invariant: writing to a String succeeds");
        writeln!(
            main,
            "<p class=\"plan {name}\" data-breakpoint=\"{name}\">Plan: <span class=\"value\">{}</span></p>",
            plan(seed, index)
        )
        .expect("invariant: writing to a String succeeds");
    }
    for (name, query) in DENSITIES {
        let class = format!("x{}", &name[..1]);
        writeln!(
            style,
            "@media {query} {{ .density.{class} {{ display: block; }} }}"
        )
        .expect("invariant: writing to a String succeeds");
        writeln!(
            main,
            "<p class=\"density {class}\">Density: <span class=\"value\">{name}</span></p>"
        )
        .expect("invariant: writing to a String succeeds");
    }
    style.push_str("</style>\n");
    let srcset = DENSITIES
        .iter()
        .map(|(name, _)| format!("{} {name}", dot(name)))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(
        main,
        "<img src=\"{}\" srcset=\"{srcset}\" alt=\"A dot drawn for the screen's density\" width=\"16\" height=\"16\">",
        dot("1x")
    )
    .expect("invariant: writing to a String succeeds");
    document("Layout", &style, &main, "")
}

/// What task `task`'s requests refute: an image of a density other than
/// its viewport's, which only another device pixel ratio fetches.
pub(super) fn findings(task: &str, requests: &[Seen]) -> Vec<Finding> {
    let Some(index) = task
        .strip_prefix(FAMILY)
        .and_then(|rest| rest.strip_prefix('/'))
        .and_then(|ordinal| ordinal.parse::<usize>().ok())
        .and_then(|ordinal| ordinal.checked_sub(1))
        .filter(|index| *index < TASK_LAYOUTS.len())
    else {
        return Vec::new();
    };
    let expected = dot(DENSITIES[TASK_LAYOUTS[index].1].0);
    requests
        .iter()
        .filter(|request| {
            request.target != expected
                && DENSITIES
                    .iter()
                    .any(|(name, _)| dot(name) == request.target)
        })
        .map(|request| Finding {
            check: "density_image".to_owned(),
            at: "/layout".to_owned(),
            expected: Some(expected.clone().into()),
            observed: request.target.clone().into(),
        })
        .collect()
}

/// The family's tasks for `seed`. Ordinals are append-only (ADR 0063).
pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    const DESCRIPTIONS: [&str; 3] = [
        "Open this page in a viewport 390 CSS pixels wide and 844 high at a device pixel \
         ratio of 3, as a phone shows it, and extract its layout as one Record with the fields \
         `id` (`layout`), `breakpoint` (the shown plan's `data-breakpoint`), `plan` and \
         `density`, as the page shows them.",
        "Open this page in a viewport 820 CSS pixels wide and 1,180 high at a device pixel \
         ratio of 2, as a tablet shows it, and extract its layout as one Record with the \
         fields `id` (`layout`), `breakpoint` (the shown plan's `data-breakpoint`), `plan` and \
         `density`, as the page shows them.",
        "Open this page in a viewport 1,440 CSS pixels wide and 900 high at a device pixel \
         ratio of 1, as a desktop shows it, and extract its layout as one Record with the \
         fields `id` (`layout`), `breakpoint` (the shown plan's `data-breakpoint`), `plan` and \
         `density`, as the page shows them.",
    ];
    (1..)
        .zip(TASK_LAYOUTS.into_iter().zip(DESCRIPTIONS))
        .map(|(ordinal, ((breakpoint, density), description))| {
            let host = task::task_host(ordinal, HOST);
            Task {
                entry: Entry {
                    id: format!("{FAMILY}/{ordinal}"),
                    family: FAMILY,
                    kind: Kind::Extract,
                    start_url: format!("{}/layout", task::origin(&host, port)),
                    description,
                    account: None,
                    time_steps: None,
                },
                host,
                expected: Expected::records(vec![Record::from([
                    ("id", "layout".to_owned()),
                    ("breakpoint", BREAKPOINTS[breakpoint].0.to_owned()),
                    ("plan", plan(seed, breakpoint)),
                    ("density", DENSITIES[density].0.to_owned()),
                ])]),
            }
        })
        .collect()
}
