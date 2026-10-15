//! Snippet for Hooks and injectable components in the SDK API (#481, ADR 0104), on golden
//! scenario 05's crawl.

use goodfetch_sdk_sketch::schemars::{self, JsonSchema};
use std::pin::pin;

use futures_util::future::{Either, select};
use goodfetch_sdk_sketch::{
    AfterResponseContext, BeforeRequestContext, BlockCause, ConnectOptions, Crawl, CrawlContext, Error, ErrorCode, Goodfetch, HookOn,
    PageStageContext, ProxyAnswer, RecordCommitContext, RequestChange, ResponseChange, SecretBinding, SecretReference, Seed, Settings,
    secret,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A book.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Book {
    title: String,
    price: String,
    url: String,
}

/// The Payload of a book request.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct BookPayload {
    category: String,
}

/// Leases a proxy host from the program's own pool.
async fn lease_proxy(domain: &str) -> Result<String, std::io::Error> {
    let _ = domain;
    unimplemented!()
}

/// Golden scenario 05's crawl with Hooks. Hooks borrow the Labels and `books`: they run inside the
/// future of `serve`, as handlers do.
async fn crawl_with_hooks() -> Result<(), Error> {
    let mut crawl = Crawl::new();
    let books = crawl.dataset::<Book>("books")?;
    let home = crawl.label::<()>("home")?;
    let book = crawl.label::<BookPayload>("book")?;

    // Before request, scoped by Label handle: the Payload is typed, and the change is a returned
    // value. The header's value is a Secret placeholder, which the engine adds after the last
    // Hook, so the key never passes through the program.
    crawl.before_request(HookOn::from(&book).named("catalog-key"), async |ctx: BeforeRequestContext<BookPayload>| {
        // Also the stage before a retry: stop retrying a request the site keeps timing out on.
        if ctx.previous_error().is_some_and(|e| *e.code() == ErrorCode::DeadlineExceeded) && ctx.request().attempt() > 2 {
            return Err(ctx.skip("gave up after timeouts"));
        }
        Ok(RequestChange::none()
            .header("x-catalog-key", secret("catalog_key"))
            .header("x-category", ctx.request().payload().category.as_str()))
    })?;

    // After response: a soft block served with a 200 is marked here, and feeds escalation by new
    // Session.
    crawl.after_response(HookOn::every_label(), async |ctx: AfterResponseContext<Value>| {
        if ctx.response().status() == 200 && ctx.response().text().await?.contains("Access denied") {
            return Err(ctx.block(BlockCause::IpBlocked, "soft block page"));
        }
        Ok(ResponseChange::none())
    })?;

    // Page ready, unscoped: runs just before every handler, on every tier, as Crawlee's
    // `router.use`.
    crawl.page_ready(HookOn::every_label(), async |ctx: PageStageContext<Value>| {
        let _ = (ctx.request().url(), ctx.tier());
        Ok(())
    })?;

    // Before a Record's commit, scoped by Dataset: it sees and returns the Dataset's Record type.
    crawl.before_record_commit(&books, async |ctx: RecordCommitContext<Book>| {
        if ctx.record().title.trim().is_empty() {
            return Err(ctx.drop_record("no title"));
        }
        let mut record = ctx.into_record();
        record.title = record.title.trim().to_owned();
        Ok(record)
    })?;

    crawl.handle(&home, async |ctx: CrawlContext<()>| {
        let links = ctx.page().locator("article.product_pod h3 a");
        ctx.enqueue_links(&links, &book, BookPayload { category: "all".to_owned() }).await?;
        Ok(())
    })?;
    crawl.handle(&book, async |ctx: CrawlContext<BookPayload>| {
        let main = ctx.page().locator(".product_main");
        let record = Book {
            title: main.locator("h1").text_content().await?,
            price: main.locator(".price_color").text_content().await?,
            url: ctx.page().url(),
        };
        ctx.push(&books, &record).await
    })?;

    let catalog_key: SecretReference = "env:CATALOG_KEY".parse()?;
    // The program's own Proxy provider, an SDK handler declared by name when connecting: the only one
    // of its kind, so it becomes the proxies setting's value with the origin `Registered`.
    let gf = Goodfetch::connect(
        ConnectOptions::default()
            .proxy_provider("office-pool")
            .settings(Settings::new().secret("catalog_key", SecretBinding::reference(catalog_key, ["books.toscrape.com"]))),
    )
    .await?;
    let mut proxies = gf.proxy_requests("office-pool")?;
    let answer_proxies = async {
        while let Some(call) = proxies.next().await {
            let call = call?;
            match lease_proxy(&call.request().domain).await {
                Ok(host) => {
                    let password: SecretReference = "env:OFFICE_PROXY_PASSWORD".parse()?;
                    call.answer(ProxyAnswer::new(&format!("http://{host}:3128")).with_credentials("crawler", password));
                }
                Err(error) => call.fail(Box::new(error)), // `proxy_handler_failed`
            }
        }
        Ok::<(), Error>(())
    };
    let run = gf.foxhound().start_crawl(&crawl, [Seed::new("https://books.toscrape.com/", &home, ())]).await?;
    // Handlers and Hooks are answered until the Run ends; the proxy calls meanwhile.
    match select(pin!(crawl.serve(&run)), pin!(answer_proxies)).await {
        Either::Left((served, _)) => {
            let _report = served?; // the Run's report (ADR 0102)
        }
        Either::Right((answered, _)) => answered?,
    }
    run.as_run().close().await?;
    gf.close().await
}

fn main() {
    let _ = crawl_with_hooks;
}
