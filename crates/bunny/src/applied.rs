//! The config that matches the system: a copy kept inside the system.
//!
//! A generation is a snapshot taken *before* a change, but by then the
//! config has usually already changed (`bunny add` edits it first; so do
//! you). So after each successful switch, bunny keeps a copy of the config
//! and lock it just applied in `/var/lib/bunny/applied/`. That folder is part
//! of the system, so every later snapshot carries the config that matches it.
//! On rollback (from bunny, or the boot menu) the copy in the restored system
//! is put back where the config lives, so config and system match again.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Relative to the root.
pub const DIR: &str = "var/lib/bunny/applied";
const CONFIG: &str = "bunny.toml";
const LOCK: &str = "bunny.lock";
/// Where the config lives, so a rollback knows where to put it back.
const PATH_FILE: &str = "config-path";

/// Keeps a copy of the config (and its lock) that was just applied.
pub fn save(root: &Path, config: &Path) -> Result<()> {
    let dir = root.join(DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    std::fs::copy(config, dir.join(CONFIG))
        .with_context(|| format!("Couldn't keep a copy of {}", config.display()))?;
    let lock = crate::lock::path_for(config);
    if lock.exists() {
        std::fs::copy(&lock, dir.join(LOCK))
            .with_context(|| format!("Couldn't keep a copy of {}", lock.display()))?;
    } else {
        let _ = std::fs::remove_file(dir.join(LOCK));
    }
    let absolute = if config.is_absolute() {
        config.to_path_buf()
    } else {
        std::env::current_dir()?.join(config)
    };
    std::fs::write(dir.join(PATH_FILE), format!("{}\n", absolute.display()))?;
    Ok(())
}

/// Puts the saved config back in a restored system (mounted at `system`).
/// Only for a config inside the system itself (under /etc): one in /home is
/// on its own subvolume and isn't rolled back, so it's left alone. Returns
/// the config's path if it was put back.
pub fn restore_into(system: &Path) -> Result<Option<PathBuf>> {
    let dir = system.join(DIR);
    let Ok(recorded) = std::fs::read_to_string(dir.join(PATH_FILE)) else {
        return Ok(None);
    };
    let config = PathBuf::from(recorded.trim());
    let Ok(inside) = config.strip_prefix("/etc") else {
        return Ok(None);
    };
    if !dir.join(CONFIG).is_file() || inside.as_os_str().is_empty() {
        return Ok(None);
    }
    let target = system.join("etc").join(inside);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(dir.join(CONFIG), &target)
        .with_context(|| format!("Couldn't put back {}", config.display()))?;
    let lock_target = crate::lock::path_for(&target);
    if dir.join(LOCK).is_file() {
        std::fs::copy(dir.join(LOCK), &lock_target)?;
    } else {
        let _ = std::fs::remove_file(&lock_target);
    }
    Ok(Some(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_puts_back_a_config_under_etc() {
        let system = tempfile::tempdir().unwrap();
        let s = system.path();
        std::fs::create_dir_all(s.join(DIR)).unwrap();
        std::fs::write(s.join(DIR).join(CONFIG), "packages = [\"app-misc/tmux\"]\n").unwrap();
        std::fs::write(s.join(DIR).join(LOCK), "version = 1\n").unwrap();
        std::fs::write(s.join(DIR).join(PATH_FILE), "/etc/bunny/bunny.toml\n").unwrap();
        // The restored system has the config as edited after the snapshot.
        std::fs::create_dir_all(s.join("etc/bunny")).unwrap();
        std::fs::write(
            s.join("etc/bunny/bunny.toml"),
            "packages = [\"app-misc/tmux\", \"sys-process/htop\"]\n",
        )
        .unwrap();

        let put_back = restore_into(s).unwrap();
        assert_eq!(put_back, Some(PathBuf::from("/etc/bunny/bunny.toml")));
        assert_eq!(
            std::fs::read_to_string(s.join("etc/bunny/bunny.toml")).unwrap(),
            "packages = [\"app-misc/tmux\"]\n"
        );
        assert_eq!(
            std::fs::read_to_string(s.join("etc/bunny/bunny.lock")).unwrap(),
            "version = 1\n"
        );
    }

    #[test]
    fn leaves_a_config_in_home_alone() {
        let system = tempfile::tempdir().unwrap();
        let s = system.path();
        std::fs::create_dir_all(s.join(DIR)).unwrap();
        std::fs::write(s.join(DIR).join(CONFIG), "x").unwrap();
        std::fs::write(s.join(DIR).join(PATH_FILE), "/home/you/bunny.toml\n").unwrap();
        assert_eq!(restore_into(s).unwrap(), None);
    }

    #[test]
    fn nothing_saved_means_nothing_to_put_back() {
        let system = tempfile::tempdir().unwrap();
        assert_eq!(restore_into(system.path()).unwrap(), None);
    }

    #[test]
    fn save_records_the_config_and_lock() {
        let root = tempfile::tempdir().unwrap();
        let etc = tempfile::tempdir().unwrap();
        let config = etc.path().join("bunny.toml");
        std::fs::write(&config, "packages = []\n").unwrap();
        std::fs::write(etc.path().join("bunny.lock"), "version = 1\n").unwrap();
        save(root.path(), &config).unwrap();
        let dir = root.path().join(DIR);
        assert_eq!(
            std::fs::read_to_string(dir.join(CONFIG)).unwrap(),
            "packages = []\n"
        );
        assert!(dir.join(LOCK).is_file());
        assert_eq!(
            std::fs::read_to_string(dir.join(PATH_FILE)).unwrap().trim(),
            config.display().to_string()
        );
    }
}
