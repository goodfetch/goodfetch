//! The change as the floor guard reads it: `git diff`'s unified output with no
//! context, split into each file's added and removed lines.

/// A line a change added or removed, and the hunk it sits in: lines of one hunk
/// with neighbouring indexes are neighbours in the file.
pub(super) struct Line {
    pub(super) hunk: usize,
    pub(super) text: String,
}

/// One file of the change. A deleted file keeps its old path.
#[derive(Default)]
pub(super) struct FileChange {
    pub(super) path: String,
    /// The file existed at the merge base.
    pub(super) existed: bool,
    pub(super) added: Vec<Line>,
    pub(super) removed: Vec<Line>,
}

const NO_FILE: &str = "/dev/null";

fn path_of(header: &str) -> &str {
    // Git ends a path holding a space with a tab.
    let header = header.strip_suffix('\t').unwrap_or(header);
    header
        .strip_prefix("a/")
        .or_else(|| header.strip_prefix("b/"))
        .unwrap_or(header)
}

/// Parses the output of `git diff --unified=0 --no-renames`. Inside a hunk every
/// line is content, so an added `++i` (shown as `+++i`) is a line, not a header;
/// `---` and `+++` are headers only between a file's `diff` line and its first
/// hunk.
pub(super) fn parse(diff: &str) -> Vec<FileChange> {
    let mut files: Vec<FileChange> = Vec::new();
    let mut in_header = false;
    let mut hunk = 0;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            files.push(FileChange::default());
            in_header = true;
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            in_header = false;
            hunk += 1;
        } else if in_header {
            if let Some(old) = line.strip_prefix("--- ") {
                let old = path_of(old);
                file.existed = old != NO_FILE;
                if file.existed {
                    old.clone_into(&mut file.path);
                }
            } else if let Some(new) = line.strip_prefix("+++ ") {
                let new = path_of(new);
                if new != NO_FILE {
                    new.clone_into(&mut file.path);
                }
            }
        } else if let Some(text) = line.strip_prefix('+') {
            file.added.push(Line {
                hunk,
                text: text.to_owned(),
            });
        } else if let Some(text) = line.strip_prefix('-') {
            file.removed.push(Line {
                hunk,
                text: text.to_owned(),
            });
        }
    }
    // A mode change or a binary file has no paths to read, and no lines.
    files.retain(|file| !file.path.is_empty());
    files
}
