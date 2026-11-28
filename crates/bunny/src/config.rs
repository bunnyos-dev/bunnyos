//! `bunny.toml`: what the user wants their system to be.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// The whole config file.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub system: System,
    /// Packages to install, as `category/name` (the world set).
    #[serde(default)]
    pub packages: Vec<String>,
    /// Services to enable at boot, by plain name.
    #[serde(default)]
    pub services: Vec<String>,
    /// Package repositories (overlays) besides Gentoo's own, by the names in
    /// Gentoo's list of repositories, e.g. `["guru"]`.
    #[serde(default)]
    pub repositories: Vec<String>,
    /// Per-package settings, keyed by `category/name`.
    #[serde(default)]
    pub pkg: BTreeMap<String, PkgSettings>,
    /// Admin rights: sudo or doas, and who gets them. Not managed if absent.
    #[serde(default)]
    pub security: Option<crate::security::Security>,
    /// Boot menu settings.
    #[serde(default)]
    pub boot: crate::boot::Boot,
    /// Developer mode: recording commands and sending reports.
    #[serde(default)]
    pub dev: crate::dev::Dev,
    /// What a graphical desktop needs from the base system.
    #[serde(default)]
    pub desktop: crate::desktop::Desktop,
    /// The host file read on top of this config, if there was one.
    #[serde(skip)]
    pub host: Option<HostLayer>,
    /// This computer's host file in /etc/bunny/hosts, when it exists but
    /// wasn't read because the config has its own `hosts/` folder without
    /// a file for this computer.
    #[serde(skip)]
    pub host_left_out: Option<std::path::PathBuf>,
}

/// `hosts/<host name>.toml` next to the config: what belongs to one
/// computer only, so the main config works on any machine. Its packages,
/// services and `[pkg]` settings are added to the main config's (its `[pkg]`
/// wins for the same package). `[machine]` records how `bunny install` set
/// up this computer's disk; bunny reads it but never changes disks from it.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Host {
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub pkg: BTreeMap<String, PkgSettings>,
    #[serde(default)]
    pub machine: Option<Machine>,
}

/// How this computer was set up (written by `bunny install`).
#[derive(Debug, Default, Deserialize, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub disk: Option<String>,
    #[serde(default)]
    pub encrypted: bool,
    pub efi_uuid: Option<String>,
    pub root_uuid: Option<String>,
    pub luks_uuid: Option<String>,
    pub home_disk: Option<String>,
    pub swap: Option<String>,
    #[serde(default)]
    pub hibernate: bool,
    pub timezone: Option<String>,
    pub keymap: Option<String>,
}

/// The host file that was read, and what it held.
#[derive(Debug, PartialEq)]
pub struct HostLayer {
    pub name: String,
    pub path: std::path::PathBuf,
    pub machine: Option<Machine>,
    /// Read from /etc/bunny/hosts because the config (a copy somewhere
    /// else) has no `hosts/` folder of its own.
    pub borrowed: bool,
}

/// This computer's name: OpenRC's setting first (so it's right inside a
/// new system during install, too), then the running kernel's.
pub fn this_host(root: &Path) -> Option<String> {
    let from_conf = std::fs::read_to_string(root.join("etc/conf.d/hostname"))
        .ok()
        .and_then(|text| {
            text.lines()
                .filter_map(|l| l.trim().strip_prefix("hostname="))
                .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
                .next_back()
        });
    let from_etc = || {
        std::fs::read_to_string(root.join("etc/hostname"))
            .ok()
            .map(|t| t.trim().to_string())
    };
    let from_kernel = || {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .map(|t| t.trim().to_string())
    };
    from_conf
        .filter(|h| !h.is_empty())
        .or_else(|| from_etc().filter(|h| !h.is_empty()))
        .or_else(|| from_kernel().filter(|h| !h.is_empty()))
}

/// Where the host file for `host` lives, for a config at `config`.
pub fn host_path(config: &Path, host: &str) -> std::path::PathBuf {
    config
        .parent()
        .unwrap_or(Path::new("."))
        .join("hosts")
        .join(format!("{host}.toml"))
}

/// Where this computer's host files live on the system.
pub const SYSTEM_HOSTS: &str = "/etc/bunny/hosts";

/// Which host file goes with the config at `config` for `host`.
///
/// `hosts/<host>.toml` next to the config, as always. A config with no
/// `hosts/` folder next to it (a copy in your home folder, say) borrows
/// this computer's file from `system_hosts` (normally /etc/bunny/hosts):
/// without it, the packages only this computer has (GRUB, the kernel…)
/// would look like they should be removed. Returns the file and whether
/// it was borrowed, or `Err` with this computer's file when the config
/// has a `hosts/` folder without a file for this computer (so it's left
/// out on purpose, and bunny says so).
pub fn host_file_for(
    config: &Path,
    host: &str,
    system_hosts: Option<&Path>,
) -> std::result::Result<Option<(std::path::PathBuf, bool)>, std::path::PathBuf> {
    let next_to = host_path(config, host);
    if next_to.is_file() {
        return Ok(Some((next_to, false)));
    }
    let Some(system_hosts) = system_hosts else {
        return Ok(None);
    };
    let system = system_hosts.join(format!("{host}.toml"));
    if !system.is_file() || same_file(&system, &next_to) {
        return Ok(None);
    }
    if next_to.parent().is_some_and(Path::is_dir) {
        return Err(system);
    }
    Ok(Some((system, true)))
}

/// Whether two paths name the same file (or the same missing one).
fn same_file(a: &Path, b: &Path) -> bool {
    match (
        a.canonicalize(),
        b.parent().and_then(|p| p.canonicalize().ok()),
    ) {
        (Ok(a), Some(b_dir)) => b.file_name().is_some_and(|n| a == b_dir.join(n)),
        _ => a == b,
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct System {
    #[serde(default)]
    pub init: Init,
    #[serde(default = "default_true")]
    pub multilib: bool,
    /// Gentoo's plain profile ("minimal") or its desktop one ("desktop").
    /// Not set: bunny leaves the profile as it is.
    #[serde(default)]
    pub profile: Option<crate::profile::Profile>,
}

impl Default for System {
    fn default() -> Self {
        Self {
            init: Init::default(),
            multilib: true,
            profile: None,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Init {
    #[default]
    Openrc,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PkgSettings {
    /// USE flags, e.g. `["pgo", "-telemetry"]`.
    #[serde(default, rename = "use")]
    pub use_flags: Vec<String>,
    /// Versions to accept beyond stable ones, e.g. `["~amd64"]` for testing
    /// versions (Portage's package.accept_keywords).
    #[serde(default)]
    pub keywords: Vec<String>,
    /// `false`: always compile this package instead of downloading Gentoo's
    /// prebuilt copy (emerge's `--usepkg-exclude`). For a package whose
    /// prebuilt copies were all built with flags that don't fit (bunny's
    /// fixer sets this when that's the only way). Unset: prebuilt when a
    /// matching one exists, like everything else.
    #[serde(default)]
    pub prebuilt: Option<bool>,
}

/// Something wrong with the config, explained for humans.
#[derive(Debug, PartialEq)]
pub struct Problem {
    pub what: String,
    pub fix: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n    Fix: {}", self.what, self.fix)
    }
}

impl Config {
    /// USE flags per package as they should be applied: the config's own,
    /// plus Bunny Linux defaults for packages bunny installs itself and the
    /// ones `[desktop]` needs. A package's own `[pkg]` entry always wins over
    /// a default.
    pub fn use_flags(&self) -> BTreeMap<String, Vec<String>> {
        let mut flags: BTreeMap<String, Vec<String>> = self
            .pkg
            .iter()
            .filter(|(_, s)| !s.use_flags.is_empty())
            .map(|(atom, s)| (atom.clone(), s.use_flags.clone()))
            .collect();
        for (atom, defaults) in self.desktop.use_flags() {
            if !self.pkg.contains_key(*atom) {
                flags.insert(
                    atom.to_string(),
                    defaults.iter().map(|f| f.to_string()).collect(),
                );
            }
        }
        if let Some(sec) = &self.security {
            for (atom, defaults) in sec.privilege.default_use() {
                if !self.pkg.contains_key(*atom) {
                    flags.insert(
                        atom.to_string(),
                        defaults.iter().map(|f| f.to_string()).collect(),
                    );
                }
            }
        }
        flags
    }

    /// Services to start at boot: `services`, plus the ones `[desktop]` needs.
    pub fn wanted_services(&self) -> Vec<String> {
        let mut all = self.services.clone();
        for service in self.desktop.services() {
            if !all.iter().any(|s| s == service) {
                all.push(service.to_string());
            }
        }
        all
    }

    /// Whether a package, service or package's USE flags are in effect only
    /// because of `[desktop]` (not also written in the config itself), so
    /// `plan` and `diff` can say where they come from.
    pub fn added_by_desktop(&self, name: &str) -> bool {
        let package =
            self.desktop.packages().contains(&name) && !self.packages.iter().any(|p| p == name);
        let service =
            self.desktop.services().contains(&name) && !self.services.iter().any(|s| s == name);
        let flags = self.desktop.use_flags().iter().any(|(a, _)| *a == name)
            && !self.pkg.contains_key(name);
        package || service || flags
    }

    /// Packages the config says to always compile (`prebuilt = false`).
    pub fn compile_from_source(&self) -> Vec<String> {
        self.pkg
            .iter()
            .filter(|(_, s)| s.prebuilt == Some(false))
            .map(|(atom, _)| atom.clone())
            .collect()
    }

    /// Accepted keywords per package, from `[pkg] keywords`.
    pub fn keywords(&self) -> BTreeMap<String, Vec<String>> {
        self.pkg
            .iter()
            .filter(|(_, s)| !s.keywords.is_empty())
            .map(|(atom, s)| (atom.clone(), s.keywords.clone()))
            .collect()
    }

    /// Reads the config and, if there is one, this computer's host file
    /// (borrowed from /etc/bunny/hosts for a copy with no `hosts/` folder;
    /// see [`host_file_for`]).
    pub fn load(path: &Path) -> Result<Self> {
        let host = this_host(Path::new("/"));
        Self::load_with(path, host.as_deref(), Some(Path::new(SYSTEM_HOSTS)))
    }

    /// Reads the config and the host file for `host` next to it (if it
    /// exists).
    pub fn load_for_host(path: &Path, host: Option<&str>) -> Result<Self> {
        Self::load_with(path, host, None)
    }

    /// Reads the config and the host file for `host`, borrowing it from
    /// `system_hosts` as [`host_file_for`] says.
    pub fn load_with(path: &Path, host: Option<&str>, system_hosts: Option<&Path>) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Couldn't read {}", path.display()))?;
        let mut config = Self::parse(&text)
            .with_context(|| format!("{} has a mistake in it", path.display()))?;
        if let Some(name) = host {
            match host_file_for(path, name, system_hosts) {
                Ok(Some((host_file, borrowed))) => {
                    let text = std::fs::read_to_string(&host_file)
                        .with_context(|| format!("Couldn't read {}", host_file.display()))?;
                    let layer: Host = toml::from_str(&text).with_context(|| {
                        format!(
                            "{} (the settings for this computer) has a mistake in it",
                            host_file.display()
                        )
                    })?;
                    config.add_host(name, host_file, layer);
                    if let Some(h) = config.host.as_mut() {
                        h.borrowed = borrowed;
                    }
                }
                Ok(None) => {}
                Err(left_out) => config.host_left_out = Some(left_out),
            }
        }
        Ok(config)
    }

    /// Adds a host file's settings on top of this config.
    pub fn add_host(&mut self, name: &str, path: std::path::PathBuf, host: Host) {
        for atom in host.packages {
            if !self.packages.contains(&atom) {
                self.packages.push(atom);
            }
        }
        for service in host.services {
            if !self.services.contains(&service) {
                self.services.push(service);
            }
        }
        for (atom, settings) in host.pkg {
            self.pkg.insert(atom, settings);
        }
        self.host = Some(HostLayer {
            name: name.to_string(),
            path,
            machine: host.machine,
            borrowed: false,
        });
    }

    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|err| {
            let message = err.to_string();
            match misplaced_top_level_key(&message) {
                Some(key) => anyhow::anyhow!(
                    "{message}\n\
                     `{key}` ended up inside a [section] because it comes after one.\n\
                     In TOML, everything below a [section] header belongs to that section.\n\
                     Fix: move `{key}` above the first [section] in the file."
                ),
                None => anyhow::Error::new(err),
            }
        })
    }

    /// Checks that go beyond "is this valid TOML".
    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();

        for atom in &self.packages {
            if !is_package_name(atom) {
                problems.push(Problem {
                    what: format!("\"{atom}\" in `packages` isn't a full package name."),
                    fix: "Use category/name, for example \"www-client/firefox\".".into(),
                });
            }
        }
        check_duplicates(&self.packages, "packages", &mut problems);

        for name in &self.services {
            if name.is_empty() || name.contains('/') || name.contains(char::is_whitespace) {
                problems.push(Problem {
                    what: format!("\"{name}\" in `services` isn't a valid service name."),
                    fix: "Use the plain service name, for example \"sshd\".".into(),
                });
            }
        }
        check_duplicates(&self.services, "services", &mut problems);

        for (atom, settings) in &self.pkg {
            if !is_package_name(atom) {
                problems.push(Problem {
                    what: format!("[pkg.\"{atom}\"] isn't a full package name."),
                    fix: "Use category/name, for example [pkg.\"www-client/firefox\"].".into(),
                });
            }
            for flag in &settings.use_flags {
                if !is_use_flag(flag) {
                    problems.push(Problem {
                        what: format!("\"{flag}\" in the USE flags for {atom} isn't a valid flag."),
                        fix: "Flags are words like \"pgo\", or \"-telemetry\" to turn one off."
                            .into(),
                    });
                }
            }
            check_use_conflicts(atom, &settings.use_flags, &mut problems);
            for keyword in &settings.keywords {
                if !is_keyword(keyword) {
                    problems.push(Problem {
                        what: format!("\"{keyword}\" in the keywords for {atom} isn't a keyword."),
                        fix: "Use \"~amd64\" for testing versions (or \"**\" for any version, \
                              including unreleased ones from git)."
                            .into(),
                    });
                }
            }
        }

        for name in &self.repositories {
            if !is_repository_name(name) {
                problems.push(Problem {
                    what: format!("\"{name}\" in `repositories` isn't a repository name."),
                    fix: "Use the name from Gentoo's list, like \"guru\" \
                          (see https://repos.gentoo.org)."
                        .into(),
                });
            } else if name == "gentoo" {
                problems.push(Problem {
                    what: "\"gentoo\" is in `repositories`, but Gentoo's own is always there."
                        .into(),
                    fix: "Remove it from the list.".into(),
                });
            }
        }
        check_duplicates(&self.repositories, "repositories", &mut problems);

        problems
    }
}

/// Top-level keys that are easy to accidentally put below a `[section]`.
const TOP_LEVEL_KEYS: [&str; 3] = ["packages", "services", "repositories"];

/// If a TOML error is a top-level key that landed inside a section, returns it.
fn misplaced_top_level_key(message: &str) -> Option<&'static str> {
    // At the top level the error lists `system` as an expected field;
    // inside a section it doesn't.
    if message.contains("`system`") {
        return None;
    }
    TOP_LEVEL_KEYS
        .into_iter()
        .find(|key| message.contains(&format!("unknown field `{key}`")))
}

/// `category/name`, optionally with a slot (`category/name:slot`),
/// using the characters Gentoo allows.
pub fn is_package_name(s: &str) -> bool {
    let (atom, slot) = match s.split_once(':') {
        Some((atom, slot)) => (atom, Some(slot)),
        None => (s, None),
    };
    let Some((cat, name)) = atom.split_once('/') else {
        return false;
    };
    let ok = |part: &str| {
        !part.is_empty()
            && !part.starts_with(['-', '.', '+'])
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+_.-".contains(c))
    };
    ok(cat) && ok(name) && !name.contains('/') && slot.is_none_or(ok)
}

/// A keyword for package.accept_keywords: `amd64`, `~amd64`, `**`, `~*`, `*`.
fn is_keyword(s: &str) -> bool {
    if matches!(s, "**" | "~*" | "*") {
        return true;
    }
    let arch = s.strip_prefix('~').unwrap_or(s);
    let mut chars = arch.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A repository name as Portage allows: letters, digits, `_` and `-`, not
/// starting with `-`.
pub fn is_repository_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A USE flag, optionally prefixed with `-` to disable it.
fn is_use_flag(s: &str) -> bool {
    let flag = s.strip_prefix('-').unwrap_or(s);
    let mut chars = flag.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "+_@-".contains(c))
}

fn check_duplicates(items: &[String], field: &str, problems: &mut Vec<Problem>) {
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        if !seen.insert(item) {
            problems.push(Problem {
                what: format!("\"{item}\" is listed twice in `{field}`."),
                fix: "Remove one of them.".into(),
            });
        }
    }
}

fn check_use_conflicts(atom: &str, flags: &[String], problems: &mut Vec<Problem>) {
    for flag in flags.iter().filter(|f| !f.starts_with('-')) {
        if flags.iter().any(|f| f.strip_prefix('-') == Some(flag)) {
            problems.push(Problem {
                what: format!("{atom} has the USE flag \"{flag}\" both on and off."),
                fix: format!("Keep either \"{flag}\" or \"-{flag}\"."),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_keywords_and_repositories() {
        let c = Config::parse(
            "repositories = [\"guru\", \"hyproverlay\"]\n\
             [pkg.\"gui-wm/dwl\"]\nkeywords = [\"~amd64\"]\n\
             [pkg.\"gui-libs/scenefx\"]\nkeywords = [\"**\"]\n",
        )
        .unwrap();
        assert!(c.problems().is_empty(), "{:?}", c.problems());
        assert_eq!(c.keywords()["gui-wm/dwl"], ["~amd64"]);
        assert!(!c.use_flags().contains_key("gui-wm/dwl"));

        let bad = Config::parse(
            "repositories = [\"guru\", \"guru\", \"my repo\", \"gentoo\"]\n\
             [pkg.\"gui-wm/dwl\"]\nkeywords = [\"amd64~\"]\n",
        )
        .unwrap();
        let text: Vec<String> = bad.problems().iter().map(|p| p.what.clone()).collect();
        assert!(
            text.iter()
                .any(|t| t.contains("\"amd64~\" in the keywords")),
            "{text:?}"
        );
        assert!(text
            .iter()
            .any(|t| t.contains("\"my repo\" in `repositories`")));
        assert!(text.iter().any(|t| t.contains("\"guru\" is listed twice")));
        assert!(text
            .iter()
            .any(|t| t.contains("Gentoo's own is always there")));

        // The section trap catches `repositories` too.
        let err = Config::parse("[system]\nrepositories = []")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("`repositories` ended up inside a [section]"),
            "{err}"
        );
    }

    #[test]
    fn adds_the_host_file_on_top() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("bunny.toml");
        std::fs::write(
            &config,
            "packages = [\"app-misc/tmux\", \"sys-fs/btrfs-progs\"]\nservices = [\"sshd\"]\n\
             [pkg.\"app-misc/tmux\"]\nuse = [\"vim-syntax\"]\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("hosts")).unwrap();
        std::fs::write(
            dir.path().join("hosts/laptop.toml"),
            "packages = [\"sys-fs/btrfs-progs\", \"sys-fs/cryptsetup\"]\nservices = [\"tlp\"]\n\
             [pkg.\"app-misc/tmux\"]\nuse = [\"-vim-syntax\"]\n\
             [machine]\ndisk = \"/dev/vda\"\nencrypted = true\ntimezone = \"UTC\"\n",
        )
        .unwrap();

        let c = Config::load_for_host(&config, Some("laptop")).unwrap();
        assert_eq!(
            c.packages,
            ["app-misc/tmux", "sys-fs/btrfs-progs", "sys-fs/cryptsetup"]
        );
        assert_eq!(c.services, ["sshd", "tlp"]);
        assert_eq!(c.pkg["app-misc/tmux"].use_flags, ["-vim-syntax"]);
        let host = c.host.as_ref().unwrap();
        assert_eq!(host.name, "laptop");
        assert!(host.machine.as_ref().unwrap().encrypted);
        // Listed in both files: not a duplicate.
        assert!(c.problems().is_empty(), "{:?}", c.problems());

        // Another computer: just the main config.
        let c = Config::load_for_host(&config, Some("desktop")).unwrap();
        assert_eq!(c.packages.len(), 2);
        assert!(c.host.is_none());
    }

    #[test]
    fn a_copy_elsewhere_borrows_this_computers_host_file() {
        // /etc/bunny with this computer's host file, and a copy of
        // bunny.toml in a home folder with no hosts/ next to it.
        let sys = tempfile::tempdir().unwrap();
        let hosts = sys.path().join("etc/bunny/hosts");
        std::fs::create_dir_all(&hosts).unwrap();
        std::fs::write(
            hosts.join("desktop.toml"),
            "packages = [\"sys-boot/grub\", \"sys-kernel/gentoo-kernel-bin\"]\n",
        )
        .unwrap();
        let home = tempfile::tempdir().unwrap();
        let copy = home.path().join("bunny.toml");
        std::fs::write(&copy, "packages = [\"app-misc/tmux\"]\n").unwrap();

        let c = Config::load_with(&copy, Some("desktop"), Some(&hosts)).unwrap();
        // Without the host file, GRUB and the kernel would be "removed".
        assert!(
            c.packages.contains(&"sys-boot/grub".to_string()),
            "{:?}",
            c.packages
        );
        let host = c.host.as_ref().unwrap();
        assert!(host.borrowed);
        assert_eq!(host.path, hosts.join("desktop.toml"));

        // A copy with its own hosts/ folder, but no file for this computer:
        // left out on purpose, and remembered so bunny can say so.
        std::fs::create_dir_all(home.path().join("hosts")).unwrap();
        let c = Config::load_with(&copy, Some("desktop"), Some(&hosts)).unwrap();
        assert!(c.host.is_none());
        assert_eq!(c.host_left_out, Some(hosts.join("desktop.toml")));

        // Its own file wins, and isn't borrowed.
        std::fs::write(home.path().join("hosts/desktop.toml"), "packages = []\n").unwrap();
        let c = Config::load_with(&copy, Some("desktop"), Some(&hosts)).unwrap();
        let host = c.host.as_ref().unwrap();
        assert!(!host.borrowed);
        assert_eq!(c.host_left_out, None);

        // The real config's own folder is never "borrowed" from itself.
        let real = sys.path().join("etc/bunny/bunny.toml");
        std::fs::write(&real, "").unwrap();
        let c = Config::load_with(&real, Some("desktop"), Some(&hosts)).unwrap();
        assert!(!c.host.as_ref().unwrap().borrowed);
        // Nothing for this computer anywhere: nothing to say.
        let c = Config::load_with(&copy, Some("laptop"), Some(&hosts)).unwrap();
        assert!(c.host.is_none() && c.host_left_out.is_none());
    }

    #[test]
    fn explains_a_mistake_in_the_host_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("bunny.toml");
        std::fs::write(&config, "").unwrap();
        std::fs::create_dir_all(dir.path().join("hosts")).unwrap();
        std::fs::write(dir.path().join("hosts/laptop.toml"), "pakages = []\n").unwrap();
        let err = format!(
            "{:#}",
            Config::load_for_host(&config, Some("laptop")).unwrap_err()
        );
        assert!(
            err.contains("hosts/laptop.toml (the settings for this computer)"),
            "{err}"
        );
    }

    #[test]
    fn finds_the_host_name() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("etc/conf.d")).unwrap();
        std::fs::write(root.path().join("etc/hostname"), "other\n").unwrap();
        std::fs::write(
            root.path().join("etc/conf.d/hostname"),
            "# Set to the hostname of this machine\nhostname=\"laptop\"\n",
        )
        .unwrap();
        assert_eq!(this_host(root.path()).as_deref(), Some("laptop"));
        std::fs::remove_file(root.path().join("etc/conf.d/hostname")).unwrap();
        assert_eq!(this_host(root.path()).as_deref(), Some("other"));
    }

    #[test]
    fn empty_config_uses_defaults() {
        let config = Config::parse("").unwrap();
        assert_eq!(config.system.init, Init::Openrc);
        assert!(config.system.multilib);
        assert!(config.problems().is_empty());
    }

    #[test]
    fn parses_a_full_config() {
        let config = Config::parse(
            r#"
            packages = ["www-client/firefox", "app-editors/neovim"]
            services = ["sshd"]

            [system]
            init = "openrc"
            multilib = false

            [pkg."www-client/firefox"]
            use = ["pgo", "-telemetry"]
            "#,
        )
        .unwrap();
        assert_eq!(config.packages.len(), 2);
        assert!(!config.system.multilib);
        assert_eq!(
            config.pkg["www-client/firefox"].use_flags,
            vec!["pgo", "-telemetry"]
        );
        assert!(config.problems().is_empty());
    }

    #[test]
    fn rejects_unknown_fields() {
        let err = Config::parse("pakages = []").unwrap_err().to_string();
        assert!(err.contains("pakages"), "{err}");
    }

    #[test]
    fn explains_keys_placed_after_a_section() {
        let err = Config::parse("[system]\nmultilib = true\npackages = []")
            .unwrap_err()
            .to_string();
        assert!(err.contains("move `packages` above"), "{err}");
    }

    #[test]
    fn plain_typos_get_no_section_hint() {
        let err = Config::parse("pakages = []").unwrap_err().to_string();
        assert!(!err.contains("above the first"), "{err}");
    }

    #[test]
    fn rejects_systemd_for_now() {
        assert!(Config::parse("[system]\ninit = \"systemd\"").is_err());
    }

    #[test]
    fn finds_problems() {
        let config = Config::parse(
            r#"
            packages = ["firefox", "app-editors/neovim", "app-editors/neovim"]
            services = ["bad name"]
            [pkg."www-client/firefox"]
            use = ["pgo", "-pgo", "bad!flag"]
            "#,
        )
        .unwrap();
        let problems = config.problems();
        let text: Vec<_> = problems.iter().map(|p| p.what.as_str()).collect();
        assert_eq!(problems.len(), 5, "{text:#?}");
    }

    #[test]
    fn package_names() {
        assert!(is_package_name("www-client/firefox"));
        assert!(is_package_name("dev-libs/libsigc++"));
        assert!(is_package_name("net-misc/freerdp:3"));
        assert!(is_package_name("sys-fs/fuse:0"));
        assert!(!is_package_name("sys-fs/fuse:"));
        assert!(!is_package_name("sys-fs/fuse::gentoo"));
        assert!(!is_package_name("firefox"));
        assert!(!is_package_name("a/b/c"));
        assert!(!is_package_name("/firefox"));
    }

    #[test]
    fn use_flags() {
        assert!(is_use_flag("pgo"));
        assert!(is_use_flag("-telemetry"));
        assert!(is_use_flag("python_targets_python3_12"));
        assert!(!is_use_flag("-"));
        assert!(!is_use_flag("bad flag"));
    }

    #[test]
    fn desktop_adds_flags_services_and_packages_unless_set() {
        let c = Config::parse(
            "services = [\"sshd\", \"dbus\"]\n[desktop]\nwayland = true\n\
             [pkg.\"sys-auth/seatd\"]\nuse = [\"-elogind\", \"server\"]\n",
        )
        .unwrap();
        assert!(c.problems().is_empty(), "{:?}", c.problems());
        let flags = c.use_flags();
        assert_eq!(flags["sys-auth/pambase"], ["elogind"]);
        // Your own [pkg] entry wins.
        assert_eq!(flags["sys-auth/seatd"], ["-elogind", "server"]);
        assert_eq!(c.wanted_services(), ["sshd", "dbus", "elogind"]);
        assert!(c.added_by_desktop("sys-auth/pambase"));
        assert!(!c.added_by_desktop("sys-auth/seatd"));
        assert!(c.added_by_desktop("elogind"));
        assert!(!c.added_by_desktop("dbus"), "also in `services`");
        assert!(c.added_by_desktop("sys-auth/elogind"));
        assert!(!c.added_by_desktop("sshd"));

        // Without the section (or with it off): nothing extra.
        for text in ["", "[desktop]\nwayland = false\n"] {
            let c = Config::parse(text).unwrap();
            assert!(c.use_flags().is_empty());
            assert!(c.wanted_services().is_empty());
            assert!(!c.added_by_desktop("elogind"));
        }
        assert!(Config::parse("[desktop]\nwm = \"mango\"\n").is_err());
        // The section trap still explains a top-level key below [desktop].
        let err = Config::parse("[desktop]\nwayland = true\nservices = []")
            .unwrap_err()
            .to_string();
        assert!(err.contains("move `services` above"), "{err}");
    }

    #[test]
    fn sudo_gets_bunnyos_default_flags_unless_set() {
        let c = Config::parse("[security]\nprivilege = \"sudo\"\nadmins = [\"you\"]\n").unwrap();
        assert_eq!(c.use_flags()["app-admin/sudo"], ["-sendmail"]);
        let c = Config::parse(
            "[security]\nprivilege = \"sudo\"\n[pkg.\"app-admin/sudo\"]\nuse = [\"sendmail\"]\n",
        )
        .unwrap();
        assert_eq!(c.use_flags()["app-admin/sudo"], ["sendmail"]);
        let c = Config::parse("[security]\nprivilege = \"doas\"\n").unwrap();
        assert!(c.use_flags().is_empty());
    }
}
