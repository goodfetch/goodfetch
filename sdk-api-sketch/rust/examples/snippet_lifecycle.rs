//! Snippet for Entry points and lifecycle (#465): connect, open a Run, close explicitly or in scope.

use goodfetch_collie::{ConnectOptions, Error, Collie};

async fn explicit() -> Result<(), Error> {
    let collie = Collie::connect(ConnectOptions::default()).await?;
    let run = collie.start().await?;
    let page = run.new_page().await?;
    page.close().await?;
    collie.close().await // closes the Run it opened; Drop alone would only warn
}

async fn scoped() -> Result<String, Error> {
    Collie::scope(ConnectOptions::default(), async |collie| {
        let run = collie.start().await?;
        Ok(run.id().to_owned())
    })
    .await
}

fn main() {
    let _ = (explicit(), scoped());
}
