mod fixtures;

use anyhow::Result;
use fixtures::*;
use rstest::*;
use std::fs;

#[rstest]
fn test_spurious_copyup_is_pruned(mut sandbox: SandboxManager) -> Result<()> {
    // Create a file outside the sandbox
    let file = sandbox.test_filename("copyup_test.txt");
    fs::write(&file, "original content")?;

    // Open the file for writing inside the sandbox but don't change anything.
    // This triggers an OverlayFS copy-up without actual modification.
    sandbox.run(&[
        "sh",
        "-c",
        &format!(
            "python3 -c \"f = open('{}', 'r+'); f.close()\"",
            file
        ),
    ])?;

    // Status should show no changes for this file since it's identical
    sandbox.run(&["status"])?;
    assert!(
        !sandbox.last_stdout.contains("copyup_test"),
        "Unchanged file should not appear in status. Got: {}",
        sandbox.last_stdout
    );

    // Diff should also show nothing
    sandbox.run(&["diff"])?;
    assert!(
        !sandbox.last_stdout.contains("copyup_test"),
        "Unchanged file should not appear in diff. Got: {}",
        sandbox.last_stdout
    );

    Ok(())
}

#[rstest]
fn test_actually_modified_file_is_not_pruned(
    mut sandbox: SandboxManager,
) -> Result<()> {
    // Create a file outside the sandbox
    let file = sandbox.test_filename("real_change.txt");
    fs::write(&file, "original content")?;

    // Actually modify the file inside the sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'modified content' > {}", file),
    ])?;

    // Status should show this file as changed
    sandbox.run(&["status"])?;
    assert!(
        sandbox.last_stdout.contains("real_change"),
        "Modified file should appear in status. Got: {}",
        sandbox.last_stdout
    );

    // Diff should show the change
    sandbox.run(&["diff"])?;
    assert!(
        sandbox.last_stdout.contains("modified content"),
        "Modified file should appear in diff. Got: {}",
        sandbox.last_stdout
    );

    Ok(())
}

#[rstest]
fn test_metadata_change_is_not_pruned(
    mut sandbox: SandboxManager,
) -> Result<()> {
    // Create a file outside the sandbox
    let file = sandbox.test_filename("meta_change.txt");
    fs::write(&file, "same content")?;

    // Change permissions inside the sandbox (content stays the same)
    sandbox.run(&["chmod", "777", &file])?;

    // Status should show this file as changed (mode differs)
    sandbox.run(&["status"])?;
    assert!(
        sandbox.last_stdout.contains("meta_change"),
        "File with changed permissions should appear in status. Got: {}",
        sandbox.last_stdout
    );

    Ok(())
}
