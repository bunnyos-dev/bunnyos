//! `bunny import`: write a `bunny.toml` that describes an existing system.
//! Read-only: it only reads the system and produces text.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{is_package_name, Init};
use crate::init;
use crate::portage;

/// What was found on the system.
#[derive(Debug, Default)]
pub struct Imported {
    pub packages: Vec<String>,
    pub services: Vec<String>,
    pub multilib: bool,
    /// USE flags per package, in the order they were found.
    pub use_flags: BTreeMap<String, Vec<String>>,
    /// package.use lines bunny can't represent yet, with the file they came from.
    pub skipped: Vec<(PathBuf, String)>,
    /// How many package.use lines had a version number that was dropped.
    pub versioned: usize,
    /// Whether make.conf sets global USE flags (not managed by bunny yet).
    pub global_use: bool,
    /// Services set to start at boot whose start script is gone. Left out of
    /// the config so another machine doesn't try to enable them.
    pub leftover_services: Vec<String>,
    /// The admin tool installed ("sudo" or "doas"), for the suggested [security].
    pub admin_tool: Option<&'static str>,
    /// Users in the wheel group (except root), for the suggested [security].
    pub wheel: Vec<String>,
}

pub fn from_system(root: &Path) -> Result<Imported> {
    let packages = portage::read_world(root)
        .context("Couldn't read the world set")?
        .into_iter()
        .collect();
    let init = init::for_config(Init::Openrc, root);
    let (services, leftover_services): (Vec<String>, Vec<String>) = init
        .user_services()
        .context("Couldn't read which services start at boot")?
        .into_iter()
        .partition(|s| init.service_exists(s));

    let package_use = read_package_use(root)?;
    Ok(Imported {
        packages,
        services,
        multilib: !profile_is_no_multilib(root),
        global_use: make_conf_sets_use(root),
        use_flags: package_use.flags,
        skipped: package_use.skipped,
        versioned: package_use.versioned,
        admin_tool: if packages_has(root, "app-admin/doas") {
            Some("doas")
        } else if packages_has(root, "app-admin/sudo") {
            Some("sudo")
        } else {
            None
        },
        wheel: crate::security::group_members(root, "wheel")
            .into_iter()
            .filter(|u| u != "root")
            .collect(),
        leftover_services,
    })
}

/// Whether a package is installed (any version).
fn packages_has(root: &Path, atom: &str) -> bool {
    let Some((category, name)) = atom.split_once('/') else {
        return false;
    };
    std::fs::read_dir(root.join("var/db/pkg").join(category))
        .map(|entries| {
            entries.flatten().any(|e| {
                let dir = e.file_name().to_string_lossy().into_owned();
                dir.strip_prefix(name)
                    .and_then(|rest| rest.strip_prefix('-'))
                    .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
            })
        })
        .unwrap_or(false)
}

/// Per-package USE flags from every file in `/etc/portage/package.use`.
#[derive(Debug, Default)]
pub struct PackageUse {
    pub flags: BTreeMap<String, Vec<String>>,
    /// Lines bunny can't represent yet, with the file they came from.
    pub skipped: Vec<(PathBuf, String)>,
    /// How many lines had a version number that was dropped.
    pub versioned: usize,
}

pub fn read_package_use(root: &Path) -> Result<PackageUse> {
    let mut result = PackageUse::default();
    for file in package_use_files(&root.join("etc/portage/package.use"))? {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("Couldn't read {}", file.display()))?;
        let shown = Path::new("/").join(file.strip_prefix(root).unwrap_or(&file));
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            match parse_use_line(line) {
                Some((atom, flags, versioned)) => {
                    if versioned {
                        result.versioned += 1;
                    }
                    let entry = result.flags.entry(atom).or_default();
                    for flag in flags {
                        merge_flag(entry, flag);
                    }
                }
                None => result.skipped.push((shown.clone(), line.to_string())),
            }
        }
    }
    Ok(result)
}

/// package.use can be a single file or a directory of files.
fn package_use_files(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut files = Vec::new();
    if path.is_dir() {
        for entry in
            std::fs::read_dir(path).with_context(|| format!("Couldn't read {}", path.display()))?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // Portage ignores hidden files and editor backups.
            if name.starts_with('.') || name.ends_with('~') {
                continue;
            }
            files.extend(package_use_files(&entry.path())?);
        }
    }
    files.sort();
    Ok(files)
}

/// Parses `category/name flag -flag GROUP: value`.
///
/// Returns the atom, its flags, and whether a version number was dropped from
/// the atom. Returns `None` for lines bunny can't represent yet (version
/// ranges like `<`, package wildcards, flag wildcards like `-*`).
fn parse_use_line(line: &str) -> Option<(String, Vec<String>, bool)> {
    let mut words = line.split_whitespace();
    let (atom, versioned) = normalize_atom(words.next()?)?;
    let mut flags = Vec::new();
    let mut group: Option<String> = None;
    for word in words {
        if word.contains('*') {
            return None;
        }
        if let Some(name) = word.strip_suffix(':') {
            // USE_EXPAND, e.g. `PYTHON_TARGETS: python3_12` → `python_targets_python3_12`
            group = Some(name.to_lowercase());
            continue;
        }
        let (minus, flag) = match word.strip_prefix('-') {
            Some(flag) => ("-", flag),
            None => ("", word),
        };
        match &group {
            Some(group) => flags.push(format!("{minus}{group}_{flag}")),
            None => flags.push(format!("{minus}{flag}")),
        }
    }
    Some((atom, flags, versioned))
}

/// Turns a package.use atom into plain `category/name[:slot]`.
///
/// - `>=cat/pkg-1.2.3`, `=cat/pkg-1.2*` and `~cat/pkg-1.2` lose their version
///   (these are what `emerge --autounmask-write` produces), so the flags apply
///   to every version.
/// - A `::repo` suffix is dropped.
/// - `<`, `<=` and `>` ranges, and wildcards like `*/*`, return `None`.
fn normalize_atom(raw: &str) -> Option<(String, bool)> {
    let raw = raw.split_once("::").map_or(raw, |(atom, _repo)| atom);
    let (body, versioned) = if let Some(rest) = raw.strip_prefix(">=") {
        (rest, true)
    } else if raw.starts_with(['<', '>']) {
        return None;
    } else if let Some(rest) = raw.strip_prefix(['=', '~']) {
        (rest, true)
    } else {
        (raw, false)
    };
    if body.contains('*') && !versioned {
        return None;
    }

    let atom = if versioned {
        let (with_version, slot) = match body.split_once(':') {
            Some((a, s)) => (a, Some(s)),
            None => (body, None),
        };
        let name = strip_version(with_version)?;
        match slot {
            Some(slot) => format!("{name}:{slot}"),
            None => name.to_string(),
        }
    } else {
        body.to_string()
    };

    is_package_name(&atom).then_some((atom, versioned))
}

/// `cat/name-1.2.3-r1` → `cat/name`. The version starts at the first `-`
/// followed by a digit where the rest is a valid version, so names like
/// `font-adobe-100dpi` aren't cut in the wrong place.
pub fn strip_version(s: &str) -> Option<&str> {
    s.match_indices('-')
        .map(|(i, _)| i)
        .find(|&i| is_version(&s[i + 1..]))
        .map(|i| &s[..i])
}

/// Gentoo version syntax: `1.2.3`, `1.2b`, `1.2_rc3`, `1.2_p1-r2`, `1.2*`.
fn is_version(s: &str) -> bool {
    let s = s.strip_suffix('*').unwrap_or(s);
    let s = match s.rsplit_once("-r") {
        Some((v, rev)) if !rev.is_empty() && rev.bytes().all(|b| b.is_ascii_digit()) => v,
        _ => s,
    };
    let (main, suffixes) = match s.split_once('_') {
        Some((m, rest)) => (m, Some(rest)),
        None => (s, None),
    };
    let main = main
        .strip_suffix(|c: char| c.is_ascii_lowercase())
        .unwrap_or(main);
    let numbers_ok = !main.is_empty()
        && main
            .split('.')
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let suffixes_ok = suffixes.is_none_or(|rest| {
        rest.split('_').all(|part| {
            ["alpha", "beta", "pre", "rc", "p"].iter().any(|kind| {
                part.strip_prefix(kind)
                    .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            })
        })
    });
    numbers_ok && suffixes_ok
}

/// Adds a flag, replacing any earlier setting of the same flag
/// (later lines win, like in Portage).
fn merge_flag(flags: &mut Vec<String>, flag: String) {
    let name = flag.trim_start_matches('-');
    flags.retain(|f| f.trim_start_matches('-') != name);
    flags.push(flag);
}

fn profile_is_no_multilib(root: &Path) -> bool {
    std::fs::read_link(root.join("etc/portage/make.profile"))
        .map(|target| target.to_string_lossy().contains("no-multilib"))
        .unwrap_or(false)
}

fn make_conf_sets_use(root: &Path) -> bool {
    std::fs::read_to_string(root.join("etc/portage/make.conf"))
        .map(|text| {
            text.lines()
                .any(|l| l.trim_start().starts_with("USE=") || l.trim_start().starts_with("USE ="))
        })
        .unwrap_or(false)
}

fn quote(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

impl Imported {
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# bunny.toml, imported from this system by `bunny import`.\n\
             # Review it, then run `bunny plan` to check it matches.\n\n",
        );

        list(&mut out, "packages", &self.packages);
        list(&mut out, "services", &self.services);

        out.push_str("[system]\ninit = \"openrc\"\n");
        let _ = writeln!(out, "multilib = {}", self.multilib);

        for (atom, flags) in &self.use_flags {
            if flags.is_empty() {
                continue;
            }
            let flags: Vec<_> = flags.iter().map(|f| quote(f)).collect();
            let _ = write!(
                out,
                "\n[pkg.{}]\nuse = [{}]\n",
                quote(atom),
                flags.join(", ")
            );
        }

        if !self.leftover_services.is_empty() {
            let _ = write!(
                out,
                "\n# Left out of `services`: set to start at boot, but no longer installed\n\
                 # (probably left behind by a removed package): {}\n",
                self.leftover_services.join(", ")
            );
        }

        if self.global_use {
            out.push_str(
                "\n# Your make.conf sets global USE flags. bunny doesn't manage make.conf yet,\n\
                 # so those stay where they are for now.\n",
            );
        }

        if self.versioned > 0 {
            let _ = write!(
                out,
                "\n# {} package.use line(s) were tied to a version (like `>=cat/pkg-1.2`,\n\
                 # often written by `emerge --autounmask-write`). Their flags now apply to\n\
                 # every version of the package.\n",
                self.versioned
            );
        }

        if !self.skipped.is_empty() {
            out.push_str(
                "\n# These package.use lines couldn't be imported yet (`<`/`>` version ranges\n\
                 # or wildcards). They stay in their files and keep working:\n",
            );
            for (file, line) in &self.skipped {
                let _ = writeln!(out, "#   {}: {line}", file.display());
            }
        }
        let admins: Vec<String> = if self.wheel.is_empty() {
            vec!["\"you\"".to_string()]
        } else {
            self.wheel.iter().map(|u| quote(u)).collect()
        };
        let _ = write!(
            out,
            "\n# Admin rights. Uncomment these lines to let bunny manage them: it writes\n\
             # /etc/sudoers.d/bunny (sudo) or /etc/doas.conf (doas) and keeps these users\n\
             # in the wheel group.\n\
             # [security]\n\
             # privilege = \"{}\"   # \"sudo\" or \"doas\"\n\
             # persist = true       # remember your password for a few minutes\n\
             # admins = [{}]\n",
            self.admin_tool.unwrap_or("sudo"),
            admins.join(", ")
        );
        out
    }

    pub fn summary(&self) -> String {
        let mut text = format!(
            "Imported {} packages, {} services, and USE flags for {} packages.",
            self.packages.len(),
            self.services.len(),
            self.use_flags.values().filter(|f| !f.is_empty()).count(),
        );
        if !self.leftover_services.is_empty() {
            let _ = write!(
                text,
                "\nLeft out {} service(s) that start at boot but aren't installed anymore: {}.\n\
                 Clean up with: {} rc-update del <name>",
                self.leftover_services.len(),
                self.leftover_services.join(", "),
                crate::security::admin()
            );
        }
        if self.versioned > 0 {
            let _ = write!(
                text,
                "\n{} line(s) were tied to a version; their flags now apply to every version.",
                self.versioned
            );
        }
        if !self.skipped.is_empty() {
            let _ = write!(
                text,
                "\n{} package.use line(s) couldn't be imported; they're listed at the end as comments.",
                self.skipped.len()
            );
        }
        text
    }
}

fn list(out: &mut String, key: &str, items: &[String]) {
    let _ = writeln!(out, "{key} = [");
    for item in items {
        let _ = writeln!(out, "    {},", quote(item));
    }
    out.push_str("]\n\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::plan;

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fake_system() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            portage::WORLD_FILE,
            "app-editors/neovim\nnet-misc/freerdp:3\nwww-client/firefox\n",
        );
        for (level, service) in [
            ("default", "sshd"),
            ("default", "tailscale"),
            ("boot", "elogind"),
        ] {
            write(root, &format!("etc/runlevels/{level}/{service}"), "");
            write(root, &format!("etc/init.d/{service}"), "");
        }
        // Left behind by a removed package: enabled, but no start script.
        write(root, "etc/runlevels/default/tlp", "");
        write(
            root,
            "etc/portage/package.use/browser",
            "# comment\nwww-client/firefox pgo -telemetry\nwww-client/firefox telemetry\n",
        );
        write(
            root,
            "etc/portage/package.use/python",
            "dev-lang/python:3.12 sqlite PYTHON_TARGETS: python3_12 -python3_11\n\
             >=dev-libs/foo-1.2 bar\n\
             <dev-libs/old-2 baz\n\
             media-gfx/swayimg LUA_SINGLE_TARGET: -* lua5-4\n\
             */* PYTHON_TARGETS: python3_13\n",
        );
        write(root, "etc/portage/package.use/.hidden", "x/y z\n");
        write(root, "etc/portage/make.conf", "USE=\"wayland -X\"\n");
        dir
    }

    #[test]
    fn imports_a_system() {
        let root = fake_system();
        let imported = from_system(root.path()).unwrap();
        assert_eq!(imported.packages.len(), 3);
        assert_eq!(imported.services, ["sshd", "tailscale"]);
        assert_eq!(imported.leftover_services, ["tlp"]);
        assert!(imported.multilib);
        assert!(imported.global_use);
        assert_eq!(
            imported.use_flags["www-client/firefox"],
            ["pgo", "telemetry"]
        );
        assert_eq!(
            imported.use_flags["dev-lang/python:3.12"],
            [
                "sqlite",
                "python_targets_python3_12",
                "-python_targets_python3_11"
            ]
        );
        assert_eq!(imported.use_flags["dev-libs/foo"], ["bar"]);
        assert_eq!(imported.versioned, 1);
        // `<` range, `-*` wildcard, `*/*` wildcard
        assert_eq!(imported.skipped.len(), 3, "{:#?}", imported.skipped);
        assert!(!imported.use_flags.contains_key("x/y"));
    }

    #[test]
    fn normalizes_atoms() {
        let n = |s| normalize_atom(s);
        assert_eq!(
            n("www-client/firefox"),
            Some(("www-client/firefox".into(), false))
        );
        assert_eq!(
            n(">=media-libs/libsndfile-1.2.2"),
            Some(("media-libs/libsndfile".into(), true))
        );
        assert_eq!(
            n("=dev-lang/rust-1.85.0-r1"),
            Some(("dev-lang/rust".into(), true))
        );
        assert_eq!(n("=sys-libs/foo-1.2*"), Some(("sys-libs/foo".into(), true)));
        assert_eq!(
            n("~app-misc/bar-3.0_rc2"),
            Some(("app-misc/bar".into(), true))
        );
        assert_eq!(
            n(">=media-fonts/font-adobe-100dpi-1.0.4"),
            Some(("media-fonts/font-adobe-100dpi".into(), true))
        );
        assert_eq!(
            n(">=dev-lang/python-3.12.4:3.12"),
            Some(("dev-lang/python:3.12".into(), true))
        );
        assert_eq!(
            n("gui-apps/quickshell::guru"),
            Some(("gui-apps/quickshell".into(), false))
        );
        assert_eq!(n("<dev-libs/old-2"), None);
        assert_eq!(n("*/*"), None);
        assert_eq!(n("dev-python/*"), None);
    }

    #[test]
    fn versions() {
        for ok in [
            "1",
            "1.2.3",
            "1.2b",
            "1.2_rc3",
            "1.2_p1-r2",
            "1.2*",
            "2026.11.16",
        ] {
            assert!(is_version(ok), "{ok}");
        }
        for bad in ["", "dpi-1.0", "abc", "1..2", "1.2_foo", "-r1"] {
            assert!(!is_version(bad), "{bad}");
        }
    }

    #[test]
    fn imported_config_is_valid_and_matches_the_system() {
        let root = fake_system();
        let text = from_system(root.path()).unwrap().render();
        let config = Config::parse(&text).unwrap_or_else(|e| panic!("{e}\n---\n{text}"));
        assert!(config.problems().is_empty(), "{:?}", config.problems());

        let plan = plan::make(&config, root.path()).unwrap();
        assert!(plan.add_packages.is_empty());
        assert!(plan.remove_packages.is_empty());
        assert!(plan.enable_services.is_empty());
        assert!(plan.extra_services.is_empty());
    }

    #[test]
    fn detects_no_multilib_profile() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("etc/portage")).unwrap();
        std::os::unix::fs::symlink(
            "../../var/db/repos/gentoo/profiles/default/linux/amd64/23.0/no-multilib",
            dir.path().join("etc/portage/make.profile"),
        )
        .unwrap();
        assert!(!from_system(dir.path()).unwrap().multilib);
    }
}
