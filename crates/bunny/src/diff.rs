//! `bunny diff`: your config and your system side by side.
//!
//! `plan` answers "what would a switch do?". `diff` answers "where do they
//! differ, and which way do I want to fix it?": for every difference it says
//! which side it's on, and how to make the config match the system instead.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::lock::Drift;
use crate::plan::{Plan, UseChange};

/// Everything that differs, ready to render.
pub struct Diff<'a> {
    pub config: &'a str,
    pub plan: &'a Plan,
    /// Differences from bunny.lock, if there's a lock.
    pub drift: Option<&'a Drift>,
}

impl Diff<'_> {
    pub fn is_empty(&self) -> bool {
        self.plan.is_empty()
            && self.plan.extra_services.is_empty()
            && self.plan.extra_repositories.is_empty()
            && self.plan.missing_services.is_empty()
            && self.plan.security.is_empty()
            && self.plan.boot.is_empty()
            && self.drift.is_none_or(Drift::is_empty)
    }

    pub fn render(&self) -> String {
        let p = self.plan;
        let mut out = String::new();
        if self.is_empty() {
            let _ = writeln!(
                out,
                "✓ No differences: this system matches {}{}.",
                self.config,
                if self.drift.is_some() {
                    " and bunny.lock"
                } else {
                    ""
                }
            );
            return out;
        }
        let _ = writeln!(out, "Comparing {} with this system.\n", self.config);

        // Packages
        if !p.add_packages.is_empty() || !p.remove_packages.is_empty() {
            out.push_str("Packages\n");
            for atom in &p.add_packages {
                let why = if p.unknown_packages.contains(atom) {
                    "in your config, but no such package exists (typo?)"
                } else {
                    "in your config, not installed yet"
                };
                let _ = writeln!(out, "  config only   {}   ({why})", p.label(atom));
            }
            for atom in &p.remove_packages {
                let _ = writeln!(
                    out,
                    "  system only   {atom}   (installed outside bunny; a switch would remove it)"
                );
            }
            out.push('\n');
        }

        // USE flags
        if !p.use_changes.is_empty() {
            out.push_str("USE flags\n");
            for change in &p.use_changes {
                let (config_only, system_only) = flag_sides(change);
                let _ = writeln!(out, "  {}", p.label(&change.atom));
                if !config_only.is_empty() {
                    let _ = writeln!(out, "    config only   {}", describe_flags(&config_only));
                }
                if !system_only.is_empty() {
                    let _ = writeln!(out, "    system only   {}", describe_flags(&system_only));
                }
            }
            out.push('\n');
        }

        // Graphics cards and CPU features
        if !p.hardware_changes.is_empty() {
            out.push_str("Graphics cards and CPU features ([hardware])\n");
            for change in &p.hardware_changes {
                let _ = writeln!(
                    out,
                    "  {} ({})   config: {}, system: {}",
                    change.group.plain(),
                    change.group.portage(),
                    change.render_to(),
                    crate::hardware::show(&change.from)
                );
            }
            out.push('\n');
        }

        // Testing versions
        if !p.keyword_changes.is_empty() {
            out.push_str("Testing versions ([pkg] keywords)\n");
            for change in &p.keyword_changes {
                let show = |k: &[String]| {
                    if k.is_empty() {
                        "stable only".to_string()
                    } else {
                        k.join(" ")
                    }
                };
                let _ = writeln!(
                    out,
                    "  {}   config: {}, system: {}",
                    change.atom,
                    show(&change.to),
                    show(&change.from)
                );
            }
            out.push('\n');
        }

        // Repositories
        if !p.add_repositories.is_empty()
            || !p.remove_repositories.is_empty()
            || !p.extra_repositories.is_empty()
        {
            out.push_str("Repositories\n");
            for r in &p.add_repositories {
                let _ = writeln!(out, "  config only   {r}   (not set up yet)");
            }
            for r in &p.remove_repositories {
                let _ = writeln!(
                    out,
                    "  system only   {r}   (set up by bunny before; a switch stops using it)"
                );
            }
            for r in &p.extra_repositories {
                let _ = writeln!(
                    out,
                    "  system only   {r}   (set up outside bunny; a switch leaves it alone)"
                );
            }
            out.push('\n');
        }

        // Services
        let has_services = !p.enable_services.is_empty()
            || !p.missing_services.is_empty()
            || !p.extra_services.is_empty();
        if has_services {
            let _ = writeln!(out, "Services at boot ({})", p.init_name);
            for s in &p.enable_services {
                let _ = writeln!(
                    out,
                    "  config only   {}   (not started at boot yet)",
                    p.label(s)
                );
            }
            for s in &p.missing_services {
                let _ = writeln!(
                    out,
                    "  config only   {}   (not installed, so it can't start yet)",
                    p.label(s)
                );
            }
            for s in &p.extra_services {
                let _ = writeln!(
                    out,
                    "  system only   {s}   (turned on outside bunny; a switch leaves it alone)"
                );
            }
            out.push('\n');
        }

        // Admin rights
        if let (false, Some(privilege)) = (p.security.is_empty(), p.security.privilege) {
            let _ = writeln!(out, "Admin rights ({})", privilege.command());
            if let Some((path, _)) = &p.security.file {
                let _ = writeln!(
                    out,
                    "  config only   {}   (not written yet, or changed by hand)",
                    path.display()
                );
            }
            for user in &p.security.add_to_wheel {
                let _ = writeln!(
                    out,
                    "  config only   {user} in the wheel group   (not added yet)"
                );
            }
            out.push('\n');
        }

        // Boot menu
        if self.plan.boot.install_module.is_some() {
            out.push_str("Boot menu\n  config only   the undo step in the startup image   (not added yet)\n\n");
        }
        if self.plan.boot.remove_entry {
            out.push_str("Boot menu\n  system only   bunny's entries (Undo, Earlier versions)   (turned off in your config)\n\n");
        }

        // Versions
        if let Some(drift) = self.drift.filter(|d| !d.is_empty()) {
            out.push_str("Versions (bunny.lock)\n");
            for (name, locked, now) in &drift.changed {
                let _ = writeln!(out, "  {name}   {now} installed, {locked} locked");
            }
            for (name, v) in &drift.missing {
                let _ = writeln!(out, "  {name}   {v} locked, not installed");
            }
            for (name, v) in &drift.extra {
                let _ = writeln!(out, "  {name}   {v} installed, not in the lock");
            }
            for (repo, locked, now) in &drift.repos {
                let _ = writeln!(
                    out,
                    "  the {repo} package tree   {now} now, {locked} locked"
                );
            }
            out.push('\n');
        }

        // What to do
        out.push_str("To make the system match your config:\n    bunny switch\n");
        out.push_str("\n(`bunny plan` shows exactly what that switch would do.)\n");
        let keep = self.keep_instructions();
        if !keep.is_empty() {
            out.push_str("\nTo keep the system as it is instead:\n");
            out.push_str(&keep);
        }
        out
    }

    /// How to make the config match the system: edits to the config, then
    /// `bunny lock`. (Not `bunny add`/`remove`: those switch, which would
    /// apply the other differences too.)
    fn keep_instructions(&self) -> String {
        let p = self.plan;
        let quoted = |items: &[&String]| {
            items
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(", ")
        };
        // What [desktop] adds can't be taken out item by item: turning the
        // section off is the one edit.
        let ours = |name: &&String| !p.from_desktop.contains(*name);
        let mut edits = String::new();
        let desktop_differs = p.add_packages.iter().any(|a| p.from_desktop.contains(a))
            || p.enable_services.iter().any(|s| p.from_desktop.contains(s))
            || p.missing_services
                .iter()
                .any(|s| p.from_desktop.contains(s))
            || p.use_changes
                .iter()
                .any(|c| c.from.is_empty() && p.from_desktop.contains(&c.atom));
        if desktop_differs {
            edits.push_str(
                "    • take `wayland = true` out of [desktop] (or the whole section), or set\n      \
                 [pkg] lines of your own for the packages it marks\n",
            );
        }
        if !p.remove_packages.is_empty() {
            let items: Vec<&String> = p.remove_packages.iter().collect();
            let _ = writeln!(edits, "    • add to `packages`: {}", quoted(&items));
        }
        let items: Vec<&String> = p.add_packages.iter().filter(ours).collect();
        if !items.is_empty() {
            let _ = writeln!(edits, "    • take out of `packages`: {}", quoted(&items));
        }
        if !p.extra_services.is_empty() {
            let items: Vec<&String> = p.extra_services.iter().collect();
            let _ = writeln!(edits, "    • add to `services`: {}", quoted(&items));
        }
        let mut unwanted: Vec<&String> = p.enable_services.iter().filter(ours).collect();
        unwanted.extend(p.missing_services.iter().filter(ours));
        if !unwanted.is_empty() {
            let _ = writeln!(edits, "    • take out of `services`: {}", quoted(&unwanted));
        }
        for change in &p.use_changes {
            if change.from.is_empty() && p.from_desktop.contains(&change.atom) {
                continue;
            }
            if change.from.is_empty() {
                let _ = writeln!(
                    edits,
                    "    • remove the `use` line under [pkg.\"{}\"]",
                    change.atom
                );
            } else {
                let flags: Vec<&String> = change.from.iter().collect();
                let _ = writeln!(
                    edits,
                    "    • under [pkg.\"{}\"], set: use = [{}]",
                    change.atom,
                    quoted(&flags)
                );
            }
        }

        for change in &p.hardware_changes {
            let key = change.group.key();
            // The system's values, as [hardware] would write them. Only a
            // list of its own (not "Gentoo's defaults + …") can be copied.
            let replaces = change.from.first().is_some_and(|w| w == "-*");
            let values: Vec<&String> = change.from.iter().filter(|w| *w != "-*").collect();
            if change.from.is_empty() {
                let _ = writeln!(edits, "    • take `{key}` out of [hardware]");
            } else if replaces == (change.group == crate::hardware::Group::VideoCards) {
                let _ = writeln!(
                    edits,
                    "    • under [hardware], set: {key} = [{}]",
                    quoted(&values)
                );
            }
        }

        if p.boot.install_module.is_some() {
            edits.push_str("    • set `undo_entry = false` under [boot]\n");
        }
        if p.boot.remove_entry {
            edits.push_str("    • set `undo_entry = true` under [boot]\n");
        }
        if !p.security.is_empty() {
            edits.push_str(
                "    • remove the [security] section (bunny then leaves admin rights as they are)\n",
            );
        }

        let drift = self.drift.is_some_and(|d| !d.is_empty());
        let mut out = String::new();
        if !edits.is_empty() {
            let _ = writeln!(out, "  Edit {}:", self.config);
            out.push_str(&edits);
            out.push_str("  Then accept the installed versions:\n    bunny lock\n");
        } else if drift {
            out.push_str("  Accept the installed versions:\n    bunny lock\n");
        }
        out
    }
}

/// Flags set only in the config, and only on the system.
fn flag_sides(change: &UseChange) -> (Vec<String>, Vec<String>) {
    let to: BTreeSet<&String> = change.to.iter().collect();
    let from: BTreeSet<&String> = change.from.iter().collect();
    (
        to.difference(&from).map(|f| (*f).clone()).collect(),
        from.difference(&to).map(|f| (*f).clone()).collect(),
    )
}

/// `["pgo", "-telemetry"]` → "pgo on, telemetry off".
fn describe_flags(flags: &[String]) -> String {
    flags
        .iter()
        .map(|f| match f.strip_prefix('-') {
            Some(off) => format!("{off} off"),
            None => format!("{f} on"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            init_name: "OpenRC",
            ..Plan::default()
        }
    }

    #[test]
    fn no_differences_says_so() {
        let p = plan();
        let drift = Drift::default();
        let d = Diff {
            config: "/etc/bunny/bunny.toml",
            plan: &p,
            drift: Some(&drift),
        };
        assert!(d.is_empty());
        assert_eq!(
            d.render(),
            "✓ No differences: this system matches /etc/bunny/bunny.toml and bunny.lock.\n"
        );
    }

    #[test]
    fn packages_say_which_side_and_how_to_keep_them() {
        let mut p = plan();
        p.add_packages = vec!["media-video/mpv".into()];
        p.remove_packages = vec!["app-text/tree".into()];
        let text = Diff {
            config: "bunny.toml",
            plan: &p,
            drift: None,
        }
        .render();
        assert!(
            text.contains("config only   media-video/mpv   (in your config, not installed yet)")
        );
        assert!(text.contains(
            "system only   app-text/tree   (installed outside bunny; a switch would remove it)"
        ));
        assert!(text.contains("To make the system match your config:\n    bunny switch"));
        assert!(text.contains("  Edit bunny.toml:\n"));
        assert!(text.contains("    • add to `packages`: \"app-text/tree\"\n"));
        assert!(text.contains("    • take out of `packages`: \"media-video/mpv\"\n"));
        assert!(text.contains("  Then accept the installed versions:\n    bunny lock\n"));
    }

    #[test]
    fn typos_are_called_out() {
        let mut p = plan();
        p.add_packages = vec!["app-misc/tmuxx".into()];
        p.unknown_packages = vec!["app-misc/tmuxx".into()];
        let text = Diff {
            config: "bunny.toml",
            plan: &p,
            drift: None,
        }
        .render();
        assert!(text.contains("no such package exists (typo?)"));
    }

    #[test]
    fn use_flags_show_only_what_differs_in_words() {
        let mut p = plan();
        p.use_changes = vec![UseChange {
            atom: "www-client/firefox".into(),
            from: vec!["wayland".into(), "pgo".into()],
            to: vec!["pgo".into(), "-telemetry".into()],
        }];
        let text = Diff {
            config: "bunny.toml",
            plan: &p,
            drift: None,
        }
        .render();
        assert!(text.contains(
            "  www-client/firefox\n    config only   telemetry off\n    system only   wayland on\n"
        ));
        assert!(text.contains(
            "    • under [pkg.\"www-client/firefox\"], set: use = [\"wayland\", \"pgo\"]"
        ));
    }

    #[test]
    fn services_on_both_sides() {
        let mut p = plan();
        p.enable_services = vec!["sshd".into()];
        p.extra_services = vec!["rsyncd".into()];
        let text = Diff {
            config: "bunny.toml",
            plan: &p,
            drift: None,
        }
        .render();
        assert!(text.contains("Services at boot (OpenRC)"));
        assert!(text.contains("config only   sshd   (not started at boot yet)"));
        assert!(text.contains("system only   rsyncd   (turned on outside bunny"));
        assert!(text.contains("    • add to `services`: \"rsyncd\""));
        assert!(text.contains("    • take out of `services`: \"sshd\""));
    }

    #[test]
    fn desktop_settings_are_marked_and_turned_off_in_one_edit() {
        let mut p = plan();
        p.enable_services = vec!["elogind".into(), "sshd".into()];
        p.add_packages = vec!["sys-auth/elogind".into()];
        p.use_changes = vec![UseChange {
            atom: "sys-auth/pambase".into(),
            from: vec![],
            to: vec!["elogind".into()],
        }];
        p.from_desktop = ["elogind", "sys-auth/elogind", "sys-auth/pambase"]
            .into_iter()
            .map(String::from)
            .collect();
        let text = Diff {
            config: "bunny.toml",
            plan: &p,
            drift: None,
        }
        .render();
        assert!(
            text.contains("config only   elogind   (from [desktop])   (not started"),
            "{text}"
        );
        assert!(
            text.contains("  sys-auth/pambase   (from [desktop])\n    config only   elogind on")
        );
        assert!(text.contains("    • take `wayland = true` out of [desktop]"));
        // Only your own lines are listed one by one.
        assert!(
            text.contains("    • take out of `services`: \"sshd\"\n"),
            "{text}"
        );
        assert!(!text.contains("take out of `packages`"), "{text}");
        assert!(!text.contains("[pkg.\"sys-auth/pambase\"]"), "{text}");
    }

    #[test]
    fn extra_services_alone_count_as_a_difference() {
        let mut p = plan();
        p.extra_services = vec!["rsyncd".into()];
        let d = Diff {
            config: "bunny.toml",
            plan: &p,
            drift: None,
        };
        assert!(!d.is_empty());
    }

    #[test]
    fn version_drift_offers_bunny_lock() {
        let p = plan();
        let drift = Drift {
            changed: vec![("app-misc/tmux".into(), "3.6a".into(), "3.6b".into())],
            extra: vec![("app-text/tree".into(), "2.2.1".into())],
            ..Drift::default()
        };
        let text = Diff {
            config: "bunny.toml",
            plan: &p,
            drift: Some(&drift),
        }
        .render();
        assert!(text.contains("Versions (bunny.lock)"));
        assert!(text.contains("app-misc/tmux   3.6b installed, 3.6a locked"));
        assert!(text.contains("app-text/tree   2.2.1 installed, not in the lock"));
        assert!(text.contains("  Accept the installed versions:\n    bunny lock\n"));
    }

    #[test]
    fn admin_rights_differences_are_shown() {
        let mut p = plan();
        p.security = crate::security::Changes {
            privilege: Some(crate::security::Privilege::Sudo),
            file: Some(("/etc/sudoers.d/bunny".into(), String::new())),
            add_to_wheel: vec!["you".into()],
            problems: vec![],
        };
        let d = Diff {
            config: "bunny.toml",
            plan: &p,
            drift: None,
        };
        assert!(!d.is_empty());
        let text = d.render();
        assert!(text.contains("Admin rights (sudo)"));
        assert!(text.contains("config only   /etc/sudoers.d/bunny"));
        assert!(text.contains("config only   you in the wheel group"));
        assert!(text.contains("remove the [security] section"));
    }

    #[test]
    fn shows_hardware_differences_and_the_edit() {
        use crate::hardware::{Change, Group};
        let p = Plan {
            hardware_changes: vec![
                Change {
                    group: Group::VideoCards,
                    from: vec!["-*".into(), "nvidia".into()],
                    to: Some(vec!["-*".into(), "intel".into(), "nvidia".into()]),
                },
                Change {
                    group: Group::CpuFlags,
                    from: vec![],
                    to: Some(vec!["aes".into()]),
                },
            ],
            ..Plan::default()
        };
        let text = Diff {
            config: "bunny.toml",
            plan: &p,
            drift: None,
        }
        .render();
        assert!(
            text.contains("graphics cards (VIDEO_CARDS)   config: intel nvidia, system: nvidia"),
            "{text}"
        );
        assert!(
            text.contains("under [hardware], set: video_cards = [\"nvidia\"]"),
            "{text}"
        );
        assert!(
            text.contains("take `cpu_flags` out of [hardware]"),
            "{text}"
        );
    }
}
