//! The `pacing` family on `pacing.test`: a host whose latency rises with the
//! Proving ground's time step (`PUT /time-step`), never the wall clock, so a
//! crawl's adaptive delay has something to lengthen against. Every answer
//! is held before it is sent, for a time that is a pure function of the
//! seed and the time step, and names that hold in `Server-Timing:
//! hold;dur=<milliseconds>`. The site is a list of articles at `/`, each at
//! `/articles/<n>`.

use std::fmt::Write as _;
use std::time::Duration;

use super::{Held, Page, Visit, canonical_number, crawl_task, document, name};
use crate::generator;
use crate::task::{Record, Task};

pub(super) const HOST: &str = "pacing.test";
pub(super) const FAMILY: &str = "pacing";

/// The generator places of the family's values.
const FAMILY_PLACE: u64 = 27;
const JITTER_PLACE: u64 = 1;

const ARTICLES: u64 = 20;
/// How many time steps the task spans, from step 0.
const TIME_STEPS: u64 = 6;
/// The hold at step 0, what each step adds, and the seeded part's bound,
/// in milliseconds.
const BASE_MS: u64 = 10;
const STEP_MS: u64 = 20;
const JITTER_MS: u64 = 10;
/// The last step whose hold rises; later ones hold as long, so no hold
/// passes five seconds, the `tiers` family's longest (STYLE.md: every wait
/// is bounded).
const LAST_RISING_STEP: u64 = 245;

/// The hold at `step`, in milliseconds: it rises by more than the seeded
/// part at each step, so it rises at every step up to the last rising one.
fn hold_ms(seed: u64, step: u64) -> u64 {
    let step = step.min(LAST_RISING_STEP);
    BASE_MS
        + STEP_MS * step
        + generator::below(seed, &[FAMILY_PLACE, step, JITTER_PLACE], JITTER_MS)
}

fn article_id(number: u64) -> String {
    format!("A-{number:02}")
}

fn title(seed: u64, number: u64) -> String {
    name(seed, &[FAMILY_PLACE, number])
}

pub(super) fn respond(visit: &Visit<'_>) -> Page {
    page_at(
        visit.seed,
        visit.observations.time_step(),
        visit.request.path,
    )
}

/// The page at `path` and time step `step`, held as that step says.
fn page_at(seed: u64, step: u64, path: &str) -> Page {
    let page = if path == "/" {
        Page::ok(list(seed))
    } else {
        path.strip_prefix("/articles/")
            .and_then(canonical_number)
            .filter(|number| (1..=ARTICLES).contains(number))
            .map_or_else(Page::not_found, |number| Page::ok(article(seed, number)))
    };
    let hold = hold_ms(seed, step);
    Page {
        held: Some(Held::after(Duration::from_millis(hold))),
        ..page.header("server-timing", format!("hold;dur={hold}"))
    }
}

fn list(seed: u64) -> String {
    let mut body = String::from("<main>\n<h1>Articles</h1>\n<ul class=\"articles\">\n");
    for number in 1..=ARTICLES {
        writeln!(
            body,
            "<li><a href=\"/articles/{number}\">{}</a></li>",
            title(seed, number)
        )
        .expect("invariant: writing to a String succeeds");
    }
    body.push_str("</ul>\n</main>\n");
    document("Articles", &body)
}

fn article(seed: u64, number: u64) -> String {
    let (id, title) = (article_id(number), title(seed, number));
    document(
        &title,
        &format!(
            "<main>\n<article class=\"entry\" data-id=\"{id}\">\n<h1 class=\"title\">{title}</h1>\n</article>\n</main>\n"
        ),
    )
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let mut task = crawl_task(
        (HOST, FAMILY),
        1,
        port,
        "/",
        "Crawl the articles this page links and return one Record per article, with the \
            fields `id` and `title`, while the Proving ground's time step goes from 0 to the \
            last of the task's time steps. The site answers more slowly at each step.",
        (1..=ARTICLES)
            .map(|number| {
                Record::from([("id", article_id(number)), ("title", title(seed, number))])
            })
            .collect(),
    );
    task.entry.time_steps = Some(TIME_STEPS);
    vec![task]
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "each test runs on the paused runtime `tokio::test` builds, TESTING.md's test Clock (ADR 0060)"
)]
mod tests {
    use std::time::Duration;

    use super::page_at;
    use crate::clock;

    #[tokio::test(start_paused = true)]
    async fn each_page_is_held_for_the_time_its_server_timing_names() {
        for (step, path) in [(0, "/"), (3, "/articles/4"), (245, "/"), (400, "/nowhere")] {
            let page = page_at(1538, step, path);
            let named: u64 = page
                .headers
                .iter()
                .find(|(name, _)| *name == "server-timing")
                .and_then(|(_, value)| value.strip_prefix("hold;dur="))
                .and_then(|ms| ms.parse().ok())
                .expect("a Server-Timing hold");
            let held = page.held.expect("every page is held");

            let started = clock::now();
            held.release().await;

            assert_eq!(
                clock::now() - started,
                Duration::from_millis(named),
                "{step}"
            );
        }
    }

    #[test]
    fn the_hold_stops_rising_below_five_seconds() {
        let hold = |step| {
            page_at(1538, step, "/")
                .headers
                .into_iter()
                .find(|(name, _)| *name == "server-timing")
                .map(|(_, value)| value)
        };

        assert_eq!(hold(245), hold(u64::MAX));
        assert!(super::hold_ms(1538, 244) < super::hold_ms(1538, 245));
        assert!(super::hold_ms(1538, u64::MAX) < 5_000);
    }
}
