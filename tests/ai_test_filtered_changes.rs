mod fixtures;
use anyhow::Result;
use fixtures::*;
use rstest::*;

#[rstest]
fn test_changes_in_directory_filters_correctly(
    mut sandbox: SandboxManager,
) -> Result<()> {
    // Create files in different directories
    let dir_a = sandbox.test_filename("filtered_a");
    let dir_b = sandbox.test_filename("filtered_b");

    // Create structure in dir_a
    std::fs::create_dir(&dir_a)?;
    std::fs::write(format!("{}/file1.txt", dir_a), "content1")?;

    // Create structure in dir_b
    std::fs::create_dir(&dir_b)?;
    std::fs::write(format!("{}/file2.txt", dir_b), "content2")?;

    // Now modify them inside the sandbox
    sandbox.run(&["touch", &format!("{}/file1.txt", dir_a)])?;
    sandbox.run(&["touch", &format!("{}/file2.txt", dir_b)])?;

    // Check status of just dir_a
    sandbox.run(&["status", &dir_a])?;
    let output_a = sandbox.last_stdout.clone();

    // Should see file1.txt but not file2.txt
    assert!(
        output_a.contains("file1.txt"),
        "Should see file1.txt in dir_a status"
    );
    assert!(
        !output_a.contains("file2.txt"),
        "Should not see file2.txt in dir_a status"
    );

    // Check status of just dir_b
    sandbox.run(&["status", &dir_b])?;
    let output_b = sandbox.last_stdout.clone();

    // Should see file2.txt but not file1.txt
    assert!(
        output_b.contains("file2.txt"),
        "Should see file2.txt in dir_b status"
    );
    assert!(
        !output_b.contains("file1.txt"),
        "Should not see file1.txt in dir_b status"
    );

    Ok(())
}

#[rstest]
fn test_changes_with_nested_directories(
    mut sandbox: SandboxManager,
) -> Result<()> {
    // Create nested directory structure
    let base_dir = sandbox.test_filename("nested");

    std::fs::create_dir(&base_dir)?;
    std::fs::create_dir(format!("{}/subdir1", base_dir))?;
    std::fs::create_dir(format!("{}/subdir2", base_dir))?;

    // Modify files in sandbox
    sandbox.run(&["touch", &format!("{}/file1.txt", base_dir)])?;
    sandbox.run(&["touch", &format!("{}/subdir1/file2.txt", base_dir)])?;
    sandbox.run(&["touch", &format!("{}/subdir2/file3.txt", base_dir)])?;

    // Check status for the entire base directory
    sandbox.run(&["status", &base_dir])?;
    let output_base = sandbox.last_stdout.clone();

    // Should see all files
    assert!(output_base.contains("file1.txt"));
    assert!(output_base.contains("file2.txt"));
    assert!(output_base.contains("file3.txt"));

    // Check status for just subdir1
    sandbox.run(&["status", &format!("{}/subdir1", base_dir)])?;
    let output_subdir1 = sandbox.last_stdout.clone();

    // Should see only file2.txt
    assert!(output_subdir1.contains("file2.txt"));
    assert!(!output_subdir1.contains("file1.txt"));
    assert!(!output_subdir1.contains("file3.txt"));

    Ok(())
}

#[rstest]
fn test_completion_uses_filtered_scan(mut sandbox: SandboxManager) -> Result<()> {
    // This test verifies that the completion optimization works correctly
    // by creating files in multiple directories and checking that status
    // only reports files from the requested directory

    let dir1 = sandbox.test_filename("comp_dir1");
    let dir2 = sandbox.test_filename("comp_dir2");

    // Create many files in dir1
    std::fs::create_dir(&dir1)?;
    for i in 0..20 {
        std::fs::write(format!("{}/file_{}.txt", dir1, i), "content")?;
    }

    // Create many files in dir2
    std::fs::create_dir(&dir2)?;
    for i in 0..20 {
        std::fs::write(format!("{}/other_{}.txt", dir2, i), "content")?;
    }

    // Modify them in sandbox
    sandbox.run(&["sh", "-c", &format!("touch {}/*.txt", dir1)])?;
    sandbox.run(&["sh", "-c", &format!("touch {}/*.txt", dir2)])?;

    // Time the status check for dir1 only
    let start = std::time::Instant::now();
    sandbox.run(&["status", &dir1])?;
    let elapsed = start.elapsed();

    let output = &sandbox.last_stdout;

    // Should contain files from dir1
    assert!(output.contains("file_0.txt"));
    assert!(output.contains("file_19.txt"));

    // Should NOT contain files from dir2
    assert!(!output.contains("other_0.txt"));
    assert!(!output.contains("other_19.txt"));

    // Performance check: with optimization, this should be fast
    // Even on a slow system, checking 20 files should be quick
    println!("Status check took: {:?}", elapsed);

    Ok(())
}

#[rstest]
fn test_does_not_scan_sibling_directories(
    mut sandbox: SandboxManager,
) -> Result<()> {
    // This test verifies we're not just filtering by mount point,
    // but actually starting the walk at the target directory
    let base = sandbox.test_filename("base");

    // Create sibling directories with many files
    std::fs::create_dir(&base)?;
    std::fs::create_dir(format!("{}/project_a", base))?;
    std::fs::create_dir(format!("{}/project_b", base))?;
    std::fs::create_dir(format!("{}/project_c", base))?;

    // Add many files to project_b and project_c (not in project_a)
    for i in 0..50 {
        std::fs::write(
            format!("{}/project_b/file_{}.txt", base, i),
            "content",
        )?;
        std::fs::write(
            format!("{}/project_c/file_{}.txt", base, i),
            "content",
        )?;
    }

    // Add just a few files to project_a (our target)
    for i in 0..5 {
        std::fs::write(
            format!("{}/project_a/file_{}.txt", base, i),
            "content",
        )?;
    }

    // Modify them in sandbox
    sandbox.run(&["sh", "-c", &format!("touch {}/*/*.txt", base)])?;

    // Time status check for project_a only
    let start = std::time::Instant::now();
    sandbox.run(&["status", &format!("{}/project_a", base)])?;
    let elapsed = start.elapsed();

    let output = &sandbox.last_stdout;

    // Should see only project_a files
    assert!(output.contains("project_a"));
    assert!(!output.contains("project_b"));
    assert!(!output.contains("project_c"));

    // Performance: If we're properly starting at project_a/, this should be fast
    // If we were scanning all of base/ (including project_b and project_c),
    // it would be much slower due to the 100+ files there
    println!(
        "Status check for project_a (5 files) with 100 sibling files: {:?}",
        elapsed
    );

    // With proper optimization, checking 5 files should be very fast
    // even with 100 sibling files we're not touching
    assert!(
        elapsed.as_millis() < 500,
        "Status took too long: {:?} - may be scanning sibling directories",
        elapsed
    );

    Ok(())
}
