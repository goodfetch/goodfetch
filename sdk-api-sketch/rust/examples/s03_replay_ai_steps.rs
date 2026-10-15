//! Golden scenario 03, "Repeat AI steps with no model call" (ADRs 0091, 0101, 0102).

use goodfetch_collie::schemars::{self, JsonSchema};
use goodfetch_collie::settings::ai;
use goodfetch_collie::{
    Collie, ConnectOptions, Error, Expected, Schema, Settings, StepSource, expect,
};
use serde::Deserialize;

/// The author, as the task returns it.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Author {
    /// The author's name.
    pub name: String,
    /// The birth date, as the page writes it.
    pub birth_date: String,
    /// The birthplace.
    pub birthplace: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // The model is called only for a step the Plan file lacks or a step that drifted (ADR 0091).
    let settings = Settings::new().set(ai::MODEL, Some("anthropic/claude-sonnet-4-5".into()));
    let options = ConnectOptions::default().settings(settings);
    Collie::scope(options, async |collie| {
        // One entry per call, kept next to the program; a new version is written only after a
        // `verified` Run.
        let plan = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/quotes.plan.json");
        let run = collie.start().plan(plan).await?;
        let page = run.new_page().await?;
        page.goto("https://quotes.toscrape.com/").await?;
        // Replayed with no model call when its Target fingerprint still matches; on Drift, this
        // step alone gets one Repair, and a Drift left (`plan.frozen`, or a Repair that drifted
        // too) fails with `plan_drift`, naming it.
        let step = async |instruction: &str| -> Result<(), Error> {
            let step = page.act(instruction).await?.step;
            if step.source == StepSource::Repair {
                println!(
                    r#"The site changed: "{}" went back to the model."#,
                    step.instruction
                );
            }
            Ok(())
        };
        // Success checks without a model: the Run is `verified`, and its Plan kept, only if they
        // all hold, so a step that went to another tag, page or author is never kept.
        step(r#"open the tag "love" from the tag list"#).await?;
        step("go to its second page").await?;
        expect(&page)
            .to_have_url("https://quotes.toscrape.com/tag/love/page/2/")
            .await?;
        let first_author = page.locator(".quote .author").first().text_content().await?;
        step("open the about page of the first quote's author").await?;
        expect(&page)
            .to_have_url(Expected::regex(r"^https://quotes\.toscrape\.com/author/"))
            .await?;
        expect(&page.locator(".author-title"))
            .to_have_text(first_author.as_str())
            .await?;
        // Compiled by default: the model writes each field's locator once, then values are
        // copied from the page.
        let author = page
            .extract(
                "the author's name, birth date and birthplace",
                Schema::<Author>::of(),
            )
            .await?
            .output;
        let report = run.finish().await?;
        println!(
            "{author:?} Plan {:?} {:?}",
            report.plan.status, report.plan.changed_steps
        );
        Ok(())
    })
    .await
}
