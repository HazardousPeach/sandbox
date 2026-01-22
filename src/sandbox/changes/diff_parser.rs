use std::path::Path;

use anyhow::{Context, Result};
use diffy::DiffOptions;

use super::ChangeEntry;
use super::hunk::{DiffHunk, FileHunks, HunkLine};

/// Parse file contents into hunks using diffy
pub fn parse_file_to_hunks(
    original: &[u8],
    modified: &[u8],
    path: &Path,
    change_entry: ChangeEntry,
) -> Result<FileHunks> {
    // Check for binary content
    if FileHunks::is_binary_content(original)
        || FileHunks::is_binary_content(modified)
    {
        return Ok(FileHunks {
            path: path.to_path_buf(),
            original_content: Some(original.to_vec()),
            modified_content: Some(modified.to_vec()),
            hunks: Vec::new(),
            is_binary: true,
            change_entry,
        });
    }

    // Convert to strings for text diffing
    let original_str = String::from_utf8_lossy(original).into_owned();
    let modified_str = String::from_utf8_lossy(modified).into_owned();

    // Create patch using diffy with context lines
    let patch = create_patch_with_context(&original_str, &modified_str, 3);

    // Parse the patch into our hunk structures
    let hunks = parse_patch_to_hunks(&patch)?;

    Ok(FileHunks {
        path: path.to_path_buf(),
        original_content: Some(original.to_vec()),
        modified_content: Some(modified.to_vec()),
        hunks,
        is_binary: false,
        change_entry,
    })
}

/// Create a patch with specified context lines
fn create_patch_with_context(
    original: &str,
    modified: &str,
    context: usize,
) -> String {
    let mut opts = DiffOptions::new();
    opts.set_context_len(context);
    let patch = opts.create_patch(original, modified);
    patch.to_string()
}

/// Parse a unified diff patch string into DiffHunk structures
fn parse_patch_to_hunks(patch_str: &str) -> Result<Vec<DiffHunk>> {
    let mut hunks = Vec::new();
    let mut current_hunk: Option<DiffHunk> = None;
    let mut hunk_index = 0;

    for line in patch_str.lines() {
        if line.starts_with("@@") {
            // Save previous hunk if any
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }

            // Parse hunk header: @@ -start,count +start,count @@
            let (orig_range, new_range) = parse_hunk_header(line)
                .context("Failed to parse hunk header")?;

            current_hunk = Some(DiffHunk {
                index: hunk_index,
                header: line.to_string(),
                original_range: orig_range,
                new_range,
                lines: Vec::new(),
            });
            hunk_index += 1;
        } else if line.starts_with("---") || line.starts_with("+++") {
            // Skip file headers
            continue;
        } else if let Some(ref mut hunk) = current_hunk {
            // Parse hunk content
            if let Some(content) = line.strip_prefix('+') {
                hunk.lines.push(HunkLine::Added(content.to_string()));
            } else if let Some(content) = line.strip_prefix('-') {
                hunk.lines.push(HunkLine::Removed(content.to_string()));
            } else if let Some(content) = line.strip_prefix(' ') {
                hunk.lines.push(HunkLine::Context(content.to_string()));
            } else if line.is_empty() || line == "\\ No newline at end of file"
            {
                // Handle empty context lines or no-newline marker
                if line.is_empty() {
                    hunk.lines.push(HunkLine::Context(String::new()));
                }
            } else {
                // Treat as context line (some diffs don't prefix context with space)
                hunk.lines.push(HunkLine::Context(line.to_string()));
            }
        }
    }

    // Save last hunk
    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    Ok(hunks)
}

/// Parse a hunk header like "@@ -1,5 +1,7 @@" into ranges
fn parse_hunk_header(header: &str) -> Result<((usize, usize), (usize, usize))> {
    // Format: @@ -start,count +start,count @@ optional_context
    let header = header.trim_start_matches("@@").trim();

    let parts: Vec<&str> = header.split_whitespace().collect();
    if parts.len() < 2 {
        anyhow::bail!("Invalid hunk header: {}", header);
    }

    let orig_range =
        parse_range(parts[0]).context("Failed to parse original range")?;
    let new_range =
        parse_range(parts[1]).context("Failed to parse new range")?;

    Ok((orig_range, new_range))
}

/// Parse a range like "-1,5" or "+1,7" into (start, count)
fn parse_range(range_str: &str) -> Result<(usize, usize)> {
    let range_str = range_str.trim_start_matches(['-', '+']);

    if let Some((start, count)) = range_str.split_once(',') {
        Ok((start.parse()?, count.parse()?))
    } else {
        // Single line: "-1" means start=1, count=1
        Ok((range_str.parse()?, 1))
    }
}

/// Create hunks for a new file (entire file is one "added" hunk)
pub fn create_new_file_hunks(
    content: &[u8],
    path: &Path,
    change_entry: ChangeEntry,
) -> Result<FileHunks> {
    if FileHunks::is_binary_content(content) {
        return Ok(FileHunks {
            path: path.to_path_buf(),
            original_content: None,
            modified_content: Some(content.to_vec()),
            hunks: Vec::new(),
            is_binary: true,
            change_entry,
        });
    }

    let content_str = String::from_utf8_lossy(content);
    let lines: Vec<&str> = content_str.lines().collect();

    let hunk_lines: Vec<HunkLine> = lines
        .iter()
        .map(|l| HunkLine::Added((*l).to_string()))
        .collect();

    let line_count = hunk_lines.len();

    let hunk = DiffHunk {
        index: 0,
        header: format!("@@ -0,0 +1,{} @@", line_count),
        original_range: (0, 0),
        new_range: (1, line_count),
        lines: hunk_lines,
    };

    Ok(FileHunks {
        path: path.to_path_buf(),
        original_content: None,
        modified_content: Some(content.to_vec()),
        hunks: vec![hunk],
        is_binary: false,
        change_entry,
    })
}

/// Create hunks for a deleted file (entire file is one "removed" hunk)
pub fn create_deleted_file_hunks(
    content: &[u8],
    path: &Path,
    change_entry: ChangeEntry,
) -> Result<FileHunks> {
    if FileHunks::is_binary_content(content) {
        return Ok(FileHunks {
            path: path.to_path_buf(),
            original_content: Some(content.to_vec()),
            modified_content: None,
            hunks: Vec::new(),
            is_binary: true,
            change_entry,
        });
    }

    let content_str = String::from_utf8_lossy(content);
    let lines: Vec<&str> = content_str.lines().collect();

    let hunk_lines: Vec<HunkLine> = lines
        .iter()
        .map(|l| HunkLine::Removed((*l).to_string()))
        .collect();

    let line_count = hunk_lines.len();

    let hunk = DiffHunk {
        index: 0,
        header: format!("@@ -1,{} +0,0 @@", line_count),
        original_range: (1, line_count),
        new_range: (0, 0),
        lines: hunk_lines,
    };

    Ok(FileHunks {
        path: path.to_path_buf(),
        original_content: Some(content.to_vec()),
        modified_content: None,
        hunks: vec![hunk],
        is_binary: false,
        change_entry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hunk_header() {
        let (orig, new) = parse_hunk_header("@@ -1,5 +1,7 @@").unwrap();
        assert_eq!(orig, (1, 5));
        assert_eq!(new, (1, 7));
    }

    #[test]
    fn test_parse_hunk_header_single_line() {
        let (orig, new) = parse_hunk_header("@@ -1 +1 @@").unwrap();
        assert_eq!(orig, (1, 1));
        assert_eq!(new, (1, 1));
    }

    #[test]
    fn test_parse_range() {
        assert_eq!(parse_range("-1,5").unwrap(), (1, 5));
        assert_eq!(parse_range("+10,20").unwrap(), (10, 20));
        assert_eq!(parse_range("-1").unwrap(), (1, 1));
    }

    #[test]
    fn test_parse_file_to_hunks_simple() {
        let original = b"line1\nline2\nline3\n";
        let modified = b"line1\nmodified\nline3\n";

        let path = Path::new("/test/file.txt");
        let change_entry = ChangeEntry::test_entry(path);

        let file_hunks =
            parse_file_to_hunks(original, modified, path, change_entry)
                .unwrap();

        assert!(!file_hunks.is_binary);
        assert_eq!(file_hunks.hunks.len(), 1);

        let hunk = &file_hunks.hunks[0];
        assert!(
            hunk.lines
                .iter()
                .any(|l| matches!(l, HunkLine::Removed(s) if s == "line2"))
        );
        assert!(
            hunk.lines
                .iter()
                .any(|l| matches!(l, HunkLine::Added(s) if s == "modified"))
        );
    }

    #[test]
    fn test_binary_detection() {
        let binary_content = b"hello\x00world";
        assert!(FileHunks::is_binary_content(binary_content));

        let text_content = b"hello world\n";
        assert!(!FileHunks::is_binary_content(text_content));
    }
}
