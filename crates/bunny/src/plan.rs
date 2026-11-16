//! `bunny plan`: compare the config to a system and describe the difference.
//! Read-only: nothing here changes the system.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::import;
use crate::init;
use crate::portage;

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub init_name: &'static str,
    /// Packages in the config but not in the world set.
    pub add_packages: Vec<String>,
    /// Packages in the world set but not in the config.
    pub remove_packages: Vec<String>,
    /// Packages whose USE flags would change.
    pub use_changes: Vec<UseChange>,
    /// Services to enable at boot.
    pub enable_services: Vec<String>,
    /// Services in the config that aren't installed (yet).
    pub missing_services: Vec<String>,
    /// Services enabled at boot that the config doesn't mention.
    pub extra_services: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct UseChange {
    pub atom: String,
    /// Flags set on the system now (empty if none).
    pub from: Vec<String>,
    /// Flags the config asks for (empty if none).
    pub to: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.add_packages.is_empty()
            && self.remove_packages.is_empty()
            && self.use_changes.is_empty()
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

        if !self.use_changes.is_empty() {
            out.push_str("USE flag changes:\n");
            for change in &self.use_changes {
                let show = |flags: &[String]| {
                    if flags.is_empty() {
                        "(none)".to_string()
                    } else {
                        flags.join(" ")
                    }
                };
                let _ = writeln!(
                    out,
                    "  ~ {}: {} → {}",
                    change.atom,
                    show(&change.from),
                    show(&change.to)
                );
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

/// Compares the flags in effect (from every package.use file) with the config.
/// Order doesn't matter: `pgo -telemetry` equals `-telemetry pgo`.
fn use_changes(config: &Config, root: &Path) -> Result<Vec<UseChange>> {
    let current = import::read_package_use(root)?.flags;
    let wanted: BTreeMap<&String, &Vec<String>> = config
        .pkg
        .iter()
        .filter(|(_, s)| !s.use_flags.is_empty())
        .map(|(atom, s)| (atom, &s.use_flags))
        .collect();

    let as_set = |flags: &[String]| flags.iter().cloned().collect::<BTreeSet<_>>();
    let mut atoms: BTreeSet<&String> = wanted.keys().copied().collect();
    atoms.extend(current.keys().filter(|a| !current[*a].is_empty()));

    Ok(atoms
        .into_iter()
        .filter_map(|atom| {
            let from = current.get(atom).cloned().unwrap_or_default();
            let to = wanted.get(atom).map(|f| (*f).clone()).unwrap_or_default();
            (as_set(&from) != as_set(&to)).then(|| UseChange {
                atom: atom.clone(),
                from,
                to,
            })
        })
        .collect())
}

/// Compare `config` to the system rooted at `root`.
pub fn make(config: &Config, root: &Path) -> Result<Plan> {
    let current_world = portage::read_world(root).context("Couldn't read the world set")?;
    let wanted_world = portage::wanted_world(config);

    let use_changes = use_changes(config, root)?;

    let init = init::for_config(config.system.init, root);
    let enabled = init
        .enabled_services()
        .context("Couldn't read which services start at boot")?;
    let user_enabled = init
        .user_services()
        .context("Couldn't read which services start at boot")?;
    let wanted_services: BTreeSet<String> = config.services.iter().cloned().collect();

    Ok(Plan {
        init_name: init.name(),
        add_packages: wanted_world.difference(&current_world).cloned().collect(),
        remove_packages: current_world.difference(&wanted_world).cloned().collect(),
        use_changes,
        enable_services: wanted_services.difference(&enabled).cloned().collect(),
        missing_services: wanted_services
            .iter()
            .filter(|s| !init.service_exists(s))
            .cloned()
            .collect(),
        extra_services: user_enabled.difference(&wanted_services).cloned().collect(),
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
        assert_eq!(
            plan.use_changes,
            [UseChange {
                atom: "www-client/firefox".into(),
                from: vec![],
                to: vec!["pgo".into()],
            }]
        );
        assert_eq!(plan.enable_services, ["bluetooth", "sshd"]);
        assert_eq!(plan.missing_services, ["bluetooth"]);
        assert_eq!(plan.extra_services, ["elogind"]);
        assert!(!plan.is_empty());
        assert!(plan.render().contains("Packages to install"));
    }

    #[test]
    fn flags_already_set_in_other_files_need_no_change() {
        let root = fake_root();
        let file = root.path().join("etc/portage/package.use/mine");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "www-client/firefox -telemetry pgo\napp-misc/old a\n").unwrap();
        let config =
            Config::parse("[pkg.\"www-client/firefox\"]\nuse = [\"pgo\", \"-telemetry\"]").unwrap();
        let plan = make(&config, root.path()).unwrap();
        // firefox matches; app-misc/old's flag isn't in the config.
        assert_eq!(
            plan.use_changes,
            [UseChange {
                atom: "app-misc/old".into(),
                from: vec!["a".into()],
                to: vec![],
            }]
        );
    }

    #[test]
    fn services_in_the_boot_runlevel_count_as_enabled() {
        let root = fake_root();
        let boot = root.path().join("etc/runlevels/boot");
        std::fs::create_dir_all(&boot).unwrap();
        std::fs::write(boot.join("sshd"), "").unwrap();
        let config = Config::parse("services = [\"sshd\", \"elogind\"]").unwrap();
        let plan = make(&config, root.path()).unwrap();
        assert!(plan.enable_services.is_empty(), "{plan:#?}");
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
