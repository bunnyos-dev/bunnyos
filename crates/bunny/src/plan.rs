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
    /// Packages whose accepted keywords (testing versions) would change.
    pub keyword_changes: Vec<UseChange>,
    /// Graphics cards and CPU features (`[hardware]`) that would change.
    pub hardware_changes: Vec<crate::hardware::Change>,
    /// Repositories to add (in the config, not set up yet).
    pub add_repositories: Vec<String>,
    /// Repositories bunny set up that the config no longer lists.
    pub remove_repositories: Vec<String>,
    /// Repositories set up outside bunny that the config doesn't list.
    pub extra_repositories: Vec<String>,
    /// Some repositories in the config haven't been downloaded yet, so
    /// packages can only be checked after the switch gets them.
    pub check_after_sync: bool,
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
    /// The boot menu's undo entry (`[boot]`).
    pub boot: crate::boot::Changes,
    /// What's left to set up for the console font that draws ✓ ✗ ⚠.
    pub console_font: Vec<crate::console_font::Step>,
    /// Packages, services and packages' USE flags that are only in effect
    /// because of `[desktop]`, so they can be marked "from [desktop]".
    pub from_desktop: BTreeSet<String>,
    /// Packages to compile instead of downloading (`prebuilt = false`),
    /// passed to emerge as `--usepkg-exclude`.
    pub compile_from_source: Vec<String>,
    /// A change of Gentoo profile (`[system] profile`), or why not.
    pub profile: crate::profile::Wanted,
}

/// The note shown after something `[desktop]` adds.
pub const FROM_DESKTOP: &str = "(from [desktop])";

#[derive(Debug, PartialEq)]
pub struct UseChange {
    pub atom: String,
    /// Flags set on the system now (empty if none).
    pub from: Vec<String>,
    /// Flags the config asks for (empty if none).
    pub to: Vec<String>,
}

impl Plan {
    /// `name`, marked "(from [desktop])" when that's where it comes from.
    pub fn label(&self, name: &str) -> String {
        if self.from_desktop.contains(name) {
            format!("{name}   {FROM_DESKTOP}")
        } else {
            name.to_string()
        }
    }

    fn labels(&self, names: &[String]) -> Vec<String> {
        names.iter().map(|n| self.label(n)).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.add_packages.is_empty()
            && self.remove_packages.is_empty()
            && self.use_changes.is_empty()
            && self.keyword_changes.is_empty()
            && self.hardware_changes.is_empty()
            && self.add_repositories.is_empty()
            && self.remove_repositories.is_empty()
            && !self.check_after_sync
            && self.enable_services.is_empty()
            && self.security.is_empty()
            && self.boot.is_empty()
            && self.console_font.is_empty()
            && !matches!(self.profile, crate::profile::Wanted::Change(_))
    }

    /// The changes, each as a short phrase: "+ tmux", "start cronie",
    /// "USE flags for firefox, mpv" (one phrase for all USE changes).
    pub fn summary_parts(&self) -> Vec<String> {
        let short = |atom: &String| {
            let name = atom.split(':').next().unwrap_or(atom);
            name.rsplit('/').next().unwrap_or(name).to_string()
        };
        let mut parts: Vec<String> = Vec::new();
        if let crate::profile::Wanted::Change(c) = &self.profile {
            parts.push(format!("{} profile", c.to.name()));
        }
        parts.extend(
            self.add_repositories
                .iter()
                .map(|r| format!("+ repository {r}")),
        );
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
        parts.extend(
            self.hardware_changes
                .iter()
                .map(|c| c.group.plain().to_string()),
        );
        if !self.keyword_changes.is_empty() {
            let names: Vec<_> = self
                .keyword_changes
                .iter()
                .map(|c| short(&c.atom))
                .collect();
            parts.push(format!("testing versions for {}", names.join(", ")));
        }
        parts.extend(
            self.remove_repositories
                .iter()
                .map(|r| format!("- repository {r}")),
        );
        parts.extend(self.enable_services.iter().map(|s| format!("start {s}")));
        if !self.security.is_empty() {
            if let Some(p) = self.security.privilege {
                parts.push(format!("admin rights: {}", p.command()));
            }
        }
        if self.boot.install_module.is_some() {
            parts.push("boot menu undo".into());
        }
        if !self.boot.tidy.is_empty() {
            parts.push("tidier boot menu".into());
        }
        if !self.console_font.is_empty() {
            parts.push("console font".into());
        }
        if self.boot.remove_entry {
            parts.push("- boot menu undo".into());
        }
        parts
    }

    /// How many changes [`Plan::summary`] lists (before it shortens the
    /// list), so a count next to it always matches.
    pub fn change_count(&self) -> usize {
        self.summary_parts().len()
    }

    /// A one-line summary, e.g. "+ tmux, + cronie, start cronie".
    pub fn summary(&self) -> String {
        let mut parts = self.summary_parts();
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

        match &self.profile {
            crate::profile::Wanted::Change(c) => {
                let _ = writeln!(out, "Profile: {} → {}", c.from.name(), c.to.name());
                out.push_str(match c.to {
                    crate::profile::Profile::Desktop => {
                        "  Gentoo's desktop profile: the features most desktops need (Wayland, X,\n  \
                         sound, D-Bus, …) are on by default. Packages whose features change are\n  \
                         downloaded again (Gentoo prebuilds for this profile) or rebuilt.\n\n"
                    }
                    crate::profile::Profile::Minimal => {
                        "  Gentoo's plain profile: only what a system needs is on by default.\n  \
                         Packages whose features change are downloaded again or rebuilt;\n  \
                         a desktop still works, with more settings in bunny.toml.\n\n"
                    }
                });
            }
            // A note, not ⚠: nothing is wrong with the system.
            crate::profile::Wanted::Unsupported(why) => {
                let _ = writeln!(out, "· Profile: {why}\n");
            }
            crate::profile::Wanted::Nothing => {}
        }

        if !self.add_repositories.is_empty() {
            section(&mut out, "Repositories to add", "+", &self.add_repositories);
        }
        // Bunny Linux's own repository needs no warning.
        if self
            .add_repositories
            .iter()
            .any(|r| crate::overlays::built_in(r).is_none())
        {
            // A note, not a warning: "·" rather than ⚠, because ⚠ and ✗ mean
            // "something went wrong" (developer mode's "auto" reports them).
            out.push_str(
                "  · Repositories besides Gentoo's own aren't checked by Gentoo's developers,\n    \
                 and their packages run build steps as root. Only add ones you trust.\n\n",
            );
        }
        if !self.remove_repositories.is_empty() {
            section(
                &mut out,
                "Repositories no longer in your config (bunny stops using them)",
                "-",
                &self.remove_repositories,
            );
            out.push_str(
                "  Packages you installed from them stay installed, but get no more updates.\n\n",
            );
        }
        if self.check_after_sync {
            out.push_str(
                "Repositories in your config still to be downloaded; packages are checked once\n\
                 they are (during the switch).\n\n",
            );
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
        section(
            &mut out,
            "Packages to install",
            "+",
            &self.labels(&installable),
        );
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
                let mark = if self.from_desktop.contains(&change.atom) {
                    format!("   {FROM_DESKTOP}")
                } else {
                    String::new()
                };
                let _ = writeln!(
                    out,
                    "  ~ {}: {} → {}{mark}",
                    change.atom,
                    show(&change.from),
                    show(&change.to)
                );
            }
            out.push('\n');
        }

        if !self.hardware_changes.is_empty() {
            out.push_str("Graphics cards and CPU features ([hardware]):\n");
            for change in &self.hardware_changes {
                let _ = writeln!(
                    out,
                    "  ~ {} ({}): {} → {}",
                    change.group.plain(),
                    change.group.portage(),
                    crate::hardware::show(&change.from),
                    change.render_to()
                );
            }
            out.push_str(
                "    Packages that use them (Mesa, video codecs, crypto libraries…) are rebuilt\n    \
                 to match. Settings that differ from Gentoo's prebuilt packages mean those\n    \
                 packages compile on this computer instead of downloading.\n\n",
            );
        }

        if !self.keyword_changes.is_empty() {
            out.push_str("Versions to accept beyond stable ones ([pkg] keywords):\n");
            for change in &self.keyword_changes {
                let show = |k: &[String]| {
                    if k.is_empty() {
                        "stable only".to_string()
                    } else {
                        k.join(" ")
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
        section(&mut out, &heading, "+", &self.labels(&self.enable_services));

        if !self.missing_services.is_empty() {
            out.push_str("Note: these services aren't installed yet:\n");
            for name in &self.missing_services {
                let _ = writeln!(out, "  ? {}", self.label(name));
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
            if let Some((path, text)) = &self.security.file {
                let _ = writeln!(
                    out,
                    "  ~ write {} so the wheel group can use {} (checked before it's put in place)",
                    path.display(),
                    p.command()
                );
                let kept = crate::security::kept_rules(text);
                if kept > 0 {
                    let _ = writeln!(
                        out,
                        "    Your other {kept} line{} in it stay, after bunny's rule.",
                        if kept == 1 { "" } else { "s" }
                    );
                }
            }
            for user in &self.security.add_to_wheel {
                let _ = writeln!(out, "  + add {user} to the wheel group");
            }
            out.push('\n');
        }

        if let Some(versions) = &self.boot.install_module {
            out.push_str("Boot menu:\n");
            let _ = writeln!(
                out,
                "  + add an \"Undo the last change\" step to the startup image (kernel {})",
                versions.join(", ")
            );
            out.push_str(
                "    The startup image is rebuilt; the old one is backed up first.\n\
                 \x20   After each change, the boot menu gets an entry that undoes it.\n\n",
            );
        }
        if !self.boot.tidy.is_empty() {
            out.push_str("Boot menu, tidied:\n");
            for item in &self.boot.tidy {
                let _ = writeln!(out, "  ~ {}", item.describe());
            }
            out.push_str(
                "    The files are backed up first, and undoing this change puts them back.\n\n",
            );
        }
        if !self.console_font.is_empty() {
            out.push_str("Console font, so ✓ ✗ ⚠ show on the text console:\n");
            for step in &self.console_font {
                let _ = writeln!(out, "  + {}", step.describe());
            }
            out.push_str(
                "    /etc/conf.d/consolefont is backed up first, and undoing this change puts it back.\n\n",
            );
        }
        if self.boot.remove_entry {
            out.push_str("Boot menu:\n  - remove bunny's entries (Undo the last change, Earlier versions)\n\n");
        }
        if self.boot.needs_root {
            let _ = writeln!(
                out,
                "Boot menu: not checked, because /boot can only be read as root. To check it\n  \
                 too, run with {}: {} bunny plan\n",
                crate::security::admin(),
                crate::security::admin()
            );
        }
        if !self.boot.problems.is_empty() {
            out.push_str("Note: the boot menu's undo entry can't be set up on this system:\n");
            for problem in &self.boot.problems {
                let _ = writeln!(out, "  • {problem}");
            }
            out.push_str("  Everything else still works.\n\n");
        }

        if !self.extra_repositories.is_empty() {
            out.push_str("Also set up, but not in your config's repositories (left alone):\n");
            for name in &self.extra_repositories {
                let _ = writeln!(out, "  · {name}");
            }
            out.push_str("  Add them to `repositories` to keep track of them in your config.\n\n");
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
pub fn unknown_packages(root: &Path, wanted: &[String]) -> Vec<String> {
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

/// Compares bunny's package.accept_keywords file with the config's
/// `[pkg] keywords` (other files there are left alone).
fn keyword_changes(config: &Config, root: &Path) -> Vec<UseChange> {
    let current = portage::read_bunny_keywords(root);
    let wanted = config.keywords();
    let as_set = |k: &[String]| k.iter().cloned().collect::<BTreeSet<_>>();
    let mut atoms: BTreeSet<&String> = wanted.keys().collect();
    atoms.extend(current.keys());
    atoms
        .into_iter()
        .filter_map(|atom| {
            let from = current.get(atom).cloned().unwrap_or_default();
            let to = wanted.get(atom).cloned().unwrap_or_default();
            (as_set(&from) != as_set(&to)).then(|| UseChange {
                atom: atom.clone(),
                from,
                to,
            })
        })
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
    let wanted_world = portage::wanted_world(config, root);

    let use_changes = use_changes(config, root)?;

    let init = init::for_config(config.system.init, root);
    let enabled = init
        .enabled_services()
        .context("Couldn't read which services start at boot")?;
    let user_enabled = init
        .user_services()
        .context("Couldn't read which services start at boot")?;
    let wanted_services: BTreeSet<String> = config.wanted_services().into_iter().collect();

    let configured = portage::configured_repositories(root);
    let wanted_repos: BTreeSet<String> = config.repositories.iter().cloned().collect();
    let check_after_sync = wanted_repos
        .iter()
        .any(|r| !root.join("var/db/repos").join(r).is_dir());
    let to_add: Vec<String> = wanted_world.difference(&current_world).cloned().collect();
    let from_desktop: BTreeSet<String> = wanted_world
        .iter()
        .chain(&wanted_services)
        .chain(config.use_flags().keys())
        .filter(|name| config.added_by_desktop(name))
        .cloned()
        .collect();

    Ok(Plan {
        init_name: init.name(),
        compile_from_source: config.compile_from_source(),
        profile: crate::profile::wanted(root, config.system.profile),
        unknown_packages: if check_after_sync {
            Vec::new()
        } else {
            unknown_packages(root, &to_add)
        },
        keyword_changes: keyword_changes(config, root),
        hardware_changes: {
            let (all, bunnys) = import::read_package_use(root)?.hardware_groups();
            crate::hardware::changes(&config.hardware, &all, &bunnys)
        },
        add_repositories: wanted_repos
            .iter()
            .filter(|r| !configured.contains_key(*r))
            .cloned()
            .collect(),
        remove_repositories: configured
            .iter()
            .filter(|(name, ours)| **ours && !wanted_repos.contains(*name))
            .map(|(name, _)| name.clone())
            .collect(),
        extra_repositories: configured
            .iter()
            .filter(|(name, ours)| !**ours && !wanted_repos.contains(*name))
            .map(|(name, _)| name.clone())
            .collect(),
        check_after_sync,
        // In the config but not installed, even if the world file lists it
        // (emerge can leave a package out and still record it there).
        add_packages: wanted_world
            .iter()
            .filter(|a| {
                !current_world.contains(*a) || portage::is_installed(root, a) == Some(false)
            })
            .cloned()
            .collect(),
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
        from_desktop,
        boot: crate::boot::changes(&config.boot, root),
        console_font: crate::console_font::needed(root),
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

    #[test]
    fn the_change_count_matches_the_summary() {
        // From the Saturday test: "2 changes not applied yet: + tree,
        // tidier boot menu, console font" (three things listed).
        let p = Plan {
            add_packages: vec!["app-text/tree".into()],
            boot: crate::boot::Changes {
                install_module: Some(vec!["6.12.58".into()]),
                ..Default::default()
            },
            console_font: vec![
                crate::console_font::Step::Font,
                crate::console_font::Step::Setting,
            ],
            ..Plan::default()
        };
        assert_eq!(p.summary(), "+ tree, boot menu undo, console font");
        assert_eq!(p.change_count(), 3);

        // Shortened summaries still count everything.
        let many = Plan {
            add_packages: (1..=9).map(|n| format!("app-misc/p{n}")).collect(),
            ..Plan::default()
        };
        assert!(many.summary().ends_with("and 3 more"), "{}", many.summary());
        assert_eq!(many.change_count(), 9);
    }

    use super::*;

    #[test]
    fn asks_for_admin_rights_to_check_the_boot_menu() {
        let plan = Plan {
            boot: crate::boot::Changes {
                needs_root: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = plan.render();
        assert!(
            text.contains("Boot menu: not checked, because /boot can only be read as root."),
            "{text}"
        );
        assert!(text.contains(" bunny plan"), "{text}");
        assert!(!text.contains("can't be set up"), "{text}");
    }

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
    fn desktop_settings_are_planned_and_marked() {
        let root = fake_root();
        let r = root.path();
        std::fs::write(r.join("etc/init.d/dbus"), "").unwrap();
        let config = Config::parse(
            "packages = [\"app-editors/neovim\", \"app-misc/old-thing\"]\n\
             services = [\"elogind\"]\n[desktop]\nwayland = true\n",
        )
        .unwrap();
        let plan = make(&config, r).unwrap();
        assert_eq!(plan.add_packages, ["sys-apps/dbus", "sys-auth/elogind"]);
        assert_eq!(plan.enable_services, ["dbus"]);
        let atoms: Vec<&str> = plan.use_changes.iter().map(|c| c.atom.as_str()).collect();
        assert_eq!(atoms, ["sys-auth/pambase", "sys-auth/seatd"]);
        let text = plan.render();
        assert!(
            text.contains("  + sys-apps/dbus   (from [desktop])\n"),
            "{text}"
        );
        assert!(text.contains("  ~ sys-auth/pambase: (none) → elogind   (from [desktop])\n"));
        assert!(text.contains("  + dbus   (from [desktop])\n"));
        // elogind is in `services` too, so it's yours, not [desktop]'s.
        assert!(!plan.from_desktop.contains("elogind"));
    }

    #[test]
    fn a_package_in_the_world_file_but_not_installed_is_still_to_add() {
        // emerge can leave a package out (only a testing version) and still
        // write it in the world file; the plan mustn't call that done.
        let root = fake_root();
        let r = root.path();
        let config =
            Config::parse("packages = [\"app-editors/neovim\", \"gui-wm/dwl\"]\n").unwrap();
        std::fs::write(portage::world_path(r), "app-editors/neovim\ngui-wm/dwl\n").unwrap();
        std::fs::create_dir_all(r.join("var/db/pkg/app-editors/neovim-0.11.5")).unwrap();
        let plan = make(&config, r).unwrap();
        assert_eq!(plan.add_packages, ["gui-wm/dwl"]);
        assert!(plan.remove_packages.is_empty());
        // Installed under another name prefix doesn't count (dwl vs dwlb).
        std::fs::create_dir_all(r.join("var/db/pkg/gui-wm/dwlb-0.2")).unwrap();
        assert_eq!(make(&config, r).unwrap().add_packages, ["gui-wm/dwl"]);
        std::fs::create_dir_all(r.join("var/db/pkg/gui-wm/dwl-0.7-r1")).unwrap();
        assert!(make(&config, r).unwrap().add_packages.is_empty());
    }

    #[test]
    fn desktop_matches_the_same_settings_made_by_hand() {
        // The dry run's manual steps, then the same thing as [desktop]:
        // nothing to do either way.
        let root = fake_root();
        let r = root.path();
        let boot = r.join("etc/runlevels/boot");
        std::fs::create_dir_all(&boot).unwrap();
        std::fs::remove_file(r.join("etc/runlevels/default/elogind")).unwrap();
        std::fs::write(boot.join("elogind"), "").unwrap();
        std::fs::write(r.join("etc/init.d/dbus"), "").unwrap();
        std::fs::write(r.join("etc/runlevels/default/dbus"), "").unwrap();
        std::fs::create_dir_all(r.join("etc/portage/package.use")).unwrap();
        std::fs::write(
            r.join("etc/portage/package.use/zz-bunny"),
            "sys-auth/pambase elogind\nsys-auth/seatd elogind\n",
        )
        .unwrap();
        std::fs::write(
            r.join(portage::WORLD_FILE),
            "app-editors/neovim\nsys-apps/dbus\nsys-auth/elogind\n",
        )
        .unwrap();
        let by_hand = Config::parse(
            "packages = [\"app-editors/neovim\", \"sys-apps/dbus\", \"sys-auth/elogind\"]\n\
             services = [\"elogind\", \"dbus\"]\n\
             [pkg.\"sys-auth/pambase\"]\nuse = [\"elogind\"]\n\
             [pkg.\"sys-auth/seatd\"]\nuse = [\"elogind\"]\n",
        )
        .unwrap();
        let plan = make(&by_hand, r).unwrap();
        assert!(plan.is_empty(), "{plan:#?}");
        let desktop =
            Config::parse("packages = [\"app-editors/neovim\"]\n[desktop]\nwayland = true\n")
                .unwrap();
        let plan = make(&desktop, r).unwrap();
        assert!(plan.is_empty(), "{plan:#?}");
        assert!(plan.extra_services.is_empty());
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

    #[test]
    fn the_repository_trust_note_is_not_a_problem() {
        // Developer mode's report = "auto" sends any command that shows ⚠ or ✗;
        // a switch that adds a repository and works must not be one of them.
        let plan = Plan {
            add_repositories: vec!["guru".into()],
            ..Plan::default()
        };
        let shown = plan.render();
        assert!(shown.contains("Only add ones you trust"), "{shown}");
        let worked = crate::dev::Meta {
            exit_code: Some(0),
            ..crate::dev::Meta::default()
        };
        assert!(!crate::dev::had_problem(&worked, &shown), "{shown}");
    }
}
