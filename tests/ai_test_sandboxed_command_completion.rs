use anyhow::Result;

mod fixtures;
use fixtures::SandboxManager;

/// Test that completion works for sandboxed commands by running completion inside the sandbox
#[test]
fn test_sandboxed_command_file_completion() -> Result<()> {
    let mut manager = SandboxManager::new();

    // Create sandbox with some unique files
    manager.run(&["sh", "-c", "echo 'test' > /tmp/sandbox_test_file.txt"])?;
    manager.run(&[
        "sh",
        "-c",
        "mkdir -p /tmp/sandbox_dir && echo 'nested' > /tmp/sandbox_dir/nested.txt",
    ])?;

    // Verify the sandbox has the files
    manager.run(&["ls", "-la", "/tmp/sandbox_test_file.txt"])?;

    // Now test that we can complete these files from within the sandbox
    // The completion works by running `sandbox bash -c "compgen -f -- prefix"`
    // Use manager.run() which handles sudo internally
    manager.run(&["bash", "-c", "compgen -f -- /tmp/sandbox_test"])?;
    let stdout = manager.last_stdout.clone();

    println!("Completion results: {}", stdout);

    // Should find the file we created in the sandbox
    assert!(
        stdout.contains("sandbox_test_file.txt"),
        "Expected completion to find sandbox_test_file.txt, got: {}",
        stdout
    );

    Ok(())
}

/// Test that completion finds files only in the sandbox (not on host)
#[test]
fn test_completion_sees_sandbox_only_files() -> Result<()> {
    let mut manager = SandboxManager::new();

    // Create a file that only exists in the sandbox
    let unique_filename = format!("/tmp/only_in_sandbox_{}.txt", manager.name);
    manager.run(&["sh", "-c", &format!("echo 'sandbox only' > {}", unique_filename)])?;

    // Verify it doesn't exist on the host
    let host_check = std::path::Path::new(&unique_filename).exists();
    assert!(
        !host_check,
        "File should not exist on host before accepting changes"
    );

    // But completion inside the sandbox should find it
    manager.run(&["bash", "-c", &format!("compgen -f -- {}", unique_filename)])?;
    let stdout = manager.last_stdout.clone();
    println!("Completion results for sandbox-only file: {}", stdout);

    assert!(
        stdout.contains(&unique_filename),
        "Expected completion to find sandbox-only file, got: {}",
        stdout
    );

    Ok(())
}

/// Test directory completion inside sandbox
#[test]
fn test_directory_completion_in_sandbox() -> Result<()> {
    let mut manager = SandboxManager::new();

    // Create nested directories in sandbox
    manager.run(&[
        "bash",
        "-c",
        "mkdir -p /tmp/sandbox_dir_{1,2,3}/subdir",
    ])?;

    // Test directory completion
    manager.run(&["bash", "-c", "compgen -d -- /tmp/sandbox_dir_"])?;
    let stdout = manager.last_stdout.clone();
    println!("Directory completion results: {}", stdout);

    // Should find all three directories
    assert!(
        stdout.contains("sandbox_dir_1") && stdout.contains("sandbox_dir_2"),
        "Expected completion to find directories, got: {}",
        stdout
    );

    Ok(())
}

/// Test that completion works with partial paths
#[test]
fn test_partial_path_completion() -> Result<()> {
    let mut manager = SandboxManager::new();

    // Create files with common prefix
    manager.run(&[
        "sh",
        "-c",
        "echo 'a' > /tmp/myapp_config.txt && echo 'b' > /tmp/myapp_data.txt && echo 'c' > /tmp/myapp_log.txt",
    ])?;

    // Complete with partial prefix
    manager.run(&["bash", "-c", "compgen -f -- /tmp/myapp_"])?;
    let stdout = manager.last_stdout.clone();
    println!("Partial path completion results: {}", stdout);

    // Should find all three files
    assert!(
        stdout.contains("myapp_config.txt")
            && stdout.contains("myapp_data.txt")
            && stdout.contains("myapp_log.txt"),
        "Expected completion to find all myapp_* files, got: {}",
        stdout
    );

    Ok(())
}

/// Test that completion handles empty results gracefully
#[test]
fn test_completion_no_matches() -> Result<()> {
    let mut manager = SandboxManager::new();

    // Try to complete a prefix that won't match anything
    // Note: compgen returns exit code 1 when no matches, so use || true to make it succeed
    manager.run(&["bash", "-c", "compgen -f -- /tmp/nonexistent_file_xyz_ || true"])?;
    let stdout = manager.last_stdout.clone();

    // Should return empty (or just whitespace)
    assert!(
        stdout.trim().is_empty(),
        "Expected no completions for nonexistent prefix, got: {}",
        stdout
    );

    Ok(())
}
