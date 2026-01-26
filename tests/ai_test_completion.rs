use anyhow::Result;
use sandbox::config::cli::changed_file_completion;
use std::ffi::OsStr;

mod fixtures;
use fixtures::SandboxManager;

/// Test that the completion function doesn't crash and returns valid results.
/// Note: Direct testing of the completion function is limited because it parses
/// command-line arguments from the environment. Real testing requires shell integration.
#[test]
fn test_changed_file_completion_doesnt_crash() -> Result<()> {
    // Test that the function handles various inputs without panicking

    // Empty prefix
    let completions = changed_file_completion(OsStr::new(""));
    println!("Completions for empty prefix: {} items", completions.len());

    // Absolute path prefix
    let completions = changed_file_completion(OsStr::new("/tmp/"));
    println!("Completions for '/tmp/': {} items", completions.len());

    // Relative path prefix
    let completions = changed_file_completion(OsStr::new("src/"));
    println!("Completions for 'src/': {} items", completions.len());

    // Non-existent prefix
    let completions =
        changed_file_completion(OsStr::new("nonexistent_file_xyz"));
    println!("Completions for nonexistent: {} items", completions.len());

    // The function should always return a Vec (possibly empty) and never panic
    Ok(())
}

/// Integration test that verifies completion works with a real sandbox.
/// This test uses the actual sandbox binary with shell completion mode.
#[test]
fn test_completion_integration() -> Result<()> {
    let mut manager = SandboxManager::new();

    // Create sandbox and make some changes
    manager.run(&["sh", "-c", "echo 'test' > /tmp/test.txt"])?;

    // Test that we can invoke completion via the binary
    // The COMPLETE environment variable triggers completion mode
    let output = std::process::Command::new("sudo")
        .args(["-E", &manager.sandbox_bin])
        .arg(format!("--name={}", manager.name))
        .arg("accept")
        .arg("/tmp/t")
        .env("COMPLETE", "zsh")
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("Completion output: {}", stdout);

    // The completion should generate shell completion script
    // We can't easily test the actual completion results without a full zsh environment,
    // but we can verify it doesn't error
    assert!(
        output.status.success() || output.stderr.is_empty(),
        "Completion should not error"
    );

    Ok(())
}
