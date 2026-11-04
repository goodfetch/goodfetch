//! Where the details window's fonts are: the system's own, so that no font
//! file ships in the Kennel's executable (ADR 0141). Each platform keeps
//! them in a few folders, laid out its own way, so the Kennel looks for
//! the files it prefers by name in those folders and their subfolders, and
//! takes the first name found, in its order of preference.
//!
//! - **macOS**: SF, then Helvetica; SF Mono, then Menlo.
//! - **Windows**: Segoe UI, then Arial, then Tahoma; Consolas, then
//!   Courier New, then Lucida Console, in `C:\Windows\Fonts`.
//! - **Linux and the others**: `DejaVuSans.ttf`, then Noto, Liberation,
//!   Ubuntu's, GNU `FreeSans.ttf` and Open Sans, and their monospace
//!   fonts, each distribution's folders included
//!   (Debian's `truetype/dejavu`, Fedora's `dejavu-sans-fonts`, Arch's
//!   `TTF`, NixOS's profile), under the person's font folder,
//!   `/usr/local/share/fonts` and `/usr/share/fonts`.
//!
//! A missing monospace font takes the proportional one; with no
//! proportional font, the window has none to draw with.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The deepest subfolder of a font folder looked in: distributions file
/// fonts at most three folders down (`truetype/dejavu`, `google-noto-vf`).
const DEPTH: usize = 4;

/// The fonts the window draws with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemFonts {
    /// The text's font.
    pub proportional: PathBuf,
    /// The monospace font; none when the system has none of the list, and
    /// the proportional one stands in.
    pub monospace: Option<PathBuf>,
}

/// The file names of the proportional fonts, preferred first.
#[cfg(target_os = "macos")]
const PROPORTIONAL: &[&str] = &["SFNS.ttf", "Helvetica.ttc"];
/// The file names of the monospace fonts, preferred first.
#[cfg(target_os = "macos")]
const MONOSPACE: &[&str] = &["SFNSMono.ttf", "Menlo.ttc"];

#[cfg(target_os = "windows")]
const PROPORTIONAL: &[&str] = &["segoeui.ttf", "arial.ttf", "tahoma.ttf"];
#[cfg(target_os = "windows")]
const MONOSPACE: &[&str] = &["consola.ttf", "cour.ttf", "lucon.ttf"];

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const PROPORTIONAL: &[&str] = &[
    "DejaVuSans.ttf",
    "NotoSans-Regular.ttf",
    "LiberationSans-Regular.ttf",
    "Ubuntu-R.ttf",
    "FreeSans.ttf",
    "OpenSans-Regular.ttf",
    // Fedora's Noto, a variable font, drawn at its default weight.
    "NotoSans[wght].ttf",
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONOSPACE: &[&str] = &[
    "DejaVuSansMono.ttf",
    "NotoSansMono-Regular.ttf",
    "LiberationMono-Regular.ttf",
    "UbuntuMono-R.ttf",
    "FreeMono.ttf",
    "NotoSansMono[wght].ttf",
];

/// The folders this platform keeps fonts in, the person's first.
fn folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = dirs::font_dir().into_iter().collect();
    if cfg!(target_os = "macos") {
        folders.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
    } else if cfg!(target_os = "windows") {
        folders.push(PathBuf::from(r"C:\Windows\Fonts"));
    } else {
        folders.extend(dirs::home_dir().map(|home| home.join(".fonts")));
        folders.extend(
            [
                "/usr/local/share/fonts",
                "/usr/share/fonts",
                "/run/current-system/sw/share/X11/fonts",
            ]
            .map(PathBuf::from),
        );
    }
    folders
}

/// The system's fonts for the window; none when it has no proportional
/// font of the list. Blocks: run it on the tracker's blocking threads.
pub fn system() -> Option<SystemFonts> {
    find(&folders(), PROPORTIONAL, MONOSPACE)
}

/// The first font of `proportional` and of `monospace`, by file name and
/// ignoring case, in `folders` and their subfolders: a name earlier in its
/// list wins over a folder earlier in `folders`, and for one name the
/// earlier folder wins. Blocks.
pub fn find(folders: &[PathBuf], proportional: &[&str], monospace: &[&str]) -> Option<SystemFonts> {
    let mut found = BTreeMap::new();
    for folder in folders {
        files(folder, DEPTH, &mut found);
    }
    let first = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| found.get(&name.to_lowercase()).cloned())
    };
    Some(SystemFonts {
        proportional: first(proportional)?,
        monospace: first(monospace),
    })
}

/// Adds each file under `folder`, `depth` folders down at most, by its
/// name in lower case, keeping a name's first path.
#[expect(
    clippy::disallowed_methods,
    reason = "called from `system`, on the tracker's blocking threads, once as the window starts (STYLE.md)"
)]
fn files(folder: &Path, depth: usize, found: &mut BTreeMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    // The order of a folder's entries is the file system's: sorted, the
    // same fonts give the same pick.
    let mut entries: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if depth > 0 {
                files(&path, depth - 1, found);
            }
        } else if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            found.entry(name.to_lowercase()).or_insert(path);
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_methods,
        reason = "the tests lay a font folder out on the test thread, outside any runtime"
    )]

    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{SystemFonts, find};

    /// A folder holding a stand-in font file at each of `files`.
    fn laid_out(files: &[&str]) -> tempfile::TempDir {
        let folder = tempfile::tempdir().unwrap();
        for file in files {
            let path = folder.path().join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"font").unwrap();
        }
        folder
    }

    fn at(folder: &Path, file: &str) -> PathBuf {
        folder.join(file)
    }

    #[test]
    fn the_preferred_font_wins_wherever_a_distribution_files_it() {
        let user = laid_out(&["fonts/NotoSans-Regular.ttf"]);
        let system = laid_out(&[
            "truetype/liberation/LiberationSans-Regular.ttf",
            "truetype/dejavu/DejaVuSans.ttf",
            "truetype/dejavu/DejaVuSansMono.ttf",
        ]);

        let found = find(
            &[user.path().join("fonts"), system.path().to_owned()],
            &["DejaVuSans.ttf", "NotoSans-Regular.ttf"],
            &["DejaVuSansMono.ttf"],
        );

        assert_eq!(
            found,
            Some(SystemFonts {
                proportional: at(system.path(), "truetype/dejavu/DejaVuSans.ttf"),
                monospace: Some(at(system.path(), "truetype/dejavu/DejaVuSansMono.ttf")),
            })
        );
    }

    #[test]
    fn a_later_name_stands_in_and_case_does_not_matter() {
        let fonts = laid_out(&["FONTS/ARIAL.TTF"]);

        let found = find(
            &[fonts.path().to_owned()],
            &["segoeui.ttf", "arial.ttf"],
            &["consola.ttf"],
        );

        assert_eq!(
            found,
            Some(SystemFonts {
                proportional: at(fonts.path(), "FONTS/ARIAL.TTF"),
                monospace: None,
            })
        );
    }

    #[test]
    fn for_one_name_the_earlier_folder_wins() {
        let user = laid_out(&["DejaVuSans.ttf"]);
        let system = laid_out(&["dejavu/DejaVuSans.ttf"]);

        let found = find(
            &[user.path().to_owned(), system.path().to_owned()],
            &["DejaVuSans.ttf"],
            &[],
        );

        assert_eq!(
            found.map(|fonts| fonts.proportional),
            Some(at(user.path(), "DejaVuSans.ttf"))
        );
    }

    #[test]
    fn with_no_proportional_font_there_is_none_to_draw_with() {
        let fonts = laid_out(&["dejavu/DejaVuSansMono.ttf"]);
        let missing = Path::new("/no/such/folder").to_owned();

        let found = find(
            &[missing, fonts.path().to_owned()],
            &["DejaVuSans.ttf"],
            &["DejaVuSansMono.ttf"],
        );

        assert_eq!(found, None);
    }

    #[test]
    fn a_font_deeper_than_the_folders_distributions_use_is_not_looked_for() {
        let fonts = laid_out(&["a/b/c/d/e/DejaVuSans.ttf"]);

        let found = find(&[fonts.path().to_owned()], &["DejaVuSans.ttf"], &[]);

        assert_eq!(found, None);
    }
}
