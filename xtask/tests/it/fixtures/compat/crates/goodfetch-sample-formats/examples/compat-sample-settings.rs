//! Writes the `sample-settings` scenario: one settings file.

const SETTINGS: &str = r#"{
  "headers": {
    "accept": "text/html"
  },
  "limits": {
    "bytes": 1048576,
    "pages": 10
  },
  "name": "sample",
  "proxy": {
    "url": "secret://proxy"
  },
  "retries": [
    {
      "delay_ms": 250,
      "status": 503
    }
  ]
}
"#;

fn main() {
    let folder = std::path::PathBuf::from(std::env::args_os().nth(1).expect("the folder"));
    std::fs::write(folder.join("settings.json"), SETTINGS).expect("writing settings.json");
}
