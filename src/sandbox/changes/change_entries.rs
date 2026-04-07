use super::file_details::*;

use anyhow::Result;
use colored::*;
use fast_glob::glob_match;
use log::debug;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum SetType {
    Create,
    Modify,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum EntryOperation {
    Set(SetType),
    Remove,
    Rename,
    Error(ChangeError),
}

/// A ChangeEntry is a single change that has been detected in the sandbox.
#[derive(Debug, Clone)]
pub struct ChangeEntry {
    /// The destination path of the change. This may or may not exist yet.
    pub destination: PathBuf,

    /// The operation that will be performed on the path.
    pub operation: EntryOperation,

    /// The current file details of the path. Note, in the case of
    /// renames, this will be the details of the source path.
    pub source: Option<FileDetails>,

    /// The staged file details of the path.
    pub staged: Option<FileDetails>,

    /// A tmp field for use when dealing with renames, this stores
    /// the temporary path used when we go from staged -> tmp -> path
    pub tmp_path: Option<PathBuf>,
}

impl ChangeEntry {
    pub fn to_json(&self) -> Value {
        let mut ret = json!({
            "destination": self.destination.display().to_string(),
            "operation": match &self.operation {
                EntryOperation::Set(SetType::Create) => "create".to_string(),
                EntryOperation::Set(SetType::Modify) => "modify".to_string(),
                EntryOperation::Remove => "remove".to_string(),
                EntryOperation::Rename => "rename".to_string(),
                EntryOperation::Error(_) => "error".to_string(),
            },
            "source": self.source.as_ref().map(|s| s.path.display().to_string()),
            "staged": self.staged.as_ref().map(|s| s.path.display().to_string()),
            "tmp_path": self.tmp_path.as_ref().map(|p| p.display().to_string()),
        });
        if let EntryOperation::Error(error) = &self.operation {
            ret["error"] = match error {
                ChangeError::UnsupportedFileType => {
                    Value::String("unsupported_file_type".to_string())
                }
                ChangeError::RedirectPathNotFound => {
                    Value::String("redirect_path_not_found".to_string())
                }
            };
        }
        ret
    }
}

#[derive(Debug, Clone)]
pub struct ChangeEntries(pub Vec<ChangeEntry>);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ChangeError {
    UnsupportedFileType,
    RedirectPathNotFound,
}

impl std::fmt::Display for ChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeError::UnsupportedFileType => {
                write!(
                    f,
                    "Unsupported file type. Only files, directories, and symlinks are supported."
                )
            }
            ChangeError::RedirectPathNotFound => {
                write!(
                    f,
                    "Redirect path not found. Was the underlying path moved or deleted?"
                )
            }
        }
    }
}

impl ChangeEntry {
    pub fn display(&self, cwd: &PathBuf) -> Result<String> {
        let destination_path = match self.destination.strip_prefix(cwd) {
            Ok(path) => path,
            Err(_) => &self.destination,
        };
        let source_path = match &self.source {
            Some(source) => match source.path.strip_prefix(cwd) {
                Ok(path) => path,
                Err(_) => &source.path,
            },
            None => &self.destination,
        };

        Ok(match &self.operation {
            EntryOperation::Set(added) => {
                if *added == SetType::Create {
                    format!(
                        "   {} {}",
                        "+".green(),
                        destination_path.display().to_string().green()
                    )
                } else {
                    format!(
                        "   {} {}",
                        "~".yellow(),
                        destination_path.display().to_string().yellow()
                    )
                }
            }
            EntryOperation::Remove => format!(
                "   {} {}",
                "-".red(),
                //relative_path.display().to_string().red()
                source_path.display().to_string().red()
            ),
            EntryOperation::Error(e) => format!(
                "   {} {} (error: {})",
                "!".red(),
                destination_path.display().to_string().red(),
                e.to_string().red()
            ),
            EntryOperation::Rename => {
                format!(
                    "   {} {} -> {}",
                    ">".yellow(),
                    source_path.display().to_string().yellow(),
                    destination_path.display().to_string().yellow()
                )
            }
        })
    }

    /* In the case of a Set(Modify) operation, this will check to see if the source and staged
     * paths differ in meaningful ways. For directories, we check metadata (uid, gid, mode).
     * For regular files, we also compare sizes and file contents to detect spurious
     * OverlayFS copy-ups (files opened for writing but not actually modified).
     */
    pub fn is_actually_modified(&self) -> bool {
        if let EntryOperation::Set(SetType::Modify) = &self.operation {
            if let Some(staged) = &self.staged {
                if let Some(source) = &self.source {
                    if staged.stat.st_gid != source.stat.st_gid {
                        debug!(
                            "{}: gid changed from {} to {}",
                            self.destination.display(),
                            source.stat.st_gid,
                            staged.stat.st_gid
                        );
                        return true;
                    }
                    if staged.stat.st_uid != source.stat.st_uid {
                        debug!(
                            "{}: uid changed from {} to {}",
                            self.destination.display(),
                            source.stat.st_uid,
                            staged.stat.st_uid
                        );
                        return true;
                    }
                    if staged.stat.st_mode != source.stat.st_mode {
                        debug!(
                            "{}: mode changed from {} to {}",
                            self.destination.display(),
                            source.stat.st_mode,
                            staged.stat.st_mode
                        );
                        return true;
                    }

                    if staged.is_dir() {
                        return false;
                    }

                    if staged.is_file() && source.is_file() {
                        if staged.stat.st_size != source.stat.st_size {
                            debug!(
                                "{}: size changed from {} to {}",
                                self.destination.display(),
                                source.stat.st_size,
                                staged.stat.st_size
                            );
                            return true;
                        }
                        return !files_have_equal_contents(
                            &staged.path,
                            &source.path,
                        );
                    }

                    return true;
                }
            }
        }

        true
    }
}

fn files_have_equal_contents(a: &Path, b: &Path) -> bool {
    let (Ok(fa), Ok(fb)) = (File::open(a), File::open(b)) else {
        return false;
    };
    let mut ra = BufReader::new(fa);
    let mut rb = BufReader::new(fb);
    let mut buf_a = [0u8; 8192];
    let mut buf_b = [0u8; 8192];
    loop {
        let na = match ra.read(&mut buf_a) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let nb = match rb.read(&mut buf_b) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if na != nb || buf_a[..na] != buf_b[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

impl ChangeEntries {
    pub fn iter(&self) -> std::slice::Iter<'_, ChangeEntry> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, ChangeEntry> {
        self.0.iter_mut()
    }

    pub fn sort_by(&mut self, by: fn(&ChangeEntry, &ChangeEntry) -> Ordering) {
        self.0.sort_by(by);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn matching(&self, cwd: &Path, patterns: &[String]) -> ChangeEntries {
        let patterns: Vec<String> = patterns
            .iter()
            .flat_map(|pattern| {
                let negate = pattern.starts_with("!");
                let pattern = if negate {
                    pattern[1..].to_string()
                } else {
                    pattern.to_string()
                };

                let pattern = if pattern.starts_with("/") {
                    pattern.to_string()
                } else {
                    format!("{}/{}", cwd.display(), pattern)
                };

                let path = PathBuf::from(pattern);
                let mut normalized = PathBuf::new();
                for component in path.components() {
                    if component == std::path::Component::ParentDir {
                        normalized.pop();
                    } else {
                        normalized.push(component);
                    }
                }
                let pattern = normalized.display().to_string();

                // Special case for root directory. Other trailing slashes are
                // stripped by PathBuf normalization above.
                let patterns = if pattern == "/" {
                    vec![format!("{}**", pattern)]
                } else {
                    // Check if this pattern + "/" matches the beginning of any change entry's path
                    let pattern_with_slash = format!("{}/", pattern);
                    let is_directory_prefix = self.0.iter().any(|change| {
                        let dest_str = change.destination.display().to_string();
                        dest_str.starts_with(&pattern_with_slash)
                    });
                    if is_directory_prefix {
                        // Return both the directory itself AND its contents
                        vec![pattern.clone(), format!("{}/**", pattern)]
                    } else {
                        vec![pattern]
                    }
                };

                patterns.into_iter().map(move |p| {
                    if negate {
                        format!("!{}", p)
                    } else {
                        p
                    }
                })
            })
            .collect();

        let cwd_str = format!("{}/", cwd.display());
        ChangeEntries(
            self.0
                .iter()
                .filter(|change| {
                    if patterns.is_empty() {
                        change
                            .destination
                            .display()
                            .to_string()
                            .starts_with(&cwd_str)
                    } else {
                        patterns.iter().any(|pattern| {
                            glob_match(
                                pattern,
                                change.destination.display().to_string(),
                            )
                        })
                    }
                })
                .cloned()
                .collect(),
        )
    }

    /* This should match the print logic in status */
    pub fn count_actionable_changes(&self) -> usize {
        self.0
            .iter()
            .filter(|change| match &change.operation {
                EntryOperation::Set(_) => change.is_actually_modified(),
                EntryOperation::Remove | EntryOperation::Rename => true,
                EntryOperation::Error(_) => false,
            })
            .count()
    }

    pub fn calculate_non_matching_count(
        all_changes: &ChangeEntries,
        matching_changes: &ChangeEntries,
    ) -> usize {
        let all_actionable = all_changes.count_actionable_changes();
        let matching_actionable = matching_changes.count_actionable_changes();
        all_actionable.saturating_sub(matching_actionable)
    }
}

#[cfg(test)]
impl ChangeEntry {
    /// Create a simple ChangeEntry for testing purposes
    pub fn test_entry(path: &Path) -> Self {
        Self {
            destination: path.to_path_buf(),
            operation: EntryOperation::Set(SetType::Modify),
            source: None,
            staged: None,
            tmp_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern() {
        let assert_pattern_matches =
            |paths: &[&str], patterns: &[&str], expected_matches: &[&str]| {
                let change_entries = ChangeEntries(
                    paths
                        .iter()
                        .map(|path| ChangeEntry {
                            destination: PathBuf::from(path),
                            operation: EntryOperation::Set(SetType::Modify),
                            source: None,
                            staged: None,
                            tmp_path: None,
                        })
                        .collect(),
                );

                let cwd = PathBuf::from("/relative");
                let patterns: Vec<String> = patterns
                    .iter()
                    .map(|pattern| pattern.to_string())
                    .collect();
                let changes = change_entries.matching(&cwd, &patterns);
                assert_eq!(changes.0.len(), expected_matches.len());
                for (i, expected) in expected_matches.iter().enumerate() {
                    assert_eq!(
                        changes.0[i].destination,
                        PathBuf::from(expected)
                    );
                }
            };

        assert_pattern_matches(
            &["/relative/path/to/file.rs", "/absolute/path/to/file.rs"],
            &["**/*.rs"],
            &["/relative/path/to/file.rs"],
        );
        assert_pattern_matches(
            &["/relative/path/to/file.rs", "/absolute/path/to/file.rs"],
            &["path/to/file.rs"],
            &["/relative/path/to/file.rs"],
        );
        assert_pattern_matches(
            &["/relative/path/to/file.rs", "/absolute/path/to/file.rs"],
            &["/absolute/path/to/file.rs"],
            &["/absolute/path/to/file.rs"],
        );
        assert_pattern_matches(
            &["/relative/path/to/file.rs", "/absolute/path/to/file.rs"],
            &["/**/*.rs"],
            &["/relative/path/to/file.rs", "/absolute/path/to/file.rs"],
        );
        assert_pattern_matches(
            &["/relative/path/to/file.rs", "/absolute/path/to/file.rs"],
            &["!/absolute/**/*.rs"],
            &["/relative/path/to/file.rs"],
        );
        assert_pattern_matches(
            &["/relative/src/foo1.c", "/relative/src/foo2.c"],
            &["src/"],
            &["/relative/src/foo1.c", "/relative/src/foo2.c"],
        );
        assert_pattern_matches(
            &[
                "/relative/src/foo1.c",
                "/relative/src/foo2.c",
                "/up/a/dir/bar/baz.c",
            ],
            &["../up/a"],
            &["/up/a/dir/bar/baz.c"],
        );
        assert_pattern_matches(
            &[
                "/relative/src/foo1.c",
                "/relative/src/foo2.c",
                "/up/a/dir/bar/baz.c",
                "/up/another/dir/bar/baz.c",
            ],
            &["../up/a"],
            &["/up/a/dir/bar/baz.c"],
        );
        assert_pattern_matches(
            &[
                "/relative/src/foo1.c",
                "/relative/src/foo2.c",
                "/up/a/dir/bar/baz.c",
                "/up/another/dir/bar/baz.c",
            ],
            &["**/baz.c"],
            &[],
        );
    }

    #[test]
    fn test_change_entry_display() {
        let path = PathBuf::from("/");
        let tmp = FileDetails::from_path(&path).unwrap().unwrap();

        let mut change_entry = ChangeEntry {
            destination: PathBuf::from("/root/test"),
            operation: EntryOperation::Set(SetType::Create),
            source: Some(tmp.clone()),
            staged: Some(tmp.clone()),
            tmp_path: None,
        };

        let result = change_entry.display(&PathBuf::from("/"));
        assert!(result.unwrap().contains("+"));

        change_entry.operation = EntryOperation::Set(SetType::Modify);
        let result = change_entry.display(&PathBuf::from("/"));
        assert!(result.unwrap().contains("~"));

        change_entry.operation = EntryOperation::Remove;
        let result = change_entry.display(&PathBuf::from("/"));
        assert!(result.unwrap().contains("-"));

        change_entry.operation = EntryOperation::Rename;
        let result = change_entry.display(&PathBuf::from("/"));
        assert!(result.unwrap().contains(">"));

        change_entry.operation =
            EntryOperation::Error(ChangeError::UnsupportedFileType);
        let result = change_entry.display(&PathBuf::from("/"));
        assert!(result.unwrap().contains("!"));

        // rename
        change_entry.operation = EntryOperation::Rename;
        let result = change_entry.display(&PathBuf::from("/"));
        assert!(result.unwrap().contains(">"));

        change_entry.operation = EntryOperation::Rename;
        let result = change_entry.display(&PathBuf::from("/"));
        assert!(result.unwrap().contains(">"));
    }

    /* Rounds out some coverage the is_actually_modified() method where we don't
     * call it in a way that these cases would get exercised normally */
    #[test]
    fn test_change_entries_is_actually_modified() {
        let path = PathBuf::from("/");
        let tmp = FileDetails::from_path(&path).unwrap().unwrap();

        let change_entry = ChangeEntry {
            destination: PathBuf::from("/root/test"),
            operation: EntryOperation::Set(SetType::Modify),
            source: Some(tmp.clone()),
            staged: None,
            tmp_path: None,
        };

        assert!(change_entry.is_actually_modified());

        let change_entry = ChangeEntry {
            destination: PathBuf::from("/root/test"),
            operation: EntryOperation::Set(SetType::Modify),
            source: None,
            staged: Some(tmp.clone()),
            tmp_path: None,
        };

        assert!(change_entry.is_actually_modified());
    }
}
