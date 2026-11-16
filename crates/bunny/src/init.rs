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
    /// Services that start at boot.
    fn enabled_services(&self) -> io::Result<BTreeSet<String>>;
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

    fn runlevel_dir(&self) -> PathBuf {
        self.root.join("etc/runlevels/default")
    }
}

impl InitSystem for OpenRc {
    fn name(&self) -> &'static str {
        "OpenRC"
    }

    /// Reads the `default` runlevel. Doesn't need root.
    fn enabled_services(&self) -> io::Result<BTreeSet<String>> {
        match std::fs::read_dir(self.runlevel_dir()) {
            Ok(entries) => entries
                .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeSet::new()),
            Err(e) => Err(e),
        }
    }

    fn service_exists(&self, name: &str) -> bool {
        self.root.join("etc/init.d").join(name).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_default_runlevel() {
        let dir = tempfile::tempdir().unwrap();
        let runlevel = dir.path().join("etc/runlevels/default");
        let initd = dir.path().join("etc/init.d");
        std::fs::create_dir_all(&runlevel).unwrap();
        std::fs::create_dir_all(&initd).unwrap();
        std::fs::write(initd.join("sshd"), "").unwrap();
        std::fs::write(runlevel.join("sshd"), "").unwrap();

        let openrc = OpenRc::new(dir.path());
        assert_eq!(
            openrc.enabled_services().unwrap(),
            BTreeSet::from(["sshd".to_string()])
        );
        assert!(openrc.service_exists("sshd"));
        assert!(!openrc.service_exists("bluetooth"));
    }

    #[test]
    fn missing_runlevel_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(OpenRc::new(dir.path())
            .enabled_services()
            .unwrap()
            .is_empty());
    }
}
