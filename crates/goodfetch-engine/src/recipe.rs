//! A Recipe read from its package's folder (issue 1663): its manifest,
//! `goodfetch-recipe.toml`, under the format registry's rule for
//! `recipe-manifest` (ADR 0030), and the files its package holds, which
//! are every file of the folder but the ones its `.goodfetchignore` files
//! leave out and the ones a package always does: version control, secrets
//! files, dependency folders and build output.

use std::path::Path;

use goodfetch_protocol::formats::{Compatibility, RECIPE_MANIFEST};
use goodfetch_protocol::recipe::{
    self, ALWAYS_IGNORED, RECIPE_IGNORE_FILE, RECIPE_INVALID, RECIPE_MANIFEST_FILE,
    RecipeInvalidDetails, RecipeManifest,
};
use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use toml_edit::DocumentMut;

use crate::call_error::CallError;
use crate::run::format_refusal;
use crate::settings::json_of_table;

/// A Recipe's package: its manifest and the files it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipePackage {
    /// Its manifest.
    pub manifest: RecipeManifest,
    /// Its files, by their paths from the package's root with `/` between
    /// their parts, in byte order: the manifest, the entry and every other
    /// file the ignore rules keep, lockfiles included.
    pub files: Vec<String>,
}

/// Reads the package whose root is `root`: its manifest, then its files,
/// which must hold the manifest and the entry it names.
///
/// # Errors
///
/// `recipe_invalid` for a folder with no manifest, a manifest that is not
/// valid, a symbolic link or a path that is not UTF-8 in the package, or an
/// entry the package does not hold; `format_version_unsupported` for a
/// manifest of a version this engine does not read.
pub fn package(root: &Path) -> Result<RecipePackage, CallError> {
    let text = std::fs::read_to_string(root.join(RECIPE_MANIFEST_FILE)).map_err(|error| {
        invalid(RecipeInvalidDetails {
            field: RECIPE_MANIFEST_FILE.to_owned(),
            reason: format!("the package's root holds no readable manifest: {error}"),
            fix: format!("write `{RECIPE_MANIFEST_FILE}` at the package's root"),
        })
    })?;
    let manifest = manifest(&text)?;
    let files = files(root)?;
    if files
        .binary_search_by(|file| file.as_str().cmp(RECIPE_MANIFEST_FILE))
        .is_err()
    {
        return Err(invalid(RecipeInvalidDetails {
            field: RECIPE_IGNORE_FILE.to_owned(),
            reason: format!("an ignore rule leaves `{RECIPE_MANIFEST_FILE}` out of the package"),
            fix: format!("remove the rule that matches `{RECIPE_MANIFEST_FILE}`"),
        }));
    }
    let entry = manifest.entry.path();
    if files
        .binary_search_by(|file| file.as_str().cmp(entry))
        .is_err()
    {
        return Err(invalid(RecipeInvalidDetails {
            field: format!("entry.{}", manifest.entry.key()),
            reason: format!(
                "`{entry}` is not in the package: the folder lacks it, or an ignore rule leaves it out"
            ),
            fix: "name a file of the package, by its path from the package's root".to_owned(),
        }));
    }
    Ok(RecipePackage { manifest, files })
}

/// Reads a manifest's text: its version under the registry's rule for
/// `recipe-manifest`, then its fields.
///
/// # Errors
///
/// `recipe_invalid` for a manifest that does not parse or is not valid,
/// naming its field; `format_version_unsupported` for one of a version this
/// engine does not read.
pub fn manifest(text: &str) -> Result<RecipeManifest, CallError> {
    let document: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| {
        invalid(RecipeInvalidDetails {
            field: RECIPE_MANIFEST_FILE.to_owned(),
            reason: format!("it is not TOML: {}", error.message()),
            fix: "write the manifest in TOML".to_owned(),
        })
    })?;
    let document = json_of_table(document.as_table());
    let found = recipe::version_of(&document).map_err(invalid)?;
    match RECIPE_MANIFEST
        .rule
        .compatibility(RECIPE_MANIFEST.version, found)
    {
        Compatibility::Compatible => recipe::read(document).map_err(invalid),
        refused => Err(format_refusal(
            &RECIPE_MANIFEST,
            &found.to_string(),
            refused,
        )),
    }
}

/// The files of the package whose root is `root`, in byte order.
fn files(root: &Path) -> Result<Vec<String>, CallError> {
    let always = always_ignored(root);
    let mut files = Vec::new();
    let walk = WalkBuilder::new(root)
        .standard_filters(false)
        .follow_links(false)
        .add_custom_ignore_filename(RECIPE_IGNORE_FILE)
        // A folder a rule matches is never entered, so matching each path
        // alone also leaves out everything beneath it.
        .filter_entry(move |entry| {
            let is_folder = entry.file_type().is_some_and(|kind| kind.is_dir());
            entry.depth() == 0 || !always.matched(entry.path(), is_folder).is_ignore()
        })
        .build();
    for entry in walk {
        let entry = entry.map_err(|error| {
            invalid(RecipeInvalidDetails {
                field: RECIPE_MANIFEST_FILE.to_owned(),
                reason: format!("the package's folder does not read: {error}"),
                fix: "make every file of the package readable".to_owned(),
            })
        })?;
        if entry.depth() == 0 {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("invariant: the walk yields paths beneath its root");
        let path = package_path(relative)?;
        if entry.path_is_symlink() {
            return Err(invalid(RecipeInvalidDetails {
                field: path,
                reason: "it is a symbolic link, which a package does not hold".to_owned(),
                fix: format!(
                    "put the file itself there, or leave it out in `{RECIPE_IGNORE_FILE}`"
                ),
            }));
        }
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            files.push(path);
        }
    }
    files.sort_unstable();
    Ok(files)
}

/// The rules a package always applies, which no ignore file overrides.
fn always_ignored(root: &Path) -> Gitignore {
    let mut rules = GitignoreBuilder::new(root);
    for rule in ALWAYS_IGNORED {
        rules
            .add_line(None, rule)
            .expect("invariant: every rule a package always applies is valid");
    }
    rules
        .build()
        .expect("invariant: the rules a package always applies build")
}

/// `relative`'s parts joined by `/`.
fn package_path(relative: &Path) -> Result<String, CallError> {
    let parts = relative
        .iter()
        .map(|part| part.to_str())
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            invalid(RecipeInvalidDetails {
                field: relative.to_string_lossy().into_owned(),
                reason: "its path is not UTF-8".to_owned(),
                fix: format!("rename it, or leave it out in `{RECIPE_IGNORE_FILE}`"),
            })
        })?;
    Ok(parts.join("/"))
}

fn invalid(details: RecipeInvalidDetails) -> CallError {
    let message = format!(
        "The Recipe is not valid: `{}`: {}. Fix: {}.",
        details.field, details.reason, details.fix
    );
    CallError::new(&RECIPE_INVALID, message, &details)
}

#[cfg(test)]
mod tests;
