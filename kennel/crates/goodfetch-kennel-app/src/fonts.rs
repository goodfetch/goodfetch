//! The details window's fonts: the system's own, found by
//! `goodfetch_kennel::fonts` and read once as the window starts, so that no
//! font file ships in the Kennel's executable.

use std::path::Path;
use std::sync::Arc;

use eframe::egui::{FontData, FontDefinitions, FontFamily};
use goodfetch_kennel::fonts::{self, SystemFonts};

/// The window's fonts, from the system's; none when the system has no
/// proportional font of the list or it cannot be read. A missing monospace
/// font takes the proportional one. Blocks: run it on the tracker's
/// blocking threads.
pub(crate) fn system() -> Option<FontDefinitions> {
    let SystemFonts {
        proportional,
        monospace,
    } = fonts::system()?;
    let proportional = read(&proportional)?;
    let monospace = monospace
        .and_then(|path| read(&path))
        .unwrap_or_else(|| proportional.clone());
    let mut fonts = FontDefinitions::empty();
    for (family, name, data) in [
        (
            FontFamily::Proportional,
            "system-proportional",
            proportional,
        ),
        (FontFamily::Monospace, "system-monospace", monospace),
    ] {
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
        fonts.families.insert(family, vec![name.to_owned()]);
    }
    Some(fonts)
}

/// The font at `path`, its first face for a collection.
fn read(path: &Path) -> Option<FontData> {
    #[expect(
        clippy::disallowed_methods,
        reason = "read on the tracker's blocking threads, once as the window starts (STYLE.md)"
    )]
    let bytes = std::fs::read(path)
        .inspect_err(|error| {
            tracing::warn!(path = %path.display(), %error, "the details window's font cannot be read");
        })
        .ok()?;
    tracing::debug!(path = %path.display(), "the details window's font");
    Some(FontData::from_blob(Arc::new(bytes), 0))
}
