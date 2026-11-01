//! Foxhound's Console build, taken by path from `xtask console build`'s
//! output (ADR 0096): the brotli files of its folder, each served at its
//! path without the `.br` suffix, and the routes `routes.json` lists, read
//! once at start into the engine's own `ConsoleBuild`, which the gateway
//! then serves as Foxhound's executable serves its embedded copy. The
//! values live as long as the process, as an embedded build does.

use std::io;
use std::path::{Path, PathBuf};

use goodfetch_engine::{ConsoleBuild, ConsoleFile};

/// The routes of the build's screens, in its folder.
const ROUTES: &str = "routes.json";

/// The build in `folder`.
///
/// # Errors
///
/// When the folder cannot be read or holds no `index.html.br`.
pub(crate) async fn read(folder: &Path) -> io::Result<&'static ConsoleBuild> {
    let mut found = Vec::new();
    let mut folders = vec![PathBuf::new()];
    while let Some(relative) = folders.pop() {
        let mut entries = tokio::fs::read_dir(folder.join(&relative)).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = relative.join(entry.file_name());
            if entry.file_type().await?.is_dir() {
                folders.push(path);
            } else if let Some(served) = path.to_string_lossy().strip_suffix(".br") {
                let bytes = tokio::fs::read(entry.path()).await?;
                found.push((format!("/{}", served.replace('\\', "/")), bytes));
            }
        }
    }
    if !found.iter().any(|(path, _)| path == "/index.html") {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "{} holds no Console build: run `cargo run -p xtask -- console build`",
                folder.display()
            ),
        ));
    }
    let routes = routes(folder).await?;
    let files: Vec<ConsoleFile> = found
        .into_iter()
        .map(|(path, bytes)| {
            ConsoleFile::new(
                Box::leak(path.into_boxed_str()),
                Box::leak(bytes.into_boxed_slice()),
            )
        })
        .collect();
    let routes: Vec<&'static str> = routes
        .into_iter()
        .map(|route| &*Box::leak(route.into_boxed_str()))
        .collect();
    Ok(Box::leak(Box::new(ConsoleBuild::new(
        Box::leak(files.into_boxed_slice()),
        Box::leak(routes.into_boxed_slice()),
    ))))
}

/// The routes `routes.json` lists, `{ "routes": ["/settings", …] }`.
async fn routes(folder: &Path) -> io::Result<Vec<String>> {
    let text = tokio::fs::read_to_string(folder.join(ROUTES)).await?;
    let listed: serde_json::Value = serde_json::from_str(&text)?;
    Ok(listed["routes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|route| route.as_str().map(str::to_owned))
        .collect())
}
