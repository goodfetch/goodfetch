//! The details window's fonts: the system's own, read once as the window
//! starts, so that no font file ships in the Kennel's executable. Each
//! family takes the first font of its list that the system has.

use std::path::Path;
use std::sync::Arc;

use eframe::egui::{FontData, FontDefinitions, FontFamily};

/// A font file and the face it holds that the window draws with.
type Candidate = (&'static str, u32);

#[cfg(target_os = "macos")]
const PROPORTIONAL: &[Candidate] = &[
    ("/System/Library/Fonts/SFNS.ttf", 0),
    ("/System/Library/Fonts/Helvetica.ttc", 0),
];
#[cfg(target_os = "macos")]
const MONOSPACE: &[Candidate] = &[
    ("/System/Library/Fonts/SFNSMono.ttf", 0),
    ("/System/Library/Fonts/Menlo.ttc", 0),
];

#[cfg(target_os = "windows")]
const PROPORTIONAL: &[Candidate] = &[
    (r"C:\Windows\Fonts\segoeui.ttf", 0),
    (r"C:\Windows\Fonts\arial.ttf", 0),
];
#[cfg(target_os = "windows")]
const MONOSPACE: &[Candidate] = &[
    (r"C:\Windows\Fonts\consola.ttf", 0),
    (r"C:\Windows\Fonts\cour.ttf", 0),
];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const PROPORTIONAL: &[Candidate] = &[
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/TTF/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf", 0),
    ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", 0),
    ("/usr/share/fonts/noto/NotoSans-Regular.ttf", 0),
    (
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        0,
    ),
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONOSPACE: &[Candidate] = &[
    ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0),
    ("/usr/share/fonts/TTF/DejaVuSansMono.ttf", 0),
    (
        "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf",
        0,
    ),
    (
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        0,
    ),
];

/// The window's fonts, from the system's; none when the system has no
/// proportional font of the list. A missing monospace font takes the
/// proportional one. Blocks: run it on the tracker's blocking threads.
pub(crate) fn system() -> Option<FontDefinitions> {
    let proportional = first(PROPORTIONAL)?;
    let monospace = first(MONOSPACE).unwrap_or_else(|| proportional.clone());
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

/// The first font of `candidates` the system has.
fn first(candidates: &[Candidate]) -> Option<FontData> {
    candidates.iter().find_map(|(path, index)| {
        #[expect(
            clippy::disallowed_methods,
            reason = "read on the tracker's blocking threads, once as the window starts (STYLE.md)"
        )]
        let bytes = std::fs::read(Path::new(path)).ok()?;
        tracing::debug!(path, "the details window's font");
        Some(FontData::from_blob(Arc::new(bytes), *index))
    })
}
