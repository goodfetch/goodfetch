// Standalone launch with the harness flag set (src/core/browser-host.ts), used when no endpoint is given.
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

const FLAGS: [&str; 10] = [
    "--headless",
    "--remote-debugging-port=0",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--mute-audio",
    "--window-size=1280,720",
];

pub struct Browser {
    child: Child,
    profile: PathBuf,
    pub ws_endpoint: String,
}

impl Browser {
    pub async fn launch(executable: &str) -> Result<Browser, String> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let profile = std::env::temp_dir().join(format!("automan-chrome-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&profile).map_err(|error| error.to_string())?;
        let mut child = Command::new(executable)
            .args(FLAGS)
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("cannot launch browser: {error}"))?;
        let mut lines = BufReader::new(child.stderr.take().ok_or("no stderr")?).lines();
        let endpoint = tokio::time::timeout(Duration::from_secs(30), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(index) = line.find("ws://") {
                    if line.contains("DevTools listening on") {
                        return Some(line[index..].trim().to_string());
                    }
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        // Keep draining stderr so the browser never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        match endpoint {
            Some(ws_endpoint) => Ok(Browser { child, profile, ws_endpoint }),
            None => {
                let _ = child.kill().await;
                let _ = std::fs::remove_dir_all(&profile);
                Err("browser did not expose a DevTools endpoint in time".into())
            }
        }
    }

    pub async fn close(&mut self) {
        let _ = self.child.kill().await;
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}
