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
    /// Packages to install that don't exist in any package repository
    /// (usually a typo). switch refuses to start while there are any.
    pub unknown_packages: Vec<String>,
    /// Services enabled at boot that the config doesn't mention.
    pub extra_services: Vec<String>,
    /// Services set to start at boot whose start script no longer exists,
    /// usually left behind when a package was removed.
    pub leftover_services: Vec<String>,
    /// The removals were asked for (`bunny remove`), so don't suggest keeping them.
    pub removal_requested: bool,
    /// Admin rights (`[security]`).
    pub security: crate::security::Changes,
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
            && self.security.is_empty()
    }

    /// A one-line summary, e.g. "+ tmux, + cronie, start cronie".
    pub fn summary(&self) -> String {
        let short = |atom: &String| {
            let name = atom.split(':').next().unwrap_or(atom);
            name.rsplit('/').next().unwrap_or(name).to_string()
        };
        let mut parts: Vec<String> = Vec::new();
        parts.extend(self.add_packages.iter().map(|a| format!("+ {}", short(a))));
        parts.extend(
            self.remove_packages
                .iter()
                .map(|a| format!("- {}", short(a))),
        );
        if !self.use_changes.is_empty() {
            let names: Vec<_> = self.use_changes.iter().map(|c| short(&c.atom)).collect();
            parts.push(format!("USE flags for {}", names.join(", ")));
        }
        parts.extend(self.enable_services.iter().map(|s| format!("start {s}")));
        if !self.security.is_empty() {
            if let Some(p) = self.security.privilege {
                parts.push(format!("admin rights: {}", p.command()));
            }
        }

        const MAX: usize = 6;
        let extra = parts.len().saturating_sub(MAX);
        parts.truncate(MAX);
        let mut text = parts.join(", ");
        if extra > 0 {
            text.push_str(&format!(" and {extra} more"));
        }
        text
    }

    /// The plan, explained for humans.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if self.is_empty() {
            out.push_str("✓ This system already matches your config. Nothing to do.\n");
        }

        if !self.unknown_packages.is_empty() {
            out.push_str("✗ These packages don't exist in your package repositories:\n");
            for name in &self.unknown_packages {
                let _ = writeln!(out, "  ? {name}");
            }
            out.push_str(
                "  Check the spelling at https://packages.gentoo.org and fix it in your config.\n\n",
            );
        }

        let installable: Vec<String> = self
            .add_packages
            .iter()
            .filter(|p| !self.unknown_packages.contains(p))
            .cloned()
            .collect();
        section(&mut out, "Packages to install", "+", &installable);
        if !self.remove_packages.is_empty() {
            section(
                &mut out,
                "Packages not in your config (would be removed)",
                "-",
                &self.remove_packages,
            );
            if !self.removal_requested {
                out.push_str("  Add them to `packages` if you want to keep them.\n");
            }
            out.push('\n');
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

        if !self.leftover_services.is_empty() {
            out.push_str(
                "Warning: these are set to start at boot, but they're no longer installed\n\
                 (probably left behind when a package was removed). Your init system will\n\
                 complain about them on every boot:\n",
            );
            for name in &self.leftover_services {
                let _ = writeln!(
                    out,
                    "  ! {name}    fix: {} rc-update del {name}",
                    crate::security::admin()
                );
            }
            out.push('\n');
        }

        if !self.security.problems.is_empty() {
            out.push_str("✗ [security] would lock you out, so bunny won't apply it:\n");
            for problem in &self.security.problems {
                let _ = writeln!(out, "  • {problem}");
            }
            out.push('\n');
        }
        if let (false, Some(p)) = (self.security.is_empty(), self.security.privilege) {
            let _ = writeln!(out, "Admin rights ({}):", p.command());
            if let Some((path, _)) = &self.security.file {
                let _ = writeln!(
                    out,
                    "  ~ write {} so the wheel group can use {} (checked before it's put in place)",
                    path.display(),
                    p.command()
                );
            }
            for user in &self.security.add_to_wheel {
                let _ = writeln!(out, "  + add {user} to the wheel group");
            }
            out.push('\n');
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

/// Packages in `wanted` that no repository in `/var/db/repos` has and that
/// aren't installed. Skipped (empty) when there are no repositories to check.
/// Uses the same package index as `bunny search` and `bunny add`.
fn unknown_packages(root: &Path, wanted: &[String]) -> Vec<String> {
    if wanted.is_empty() {
        return Vec::new();
    }
    let available: BTreeSet<String> = crate::repo::all(root).into_iter().map(|p| p.name).collect();
    if available.is_empty() {
        return Vec::new();
    }
    let installed: BTreeSet<String> = crate::lock::installed_packages(root)
        .unwrap_or_default()
        .into_keys()
        .map(|k| k.split(':').next().unwrap_or(&k).to_string())
        .collect();
    wanted
        .iter()
        .filter(|atom| {
            let plain = atom.split(':').next().unwrap_or(atom);
            !available.contains(plain) && !installed.contains(plain)
        })
        .cloned()
        .collect()
}

/// Compares the flags in effect (from every package.use file) with the config.
/// Order doesn't matter: `pgo -telemetry` equals `-telemetry pgo`.
fn use_changes(config: &Config, root: &Path) -> Result<Vec<UseChange>> {
    let current = import::read_package_use(root)?.flags;
    let effective = config.use_flags();
    let wanted: BTreeMap<&String, &Vec<String>> = effective.iter().collect();

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
        unknown_packages: unknown_packages(
            root,
            &wanted_world
                .difference(&current_world)
                .cloned()
                .collect::<Vec<_>>(),
        ),
        add_packages: wanted_world.difference(&current_world).cloned().collect(),
        remove_packages: current_world.difference(&wanted_world).cloned().collect(),
        use_changes,
        enable_services: wanted_services.difference(&enabled).cloned().collect(),
        missing_services: wanted_services
            .iter()
            .filter(|s| !init.service_exists(s) && !enabled.contains(*s))
            .cloned()
            .collect(),
        extra_services: user_enabled
            .difference(&wanted_services)
            .filter(|s| init.service_exists(s))
            .cloned()
            .collect(),
        removal_requested: false,
        security: config
            .security
            .as_ref()
            .map(|s| crate::security::changes(s, root))
            .unwrap_or_default(),
        leftover_services: enabled
            .iter()
            .filter(|s| !init.service_exists(s))
            .cloned()
            .collect(),
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
        assert_eq!(
            plan.summary(),
            "+ firefox, - old-thing, USE flags for firefox, start bluetooth, start sshd"
        );
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
    fn finds_leftover_services() {
        let root = fake_root();
        // Enabled at boot, but no start script: left behind by a removed package.
        std::fs::write(root.path().join("etc/runlevels/default/tlp"), "").unwrap();
        let config = Config::parse("services = [\"elogind\", \"tlp\"]").unwrap();
        let plan = make(&config, root.path()).unwrap();
        assert_eq!(plan.leftover_services, ["tlp"]);
        assert!(plan.missing_services.is_empty());
        assert!(plan
            .render()
            .contains(&format!("{} rc-update del tlp", crate::security::admin())));
    }

    #[test]
    fn finds_packages_that_dont_exist() {
        let root = fake_root();
        let r = root.path();
        std::fs::create_dir_all(r.join("var/db/repos/gentoo/app-misc/tmux")).unwrap();
        std::fs::create_dir_all(r.join("var/db/repos/gentoo/net-misc/freerdp")).unwrap();
        // Installed from a repository that's gone: still counts as known.
        std::fs::create_dir_all(r.join("var/db/pkg/app-misc/oldtool-1.0")).unwrap();
        let config = Config::parse(
            r#"packages = ["app-misc/tmux", "app-misc/tmuxx", "net-misc/freerdp:3", "app-misc/oldtool"]"#,
        )
        .unwrap();
        let plan = make(&config, r).unwrap();
        assert_eq!(plan.unknown_packages, ["app-misc/tmuxx"]);
        assert!(plan
            .render()
            .contains("don't exist in your package repositories"));
        assert!(!plan.render().contains("+ app-misc/tmuxx"));
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
