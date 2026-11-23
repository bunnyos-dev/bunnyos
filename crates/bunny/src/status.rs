//! `bunny status`: one screen that says whether everything is in order.
//!
//! Gathering (reading the system) and rendering are kept apart, so the
//! wording can be tested without a real system.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::{lock, notes, plan, rollback};

/// The package tree counts as out of date after this many days.
pub const STALE_DAYS: i64 = 7;

#[derive(Debug, PartialEq)]
pub enum ConfigState {
    /// No config file at that path.
    Missing,
    /// The file couldn't be read as a config (bad TOML, unknown setting).
    Unreadable,
    /// The config has this many mistakes (`bunny check` lists them).
    Problems(usize),
    /// The system matches the config.
    Matches,
    /// A switch would change this many things; `summary` is like "+ mpv, start sshd".
    Differs { count: usize, summary: String },
}

#[derive(Debug, PartialEq)]
pub enum LockState {
    Missing,
    Unreadable,
    /// Installed versions match the lock, which has this many packages.
    Matches(usize),
    /// This many packages (or package trees) differ from the lock.
    Differs(usize),
    /// Installed packages the lock doesn't list yet, and nothing else.
    NewPackages(usize),
}

#[derive(Debug, PartialEq)]
pub struct Status {
    pub config: PathBuf,
    pub config_state: ConfigState,
    /// Services that start at boot but aren't in the config (enabled by hand).
    pub extra_services: Vec<String>,
    pub lock: PathBuf,
    pub lock_state: LockState,
    /// The last thing bunny did, if anything.
    pub last: Option<notes::State>,
    /// For a rollback: the generation number it went back to.
    pub last_to_number: Option<usize>,
    /// A rollback is done but the computer hasn't restarted yet.
    pub reboot_pending: bool,
    /// Saved generations, or `None` if there's no `/.snapshots` (no undo).
    pub generations: Option<Vec<rollback::Generation>>,
    /// When the Gentoo package tree was last synced (seconds since 1970).
    pub tree_synced: Option<i64>,
    /// Gentoo security advisories; `None` when not checked (a test root).
    pub security: Option<crate::glsa::Check>,
    /// Kernels in a separate /boot that this system has no modules (drivers)
    /// for, e.g. after undoing a kernel update from the boot menu.
    pub kernels_without_drivers: Vec<String>,
    /// Now (seconds since 1970), passed in so tests are repeatable.
    pub now: i64,
}

/// Reads everything `bunny status` shows. Never changes anything.
pub fn gather(config: &Path, root: &Path, now: i64) -> Status {
    let mut extra_services = Vec::new();
    let config_state = if !config.exists() {
        ConfigState::Missing
    } else {
        match Config::load(config) {
            Err(_) => ConfigState::Unreadable,
            Ok(cfg) => {
                let problems = cfg.problems().len();
                if problems > 0 {
                    ConfigState::Problems(problems)
                } else {
                    let made = plan::make(&cfg, root);
                    if let Ok(p) = &made {
                        extra_services = p.extra_services.clone();
                    }
                    match made {
                        Err(_) => ConfigState::Unreadable,
                        Ok(p) if p.is_empty() => ConfigState::Matches,
                        Ok(p) => ConfigState::Differs {
                            count: p.add_packages.len()
                                + p.remove_packages.len()
                                + p.use_changes.len()
                                + p.keyword_changes.len()
                                + p.add_repositories.len()
                                + p.remove_repositories.len()
                                + p.enable_services.len()
                                + usize::from(p.security.file.is_some())
                                + usize::from(!p.boot.is_empty())
                                + p.security.add_to_wheel.len(),
                            summary: p.summary(),
                        },
                    }
                }
            }
        }
    };

    let lock_path = lock::path_for(config);
    let lock_state = if !lock_path.exists() {
        LockState::Missing
    } else {
        match (lock::load(&lock_path), lock::from_system(root)) {
            (Ok(locked), Ok(now_lock)) => {
                let drift = lock::compare(&locked, &now_lock);
                if drift.is_empty() {
                    LockState::Matches(locked.packages.len())
                } else if drift.only_new_packages() {
                    LockState::NewPackages(drift.extra.len())
                } else {
                    LockState::Differs(
                        drift.changed.len()
                            + drift.missing.len()
                            + drift.extra.len()
                            + drift.repos.len(),
                    )
                }
            }
            _ => LockState::Unreadable,
        }
    };

    let generations = if root.join(".snapshots").is_dir() {
        Some(rollback::list(root).unwrap_or_default())
    } else {
        None
    };
    let last = notes::read_state(root);
    let last_to_number = match (&last, &generations) {
        (Some(s), Some(gens)) => {
            s.to.as_ref()
                .and_then(|to| gens.iter().position(|g| &g.name == to))
                .map(|i| i + 1)
        }
        _ => None,
    };

    let reboot_pending = root == Path::new("/")
        && std::fs::read_to_string("/proc/self/mountinfo")
            .ok()
            .and_then(|info| rollback::root_mount(&info).ok())
            .is_some_and(|(_, subvol)| subvol.starts_with("@replaced-"));

    Status {
        config: config.to_path_buf(),
        config_state,
        extra_services,
        lock: lock_path,
        lock_state,
        last,
        last_to_number,
        reboot_pending,
        generations,
        tree_synced: tree_synced(root),
        // glsa-check reads the real system only.
        security: (root == Path::new("/")).then(crate::glsa::check),
        kernels_without_drivers: if crate::kernels::separate_boot(&crate::kernels::mountinfo(root))
        {
            crate::kernels::missing_modules(root, root)
        } else {
            Vec::new()
        },
        now,
    }
}

/// When the Gentoo package tree was last synced: the newest of the files a
/// sync touches (rsync/webrsync write `timestamp.chk`, git updates `FETCH_HEAD`).
fn tree_synced(root: &Path) -> Option<i64> {
    let repo = root.join("var/db/repos/gentoo");
    ["metadata/timestamp.chk", ".git/FETCH_HEAD", ".git/HEAD"]
        .iter()
        .filter_map(|f| std::fs::metadata(repo.join(f)).ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .max()
}

impl Status {
    /// True when nothing needs attention.
    pub fn all_good(&self) -> bool {
        matches!(self.config_state, ConfigState::Matches)
            && self.extra_services.is_empty()
            && matches!(self.lock_state, LockState::Matches(_))
            && self.generations.is_some()
            && !self.reboot_pending
            && !self.tree_stale()
            && self.kernels_without_drivers.is_empty()
            && !matches!(&self.security, Some(crate::glsa::Check::Done(found)) if !found.is_empty())
    }

    fn tree_stale(&self) -> bool {
        self.tree_synced
            .is_some_and(|t| self.now - t > STALE_DAYS * 86_400)
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let line = |out: &mut String, mark: &str, label: &str, text: &str| {
            let _ = writeln!(out, "{mark} {label:<14}{text}");
        };
        let more = |out: &mut String, text: &str| {
            let _ = writeln!(out, "  {:<14}{text}", "");
        };

        // Your config
        let config = self.config.display();
        match &self.config_state {
            ConfigState::Missing => {
                line(&mut out, "✗", "Config", &format!("there's no {config}"));
                more(
                    &mut out,
                    &format!(
                        "Create one from this system: {} bunny import -o {config}",
                        crate::security::admin()
                    ),
                );
            }
            ConfigState::Unreadable => {
                line(&mut out, "✗", "Config", &format!("{config} can't be read"));
                more(&mut out, "Run `bunny check` to see why.");
            }
            ConfigState::Problems(n) => {
                line(
                    &mut out,
                    "✗",
                    "Config",
                    &format!("{config} has {n} mistake{}", plural(*n)),
                );
                more(
                    &mut out,
                    "Run `bunny check` to see them and how to fix them.",
                );
            }
            ConfigState::Matches if self.extra_services.is_empty() => {
                line(&mut out, "✓", "Config", "the system matches your config");
            }
            ConfigState::Matches => {
                line(
                    &mut out,
                    "⚠",
                    "Config",
                    "your packages match, but services were changed outside bunny",
                );
            }
            ConfigState::Differs { count, summary } => {
                line(
                    &mut out,
                    "⚠",
                    "Config",
                    &format!(
                        "{count} change{} not applied yet: {summary}",
                        plural(*count)
                    ),
                );
                more(
                    &mut out,
                    "`bunny diff` shows them side by side, and how to keep or undo each.",
                );
            }
        }

        if !self.extra_services.is_empty() {
            more(
                &mut out,
                &format!(
                    "Also starting at boot, not in your config: {}",
                    self.extra_services.join(", ")
                ),
            );
            more(
                &mut out,
                "Add them to `services` to keep them, or turn them off with rc-update.",
            );
        }

        // The lock
        match &self.lock_state {
            LockState::Missing => {
                line(&mut out, "⚠", "Versions", "no bunny.lock yet");
                more(
                    &mut out,
                    "It's written on the next `bunny switch` (or run `bunny lock`).",
                );
            }
            LockState::Unreadable => {
                line(
                    &mut out,
                    "✗",
                    "Versions",
                    &format!("{} can't be read", self.lock.display()),
                );
                more(&mut out, "Run `bunny lock` to write a fresh one.");
            }
            LockState::Matches(n) => line(
                &mut out,
                "✓",
                "Versions",
                &format!("all {n} packages match bunny.lock"),
            ),
            LockState::NewPackages(n) => {
                line(
                    &mut out,
                    "⚠",
                    "Versions",
                    &format!("{n} installed package{} not in bunny.lock yet", plural(*n)),
                );
                more(
                    &mut out,
                    "`bunny plan` lists them. `bunny switch` records them in the lock.",
                );
            }
            LockState::Differs(n) => {
                line(
                    &mut out,
                    "⚠",
                    "Versions",
                    &format!("{n} difference{} from bunny.lock", plural(*n)),
                );
                more(
                    &mut out,
                    "`bunny plan` lists them. `bunny switch` puts them back; `bunny lock` accepts them.",
                );
            }
        }

        // The last change
        match &self.last {
            None => line(
                &mut out,
                "·",
                "Last change",
                "none yet (bunny hasn't changed anything on this system)",
            ),
            Some(s) => {
                let when = self.when(&s.at);
                let what = s.kind_label(self.last_to_number);
                line(&mut out, "·", "Last change", &format!("{what}, {when}"));
            }
        }
        if self.reboot_pending {
            more(
                &mut out,
                &format!(
                    "⚠ Restart to finish the rollback: {} reboot",
                    crate::security::admin()
                ),
            );
        }

        // Undo
        match &self.generations {
            None => {
                line(
                    &mut out,
                    "⚠",
                    "Undo",
                    "not available: there's no /.snapshots on btrfs",
                );
                more(
                    &mut out,
                    "Changes still work, but they can't be rolled back.",
                );
            }
            Some(gens) if gens.is_empty() => line(
                &mut out,
                "✓",
                "Undo",
                "ready; a generation is saved on the next change",
            ),
            Some(gens) => {
                let newest = gens.last().map(|g| g.when()).unwrap_or_default();
                line(
                    &mut out,
                    "✓",
                    "Undo",
                    &format!(
                        "{} generation{} saved, newest {newest}",
                        gens.len(),
                        plural(gens.len())
                    ),
                );
                more(&mut out, "`bunny rollback list` shows them.");
            }
        }

        // Kernel (only shown when something's wrong)
        if !self.kernels_without_drivers.is_empty() {
            line(
                &mut out,
                "⚠",
                "Kernel",
                &format!(
                    "/boot has kernel {}, but this system has no drivers for it",
                    self.kernels_without_drivers.join(", ")
                ),
            );
            more(
                &mut out,
                "If the computer starts without network or graphics, hold Esc while starting",
            );
            more(
                &mut out,
                "and pick another kernel under \"Advanced options\".",
            );
        }

        // Updates
        match self.tree_synced {
            None => {
                line(
                    &mut out,
                    "⚠",
                    "Updates",
                    "the Gentoo package tree was never synced",
                );
                more(&mut out, "Run `bunny update`.");
            }
            Some(t) => {
                let ago = ago(self.now - t);
                if self.tree_stale() {
                    line(
                        &mut out,
                        "⚠",
                        "Updates",
                        &format!("last checked {ago}; there may be security fixes waiting"),
                    );
                    more(&mut out, "Run `bunny update`.");
                } else {
                    line(&mut out, "✓", "Updates", &format!("last checked {ago}"));
                }
            }
        }

        // Security
        match &self.security {
            None => {}
            Some(crate::glsa::Check::Done(found)) if found.is_empty() => line(
                &mut out,
                "✓",
                "Security",
                "no Gentoo security advisories affect this system",
            ),
            Some(crate::glsa::Check::Done(found)) => {
                line(
                    &mut out,
                    "⚠",
                    "Security",
                    &format!(
                        "{} Gentoo security {} affect{} this system",
                        found.len(),
                        if found.len() == 1 {
                            "advisory"
                        } else {
                            "advisories"
                        },
                        if found.len() == 1 { "s" } else { "" }
                    ),
                );
                const SHOWN: usize = 5;
                for a in found.iter().take(SHOWN) {
                    more(&mut out, &format!("  {}  {}", a.id, a.title));
                }
                if found.len() > SHOWN {
                    more(&mut out, &format!("  … and {} more", found.len() - SHOWN));
                }
                more(
                    &mut out,
                    "Run `bunny update` to get the fixes. Details: glsa-check --dump <number>",
                );
            }
            Some(crate::glsa::Check::NotInstalled) => {
                line(
                    &mut out,
                    "·",
                    "Security",
                    "not checked: glsa-check isn't installed",
                );
                more(
                    &mut out,
                    "To turn this on: bunny add app-portage/gentoolkit",
                );
            }
            Some(crate::glsa::Check::Failed(why)) => {
                line(
                    &mut out,
                    "·",
                    "Security",
                    &format!("couldn't check ({why})"),
                );
            }
        }

        out.push('\n');
        out.push_str(if self.all_good() {
            "Everything is in order.\n"
        } else {
            "Things marked ⚠ or ✗ need a look; the line under each says what to do.\n"
        });
        out
    }

    /// "Nov 17, 9:15 AM (3 hours ago)" in local time, from a
    /// `YYYYMMDD-HHMMSS` stamp (UTC), the same wording as the boot menu.
    fn when(&self, stamp: &str) -> String {
        let pretty = rollback::Generation {
            name: format!("bunny-{stamp}"),
        }
        .when();
        match parse_stamp(stamp) {
            Some(t) => format!("{pretty} ({})", ago(self.now - t)),
            None => pretty,
        }
    }
}

impl notes::State {
    fn kind_label(&self, to_number: Option<usize>) -> String {
        match self.last {
            notes::Kind::Switch => "a switch".to_string(),
            notes::Kind::Rollback => match to_number {
                Some(n) => format!("rolled back to generation {n}"),
                None => "a rollback".to_string(),
            },
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// "just now", "5 minutes ago", "3 hours ago", "2 days ago".
pub fn ago(secs: i64) -> String {
    let unit = |n: i64, word: &str| format!("{n} {word}{} ago", if n == 1 { "" } else { "s" });
    match secs {
        s if s < 60 => "just now".to_string(),
        s if s < 3_600 => unit(s / 60, "minute"),
        s if s < 86_400 => unit(s / 3_600, "hour"),
        s => unit(s / 86_400, "day"),
    }
}

/// Seconds since 1970 from a `YYYYMMDD-HHMMSS` UTC stamp.
pub fn parse_stamp(stamp: &str) -> Option<i64> {
    let (date, time) = stamp.split_once('-')?;
    if date.len() != 8 || time.len() < 6 {
        return None;
    }
    let num = |s: &str| s.parse::<i64>().ok();
    let (y, m, d) = (num(&date[..4])?, num(&date[4..6])?, num(&date[6..8])?);
    let (h, mi, s) = (num(&time[..2])?, num(&time[2..4])?, num(&time[4..6])?);
    // Calendar date to days since 1970-01-01 (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3_600 + mi * 60 + s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_795_000_000; // 2026-11-18

    fn good() -> Status {
        Status {
            config: "/etc/bunny/bunny.toml".into(),
            config_state: ConfigState::Matches,
            extra_services: vec![],
            lock: "/etc/bunny/bunny.lock".into(),
            lock_state: LockState::Matches(812),
            last: Some(notes::State {
                last: notes::Kind::Switch,
                at: "20261117-091500".into(),
                to: None,
            }),
            last_to_number: None,
            reboot_pending: false,
            generations: Some(vec![rollback::Generation {
                name: "bunny-20261117-091500".into(),
            }]),
            tree_synced: Some(NOW - 2 * 86_400),
            security: Some(crate::glsa::Check::Done(vec![])),
            kernels_without_drivers: vec![],
            now: NOW,
        }
    }

    #[test]
    fn stamps_parse_to_the_right_time() {
        assert_eq!(parse_stamp("19700101-000000"), Some(0));
        assert_eq!(parse_stamp("20000301-000000"), Some(951_868_800));
        assert_eq!(parse_stamp("20261117-091500"), Some(1_794_906_900));
        assert_eq!(parse_stamp("garbage"), None);
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(ago(10), "just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(3 * 3_600 + 5), "3 hours ago");
        assert_eq!(ago(86_400), "1 day ago");
        assert_eq!(ago(9 * 86_400), "9 days ago");
    }

    #[test]
    fn a_healthy_system_says_so() {
        let s = good();
        assert!(s.all_good());
        let text = s.render();
        assert!(text.contains("✓ Config        the system matches your config"));
        assert!(text.contains("all 812 packages match bunny.lock"));
        // Local time, like the boot menu, never UTC.
        let local = rollback::local_time("bunny-20261117-091500").unwrap();
        assert!(
            text.contains(&format!("a switch, {local} (1 day ago)")),
            "{text}"
        );
        assert!(!text.contains("UTC"), "{text}");
        assert!(text.contains("1 generation saved"));
        assert!(text.contains("last checked 2 days ago"));
        assert!(text.ends_with("Everything is in order.\n"));
    }

    #[test]
    fn unapplied_changes_point_to_switch() {
        let mut s = good();
        s.config_state = ConfigState::Differs {
            count: 2,
            summary: "+ mpv, start sshd".into(),
        };
        assert!(!s.all_good());
        let text = s.render();
        assert!(text.contains("⚠ Config        2 changes not applied yet: + mpv, start sshd"));
        assert!(text.contains("`bunny diff` shows them side by side"));
    }

    #[test]
    fn services_enabled_by_hand_are_mentioned() {
        let mut s = good();
        s.extra_services = vec!["tlp".into()];
        assert!(!s.all_good());
        let text = s.render();
        assert!(text.contains("⚠ Config        your packages match, but services"));
        assert!(!text.contains("Everything is in order"));
        assert!(text.contains("Also starting at boot, not in your config: tlp"));
    }

    #[test]
    fn a_stale_tree_suggests_updating() {
        let mut s = good();
        s.tree_synced = Some(NOW - 12 * 86_400);
        let text = s.render();
        assert!(text.contains("last checked 12 days ago; there may be security fixes waiting"));
        assert!(text.contains("Run `bunny update`."));
        assert!(!s.all_good());
    }

    #[test]
    fn rollbacks_name_the_generation_and_the_reboot() {
        let mut s = good();
        s.last = Some(notes::State {
            last: notes::Kind::Rollback,
            at: "20261117-100000".into(),
            to: Some("bunny-20261117-091500".into()),
        });
        s.last_to_number = Some(1);
        s.reboot_pending = true;
        let text = s.render();
        assert!(text.contains("rolled back to generation 1"));
        assert!(text.contains(&format!(
            "Restart to finish the rollback: {} reboot",
            crate::security::admin()
        )));
    }

    #[test]
    fn missing_pieces_say_what_to_do() {
        let mut s = good();
        s.config_state = ConfigState::Missing;
        s.lock_state = LockState::Missing;
        s.generations = None;
        s.last = None;
        s.tree_synced = None;
        let text = s.render();
        assert!(text.contains(&format!(
            "{} bunny import -o /etc/bunny/bunny.toml",
            crate::security::admin()
        )));
        assert!(text.contains("no bunny.lock yet"));
        assert!(text.contains("Undo          not available"));
        assert!(text.contains("Last change   none yet (bunny hasn't changed anything"));
        assert!(text.contains("never synced"));
    }

    #[test]
    fn gather_reads_a_fake_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".snapshots/bunny-20261117-091500")).unwrap();
        std::fs::create_dir_all(root.join("var/db/repos/gentoo/metadata")).unwrap();
        std::fs::write(
            root.join("var/db/repos/gentoo/metadata/timestamp.chk"),
            "Tue, 17 Nov 2026 06:45:00 +0000\n",
        )
        .unwrap();
        let s = gather(&root.join("etc/bunny/bunny.toml"), root, NOW);
        assert_eq!(s.config_state, ConfigState::Missing);
        assert_eq!(s.lock_state, LockState::Missing);
        assert_eq!(s.generations.as_ref().map(Vec::len), Some(1));
        assert!(s.tree_synced.is_some());
        assert!(!s.reboot_pending);
    }

    #[test]
    fn security_advisories_need_an_update() {
        let s = good();
        assert!(s
            .render()
            .contains("✓ Security      no Gentoo security advisories affect this system"));
        let mut s = good();
        s.security = Some(crate::glsa::Check::Done(vec![crate::glsa::Advisory {
            id: "202501-03".into(),
            title: "OpenSSL: Multiple vulnerabilities".into(),
            packages: vec!["dev-libs/openssl".into()],
        }]));
        assert!(!s.all_good());
        let text = s.render();
        assert!(
            text.contains("⚠ Security      1 Gentoo security advisory affects this system"),
            "{text}"
        );
        assert!(text.contains("202501-03  OpenSSL: Multiple vulnerabilities"));
        assert!(text.contains("Run `bunny update` to get the fixes."));
    }

    #[test]
    fn missing_glsa_check_is_a_suggestion_not_a_problem() {
        let mut s = good();
        s.security = Some(crate::glsa::Check::NotInstalled);
        assert!(s.all_good());
        assert!(s.render().contains("bunny add app-portage/gentoolkit"));
        s.security = None;
        assert!(!s.render().contains("Security"));
    }

    #[test]
    fn a_kernel_without_drivers_is_pointed_out() {
        let mut s = good();
        s.kernels_without_drivers = vec!["6.12.60-gentoo-dist".into()];
        let text = s.render();
        assert!(!s.all_good());
        assert!(text.contains("⚠ Kernel"), "{text}");
        assert!(text.contains("6.12.60-gentoo-dist"), "{text}");
        assert!(text.contains("Advanced options"), "{text}");
    }
}
