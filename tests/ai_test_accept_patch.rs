mod fixtures;

use anyhow::Result;
use fixtures::*;
use rstest::*;
use std::path::Path;

#[rstest]
fn test_accept_patch_single_hunk_accept(
    mut sandbox: SandboxManager,
) -> Result<()> {
    // Create a file and modify it to create a single hunk
    let filename = sandbox.test_filename("single-hunk.txt");

    // Create original file
    std::fs::write(&filename, "line1\nline2\nline3\n")?;

    // Modify inside sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'line1\nmodified\nline3' > {}", filename),
    ])?;

    // Check that there are changes
    sandbox.run(&["status"])?;
    assert!(
        sandbox.last_stdout.contains(&filename)
            || sandbox.last_stdout.contains("modified")
    );

    // Accept with -p flag, answering 'y' to accept the hunk
    sandbox.run_with_stdin(&["accept", "-p"], "y\n")?;

    // Verify the file was modified
    let content = std::fs::read_to_string(&filename)?;
    assert!(
        content.contains("modified"),
        "File should contain 'modified' after accept"
    );
    assert!(
        !content.contains("line2"),
        "File should not contain original 'line2'"
    );

    Ok(())
}

#[rstest]
fn test_accept_patch_skip_hunk(mut sandbox: SandboxManager) -> Result<()> {
    // Create a file and modify it
    let filename = sandbox.test_filename("skip-hunk.txt");

    // Create original file
    std::fs::write(&filename, "line1\nline2\nline3\n")?;

    // Modify inside sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'line1\nmodified\nline3' > {}", filename),
    ])?;

    // Accept with -p flag, answering 'n' to skip the hunk
    sandbox.run_with_stdin(&["accept", "-p"], "n\n")?;

    // Check status - changes should still be pending
    sandbox.run(&["status"])?;

    // The file on disk should still have original content
    let content = std::fs::read_to_string(&filename)?;
    assert!(
        content.contains("line2"),
        "File should still contain original 'line2' after skip"
    );

    Ok(())
}

#[rstest]
fn test_accept_patch_quit_early(mut sandbox: SandboxManager) -> Result<()> {
    // Create a file and modify it
    let filename = sandbox.test_filename("quit-early.txt");

    // Create original file
    std::fs::write(&filename, "line1\nline2\nline3\n")?;

    // Modify inside sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'line1\nmodified\nline3' > {}", filename),
    ])?;

    // Accept with -p flag, answering 'q' to quit
    sandbox.run_with_stdin(&["accept", "-p"], "q\n")?;

    // The file on disk should still have original content
    let content = std::fs::read_to_string(&filename)?;
    assert!(
        content.contains("line2"),
        "File should still contain original content after quit"
    );

    Ok(())
}

#[rstest]
fn test_accept_patch_all_remaining(mut sandbox: SandboxManager) -> Result<()> {
    // Create a file with multiple lines that will produce multiple hunks
    let filename = sandbox.test_filename("all-remaining.txt");

    // Create original file
    std::fs::write(&filename, "line1\nline2\nline3\n")?;

    // Modify inside sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'changed1\nline2\nchanged3' > {}", filename),
    ])?;

    // Accept with -p flag, answering 'a' to accept all remaining in file
    sandbox.run_with_stdin(&["accept", "-p"], "a\n")?;

    // Verify all changes were accepted
    let content = std::fs::read_to_string(&filename)?;
    assert!(
        content.contains("changed"),
        "File should contain changes after 'all'"
    );

    Ok(())
}

#[rstest]
fn test_accept_patch_done_with_file(mut sandbox: SandboxManager) -> Result<()> {
    // Create a file and modify it
    let filename = sandbox.test_filename("done-file.txt");

    // Create original file
    std::fs::write(&filename, "line1\nline2\nline3\n")?;

    // Modify inside sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'line1\nmodified\nline3' > {}", filename),
    ])?;

    // Accept with -p flag, answering 'd' to skip remaining hunks in file
    sandbox.run_with_stdin(&["accept", "-p"], "d\n")?;

    // The file should still have original content
    let content = std::fs::read_to_string(&filename)?;
    assert!(
        content.contains("line2"),
        "File should still contain original after 'd'"
    );

    Ok(())
}

#[rstest]
fn test_reject_patch_single_hunk(mut sandbox: SandboxManager) -> Result<()> {
    // Create a file and modify it
    let filename = sandbox.test_filename("reject-hunk.txt");

    // Create original file
    std::fs::write(&filename, "line1\nline2\nline3\n")?;

    // Modify inside sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'line1\nmodified\nline3' > {}", filename),
    ])?;

    // Reject with -p flag, answering 'y' to reject the hunk
    sandbox.run_with_stdin(&["reject", "-p"], "y\n")?;

    // Check status - changes should be gone
    sandbox.run(&["status"])?;

    Ok(())
}

#[rstest]
fn test_accept_patch_new_file(mut sandbox: SandboxManager) -> Result<()> {
    // Create a new file inside sandbox (doesn't exist on host)
    let filename = sandbox.test_filename("new-file.txt");

    // Create file inside sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'new content' > {}", filename),
    ])?;

    // Verify file doesn't exist on host yet
    assert!(
        !Path::new(&filename).exists(),
        "File should not exist on host yet"
    );

    // Accept with -p flag
    sandbox.run_with_stdin(&["accept", "-p"], "y\n")?;

    // Verify file was created
    assert!(
        Path::new(&filename).exists(),
        "File should exist after accept"
    );
    let content = std::fs::read_to_string(&filename)?;
    assert!(
        content.contains("new content"),
        "File should have new content"
    );

    Ok(())
}

#[rstest]
fn test_accept_patch_help_command(mut sandbox: SandboxManager) -> Result<()> {
    // Create a file and modify it
    let filename = sandbox.test_filename("help-test.txt");

    // Create original file
    std::fs::write(&filename, "line1\nline2\nline3\n")?;

    // Modify inside sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'line1\nmodified\nline3' > {}", filename),
    ])?;

    // Accept with -p flag, ask for help then quit
    sandbox.run_with_stdin(&["accept", "-p"], "?\nq\n")?;

    // Should have shown help (check stderr would contain help text)
    // The file should still have original content since we quit
    let content = std::fs::read_to_string(&filename)?;
    assert!(
        content.contains("line2"),
        "File should still contain original after help+quit"
    );

    Ok(())
}

#[rstest]
fn test_accept_patch_no_changes(mut sandbox: SandboxManager) -> Result<()> {
    // Run accept -p with no changes
    sandbox.run(&["accept", "-p"])?;

    // Should indicate no changes
    assert!(
        sandbox.last_stdout.contains("No changes")
            || sandbox.last_stdout.contains("no changes"),
        "Should indicate no changes available"
    );

    Ok(())
}

#[rstest]
fn test_reject_patch_no_changes(mut sandbox: SandboxManager) -> Result<()> {
    // Run reject -p with no changes
    sandbox.run(&["reject", "-p"])?;

    // Should indicate no changes
    assert!(
        sandbox.last_stdout.contains("No changes")
            || sandbox.last_stdout.contains("no changes"),
        "Should indicate no changes available"
    );

    Ok(())
}
