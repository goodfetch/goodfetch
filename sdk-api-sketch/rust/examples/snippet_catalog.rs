//! Snippet for Mapping the catalog to each language and surface (#471, ADR 0099): the shared
//! `Connection` trait, the raw catalog call on a connection and on a value, and names spelled by
//! Rust's rule.

use std::time::Duration;

use futures_util::TryStreamExt;
use goodfetch_sdk_sketch::{ConnectOptions, Connection, Error, Goodfetch, JsonObject, Page};
use serde_json::{Value, json};

/// A library written for both Products takes `impl Connection`: `Collie` and `Foxhound` implement it.
fn engine_version(connection: &impl Connection) -> String {
    connection.engine().version.clone()
}

fn params(value: Value) -> JsonObject {
    match value {
        Value::Object(map) => map,
        _ => JsonObject::new(),
    }
}

/// The way down (G8): an operation this SDK does not generate, such as one a newer engine minor
/// added (`page.emulate_vision_deficiency` is hypothetical here). The page fills its own `page`
/// parameter; the parameters are the wire's snake_case JSON, an optional builder method.
async fn newer_operation(page: &Page) -> Result<(), Error> {
    page.call("page.emulate_vision_deficiency").params(params(json!({ "type": "deuteranopia" }))).await?;
    Ok(())
}

/// On the connection, the parameters name every value themselves; the timeout applies as on any call.
async fn on_the_connection(connection: &impl Connection, run_id: &str) -> Result<Value, Error> {
    connection
        .call("run.migrate") // hypothetical
        .params(params(json!({ "run": run_id, "to": "daemon" })))
        .timeout(Duration::from_secs(120))
        .await
}

/// One rule: snake_case methods, a keyword as a raw identifier (`r#continue`), `CdpSession` with its
/// initialism as one word.
async fn names(page: &Page) -> Result<bool, Error> {
    let tracing_header = page
        .route("**/api/**")
        .try_for_each(async |route| route.r#continue().header("x-trace", "1").await);
    let navigate = async {
        page.wait_for_url("**/orders").await?;
        let cdp = page.new_cdp_session().await?;
        drop(cdp);
        let response = page.goto("https://books.toscrape.com/").await?;
        Ok::<_, Error>(response.is_some_and(|response| response.ok()))
    };
    let (_, ok) = futures_util::try_join!(tracing_header, navigate)?;
    Ok(ok)
}

async fn both_products() -> Result<(), Error> {
    let gf = Goodfetch::connect(ConnectOptions::default()).await?; // `Collie::connect` once #483 lands
    let _version = engine_version(&gf);
    gf.close().await
}

fn main() {
    let _ = both_products();
    let _ = |page: Page| async move {
        newer_operation(&page).await?;
        on_the_connection(&Goodfetch::connect(ConnectOptions::default()).await?, "a-run").await?;
        names(&page).await
    };
}
