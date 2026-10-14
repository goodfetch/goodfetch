//! Snippet for Entry points and lifecycle (#465): connect, open a Run, close explicitly or in scope.

use goodfetch_sdk_sketch::{ConnectOptions, Error, Goodfetch};

async fn explicit() -> Result<(), Error> {
    let gf = Goodfetch::connect(ConnectOptions::default()).await?;
    let run = gf.collie().start().await?;
    let page = run.new_page().await?;
    page.close().await?;
    gf.close().await // closes the Run it opened; Drop alone would only warn
}

async fn scoped() -> Result<String, Error> {
    Goodfetch::scope(ConnectOptions::default(), async |gf| {
        let run = gf.collie().start().await?;
        Ok(run.id().to_owned())
    })
    .await
}

fn main() {
    let _ = (explicit(), scoped());
}
