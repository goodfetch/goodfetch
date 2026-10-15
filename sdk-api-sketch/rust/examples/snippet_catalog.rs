//! Snippet for Mapping the catalog to each language and surface (#471, ADR 0099): the shared
//! `Connection` trait, the raw catalog call on a connection and on a value, and names spelled by
//! Rust's rule.

use std::time::Duration;

use futures_util::TryStreamExt;
use goodfetch_sdk_sketch::{ConnectOptions, Connection, Error, Goodfetch, JsonObject, Page, Run};
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

/// The way down (G8): `run.set_status_message` (area 3's RT-15) is not generated here, so it goes
/// raw. The Run fills its own `run` parameter; the parameters are the wire's snake_case JSON.
async fn report_progress(run: &Run, done: u32) -> Result<(), Error> {
    let message = format!("{done} pages done");
    run.call("run.set_status_message", params(json!({ "message": message }))).await?;
    Ok(())
}

/// On the connection, the parameters name every value themselves; the timeout applies as on any call.
async fn newer_operation(connection: &impl Connection, run: &Run) -> Result<Value, Error> {
    connection
        .call("run.checkpoint", params(json!({ "run": run.id() })))
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
    let _ = |run: Run, page: Page| async move {
        report_progress(&run, 10).await?;
        newer_operation(&Goodfetch::connect(ConnectOptions::default()).await?, &run).await?;
        names(&page).await
    };
}
