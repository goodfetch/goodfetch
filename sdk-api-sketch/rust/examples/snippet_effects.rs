//! Snippet for Effects a program declares in code (#500, ADR 0106). No golden scenario places an
//! order, so these programs are the sketch's own.

use goodfetch_sdk_sketch::{
    AriaRole, ConnectOptions, Crawl, CrawlContext, Effect, EffectResult, Error, ErrorCode, ErrorDetails, Goodfetch, Page, Run, Seed,
    schemars::{self, JsonSchema},
};
use serde::{Deserialize, Serialize};

/// A Collie program safe to run again after a crash: the Run is named, so a start under the same
/// ID resumes it, and the order placed before the crash comes back from the Effect journal.
async fn place_order(gf: &Goodfetch, sku: &str) -> Result<Option<String>, Error> {
    let run: Run = gf.collie().start().id(format!("order-{sku}")).await?;
    let page = run.new_page().await?;
    page.goto(&format!("https://shop.example/products/{sku}")).await?;
    page.get_by_role(AriaRole::Button.name("Add to cart")).click().await?; // not declared: adding twice is harmless here
    page.goto("https://shop.example/checkout").await?;

    let placed = run
        .effect(&format!("place-order-{sku}"), async |_effect: &Effect| {
            page.get_by_role(AriaRole::Button.name("Place order")).click().await?;
            Ok::<_, Error>(page.get_by_test_id("order-number").text_content().await?.trim().to_owned()) // kept in the journal
        })
        .await?;
    let _report = run.finish().await?;
    Ok(match placed {
        // `Journal`: placed before a crash, the button was not pressed again.
        EffectResult::Performed { output, .. } | EffectResult::Journal { output, .. } => Some(output),
        // A person said it happened; its number is not known.
        EffectResult::Reconciled { .. } => None,
        _ => None,
    })
}

/// The program's own error: a body returns it as it is, and takes Goodfetch's through `From`.
#[derive(Debug)]
enum ShopError {
    Goodfetch(Error),
    Refused(u16),
}

impl From<Error> for ShopError {
    fn from(error: Error) -> Self {
        Self::Goodfetch(error)
    }
}

impl std::fmt::Display for ShopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Goodfetch(error) => error.fmt(f),
            Self::Refused(status) => write!(f, "the shop refused the subscription: {status}"),
        }
    }
}

impl std::error::Error for ShopError {}

/// A body that sends the site's own idempotency key is safe to repeat: declare it so.
async fn subscribe(run: &Run, page: &Page, plan: &str) -> Result<u16, ShopError> {
    let result = run
        .effect(&format!("subscribe-{plan}"), async |effect: &Effect| {
            let response = page
                .request()
                .post("https://shop.example/api/subscriptions")
                .header("Idempotency-Key", effect.idempotency_key())
                .json(serde_json::json!({ "plan": plan }))
                .await?;
            match response.status() {
                status @ 200..=299 => Ok(status),
                status => Err(ShopError::Refused(status)), // returned as it is, or the source of `effect_uncertain`
            }
        })
        .repeatable()
        .await?;
    Ok(result.output().copied().unwrap_or(0))
}

/// The Payload of a listing: the seller's address to write from.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Seller {
    email: String,
}

/// In a crawl handler: one form submitted per request, never twice across retries, escalations,
/// sample comparisons or a resume. The key's namespace is the crawl request.
async fn contact_sellers(gf: &Goodfetch, emails: Vec<String>) -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let listing = crawl.label::<Seller>("listing")?;
    crawl.handle(&listing, async |ctx: CrawlContext<Seller>| {
        let page = ctx.page();
        page.get_by_label("Email").fill(ctx.request().payload().email.as_str()).await?; // preparing the page is no part of the Effect
        let sent = ctx
            .run()
            .effect("contact-seller", async |_effect: &Effect| {
                page.get_by_role(AriaRole::Button.name("Contact seller")).click().await?;
                page.get_by_text("Message sent").wait_for(Default::default()).await
            })
            .await;
        match sent {
            Ok(EffectResult::Performed { .. }) => Ok(()),
            Ok(_) => Ok(()), // already contacted
            // The click went out and the confirmation never showed: a person reconciles it in the
            // Console; the request ends `failed` with `effect_uncertain`, never retried.
            Err(error) if error.code() == &ErrorCode::EffectUncertain => {
                if let Some(ErrorDetails::EffectUncertain(uncertain)) = error.details() {
                    let _ = (&uncertain.key, &uncertain.why);
                }
                Err(error)
            }
            Err(error) => Err(error),
        }
    })?;
    let seeds: Vec<Seed> = emails
        .into_iter()
        .enumerate()
        .map(|(i, email)| Seed::new(&format!("https://market.example/listings/{i}"), &listing, Seller { email }))
        .collect();
    let run = gf.foxhound().start_crawl(&crawl, seeds).id("contact-2026-10-15").await?;
    let report = crawl.serve(&run).await?;
    for effect in &report.run.uncertain_effects {
        let _ = (&effect.key, effect.request.as_ref().map(|request| &request.url)); // needs a person
    }
    gf.close().await
}

fn main() {
    let _ = (place_order, subscribe, contact_sellers, ConnectOptions::default);
}
