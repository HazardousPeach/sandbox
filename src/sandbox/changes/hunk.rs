use std::path::PathBuf;

use super::ChangeEntry;

/// Represents a single line in a diff hunk
#[derive(Debug, Clone)]
pub enum HunkLine {
    /// Context line (unchanged)
    Context(String),
    /// Added line
    Added(String),
    /// Removed line
    Removed(String),
}

/// Represents a single hunk from a unified diff
#[derive(Debug, Clone)]
pub struct DiffHunk {
    /// The hunk index within the file (0-indexed)
    #[allow(dead_code)]
    pub index: usize,
    /// Header line (e.g., "@@ -1,5 +1,7 @@")
    pub header: String,
    /// Original line range (start, count)
    pub original_range: (usize, usize),
    /// New line range (start, count)
    pub new_range: (usize, usize),
    /// The lines of the hunk
    pub lines: Vec<HunkLine>,
}

impl DiffHunk {
    /// Check if this hunk can be split into smaller hunks
    /// A hunk can be split if it contains multiple change regions separated by context lines
    pub fn can_split(&self) -> bool {
        let mut in_change_region = false;
        let mut change_regions = 0;

        for line in &self.lines {
            match line {
                HunkLine::Context(_) => {
                    if in_change_region {
                        in_change_region = false;
                    }
                }
                HunkLine::Added(_) | HunkLine::Removed(_) => {
                    if !in_change_region {
                        in_change_region = true;
                        change_regions += 1;
                    }
                }
            }
        }

        change_regions > 1
    }

    /// Split this hunk into smaller hunks if possible
    /// Returns None if the hunk cannot be split
    pub fn split(&self) -> Option<Vec<DiffHunk>> {
        if !self.can_split() {
            return None;
        }

        let mut result = Vec::new();
        let mut current_lines: Vec<HunkLine> = Vec::new();
        let mut current_original_start = self.original_range.0;
        let mut current_new_start = self.new_range.0;
        let mut original_offset = 0usize;
        let mut new_offset = 0usize;
        let mut pending_context: Vec<HunkLine> = Vec::new();
        let mut in_change_region = false;

        for line in &self.lines {
            match line {
                HunkLine::Context(content) => {
                    if in_change_region {
                        // End of a change region - emit hunk with trailing context
                        pending_context
                            .push(HunkLine::Context(content.clone()));

                        // If we have 3+ context lines, the middle ones become leading context
                        // for the next hunk. Keep up to 3 lines of trailing context.
                        if pending_context.len() >= 6 {
                            // Finalize current hunk with 3 trailing context lines
                            for ctx in pending_context.drain(..3) {
                                current_lines.push(ctx);
                                original_offset += 1;
                                new_offset += 1;
                            }

                            let original_count = current_lines
                                .iter()
                                .filter(|l| !matches!(l, HunkLine::Added(_)))
                                .count();
                            let new_count = current_lines
                                .iter()
                                .filter(|l| !matches!(l, HunkLine::Removed(_)))
                                .count();

                            result.push(DiffHunk {
                                index: result.len(),
                                header: format!(
                                    "@@ -{},{} +{},{} @@",
                                    current_original_start,
                                    original_count,
                                    current_new_start,
                                    new_count
                                ),
                                original_range: (
                                    current_original_start,
                                    original_count,
                                ),
                                new_range: (current_new_start, new_count),
                                lines: std::mem::take(&mut current_lines),
                            });

                            // Skip middle context lines, keep last 3 as leading context for next hunk
                            let skip_count =
                                pending_context.len().saturating_sub(3);
                            for _ in 0..skip_count {
                                pending_context.remove(0);
                                original_offset += 1;
                                new_offset += 1;
                            }

                            current_original_start =
                                self.original_range.0 + original_offset;
                            current_new_start = self.new_range.0 + new_offset;

                            // Add remaining context as leading context for next hunk
                            for ctx in pending_context.drain(..) {
                                current_lines.push(ctx);
                            }

                            in_change_region = false;
                        }
                    } else {
                        // Not in change region - accumulate as potential leading context
                        pending_context
                            .push(HunkLine::Context(content.clone()));
                        // Keep only last 3 context lines as leading context
                        if pending_context.len() > 3 {
                            pending_context.remove(0);
                            original_offset += 1;
                            new_offset += 1;
                            current_original_start =
                                self.original_range.0 + original_offset;
                            current_new_start = self.new_range.0 + new_offset;
                        }
                    }
                }
                HunkLine::Added(content) => {
                    if !in_change_region {
                        // Start of change region - flush pending context as leading context
                        for ctx in pending_context.drain(..) {
                            current_lines.push(ctx);
                        }
                        in_change_region = true;
                    } else if !pending_context.is_empty() {
                        // Continuation after some context - add pending context
                        for ctx in pending_context.drain(..) {
                            current_lines.push(ctx);
                            original_offset += 1;
                            new_offset += 1;
                        }
                    }
                    current_lines.push(HunkLine::Added(content.clone()));
                    new_offset += 1;
                }
                HunkLine::Removed(content) => {
                    if !in_change_region {
                        // Start of change region - flush pending context as leading context
                        for ctx in pending_context.drain(..) {
                            current_lines.push(ctx);
                        }
                        in_change_region = true;
                    } else if !pending_context.is_empty() {
                        // Continuation after some context - add pending context
                        for ctx in pending_context.drain(..) {
                            current_lines.push(ctx);
                            original_offset += 1;
                            new_offset += 1;
                        }
                    }
                    current_lines.push(HunkLine::Removed(content.clone()));
                    original_offset += 1;
                }
            }
        }

        // Emit final hunk if we have changes
        if !current_lines.is_empty()
            || current_lines
                .iter()
                .any(|l| matches!(l, HunkLine::Added(_) | HunkLine::Removed(_)))
        {
            // Add any remaining trailing context
            for ctx in pending_context {
                current_lines.push(ctx);
            }

            let original_count = current_lines
                .iter()
                .filter(|l| !matches!(l, HunkLine::Added(_)))
                .count();
            let new_count = current_lines
                .iter()
                .filter(|l| !matches!(l, HunkLine::Removed(_)))
                .count();

            if original_count > 0 || new_count > 0 {
                result.push(DiffHunk {
                    index: result.len(),
                    header: format!(
                        "@@ -{},{} +{},{} @@",
                        current_original_start,
                        original_count,
                        current_new_start,
                        new_count
                    ),
                    original_range: (current_original_start, original_count),
                    new_range: (current_new_start, new_count),
                    lines: current_lines,
                });
            }
        }

        if result.len() > 1 { Some(result) } else { None }
    }

    /// Format this hunk for display
    pub fn format_display(&self, colorize: bool) -> String {
        use colored::Colorize;

        let mut output = String::new();

        if colorize {
            output.push_str(&self.header.cyan().to_string());
        } else {
            output.push_str(&self.header);
        }
        output.push('\n');

        for line in &self.lines {
            match line {
                HunkLine::Context(content) => {
                    output.push(' ');
                    output.push_str(content);
                    output.push('\n');
                }
                HunkLine::Added(content) => {
                    if colorize {
                        output.push_str(
                            &format!("+{}", content).green().to_string(),
                        );
                    } else {
                        output.push('+');
                        output.push_str(content);
                    }
                    output.push('\n');
                }
                HunkLine::Removed(content) => {
                    if colorize {
                        output.push_str(
                            &format!("-{}", content).red().to_string(),
                        );
                    } else {
                        output.push('-');
                        output.push_str(content);
                    }
                    output.push('\n');
                }
            }
        }

        output
    }
}

/// Represents all hunks for a single file change
#[derive(Debug)]
pub struct FileHunks {
    /// Path to the file (destination path)
    pub path: PathBuf,
    /// The original (source/lower) file content
    pub original_content: Option<Vec<u8>>,
    /// The modified (staged/upper) file content
    pub modified_content: Option<Vec<u8>>,
    /// List of hunks
    pub hunks: Vec<DiffHunk>,
    /// Whether this is a binary file
    pub is_binary: bool,
    /// Associated ChangeEntry
    pub change_entry: ChangeEntry,
}

impl FileHunks {
    /// Check if content appears to be binary
    pub fn is_binary_content(content: &[u8]) -> bool {
        // Check for null bytes which indicate binary content
        content.contains(&0)
    }
}

/// Selection state for a hunk during interactive mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkSelection {
    /// Accept this hunk
    Accept,
    /// Skip/reject this hunk
    Skip,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hunk_can_split_single_change() {
        // A hunk with a single change region cannot be split
        let hunk = DiffHunk {
            index: 0,
            header: "@@ -1,3 +1,4 @@".to_string(),
            original_range: (1, 3),
            new_range: (1, 4),
            lines: vec![
                HunkLine::Context("line1".to_string()),
                HunkLine::Added("new line".to_string()),
                HunkLine::Context("line2".to_string()),
            ],
        };
        assert!(!hunk.can_split());
    }

    #[test]
    fn test_hunk_can_split_multiple_changes() {
        // A hunk with multiple change regions separated by context can be split
        let hunk = DiffHunk {
            index: 0,
            header: "@@ -1,10 +1,12 @@".to_string(),
            original_range: (1, 10),
            new_range: (1, 12),
            lines: vec![
                HunkLine::Context("line1".to_string()),
                HunkLine::Added("new line 1".to_string()),
                HunkLine::Context("line2".to_string()),
                HunkLine::Context("line3".to_string()),
                HunkLine::Context("line4".to_string()),
                HunkLine::Context("line5".to_string()),
                HunkLine::Context("line6".to_string()),
                HunkLine::Context("line7".to_string()),
                HunkLine::Added("new line 2".to_string()),
                HunkLine::Context("line8".to_string()),
            ],
        };
        assert!(hunk.can_split());
    }

    #[test]
    fn test_hunk_format_display() {
        let hunk = DiffHunk {
            index: 0,
            header: "@@ -1,3 +1,4 @@".to_string(),
            original_range: (1, 3),
            new_range: (1, 4),
            lines: vec![
                HunkLine::Context("unchanged".to_string()),
                HunkLine::Removed("old line".to_string()),
                HunkLine::Added("new line".to_string()),
            ],
        };

        let display = hunk.format_display(false);
        assert!(display.contains("@@ -1,3 +1,4 @@"));
        assert!(display.contains(" unchanged"));
        assert!(display.contains("-old line"));
        assert!(display.contains("+new line"));
    }

    #[test]
    fn test_hunk_line_types() {
        let context = HunkLine::Context("test".to_string());
        let added = HunkLine::Added("test".to_string());
        let removed = HunkLine::Removed("test".to_string());

        assert!(matches!(context, HunkLine::Context(_)));
        assert!(matches!(added, HunkLine::Added(_)));
        assert!(matches!(removed, HunkLine::Removed(_)));
    }

    #[test]
    fn test_file_hunks_binary_detection() {
        // Text content
        assert!(!FileHunks::is_binary_content(b"hello world\n"));
        assert!(!FileHunks::is_binary_content(b"line1\nline2\nline3"));

        // Binary content (contains null byte)
        assert!(FileHunks::is_binary_content(b"hello\x00world"));
        assert!(FileHunks::is_binary_content(b"\x00"));
    }
}
