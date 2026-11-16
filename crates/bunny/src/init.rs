//! Talking to the init system.
//!
//! Everything goes through the [`InitSystem`] trait so systemd can be added
//! later as a second implementation. Version 1 only has OpenRC.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::Init;

pub trait InitSystem {
    /// Human-readable name, for messages.
    fn name(&self) -> &'static str;
    /// Every service that starts during boot, including low-level ones.
    fn enabled_services(&self) -> io::Result<BTreeSet<String>>;
    /// Services the user chose to start at boot (not the system's own
    /// early-boot services). This is what goes in `bunny.toml`.
    fn user_services(&self) -> io::Result<BTreeSet<String>>;
    /// Whether a service with this name is installed.
    fn service_exists(&self, name: &str) -> bool;
}

pub fn for_config(init: Init, root: &Path) -> Box<dyn InitSystem> {
    match init {
        Init::Openrc => Box::new(OpenRc::new(root)),
    }
}

pub struct OpenRc {
    root: PathBuf,
}

impl OpenRc {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Lists a runlevel's services. Doesn't need root.
    fn runlevel(&self, name: &str) -> io::Result<BTreeSet<String>> {
        match std::fs::read_dir(self.root.join("etc/runlevels").join(name)) {
            Ok(entries) => entries
                .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeSet::new()),
            Err(e) => Err(e),
        }
    }
}

impl InitSystem for OpenRc {
    fn name(&self) -> &'static str {
        "OpenRC"
    }

    /// The `sysinit`, `boot` and `default` runlevels together. Some services
    /// people care about (like elogind) live in `boot`, not `default`.
    fn enabled_services(&self) -> io::Result<BTreeSet<String>> {
        let mut all = self.runlevel("sysinit")?;
        all.extend(self.runlevel("boot")?);
        all.extend(self.runlevel("default")?);
        Ok(all)
    }

    fn user_services(&self) -> io::Result<BTreeSet<String>> {
        self.runlevel("default")
    }

    fn service_exists(&self, name: &str) -> bool {
        self.root.join("etc/init.d").join(name).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(root: &Path, runlevel: &str, service: &str) {
        let dir = root.join("etc/runlevels").join(runlevel);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(service), "").unwrap();
        std::fs::create_dir_all(root.join("etc/init.d")).unwrap();
        std::fs::write(root.join("etc/init.d").join(service), "").unwrap();
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_runlevels() {
        let dir = tempfile::tempdir().unwrap();
        add(dir.path(), "default", "sshd");
        add(dir.path(), "boot", "elogind");
        add(dir.path(), "sysinit", "udev");

        let openrc = OpenRc::new(dir.path());
        assert_eq!(
            openrc.enabled_services().unwrap(),
            set(&["elogind", "sshd", "udev"])
        );
        assert_eq!(openrc.user_services().unwrap(), set(&["sshd"]));
        assert!(openrc.service_exists("sshd"));
        assert!(!openrc.service_exists("bluetooth"));
    }

    #[test]
    fn missing_runlevels_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(OpenRc::new(dir.path())
            .enabled_services()
            .unwrap()
            .is_empty());
    }
}
