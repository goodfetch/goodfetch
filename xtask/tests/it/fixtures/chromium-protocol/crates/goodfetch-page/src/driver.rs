//! A driver that sends two commands through the bindings, one by its
//! method's name, and reads an event under another name.

use goodfetch_cdp::generated::page;
use goodfetch_cdp::generated::page::FrameNavigatedEvent as Navigated;

pub(crate) async fn open(connection: &Connection, url: &str) -> Result<(), NetError> {
    connection.send(page::EnableParams::default()).await?;
    connection
        .send(page::NavigateParams {
            url: url.to_owned(),
            frame_id: Some(main_frame()),
            referrer_policy: None,
        })
        .await?;
    connection.subscribe::<Navigated>();
    connection
        .send_raw("Browser.close", None, serde_json::json!({}))
        .await
}

#[cfg(test)]
mod tests {
    use goodfetch_cdp::generated::page;

    /// Test code is no use of the driver's: the build need not have it.
    #[test]
    fn reloads() {
        let _reload = page::ReloadParams::default();
    }
}
