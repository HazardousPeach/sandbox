use anyhow::{Context, Result};
use colored::*;
use log::debug;
use std::fs;
use std::path::Path;

pub fn rmdir_recursive(path: &Path) -> Result<()> {
    let root_device = nix::sys::stat::lstat(path)
        .context(format!("failed to stat {}", path.display()))?
        .st_dev;

    for entry in fs::read_dir(path)
        .context(format!("failed to read directory {}", path.display()))?
    {
        let entry = entry.context(format!(
            "failed to read directory entry in {}",
            path.display()
        ))?;
        let entry_path = entry.path();

        let entry_stat = nix::sys::stat::lstat(&entry_path)
            .context(format!("failed to stat {}", entry_path.display()))?;

        let entry_device = entry_stat.st_dev;

        #[cfg(feature = "coverage")]
        let entry_device =
            if std::env::var_os("TEST_ACCEPT_FAIL_RMDIR_ON_DIFFERENT_DEVICE")
                .is_some()
            {
                0
            } else {
                entry_device
            };

        if entry_device != root_device {
            return Err(anyhow::anyhow!(
                "Cannot remove {}: entry is on a different device",
                entry_path.display()
            ));
        }

        if entry_stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
            rmdir_recursive(&entry_path)?;
        } else {
            debug!("{}", format!("rm {}", entry_path.display()).bright_black());
            fs::remove_file(&entry_path).context(format!(
                "failed to remove {}",
                entry_path.display()
            ))?;
        }
    }

    debug!("{}", format!("rmdir {}", path.display()).bright_black());
    fs::remove_dir(path)
        .context(format!("failed to remove directory {}", path.display()))?;
    Ok(())
}
