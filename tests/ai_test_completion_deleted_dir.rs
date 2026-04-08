use anyhow::Result;

mod fixtures;
use fixtures::SandboxManager;

/// Helper to run completion mimicking real zsh usage.
fn run_completion_words(
    manager: &SandboxManager,
    words: &[&str],
    word_index: usize,
) -> Result<String> {
    let mut cmd = std::process::Command::new("sudo");
    cmd.args(["-E", &manager.sandbox_bin]);
    cmd.arg("--");
    for w in words {
        cmd.arg(w);
    }
    cmd.env("COMPLETE", "zsh");
    cmd.env("_CLAP_COMPLETE_INDEX", word_index.to_string());
    cmd.env("_CLAP_IFS", "\n");
    cmd.env("SANDBOX_NAME", &manager.name);

    let output = cmd.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    println!(
        "Completion for words[{}]='{}': stdout='{}' stderr='{}'",
        word_index,
        words.get(word_index).unwrap_or(&"?"),
        stdout.trim(),
        stderr.trim()
    );
    Ok(stdout)
}

/// Test completion with absolute paths for directory deleted from host.
#[test]
fn test_completion_deleted_dir_absolute_path() -> Result<()> {
    let mut manager = SandboxManager::new();

    let test_dir = format!("generated-test-data/{}/testdir", manager.name);
    std::fs::create_dir_all(&test_dir)?;
    std::fs::write(format!("{}/original.txt", test_dir), "original content")?;
    let abs_test_dir = std::fs::canonicalize(&test_dir)?;

    manager.run(&[
        "sh", "-c",
        &format!("echo 'modified' > {}/original.txt", abs_test_dir.display()),
    ])?;

    // Delete from host
    std::fs::remove_dir_all(&abs_test_dir)?;

    let partial = format!("{}/testdi", abs_test_dir.parent().unwrap().display());
    let result = run_completion_words(
        &manager,
        &["sandbox", "--no-config", "--ignored", "reject", &partial],
        4,
    )?;
    assert!(result.contains("testdir"), "Got: '{}'", result.trim());

    Ok(())
}

/// Test completion with relative paths for directory deleted from host.
/// This is the exact scenario the user described.
#[test]
fn test_completion_deleted_dir_relative_path() -> Result<()> {
    let mut manager = SandboxManager::new();

    // Create test structure in the test data dir
    let test_base = format!("generated-test-data/{}", manager.name);
    let abs_test_base = std::fs::canonicalize(&test_base)?;
    let test_dir = abs_test_base.join("mydir");
    std::fs::create_dir_all(&test_dir)?;
    std::fs::write(test_dir.join("file.txt"), "original")?;

    // Modify inside sandbox
    manager.run(&[
        "sh", "-c",
        &format!("echo 'changed' > {}/file.txt", test_dir.display()),
    ])?;

    // Delete from host
    std::fs::remove_dir_all(&test_dir)?;
    assert!(!test_dir.exists());

    // Use relative path completion (as user would type)
    // The completion binary's cwd matters here - we need to be in the test base
    let mut cmd = std::process::Command::new("sudo");
    cmd.args(["-E", &manager.sandbox_bin]);
    cmd.arg("--");
    cmd.arg("sandbox");
    cmd.arg("--no-config");
    cmd.arg("--ignored");
    cmd.arg("reject");
    cmd.arg("mydi");  // relative path prefix
    cmd.env("COMPLETE", "zsh");
    cmd.env("_CLAP_COMPLETE_INDEX", "4");
    cmd.env("_CLAP_IFS", "\n");
    cmd.env("SANDBOX_NAME", &manager.name);
    cmd.current_dir(&abs_test_base);

    let output = cmd.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    println!("Relative completion: stdout='{}' stderr='{}'", stdout.trim(), stderr.trim());

    assert!(
        stdout.contains("mydir"),
        "Relative completion should find 'mydir' after host deletion. Got: '{}'",
        stdout.trim()
    );

    Ok(())
}

/// Test completion for a directory created only inside the sandbox.
#[test]
fn test_completion_sandbox_only_directory() -> Result<()> {
    let mut manager = SandboxManager::new();

    let test_base = format!("generated-test-data/{}", manager.name);
    let abs_test_base = std::fs::canonicalize(&test_base)?;

    manager.run(&[
        "sh", "-c",
        &format!(
            "mkdir -p {}/newdir && echo 'new' > {}/newdir/file.txt",
            abs_test_base.display(), abs_test_base.display()
        ),
    ])?;

    let partial = format!("{}/newdi", abs_test_base.display());
    let result = run_completion_words(
        &manager,
        &["sandbox", "--no-config", "--ignored", "reject", &partial],
        4,
    )?;
    assert!(result.contains("newdir"), "Got: '{}'", result.trim());

    Ok(())
}

/// Test completion for directory deleted INSIDE the sandbox.
#[test]
fn test_completion_sandbox_deleted_directory() -> Result<()> {
    let mut manager = SandboxManager::new();

    let test_dir = format!("generated-test-data/{}/deleteme", manager.name);
    std::fs::create_dir_all(&test_dir)?;
    std::fs::write(format!("{}/file.txt", test_dir), "content")?;
    let abs_test_dir = std::fs::canonicalize(&test_dir)?;

    manager.run(&[
        "sh", "-c",
        &format!("rm -rf {}", abs_test_dir.display()),
    ])?;

    let partial = format!("{}/delete", abs_test_dir.parent().unwrap().display());
    let result = run_completion_words(
        &manager,
        &["sandbox", "--no-config", "--ignored", "reject", &partial],
        4,
    )?;
    assert!(result.contains("deleteme"), "Got: '{}'", result.trim());

    Ok(())
}
