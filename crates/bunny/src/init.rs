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
    /// Installs one of bunny's own early start-up services (`script` is its
    /// service file) and starts it at boot. Writes files only, so it works
    /// on a fake root.
    fn add_boot_service(&self, name: &str, script: &str) -> io::Result<()>;
    /// Whether that service is installed as `script` and starts at boot.
    fn has_boot_service(&self, name: &str, script: &str) -> bool;
}

/// The OpenRC runlevel a service is added to. Most go in `default`; elogind
/// goes in `boot`, as Gentoo's elogind instructions say, so logins made early (like the
/// first one on a console) already go through it.
pub fn openrc_runlevel(service: &str) -> &'static str {
    match service {
        "elogind" => "boot",
        _ => "default",
    }
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

    /// Writes `/etc/init.d/<name>` and links it into the `boot` runlevel,
    /// which is what `rc-update add <name> boot` does.
    fn add_boot_service(&self, name: &str, script: &str) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let init_d = self.root.join("etc/init.d");
        std::fs::create_dir_all(&init_d)?;
        let path = init_d.join(name);
        let tmp = init_d.join(format!(".{name}.bunny-new"));
        std::fs::write(&tmp, script)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, &path)?;
        let boot = self.root.join("etc/runlevels/boot");
        std::fs::create_dir_all(&boot)?;
        let link = boot.join(name);
        if std::fs::symlink_metadata(&link).is_err() {
            std::os::unix::fs::symlink(format!("/etc/init.d/{name}"), &link)?;
        }
        Ok(())
    }

    fn has_boot_service(&self, name: &str, script: &str) -> bool {
        std::fs::read_to_string(self.root.join("etc/init.d").join(name))
            .ok()
            .as_deref()
            == Some(script)
            && std::fs::symlink_metadata(self.root.join("etc/runlevels/boot").join(name)).is_ok()
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

    #[test]
    fn adds_a_boot_service() {
        let dir = tempfile::tempdir().unwrap();
        let rc = OpenRc::new(dir.path());
        assert!(!rc.has_boot_service("bunny-x", "#!/sbin/openrc-run\n"));
        rc.add_boot_service("bunny-x", "#!/sbin/openrc-run\n")
            .unwrap();
        // Twice is fine (the link is already there).
        rc.add_boot_service("bunny-x", "#!/sbin/openrc-run\n")
            .unwrap();
        assert!(rc.has_boot_service("bunny-x", "#!/sbin/openrc-run\n"));
        assert!(!rc.has_boot_service("bunny-x", "something else"));
        assert_eq!(
            std::fs::read_link(dir.path().join("etc/runlevels/boot/bunny-x")).unwrap(),
            PathBuf::from("/etc/init.d/bunny-x")
        );
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("etc/init.d/bunny-x"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        // Boot services aren't "your" services.
        assert!(rc.user_services().unwrap().is_empty());
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
    fn elogind_starts_in_the_boot_runlevel() {
        assert_eq!(openrc_runlevel("elogind"), "boot");
        assert_eq!(openrc_runlevel("dbus"), "default");
        assert_eq!(openrc_runlevel("sshd"), "default");
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
