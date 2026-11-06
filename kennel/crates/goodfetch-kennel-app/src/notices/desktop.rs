//! The desktop notices on Linux and Windows, through `notify-rust`: the
//! desktop's notification service over D-Bus, through the system's
//! libdbus, on Linux; a toast on Windows. Showing one blocks on the service, so it runs on the blocking
//! threads.

use goodfetch_kennel::state::Notice;
use goodfetch_kennel::tasks::Tasks;
use notify_rust::Notification;

use super::Adapter;

/// The name the desktop shows the notices under.
const APP_NAME: &str = "Kennel";

/// Shows notices through `notify-rust`.
pub(super) struct Desktop {
    tasks: Tasks,
}

impl Desktop {
    pub(super) fn new(tasks: Tasks) -> Self {
        Self { tasks }
    }
}

impl Adapter for Desktop {
    fn show(&self, notice: Notice) {
        let blocking = self.tasks.clone();
        self.tasks.spawn(async move {
            let shown = blocking
                .blocking(move || {
                    Notification::new()
                        .appname(APP_NAME)
                        .summary(&notice.title)
                        .body(&plain(&notice.body))
                        .show()
                        .map(|_shown| ())
                        .map_err(|error| error.to_string())
                })
                .await;
            if let Err(error) = shown {
                tracing::warn!(error, "a desktop notice was not shown");
            }
        });
    }
}

/// `text`, which the Text function made plain, kept plain for a Linux
/// notification service that reads a body's markup: `&`, `<` and `>` are
/// escaped. Windows reads no markup.
fn plain(text: &str) -> String {
    if cfg!(target_os = "windows") {
        return text.to_owned();
    }
    let mut plain = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => plain.push_str("&amp;"),
            '<' => plain.push_str("&lt;"),
            '>' => plain.push_str("&gt;"),
            other => plain.push(other),
        }
    }
    plain
}

#[cfg(all(test, not(target_os = "windows")))]
mod tests {
    use super::plain;

    #[test]
    fn a_body_that_looks_like_markup_shows_as_written() {
        assert_eq!(
            plain("<b>Pay</b> at <a href=\"x\">here</a> & now"),
            "&lt;b&gt;Pay&lt;/b&gt; at &lt;a href=\"x\"&gt;here&lt;/a&gt; &amp; now"
        );
    }
}
