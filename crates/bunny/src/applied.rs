//! The config that matches the system: a copy kept inside the system.
//!
//! A generation is a snapshot taken *before* a change, but by then the
//! config has usually already changed (`bunny add` edits it first; so do
//! you). So after each successful switch, bunny keeps a copy of the config
//! and lock it just applied in `/var/lib/bunny/applied/`. That folder is part
//! of the system, so every later snapshot carries the config that matches it.
//! On rollback (from bunny, or the boot menu) the copy in the restored system
//! is put back where the config lives, so config and system match again.
//! This computer's host file (`hosts/<name>.toml`) is kept and put back the
//! same way, since it's part of what was applied.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Relative to the root.
pub const DIR: &str = "var/lib/bunny/applied";
const CONFIG: &str = "bunny.toml";
const LOCK: &str = "bunny.lock";
/// Where the config lives, so a rollback knows where to put it back.
const PATH_FILE: &str = "config-path";
/// This computer's host file as applied, and where it lives.
const HOST: &str = "host.toml";
const HOST_PATH_FILE: &str = "host-path";

/// What a rollback put back.
#[derive(Debug, PartialEq)]
pub struct Restored {
    pub config: PathBuf,
    /// The host file, if one was applied with the config.
    pub host: Option<PathBuf>,
}

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
    // This computer's host file, read on top of the config (borrowed
    // from /etc/bunny/hosts for a copy with no hosts/ folder, the same
    // one the switch used).
    let system_hosts = root.join(crate::config::SYSTEM_HOSTS.trim_start_matches('/'));
    let host = crate::config::this_host(root).and_then(|name| {
        crate::config::host_file_for(&absolute, &name, Some(&system_hosts))
            .ok()
            .flatten()
            .map(|(path, _)| path)
    });
    match host {
        Some(path) => {
            std::fs::copy(&path, dir.join(HOST))
                .with_context(|| format!("Couldn't keep a copy of {}", path.display()))?;
            std::fs::write(dir.join(HOST_PATH_FILE), format!("{}\n", path.display()))?;
        }
        None => {
            let _ = std::fs::remove_file(dir.join(HOST));
            let _ = std::fs::remove_file(dir.join(HOST_PATH_FILE));
        }
    }
    Ok(())
}

/// Puts the saved config back in a restored system (mounted at `system`).
/// Only for a config inside the system itself (under /etc): one in /home is
/// on its own subvolume and isn't rolled back, so it's left alone. The host
/// file goes back too, under the same rule. Returns what was put back.
pub fn restore_into(system: &Path) -> Result<Option<Restored>> {
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
    // A config that's a link (say into /home) lives outside the system:
    // writing through the link would change the real file, which a rollback
    // shouldn't touch. Left alone, like a config in /home.
    let is_link = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink());
    if is_link(&target) || is_link(&crate::lock::path_for(&target)) {
        return Ok(None);
    }
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
    let host = restore_host(system, &dir)?;
    Ok(Some(Restored { config, host }))
}

/// Puts the saved host file back, if there is one and it lives under /etc.
fn restore_host(system: &Path, dir: &Path) -> Result<Option<PathBuf>> {
    let Ok(recorded) = std::fs::read_to_string(dir.join(HOST_PATH_FILE)) else {
        return Ok(None);
    };
    let host = PathBuf::from(recorded.trim());
    let Ok(inside) = host.strip_prefix("/etc") else {
        return Ok(None);
    };
    let safe = inside
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)));
    if !safe || inside.as_os_str().is_empty() || !dir.join(HOST).is_file() {
        return Ok(None);
    }
    let target = system.join("etc").join(inside);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(dir.join(HOST), &target)
        .with_context(|| format!("Couldn't put back {}", host.display()))?;
    Ok(Some(host))
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

        let put_back = restore_into(s).unwrap().unwrap();
        assert_eq!(put_back.config, PathBuf::from("/etc/bunny/bunny.toml"));
        assert_eq!(put_back.host, None);
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
    fn puts_the_host_file_back_too() {
        let system = tempfile::tempdir().unwrap();
        let s = system.path();
        std::fs::create_dir_all(s.join(DIR)).unwrap();
        std::fs::write(s.join(DIR).join(CONFIG), "packages = []\n").unwrap();
        std::fs::write(s.join(DIR).join(PATH_FILE), "/etc/bunny/bunny.toml\n").unwrap();
        std::fs::write(
            s.join(DIR).join(HOST),
            "packages = [\"sys-kernel/gentoo-kernel-bin\"]\n",
        )
        .unwrap();
        std::fs::write(
            s.join(DIR).join(HOST_PATH_FILE),
            "/etc/bunny/hosts/laptop.toml\n",
        )
        .unwrap();
        // Edited by hand after the switch that was applied.
        std::fs::create_dir_all(s.join("etc/bunny/hosts")).unwrap();
        std::fs::write(
            s.join("etc/bunny/hosts/laptop.toml"),
            "packages = [\"x11-drivers/nvidia-drivers\"]\n",
        )
        .unwrap();

        let put_back = restore_into(s).unwrap().unwrap();
        assert_eq!(
            put_back.host,
            Some(PathBuf::from("/etc/bunny/hosts/laptop.toml"))
        );
        assert_eq!(
            std::fs::read_to_string(s.join("etc/bunny/hosts/laptop.toml")).unwrap(),
            "packages = [\"sys-kernel/gentoo-kernel-bin\"]\n"
        );
    }

    #[test]
    fn a_host_path_outside_etc_is_left_alone() {
        let system = tempfile::tempdir().unwrap();
        let s = system.path();
        std::fs::create_dir_all(s.join(DIR)).unwrap();
        std::fs::write(s.join(DIR).join(CONFIG), "x").unwrap();
        std::fs::write(s.join(DIR).join(PATH_FILE), "/etc/bunny/bunny.toml\n").unwrap();
        std::fs::write(s.join(DIR).join(HOST), "x").unwrap();
        for path in ["/etc/../root/x.toml", "/home/you/hosts/laptop.toml"] {
            std::fs::write(s.join(DIR).join(HOST_PATH_FILE), format!("{path}\n")).unwrap();
            assert_eq!(restore_into(s).unwrap().unwrap().host, None, "{path}");
        }
        assert!(!s.join("root/x.toml").exists());
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

    #[test]
    fn leaves_a_linked_config_alone() {
        let system = tempfile::tempdir().unwrap();
        let s = system.path();
        std::fs::create_dir_all(s.join(DIR)).unwrap();
        std::fs::write(s.join(DIR).join(CONFIG), "old\n").unwrap();
        std::fs::write(s.join(DIR).join(PATH_FILE), "/etc/bunny/bunny.toml\n").unwrap();
        // /etc/bunny/bunny.toml links to the real file elsewhere (in /home).
        let real = s.join("home-config.toml");
        std::fs::write(&real, "new\n").unwrap();
        std::fs::create_dir_all(s.join("etc/bunny")).unwrap();
        std::os::unix::fs::symlink(&real, s.join("etc/bunny/bunny.toml")).unwrap();

        assert_eq!(restore_into(s).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new\n");
    }

    #[test]
    fn save_keeps_this_computers_host_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("etc/conf.d")).unwrap();
        std::fs::write(
            root.path().join("etc/conf.d/hostname"),
            "hostname=\"laptop\"\n",
        )
        .unwrap();
        let etc = tempfile::tempdir().unwrap();
        let config = etc.path().join("bunny.toml");
        std::fs::write(&config, "packages = []\n").unwrap();
        std::fs::create_dir_all(etc.path().join("hosts")).unwrap();
        std::fs::write(
            etc.path().join("hosts/laptop.toml"),
            "packages = [\"a/b\"]\n",
        )
        .unwrap();
        std::fs::write(
            etc.path().join("hosts/desktop.toml"),
            "packages = [\"c/d\"]\n",
        )
        .unwrap();
        save(root.path(), &config).unwrap();
        let dir = root.path().join(DIR);
        assert_eq!(
            std::fs::read_to_string(dir.join(HOST)).unwrap(),
            "packages = [\"a/b\"]\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(HOST_PATH_FILE))
                .unwrap()
                .trim(),
            etc.path().join("hosts/laptop.toml").display().to_string()
        );

        // Without a host file any more, the old copy is dropped.
        std::fs::remove_file(etc.path().join("hosts/laptop.toml")).unwrap();
        save(root.path(), &config).unwrap();
        assert!(!dir.join(HOST).exists());
        assert!(!dir.join(HOST_PATH_FILE).exists());
    }

    #[test]
    fn save_keeps_the_borrowed_host_file_for_a_copy() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("etc/conf.d")).unwrap();
        std::fs::write(
            root.path().join("etc/conf.d/hostname"),
            "hostname=\"laptop\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.path().join("etc/bunny/hosts")).unwrap();
        let system_host = root.path().join("etc/bunny/hosts/laptop.toml");
        std::fs::write(&system_host, "packages = [\"a/b\"]\n").unwrap();
        let home = tempfile::tempdir().unwrap();
        let copy = home.path().join("bunny.toml");
        std::fs::write(&copy, "packages = []\n").unwrap();
        save(root.path(), &copy).unwrap();
        let dir = root.path().join(DIR);
        assert_eq!(
            std::fs::read_to_string(dir.join(HOST_PATH_FILE))
                .unwrap()
                .trim(),
            system_host.display().to_string()
        );
    }
}
