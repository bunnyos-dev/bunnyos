//! `bunny plan`: compare the config to a system and describe the difference.
//! Read-only: nothing here changes the system.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::init;
use crate::portage;

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub init_name: &'static str,
    /// Packages in the config but not in the world set.
    pub add_packages: Vec<String>,
    /// Packages in the world set but not in the config.
    pub remove_packages: Vec<String>,
    /// New contents for bunny's package.use file, if it changes.
    pub package_use: Option<String>,
    /// Services to enable at boot.
    pub enable_services: Vec<String>,
    /// Services in the config that aren't installed (yet).
    pub missing_services: Vec<String>,
    /// Services enabled at boot that the config doesn't mention.
    pub extra_services: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.add_packages.is_empty()
            && self.remove_packages.is_empty()
            && self.package_use.is_none()
            && self.enable_services.is_empty()
    }

    /// The plan, explained for humans.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if self.is_empty() {
            out.push_str("✓ This system already matches your config. Nothing to do.\n");
        }

        section(&mut out, "Packages to install", "+", &self.add_packages);
        if !self.remove_packages.is_empty() {
            section(
                &mut out,
                "Packages not in your config (would be removed)",
                "-",
                &self.remove_packages,
            );
            out.push_str("  Add them to `packages` if you want to keep them.\n\n");
        }

        if let Some(text) = &self.package_use {
            out.push_str("USE flag settings will be updated:\n");
            for line in text.lines().filter(|l| !l.starts_with('#')) {
                let _ = writeln!(out, "  {line}");
            }
            out.push('\n');
        }

        let heading = format!("Services to start at boot ({})", self.init_name);
        section(&mut out, &heading, "+", &self.enable_services);

        if !self.missing_services.is_empty() {
            out.push_str("Note: these services aren't installed yet:\n");
            for name in &self.missing_services {
                let _ = writeln!(out, "  ? {name}");
            }
            out.push_str(
                "  If a package in your config provides them, they'll be enabled after it's installed.\n\n",
            );
        }

        if !self.extra_services.is_empty() {
            out.push_str("Also starting at boot, but not in your config (left alone):\n");
            for name in &self.extra_services {
                let _ = writeln!(out, "  · {name}");
            }
            out.push('\n');
        }

        out.trim_end().to_string() + "\n"
    }
}

fn section(out: &mut String, heading: &str, mark: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    let _ = writeln!(out, "{heading}:");
    for item in items {
        let _ = writeln!(out, "  {mark} {item}");
    }
    out.push('\n');
}

/// Compare `config` to the system rooted at `root`.
pub fn make(config: &Config, root: &Path) -> Result<Plan> {
    let current_world = portage::read_world(root).context("Couldn't read the world set")?;
    let wanted_world = portage::wanted_world(config);

    let new_use = portage::render_package_use(config);
    let old_use = std::fs::read_to_string(portage::package_use_path(root)).ok();
    let has_flags = config.pkg.values().any(|s| !s.use_flags.is_empty());
    let package_use = match old_use {
        Some(old) if old == new_use => None,
        None if !has_flags => None,
        _ => Some(new_use),
    };

    let init = init::for_config(config.system.init, root);
    let enabled = init
        .enabled_services()
        .context("Couldn't read which services start at boot")?;
    let wanted_services: BTreeSet<String> = config.services.iter().cloned().collect();

    Ok(Plan {
        init_name: init.name(),
        add_packages: wanted_world.difference(&current_world).cloned().collect(),
        remove_packages: current_world.difference(&wanted_world).cloned().collect(),
        package_use,
        enable_services: wanted_services.difference(&enabled).cloned().collect(),
        missing_services: wanted_services
            .iter()
            .filter(|s| !init.service_exists(s))
            .cloned()
            .collect(),
        extra_services: enabled.difference(&wanted_services).cloned().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for d in ["var/lib/portage", "etc/runlevels/default", "etc/init.d"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(
            root.join(portage::WORLD_FILE),
            "app-editors/neovim\napp-misc/old-thing\n",
        )
        .unwrap();
        for s in ["sshd", "elogind"] {
            std::fs::write(root.join("etc/init.d").join(s), "").unwrap();
        }
        std::fs::write(root.join("etc/runlevels/default/elogind"), "").unwrap();
        dir
    }

    #[test]
    fn plans_the_difference() {
        let root = fake_root();
        let config = Config::parse(
            r#"
            packages = ["app-editors/neovim", "www-client/firefox"]
            services = ["sshd", "bluetooth"]
            [pkg."www-client/firefox"]
            use = ["pgo"]
            "#,
        )
        .unwrap();
        let plan = make(&config, root.path()).unwrap();
        assert_eq!(plan.add_packages, ["www-client/firefox"]);
        assert_eq!(plan.remove_packages, ["app-misc/old-thing"]);
        assert!(plan.package_use.as_deref().unwrap().contains("pgo"));
        assert_eq!(plan.enable_services, ["bluetooth", "sshd"]);
        assert_eq!(plan.missing_services, ["bluetooth"]);
        assert_eq!(plan.extra_services, ["elogind"]);
        assert!(!plan.is_empty());
        assert!(plan.render().contains("Packages to install"));
    }

    #[test]
    fn matching_system_has_empty_plan() {
        let root = fake_root();
        let config = Config::parse(
            r#"
            packages = ["app-editors/neovim", "app-misc/old-thing"]
            services = ["elogind"]
            "#,
        )
        .unwrap();
        let plan = make(&config, root.path()).unwrap();
        assert!(plan.is_empty(), "{plan:#?}");
        assert!(plan.render().contains("already matches"));
    }
}
