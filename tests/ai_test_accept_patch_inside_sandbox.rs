mod fixtures;

use anyhow::Result;
use fixtures::*;
use rstest::*;

/// Test that verifies the fix for the bug where accepting hunks with -p
/// would cause changes to appear reversed inside the sandbox.
///
/// The bug was in calculate_remaining_for_upper() which incorrectly removed
/// accepted changes from the upper filesystem, causing them to disappear
/// from the merged view inside the sandbox.
#[rstest]
fn test_accept_patch_changes_visible_inside_sandbox(
    mut sandbox: SandboxManager,
) -> Result<()> {
    let filename = sandbox.test_filename("inside-sandbox-test.txt");

    // Create original file with multiple lines
    std::fs::write(
        &filename,
        "line1\noriginal_line2\nline3\noriginal_line4\nline5\n",
    )?;

    // Modify inside sandbox - change two separate lines (two hunks)
    sandbox.run(&[
        "sh",
        "-c",
        &format!(
            "echo 'line1\nmodified_line2\nline3\nmodified_line4\nline5' > {}",
            filename
        ),
    ])?;

    // Verify both changes are visible inside sandbox before accept
    sandbox.run(&["run", "cat", &filename])?;
    assert!(
        sandbox.last_stdout.contains("modified_line2"),
        "Before accept: sandbox should show first change"
    );
    assert!(
        sandbox.last_stdout.contains("modified_line4"),
        "Before accept: sandbox should show second change"
    );

    // Accept only the FIRST hunk (modified_line2), skip the second
    // User answers: y (accept first), n (skip second)
    sandbox.run_with_stdin(&["accept", "-p", &filename], "y\nn\n")?;

    // CRITICAL TEST: Check that BOTH changes are still visible INSIDE the sandbox
    // This is where the bug was - the accepted change would disappear from upper
    sandbox.run(&["run", "cat", &filename])?;
    let inside_sandbox = sandbox.last_stdout.clone();

    println!("Content inside sandbox after partial accept:");
    println!("{}", inside_sandbox);

    assert!(
        inside_sandbox.contains("modified_line2"),
        "BUG: Accepted change (modified_line2) should STILL be visible inside sandbox!\n\
         The upper file must contain ALL changes (both accepted and unaccepted).\n\
         Inside sandbox content: {}",
        inside_sandbox
    );

    assert!(
        inside_sandbox.contains("modified_line4"),
        "Unaccepted change (modified_line4) should still be visible inside sandbox.\n\
         Inside sandbox content: {}",
        inside_sandbox
    );

    // Verify the OUTSIDE (host) filesystem only has the accepted change
    let outside_content = std::fs::read_to_string(&filename)?;
    println!("Content outside sandbox (host filesystem):");
    println!("{}", outside_content);

    assert!(
        outside_content.contains("modified_line2"),
        "Host filesystem should have accepted change (modified_line2)"
    );

    assert!(
        !outside_content.contains("modified_line4"),
        "Host filesystem should NOT have unaccepted change (modified_line4)"
    );

    assert!(
        outside_content.contains("original_line4"),
        "Host filesystem should still have original_line4 (not modified)"
    );

    // Verify status shows only the remaining unaccepted change
    sandbox.run(&["status", &filename])?;
    println!("Status after partial accept:");
    println!("{}", sandbox.last_stdout);

    Ok(())
}

/// Test accepting all hunks - upper should be deleted entirely
#[rstest]
fn test_accept_patch_all_hunks_removes_upper(
    mut sandbox: SandboxManager,
) -> Result<()> {
    let filename = sandbox.test_filename("all-accepted.txt");

    // Create original file
    std::fs::write(&filename, "line1\noriginal\nline3\n")?;

    // Modify inside sandbox
    sandbox.run(&[
        "sh",
        "-c",
        &format!("echo 'line1\nmodified\nline3' > {}", filename),
    ])?;

    // Accept the change
    sandbox.run_with_stdin(&["accept", "-p", &filename], "y\n")?;

    // Check that change is visible both inside and outside
    let outside = std::fs::read_to_string(&filename)?;
    assert!(
        outside.contains("modified"),
        "Host should have accepted change"
    );

    sandbox.run(&["run", "cat", &filename])?;
    assert!(
        sandbox.last_stdout.contains("modified"),
        "Sandbox should see accepted change"
    );

    // Verify no more changes pending
    sandbox.run(&["status", &filename])?;
    // Should show no changes for this file
    assert!(
        !sandbox.last_stdout.contains("modified")
            || sandbox.last_stdout.contains("No changes"),
        "Should have no pending changes after accepting all"
    );

    Ok(())
}

/// Test multiple sequential partial accepts
#[rstest]
fn test_accept_patch_sequential_partial_accepts(
    mut sandbox: SandboxManager,
) -> Result<()> {
    let filename = sandbox.test_filename("sequential.txt");

    // Create file with 3 separate changes
    std::fs::write(&filename, "line1\nline2\nline3\nline4\nline5\n")?;

    sandbox.run(&[
        "sh",
        "-c",
        &format!(
            "echo 'changed1\nline2\nchanged3\nline4\nchanged5' > {}",
            filename
        ),
    ])?;

    // Accept first change only
    sandbox.run_with_stdin(&["accept", "-p", &filename], "y\nq\n")?;

    // Verify all changes still visible inside
    sandbox.run(&["run", "cat", &filename])?;
    let inside1 = sandbox.last_stdout.clone();
    assert!(
        inside1.contains("changed1"),
        "First accepted change should be visible inside"
    );
    assert!(
        inside1.contains("changed3"),
        "Second unaccepted change should be visible inside"
    );
    assert!(
        inside1.contains("changed5"),
        "Third unaccepted change should be visible inside"
    );

    // Accept second change
    sandbox.run_with_stdin(&["accept", "-p", &filename], "y\nq\n")?;

    // Verify all changes still visible inside
    sandbox.run(&["run", "cat", &filename])?;
    let inside2 = sandbox.last_stdout.clone();
    assert!(
        inside2.contains("changed1"),
        "Previously accepted change should still be visible"
    );
    assert!(
        inside2.contains("changed3"),
        "Newly accepted change should be visible"
    );
    assert!(
        inside2.contains("changed5"),
        "Remaining unaccepted change should be visible"
    );

    // Accept final change
    sandbox.run_with_stdin(&["accept", "-p", &filename], "y\n")?;

    // Verify all changes visible inside and outside
    sandbox.run(&["run", "cat", &filename])?;
    assert!(sandbox.last_stdout.contains("changed1"));
    assert!(sandbox.last_stdout.contains("changed3"));
    assert!(sandbox.last_stdout.contains("changed5"));

    let outside = std::fs::read_to_string(&filename)?;
    assert!(outside.contains("changed1"));
    assert!(outside.contains("changed3"));
    assert!(outside.contains("changed5"));

    Ok(())
}
