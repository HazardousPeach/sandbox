use std::fs;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::Config;
use crate::outln;
use crate::sandbox::Sandbox;
use crate::sandbox::changes::{
    ChangeEntry, EntryOperation, FileHunks, HunkSelection, SetType,
    changes::determine_scan_directories,
    diff_parser::{
        create_deleted_file_hunks, create_new_file_hunks, parse_file_to_hunks,
    },
};
use crate::util::sync_and_drop_caches;

/// Commands available during interactive mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractiveCommand {
    Yes,      // y - accept/reject this hunk
    No,       // n - skip this hunk
    Quit,     // q - quit (apply selections so far)
    All,      // a - accept/reject all remaining hunks in file
    Done,     // d - done with file, skip remaining hunks
    Edit,     // e - manually edit this hunk
    Split,    // s - split hunk (if possible)
    Help,     // ? - show help
    NextFile, // j - skip to next file
    PrevFile, // k - go back to previous file
}

impl InteractiveCommand {
    fn from_char(c: char) -> Option<Self> {
        match c {
            'y' | 'Y' => Some(Self::Yes),
            'n' | 'N' => Some(Self::No),
            'q' | 'Q' => Some(Self::Quit),
            'a' | 'A' => Some(Self::All),
            'd' | 'D' => Some(Self::Done),
            'e' | 'E' => Some(Self::Edit),
            's' | 'S' => Some(Self::Split),
            '?' | 'h' | 'H' => Some(Self::Help),
            'j' | 'J' => Some(Self::NextFile),
            'k' | 'K' => Some(Self::PrevFile),
            _ => None,
        }
    }
}

/// Interactive accept mode - select individual hunks to accept
pub fn accept_interactive(
    config: &Config,
    sandbox: &Sandbox,
    patterns: &[String],
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let scan_dirs = determine_scan_directories(&cwd, patterns);
    let all_changes = sandbox.changes_in_directories(&scan_dirs, config.ignored)?;
    let changes = all_changes.matching(&cwd, patterns);

    if changes.is_empty() {
        outln!("No changes in this directory to accept");
        return Ok(());
    }

    // Convert changes to FileHunks
    let file_hunks_list = changes_to_file_hunks(changes.iter())?;

    if file_hunks_list.is_empty() {
        outln!("No text changes available for interactive selection");
        return Ok(());
    }

    // Run interactive session
    let selections = run_interactive_session(&file_hunks_list, "accept")?;

    // Apply the selections
    apply_accept_selections(sandbox, &file_hunks_list, &selections)?;

    sync_and_drop_caches()?;

    Ok(())
}

/// Interactive reject mode - select individual hunks to reject
pub fn reject_interactive(
    config: &Config,
    sandbox: &Sandbox,
    patterns: &[String],
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let scan_dirs = determine_scan_directories(&cwd, patterns);
    let all_changes = sandbox.changes_in_directories(&scan_dirs, config.ignored)?;
    let changes = all_changes.matching(&cwd, patterns);

    if changes.is_empty() {
        outln!("No changes in this directory to reject");
        return Ok(());
    }

    // Convert changes to FileHunks
    let file_hunks_list = changes_to_file_hunks(changes.iter())?;

    if file_hunks_list.is_empty() {
        outln!("No text changes available for interactive selection");
        return Ok(());
    }

    // Run interactive session
    let selections = run_interactive_session(&file_hunks_list, "reject")?;

    // Apply the selections
    apply_reject_selections(sandbox, &file_hunks_list, &selections)?;

    sync_and_drop_caches()?;

    Ok(())
}

/// Convert ChangeEntries to FileHunks for interactive processing
fn changes_to_file_hunks<'a>(
    changes: impl Iterator<Item = &'a ChangeEntry>,
) -> Result<Vec<FileHunks>> {
    let mut result = Vec::new();

    for change in changes {
        match &change.operation {
            EntryOperation::Set(set_type) => {
                let staged = change.staged.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Set operation missing staged file for {}",
                        change.destination.display()
                    )
                })?;

                // Skip directories - they can't be hunked
                if staged.is_dir() {
                    continue;
                }

                // Skip symlinks for now
                if staged.is_symlink() {
                    continue;
                }

                let modified_content =
                    fs::read(&staged.path).context(format!(
                        "Failed to read staged file {}",
                        staged.path.display()
                    ))?;

                let file_hunks = match set_type {
                    SetType::Create => {
                        // New file - entire content is one hunk
                        create_new_file_hunks(
                            &modified_content,
                            &change.destination,
                            change.clone(),
                        )?
                    }
                    SetType::Modify => {
                        // Modified file - read original and generate diff
                        let original_content = fs::read(&change.destination)
                            .context(format!(
                                "Failed to read original file {}",
                                change.destination.display()
                            ))?;

                        parse_file_to_hunks(
                            &original_content,
                            &modified_content,
                            &change.destination,
                            change.clone(),
                        )?
                    }
                };

                // Skip files with no hunks (binary or no changes)
                if !file_hunks.hunks.is_empty() || file_hunks.is_binary {
                    result.push(file_hunks);
                }
            }
            EntryOperation::Remove => {
                // Deleted file - read original content
                if let Some(source) = &change.source {
                    if source.is_dir() || source.is_symlink() {
                        continue;
                    }

                    let original_content =
                        fs::read(&source.path).context(format!(
                            "Failed to read source file {}",
                            source.path.display()
                        ))?;

                    let file_hunks = create_deleted_file_hunks(
                        &original_content,
                        &change.destination,
                        change.clone(),
                    )?;

                    if !file_hunks.hunks.is_empty() || file_hunks.is_binary {
                        result.push(file_hunks);
                    }
                }
            }
            EntryOperation::Rename => {
                // For renames with content changes, we'd need to handle both
                // For now, skip - they'll be handled as whole-file operations
                continue;
            }
            EntryOperation::Error(_) => {
                // Skip errors
                continue;
            }
        }
    }

    Ok(result)
}

/// Result of running an interactive session
struct SelectionResult {
    /// Map of file index to vec of (hunk_index, selection)
    selections: Vec<Vec<HunkSelection>>,
    /// Whether the user quit early (reserved for future use)
    #[allow(dead_code)]
    quit_early: bool,
}

/// Run the interactive hunk selection session
fn run_interactive_session(
    file_hunks_list: &[FileHunks],
    action: &str,
) -> Result<SelectionResult> {
    let mut selections: Vec<Vec<HunkSelection>> = file_hunks_list
        .iter()
        .map(|fh| vec![HunkSelection::Skip; fh.hunks.len().max(1)])
        .collect();

    let mut file_idx = 0;
    let mut quit_early = false;

    while file_idx < file_hunks_list.len() {
        let file_hunks = &file_hunks_list[file_idx];
        let file_count = file_hunks_list.len();

        // Handle binary files
        if file_hunks.is_binary {
            eprint!(
                "\n{} {} (binary file {}/{}) {} entire file? [y,n,q,j,k,?] ",
                action.to_uppercase().yellow(),
                file_hunks.path.display().to_string().cyan(),
                file_idx + 1,
                file_count,
                action
            );
            io::stderr().flush()?;

            match read_command()? {
                Some(InteractiveCommand::Yes) => {
                    selections[file_idx][0] = HunkSelection::Accept;
                    file_idx += 1;
                }
                Some(InteractiveCommand::No) => {
                    file_idx += 1;
                }
                Some(InteractiveCommand::Quit) => {
                    quit_early = true;
                    break;
                }
                Some(InteractiveCommand::NextFile) => {
                    file_idx += 1;
                }
                Some(InteractiveCommand::PrevFile) => {
                    file_idx = file_idx.saturating_sub(1);
                }
                Some(InteractiveCommand::Help) => {
                    print_help(action);
                    continue;
                }
                _ => continue,
            }
            continue;
        }

        // Process hunks for this file
        let mut hunk_idx = 0;
        let mut hunks = file_hunks.hunks.clone();

        while hunk_idx < hunks.len() {
            let hunk = &hunks[hunk_idx];
            let hunk_count = hunks.len();

            // Display file and hunk info
            eprintln!(
                "\n{}: {} ({}/{})",
                "File".bright_white(),
                file_hunks.path.display().to_string().cyan(),
                file_idx + 1,
                file_count
            );
            eprintln!(
                "{} {}/{}:",
                "Hunk".bright_white(),
                hunk_idx + 1,
                hunk_count
            );

            // Display the hunk
            eprint!("{}", hunk.format_display(true));

            // Prompt for action
            let can_split = hunk.can_split();
            if can_split {
                eprint!(
                    "{} this hunk? [y,n,q,a,d,e,s,j,k,?] ",
                    action.to_uppercase().yellow()
                );
            } else {
                eprint!(
                    "{} this hunk? [y,n,q,a,d,e,j,k,?] ",
                    action.to_uppercase().yellow()
                );
            }
            io::stderr().flush()?;

            match read_command()? {
                Some(InteractiveCommand::Yes) => {
                    // Ensure selections vec is large enough
                    while selections[file_idx].len() <= hunk_idx {
                        selections[file_idx].push(HunkSelection::Skip);
                    }
                    selections[file_idx][hunk_idx] = HunkSelection::Accept;
                    hunk_idx += 1;
                }
                Some(InteractiveCommand::No) => {
                    hunk_idx += 1;
                }
                Some(InteractiveCommand::Quit) => {
                    quit_early = true;
                    break;
                }
                Some(InteractiveCommand::All) => {
                    // Accept all remaining hunks in this file
                    for i in hunk_idx..hunks.len() {
                        while selections[file_idx].len() <= i {
                            selections[file_idx].push(HunkSelection::Skip);
                        }
                        selections[file_idx][i] = HunkSelection::Accept;
                    }
                    break;
                }
                Some(InteractiveCommand::Done) => {
                    // Skip remaining hunks in this file
                    break;
                }
                Some(InteractiveCommand::Edit) => {
                    match edit_hunk(hunk, &file_hunks.path) {
                        Ok(Some(edited_hunk)) => {
                            // Replace the current hunk with the edited version
                            hunks[hunk_idx] = edited_hunk;
                            // Re-display the edited hunk and ask again
                            continue;
                        }
                        Ok(None) => {
                            // User cancelled or no changes
                            eprintln!("Edit cancelled or no changes made");
                            continue;
                        }
                        Err(e) => {
                            eprintln!("Error editing hunk: {}", e);
                            continue;
                        }
                    }
                }
                Some(InteractiveCommand::Split) => {
                    if can_split && let Some(split_hunks) = hunk.split() {
                        // Replace current hunk with split hunks
                        hunks.splice(hunk_idx..=hunk_idx, split_hunks);
                        // Resize selections for this file
                        selections[file_idx]
                            .resize(hunks.len(), HunkSelection::Skip);
                        // Don't increment hunk_idx - we'll process the first split hunk
                        continue;
                    }
                    eprintln!("Cannot split this hunk further");
                    continue;
                }
                Some(InteractiveCommand::Help) => {
                    print_help(action);
                    continue;
                }
                Some(InteractiveCommand::NextFile) => {
                    break;
                }
                Some(InteractiveCommand::PrevFile) => {
                    file_idx = file_idx.saturating_sub(1);
                    break;
                }
                None => {
                    eprintln!("Invalid command. Press '?' for help.");
                    continue;
                }
            }
        }

        if quit_early {
            break;
        }

        file_idx += 1;
    }

    Ok(SelectionResult {
        selections,
        quit_early,
    })
}

/// Read a single command from stdin
fn read_command() -> Result<Option<InteractiveCommand>> {
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    Ok(trimmed
        .chars()
        .next()
        .and_then(InteractiveCommand::from_char))
}

/// Edit a hunk using the user's preferred editor
fn edit_hunk(
    hunk: &crate::sandbox::changes::DiffHunk,
    file_path: &Path,
) -> Result<Option<crate::sandbox::changes::DiffHunk>> {
    use crate::sandbox::changes::hunk::HunkLine;
    use std::process::Command;

    // Create a temporary file with the hunk content
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir
        .join(format!("sandbox-hunk-edit-{}.patch", uuid::Uuid::new_v4()));

    // Write the hunk to the temp file with instructions
    let mut content = String::new();
    content.push_str(&format!("# Editing hunk for: {}\n", file_path.display()));
    content.push_str("# \n");
    content.push_str("# Instructions:\n");
    content.push_str(
        "#   - Lines starting with '-' will be removed from the original\n",
    );
    content.push_str("#   - Lines starting with '+' will be added\n");
    content.push_str(
        "#   - Lines starting with ' ' (space) are context (unchanged)\n",
    );
    content.push_str(
        "#   - Lines starting with '#' are comments and will be ignored\n",
    );
    content.push_str(
        "#   - To cancel, delete all non-comment lines or leave unchanged\n",
    );
    content.push_str("# \n");
    content.push_str(&format!("# Original header: {}\n", hunk.header));
    content.push_str("#\n");

    // Write the hunk lines
    for line in &hunk.lines {
        match line {
            HunkLine::Context(s) => {
                content.push(' ');
                content.push_str(s);
                content.push('\n');
            }
            HunkLine::Added(s) => {
                content.push('+');
                content.push_str(s);
                content.push('\n');
            }
            HunkLine::Removed(s) => {
                content.push('-');
                content.push_str(s);
                content.push('\n');
            }
        }
    }

    // Write to temp file
    fs::write(&temp_file, &content)?;

    // Get the editor command
    let editor = get_editor();

    // Open the editor
    let status = Command::new(&editor)
        .arg(&temp_file)
        .status()
        .context(format!("Failed to open editor: {}", editor))?;

    if !status.success() {
        // Clean up temp file
        let _ = fs::remove_file(&temp_file);
        return Ok(None);
    }

    // Read the edited content
    let edited_content = fs::read_to_string(&temp_file)?;

    // Clean up temp file
    let _ = fs::remove_file(&temp_file);

    // Parse the edited content
    parse_edited_hunk(&edited_content, hunk)
}

/// Get the user's preferred editor
fn get_editor() -> String {
    // Check VISUAL first, then EDITOR, then fall back to common editors
    if let Ok(editor) = std::env::var("VISUAL") {
        if !editor.is_empty() {
            return editor;
        }
    }

    if let Ok(editor) = std::env::var("EDITOR") {
        if !editor.is_empty() {
            return editor;
        }
    }

    // Try to find a common editor
    for editor in ["nano", "vim", "vi", "emacs"] {
        if std::process::Command::new("which")
            .arg(editor)
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return editor.to_string();
        }
    }

    // Last resort
    "vi".to_string()
}

/// Parse edited hunk content back into a DiffHunk
fn parse_edited_hunk(
    content: &str,
    original: &crate::sandbox::changes::DiffHunk,
) -> Result<Option<crate::sandbox::changes::DiffHunk>> {
    use crate::sandbox::changes::hunk::{DiffHunk, HunkLine};

    let mut lines: Vec<HunkLine> = Vec::new();
    let mut has_changes = false;

    for line in content.lines() {
        // Skip comments
        if line.starts_with('#') {
            continue;
        }

        // Empty lines in the diff should be treated as context
        if line.is_empty() {
            lines.push(HunkLine::Context(String::new()));
            continue;
        }

        let first_char = line.chars().next().unwrap_or(' ');
        let rest = if line.len() > 1 { &line[1..] } else { "" };

        match first_char {
            ' ' => {
                lines.push(HunkLine::Context(rest.to_string()));
            }
            '+' => {
                lines.push(HunkLine::Added(rest.to_string()));
                has_changes = true;
            }
            '-' => {
                lines.push(HunkLine::Removed(rest.to_string()));
                has_changes = true;
            }
            _ => {
                // Treat unrecognized prefix as context
                lines.push(HunkLine::Context(line.to_string()));
            }
        }
    }

    // If no lines or no changes, return None
    if lines.is_empty() || !has_changes {
        return Ok(None);
    }

    // Check if the content is unchanged
    if lines.len() == original.lines.len() {
        let unchanged =
            lines.iter().zip(original.lines.iter()).all(|(new, old)| {
                match (new, old) {
                    (HunkLine::Context(a), HunkLine::Context(b)) => a == b,
                    (HunkLine::Added(a), HunkLine::Added(b)) => a == b,
                    (HunkLine::Removed(a), HunkLine::Removed(b)) => a == b,
                    _ => false,
                }
            });

        if unchanged {
            return Ok(None);
        }
    }

    // Calculate new ranges
    let original_count = lines
        .iter()
        .filter(|l| !matches!(l, HunkLine::Added(_)))
        .count();
    let new_count = lines
        .iter()
        .filter(|l| !matches!(l, HunkLine::Removed(_)))
        .count();

    // Create the new hunk with updated header
    let header = format!(
        "@@ -{},{} +{},{} @@",
        original.original_range.0,
        original_count,
        original.new_range.0,
        new_count
    );

    Ok(Some(DiffHunk {
        index: original.index,
        header,
        original_range: (original.original_range.0, original_count),
        new_range: (original.new_range.0, new_count),
        lines,
    }))
}

/// Print help for interactive mode
fn print_help(action: &str) {
    eprintln!("\nInteractive {} commands:", action);
    eprintln!("  y - {} this hunk", action);
    eprintln!("  n - skip this hunk");
    eprintln!("  q - quit; do not {} this or any remaining hunks", action);
    eprintln!(
        "  a - {} this hunk and all remaining hunks in this file",
        action
    );
    eprintln!("  d - done with this file; skip remaining hunks");
    eprintln!("  e - manually edit this hunk");
    eprintln!("  s - split this hunk into smaller hunks (if possible)");
    eprintln!("  j - skip to next file");
    eprintln!("  k - go back to previous file");
    eprintln!("  ? - show this help\n");
}

/// Apply accept selections - copy selected hunks to lower filesystem
fn apply_accept_selections(
    sandbox: &Sandbox,
    file_hunks_list: &[FileHunks],
    result: &SelectionResult,
) -> Result<()> {
    let mut accepted_count = 0;

    for (file_idx, file_hunks) in file_hunks_list.iter().enumerate() {
        let file_selections = &result.selections[file_idx];

        // Check if any hunks were selected for this file
        let any_selected = file_selections.contains(&HunkSelection::Accept);
        let all_selected =
            file_selections.iter().all(|s| *s == HunkSelection::Accept);

        if !any_selected {
            continue;
        }

        // Handle binary files (whole file accept)
        if file_hunks.is_binary {
            if file_selections.first() == Some(&HunkSelection::Accept) {
                accept_whole_file(&file_hunks.change_entry)?;
                accepted_count += 1;
            }
            continue;
        }

        // Handle new file (Create) - if accepted, copy the whole file
        if matches!(
            file_hunks.change_entry.operation,
            EntryOperation::Set(SetType::Create)
        ) {
            if all_selected {
                accept_whole_file(&file_hunks.change_entry)?;
                accepted_count += 1;
            } else {
                // Partial accept of new file - apply only selected hunks
                // For new files, this means creating a file with only selected additions
                let new_content = apply_selected_hunks_to_new_file(
                    file_hunks,
                    file_selections,
                )?;
                write_to_destination_and_update_upper(
                    &file_hunks.change_entry,
                    &new_content,
                    file_hunks,
                    file_selections,
                    sandbox,
                )?;
                accepted_count += 1;
            }
            continue;
        }

        // Handle deleted file (Remove)
        if matches!(file_hunks.change_entry.operation, EntryOperation::Remove) {
            if all_selected {
                // Delete the file
                if let Some(source) = &file_hunks.change_entry.source {
                    fs::remove_file(&source.path).context(format!(
                        "Failed to remove file {}",
                        source.path.display()
                    ))?;

                    // Remove from upper
                    if let Some(staged) = &file_hunks.change_entry.staged {
                        fs::remove_file(&staged.path).ok();
                    }

                    accepted_count += 1;
                }
            }
            // Partial accept of deletion doesn't make sense - skip
            continue;
        }

        // Handle modified file
        if all_selected {
            // Accept all changes - just copy the file
            accept_whole_file(&file_hunks.change_entry)?;
            accepted_count += 1;
        } else {
            // Partial accept - apply selected hunks
            let new_content =
                apply_selected_hunks_for_accept(file_hunks, file_selections)?;
            write_to_destination_and_update_upper(
                &file_hunks.change_entry,
                &new_content,
                file_hunks,
                file_selections,
                sandbox,
            )?;
            accepted_count += 1;
        }
    }

    if accepted_count > 0 {
        outln!("{} files with changes accepted", accepted_count);
    } else {
        outln!("No changes accepted");
    }

    Ok(())
}

/// Apply reject selections - remove selected hunks from upper filesystem
fn apply_reject_selections(
    sandbox: &Sandbox,
    file_hunks_list: &[FileHunks],
    result: &SelectionResult,
) -> Result<()> {
    let mut rejected_count = 0;

    for (file_idx, file_hunks) in file_hunks_list.iter().enumerate() {
        let file_selections = &result.selections[file_idx];

        // Check if any hunks were selected for this file
        let any_selected = file_selections.contains(&HunkSelection::Accept);
        let all_selected =
            file_selections.iter().all(|s| *s == HunkSelection::Accept);

        if !any_selected {
            continue;
        }

        // Handle binary files (whole file reject)
        if file_hunks.is_binary {
            if file_selections.first() == Some(&HunkSelection::Accept) {
                reject_whole_file(&file_hunks.change_entry)?;
                rejected_count += 1;
            }
            continue;
        }

        if all_selected {
            // Reject all changes - remove the staged file
            reject_whole_file(&file_hunks.change_entry)?;
            rejected_count += 1;
        } else {
            // Partial reject - keep only non-rejected hunks in upper
            let remaining_content =
                apply_selected_hunks_for_reject(file_hunks, file_selections)?;
            write_remaining_to_upper(
                &file_hunks.change_entry,
                &remaining_content,
                sandbox,
            )?;
            rejected_count += 1;
        }
    }

    if rejected_count > 0 {
        outln!("\n{} files with changes rejected\n", rejected_count);
    } else {
        outln!("\nNo changes rejected\n");
    }

    Ok(())
}

/// Accept a whole file by copying from staged to destination
fn accept_whole_file(change: &ChangeEntry) -> Result<()> {
    if let Some(staged) = &change.staged {
        let dest = &change.destination;

        // Copy file
        fs::copy(&staged.path, dest).context(format!(
            "Failed to copy {} to {}",
            staged.path.display(),
            dest.display()
        ))?;

        // Set permissions
        set_file_permissions(dest, staged)?;

        // Remove staged file
        fs::remove_file(&staged.path).ok();
    }
    Ok(())
}

/// Reject a whole file by removing the staged file
fn reject_whole_file(change: &ChangeEntry) -> Result<()> {
    if let Some(staged) = &change.staged {
        if staged.is_dir() {
            fs::remove_dir(&staged.path).ok();
        } else {
            fs::remove_file(&staged.path).ok();
        }
    }
    Ok(())
}

/// Set file permissions to match staged file
fn set_file_permissions(
    path: &Path,
    staged: &crate::sandbox::changes::FileDetails,
) -> Result<()> {
    use nix::fcntl::AtFlags;
    use nix::sys::stat::{FchmodatFlags, Mode, fchmodat};
    use nix::unistd::{Gid, Uid, fchownat};

    fchownat(
        None,
        path,
        Some(Uid::from_raw(staged.stat.st_uid)),
        Some(Gid::from_raw(staged.stat.st_gid)),
        AtFlags::AT_SYMLINK_NOFOLLOW,
    )?;

    if (staged.stat.st_mode & libc::S_IFMT) != libc::S_IFLNK {
        fchmodat(
            None,
            path,
            Mode::from_bits_truncate(staged.stat.st_mode),
            FchmodatFlags::NoFollowSymlink,
        )?;
    }

    Ok(())
}

/// Apply selected hunks to original content for accept
fn apply_selected_hunks_for_accept(
    file_hunks: &FileHunks,
    selections: &[HunkSelection],
) -> Result<Vec<u8>> {
    use crate::sandbox::changes::hunk::HunkLine;

    let original = file_hunks
        .original_content
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No original content for file"))?;

    let original_str = String::from_utf8_lossy(original);
    let original_lines: Vec<&str> = original_str.lines().collect();

    let mut result_lines: Vec<String> = Vec::new();
    let mut original_line_idx = 0;

    for (hunk_idx, hunk) in file_hunks.hunks.iter().enumerate() {
        let selected = selections
            .get(hunk_idx)
            .map(|s| *s == HunkSelection::Accept)
            .unwrap_or(false);

        // Add lines from original up to this hunk
        let hunk_start = hunk.original_range.0.saturating_sub(1);
        while original_line_idx < hunk_start
            && original_line_idx < original_lines.len()
        {
            result_lines.push(original_lines[original_line_idx].to_string());
            original_line_idx += 1;
        }

        if selected {
            // Apply this hunk - include added lines, exclude removed lines
            for line in &hunk.lines {
                match line {
                    HunkLine::Context(content) => {
                        result_lines.push(content.clone());
                        original_line_idx += 1;
                    }
                    HunkLine::Added(content) => {
                        result_lines.push(content.clone());
                    }
                    HunkLine::Removed(_) => {
                        original_line_idx += 1;
                    }
                }
            }
        } else {
            // Skip this hunk - keep original lines
            for line in &hunk.lines {
                match line {
                    HunkLine::Context(_) | HunkLine::Removed(_) => {
                        if original_line_idx < original_lines.len() {
                            result_lines.push(
                                original_lines[original_line_idx].to_string(),
                            );
                            original_line_idx += 1;
                        }
                    }
                    HunkLine::Added(_) => {
                        // Don't add the new lines
                    }
                }
            }
        }
    }

    // Add remaining lines from original
    while original_line_idx < original_lines.len() {
        result_lines.push(original_lines[original_line_idx].to_string());
        original_line_idx += 1;
    }

    // Join with newlines and add trailing newline if original had one
    let mut result = result_lines.join("\n");
    if original.ends_with(b"\n") && !result.is_empty() {
        result.push('\n');
    }

    Ok(result.into_bytes())
}

/// Apply selected hunks to create content for a new file
fn apply_selected_hunks_to_new_file(
    file_hunks: &FileHunks,
    selections: &[HunkSelection],
) -> Result<Vec<u8>> {
    use crate::sandbox::changes::hunk::HunkLine;

    let mut result_lines: Vec<String> = Vec::new();

    for (hunk_idx, hunk) in file_hunks.hunks.iter().enumerate() {
        let selected = selections
            .get(hunk_idx)
            .map(|s| *s == HunkSelection::Accept)
            .unwrap_or(false);

        if selected {
            for line in &hunk.lines {
                if let HunkLine::Added(content) = line {
                    result_lines.push(content.clone());
                }
            }
        }
    }

    let mut result = result_lines.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }

    Ok(result.into_bytes())
}

/// Apply rejections - keep only non-rejected hunks
fn apply_selected_hunks_for_reject(
    file_hunks: &FileHunks,
    selections: &[HunkSelection],
) -> Result<Vec<u8>> {
    use crate::sandbox::changes::hunk::HunkLine;

    let original = file_hunks
        .original_content
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No original content for file"))?;

    let original_str = String::from_utf8_lossy(original);
    let original_lines: Vec<&str> = original_str.lines().collect();

    let mut result_lines: Vec<String> = Vec::new();
    let mut original_line_idx = 0;

    for (hunk_idx, hunk) in file_hunks.hunks.iter().enumerate() {
        let rejected = selections
            .get(hunk_idx)
            .map(|s| *s == HunkSelection::Accept)
            .unwrap_or(false);

        // Add lines from original up to this hunk
        let hunk_start = hunk.original_range.0.saturating_sub(1);
        while original_line_idx < hunk_start
            && original_line_idx < original_lines.len()
        {
            result_lines.push(original_lines[original_line_idx].to_string());
            original_line_idx += 1;
        }

        if rejected {
            // This hunk is rejected - keep original lines (revert the change)
            for line in &hunk.lines {
                match line {
                    HunkLine::Context(_) | HunkLine::Removed(_) => {
                        if original_line_idx < original_lines.len() {
                            result_lines.push(
                                original_lines[original_line_idx].to_string(),
                            );
                            original_line_idx += 1;
                        }
                    }
                    HunkLine::Added(_) => {
                        // Don't add the new lines - they're being rejected
                    }
                }
            }
        } else {
            // This hunk is kept - apply it (include additions, skip removals)
            for line in &hunk.lines {
                match line {
                    HunkLine::Context(content) => {
                        result_lines.push(content.clone());
                        original_line_idx += 1;
                    }
                    HunkLine::Added(content) => {
                        result_lines.push(content.clone());
                    }
                    HunkLine::Removed(_) => {
                        original_line_idx += 1;
                    }
                }
            }
        }
    }

    // Add remaining lines from original
    while original_line_idx < original_lines.len() {
        result_lines.push(original_lines[original_line_idx].to_string());
        original_line_idx += 1;
    }

    // Join with newlines
    let mut result = result_lines.join("\n");
    if original.ends_with(b"\n") && !result.is_empty() {
        result.push('\n');
    }

    Ok(result.into_bytes())
}

/// Write content to destination and update upper file
fn write_to_destination_and_update_upper(
    change: &ChangeEntry,
    content: &[u8],
    file_hunks: &FileHunks,
    selections: &[HunkSelection],
    _sandbox: &Sandbox,
) -> Result<()> {
    let dest = &change.destination;

    // Write to destination
    fs::write(dest, content)
        .context(format!("Failed to write to {}", dest.display()))?;

    // Set permissions if we have staged file info
    if let Some(staged) = &change.staged {
        set_file_permissions(dest, staged)?;
    }

    // Now update the upper file to contain only unaccepted changes
    // Calculate what should remain in upper
    let remaining = calculate_remaining_for_upper(file_hunks, selections)?;

    if let Some(staged) = &change.staged {
        if remaining.is_empty() {
            // All changes accepted - remove staged file
            fs::remove_file(&staged.path).ok();
        } else {
            // Write remaining changes to upper
            fs::write(&staged.path, &remaining).context(format!(
                "Failed to update staged file {}",
                staged.path.display()
            ))?;
        }
    }

    Ok(())
}

/// Calculate what content should remain in the upper file after partial accept
fn calculate_remaining_for_upper(
    file_hunks: &FileHunks,
    selections: &[HunkSelection],
) -> Result<Vec<u8>> {
    use crate::sandbox::changes::hunk::HunkLine;

    // The upper file after partial accept should represent:
    // The FULL modified content with ALL changes (both accepted and unaccepted).
    // In OverlayFS, when an upper file exists, it COMPLETELY REPLACES the lower file.
    // The lower file will have accepted changes, and upper must also have those changes
    // plus any unaccepted changes, so the merged view shows all changes correctly.

    let modified = file_hunks.modified_content.as_ref().ok_or_else(|| {
        anyhow::anyhow!("No modified content for calculating remaining")
    })?;

    // If all hunks were accepted, nothing remains
    if selections.iter().all(|s| *s == HunkSelection::Accept) {
        return Ok(Vec::new());
    }

    // If no hunks were accepted, return the original modified content
    if selections.iter().all(|s| *s == HunkSelection::Skip) {
        return Ok(modified.clone());
    }

    // For partial accepts, we need to calculate what should remain
    // This is complex - the upper file needs to contain a version that
    // will produce the unaccepted hunks when diffed against the new lower

    // Start from the new lower content (after accepted hunks)
    // Then apply only the unaccepted hunks to get the new upper

    let original = file_hunks.original_content.as_ref().ok_or_else(|| {
        anyhow::anyhow!("No original content for calculating remaining")
    })?;

    let original_str = String::from_utf8_lossy(original);
    let original_lines: Vec<&str> = original_str.lines().collect();

    let mut result_lines: Vec<String> = Vec::new();
    let mut original_line_idx = 0;

    for hunk in file_hunks.hunks.iter() {
        // Add lines from original up to this hunk
        let hunk_start = hunk.original_range.0.saturating_sub(1);
        while original_line_idx < hunk_start
            && original_line_idx < original_lines.len()
        {
            result_lines.push(original_lines[original_line_idx].to_string());
            original_line_idx += 1;
        }

        // Always apply all hunks to get the full modified content
        // (this represents what upper should contain)
        for line in &hunk.lines {
            match line {
                HunkLine::Context(content) => {
                    result_lines.push(content.clone());
                    original_line_idx += 1;
                }
                HunkLine::Added(content) => {
                    // Always include added lines (both accepted and unaccepted)
                    // Upper must contain the full modified state
                    result_lines.push(content.clone());
                }
                HunkLine::Removed(_) => {
                    // Never include removed lines - they're being deleted in the modified version
                    // (both for accepted and unaccepted hunks)
                    original_line_idx += 1;
                }
            }
        }
    }

    // Add remaining lines from original
    while original_line_idx < original_lines.len() {
        result_lines.push(original_lines[original_line_idx].to_string());
        original_line_idx += 1;
    }

    let mut result = result_lines.join("\n");
    if modified.ends_with(b"\n") && !result.is_empty() {
        result.push('\n');
    }

    Ok(result.into_bytes())
}

/// Write remaining content to upper file after partial reject
fn write_remaining_to_upper(
    change: &ChangeEntry,
    content: &[u8],
    _sandbox: &Sandbox,
) -> Result<()> {
    if let Some(staged) = &change.staged {
        if content.is_empty() {
            // All changes rejected - remove staged file
            fs::remove_file(&staged.path).ok();
        } else {
            // Write remaining changes
            fs::write(&staged.path, content).context(format!(
                "Failed to update staged file {}",
                staged.path.display()
            ))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::changes::hunk::{DiffHunk, HunkLine};

    #[test]
    fn test_parse_edited_hunk_basic() {
        let original = DiffHunk {
            index: 0,
            header: "@@ -1,3 +1,3 @@".to_string(),
            original_range: (1, 3),
            new_range: (1, 3),
            lines: vec![
                HunkLine::Context("line1".to_string()),
                HunkLine::Removed("old".to_string()),
                HunkLine::Added("new".to_string()),
                HunkLine::Context("line3".to_string()),
            ],
        };

        let edited = " line1\n-old\n+modified\n line3\n";
        let result = parse_edited_hunk(edited, &original).unwrap();

        assert!(result.is_some());
        let hunk = result.unwrap();
        assert!(hunk
            .lines
            .iter()
            .any(|l| matches!(l, HunkLine::Added(s) if s == "modified")));
    }

    #[test]
    fn test_parse_edited_hunk_unchanged() {
        let original = DiffHunk {
            index: 0,
            header: "@@ -1,3 +1,3 @@".to_string(),
            original_range: (1, 3),
            new_range: (1, 3),
            lines: vec![
                HunkLine::Context("line1".to_string()),
                HunkLine::Removed("old".to_string()),
                HunkLine::Added("new".to_string()),
                HunkLine::Context("line3".to_string()),
            ],
        };

        // Exact same content as original
        let edited = " line1\n-old\n+new\n line3\n";
        let result = parse_edited_hunk(edited, &original).unwrap();

        assert!(result.is_none(), "Should return None for unchanged hunk");
    }

    #[test]
    fn test_parse_edited_hunk_with_comments() {
        let original = DiffHunk {
            index: 0,
            header: "@@ -1,2 +1,2 @@".to_string(),
            original_range: (1, 2),
            new_range: (1, 2),
            lines: vec![
                HunkLine::Removed("old".to_string()),
                HunkLine::Added("new".to_string()),
            ],
        };

        let edited = "# This is a comment\n# Another comment\n-old\n+different\n";
        let result = parse_edited_hunk(edited, &original).unwrap();

        assert!(result.is_some());
        let hunk = result.unwrap();
        // Should only have 2 lines (comments ignored)
        assert_eq!(hunk.lines.len(), 2);
    }

    #[test]
    fn test_parse_edited_hunk_empty() {
        let original = DiffHunk {
            index: 0,
            header: "@@ -1,2 +1,2 @@".to_string(),
            original_range: (1, 2),
            new_range: (1, 2),
            lines: vec![
                HunkLine::Removed("old".to_string()),
                HunkLine::Added("new".to_string()),
            ],
        };

        // Only comments, no actual content
        let edited = "# All content deleted\n# User cancelled\n";
        let result = parse_edited_hunk(edited, &original).unwrap();

        assert!(result.is_none(), "Should return None for empty edit");
    }

    #[test]
    fn test_parse_edited_hunk_no_changes() {
        let original = DiffHunk {
            index: 0,
            header: "@@ -1,3 +1,3 @@".to_string(),
            original_range: (1, 3),
            new_range: (1, 3),
            lines: vec![
                HunkLine::Context("line1".to_string()),
                HunkLine::Removed("old".to_string()),
                HunkLine::Added("new".to_string()),
            ],
        };

        // Only context lines (no + or -)
        let edited = " line1\n line2\n line3\n";
        let result = parse_edited_hunk(edited, &original).unwrap();

        assert!(
            result.is_none(),
            "Should return None when no changes present"
        );
    }

    #[test]
    fn test_get_editor_fallback() {
        // This test just ensures get_editor doesn't panic
        // In a real environment it should return something
        let editor = get_editor();
        assert!(!editor.is_empty());
    }

    #[test]
    fn test_interactive_command_from_char() {
        assert_eq!(
            InteractiveCommand::from_char('y'),
            Some(InteractiveCommand::Yes)
        );
        assert_eq!(
            InteractiveCommand::from_char('n'),
            Some(InteractiveCommand::No)
        );
        assert_eq!(
            InteractiveCommand::from_char('e'),
            Some(InteractiveCommand::Edit)
        );
        assert_eq!(
            InteractiveCommand::from_char('s'),
            Some(InteractiveCommand::Split)
        );
        assert_eq!(
            InteractiveCommand::from_char('q'),
            Some(InteractiveCommand::Quit)
        );
        assert_eq!(InteractiveCommand::from_char('x'), None);
    }
}
