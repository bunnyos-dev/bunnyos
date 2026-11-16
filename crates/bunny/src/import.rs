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
    /// Whether make.conf sets global USE flags (not managed by bunny yet).
    pub global_use: bool,
}

pub fn from_system(root: &Path) -> Result<Imported> {
    let packages = portage::read_world(root)
        .context("Couldn't read the world set")?
        .into_iter()
        .collect();
    let services = init::for_config(Init::Openrc, root)
        .user_services()
        .context("Couldn't read which services start at boot")?
        .into_iter()
        .collect();

    let package_use = read_package_use(root)?;
    Ok(Imported {
        packages,
        services,
        multilib: !profile_is_no_multilib(root),
        global_use: make_conf_sets_use(root),
        use_flags: package_use.flags,
        skipped: package_use.skipped,
    })
}

/// Per-package USE flags from every file in `/etc/portage/package.use`.
#[derive(Debug, Default)]
pub struct PackageUse {
    pub flags: BTreeMap<String, Vec<String>>,
    /// Lines bunny can't represent yet, with the file they came from.
    pub skipped: Vec<(PathBuf, String)>,
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
                Some((atom, flags)) => {
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

/// Parses `category/name flag -flag GROUP: value`. Returns `None` for lines
/// bunny can't represent yet (version ranges, wildcards).
fn parse_use_line(line: &str) -> Option<(String, Vec<String>)> {
    let mut words = line.split_whitespace();
    let atom = words.next()?;
    if !is_package_name(atom) {
        return None;
    }
    let mut flags = Vec::new();
    let mut group: Option<String> = None;
    for word in words {
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
    Some((atom.to_string(), flags))
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

        if self.global_use {
            out.push_str(
                "\n# Your make.conf sets global USE flags. bunny doesn't manage make.conf yet,\n\
                 # so those stay where they are for now.\n",
            );
        }

        if !self.skipped.is_empty() {
            out.push_str(
                "\n# These package.use lines couldn't be imported yet (version ranges or\n\
                 # wildcards). They stay in their files and keep working:\n",
            );
            for (file, line) in &self.skipped {
                let _ = writeln!(out, "#   {}: {line}", file.display());
            }
        }
        out
    }

    pub fn summary(&self) -> String {
        let mut text = format!(
            "Imported {} packages, {} services, and USE flags for {} packages.",
            self.packages.len(),
            self.services.len(),
            self.use_flags.values().filter(|f| !f.is_empty()).count(),
        );
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
        write(
            root,
            "etc/portage/package.use/browser",
            "# comment\nwww-client/firefox pgo -telemetry\nwww-client/firefox telemetry\n",
        );
        write(
            root,
            "etc/portage/package.use/python",
            "dev-lang/python:3.12 sqlite PYTHON_TARGETS: python3_12 -python3_11\n\
             >=dev-libs/foo-1.2 bar\n",
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
        assert_eq!(imported.skipped.len(), 1);
        assert!(!imported.use_flags.contains_key("x/y"));
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
