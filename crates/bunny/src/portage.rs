//! Turning the config into the files Portage reads.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// Where Portage keeps the world set, relative to the system root.
pub const WORLD_FILE: &str = "var/lib/portage/world";
/// The package.use directory.
pub const PACKAGE_USE_DIR: &str = "etc/portage/package.use";
/// bunny's own package.use file. Named so Portage reads it last, so it wins.
pub const BUNNY_USE_FILE: &str = "zz-bunny";
/// package.use lines bunny can't manage yet, kept word for word. Read first.
pub const KEPT_USE_FILE: &str = "00-bunny-kept";

/// The package.accept_keywords directory, and bunny's file in it (read last).
pub const KEYWORDS_DIR: &str = "etc/portage/package.accept_keywords";
pub const BUNNY_KEYWORDS_FILE: &str = "zz-bunny";
/// Where a package.accept_keywords that was a single file goes, so bunny's
/// file can sit next to it.
pub const KEPT_KEYWORDS_FILE: &str = "00-yours";

/// repos.conf, and the prefix of the files bunny writes there (one per
/// repository in `repositories`).
pub const REPOS_CONF_DIR: &str = "etc/portage/repos.conf";
pub const BUNNY_REPO_PREFIX: &str = "bunny-";

pub fn world_path(root: &Path) -> PathBuf {
    root.join(WORLD_FILE)
}

pub fn package_use_dir(root: &Path) -> PathBuf {
    root.join(PACKAGE_USE_DIR)
}

/// Whether `atom` (`category/name`, maybe with `:slot`) is installed, from
/// Portage's database. `None` when there's no database to read (a fake root
/// in tests, or a system that isn't Gentoo).
pub fn is_installed(root: &Path, atom: &str) -> Option<bool> {
    let db = root.join("var/db/pkg");
    if !db.is_dir() {
        return None;
    }
    let atom = atom.split(':').next().unwrap_or(atom);
    let (category, name) = atom.split_once('/')?;
    let Ok(entries) = std::fs::read_dir(db.join(category)) else {
        return Some(false);
    };
    Some(entries.flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit()))
    }))
}

/// bunny's own packages, from the Bunny Linux overlay: the prebuilt release and
/// the one built from git.
pub const BUNNY_PACKAGES: [&str; 2] = ["app-admin/bunny-bin", "app-admin/bunny"];

/// Which of bunny's own packages is installed, if bunny came from the overlay.
pub fn bunny_itself(root: &Path) -> Option<&'static str> {
    let dir = root.join("var/db/pkg/app-admin");
    let installed: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    BUNNY_PACKAGES.into_iter().find(|atom| {
        let name = atom.rsplit('/').next().unwrap_or(atom);
        installed.iter().any(|entry| {
            entry
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('-'))
                .is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit()))
        })
    })
}

/// The packages the config asks for, sorted and de-duplicated, plus the ones
/// bunny keeps by itself: the admin tool from `[security]`, what `[desktop]`
/// needs, and bunny's own package, so a switch never removes bunny.
pub fn wanted_world(config: &Config, root: &Path) -> BTreeSet<String> {
    let mut world: BTreeSet<String> = config.packages.iter().cloned().collect();
    world.extend(config.desktop.packages().iter().map(|p| p.to_string()));
    if let Some(sec) = &config.security {
        world.insert(sec.privilege.package().to_string());
    }
    if let Some(own) = bunny_itself(root) {
        world.insert(own.to_string());
    }
    world
}

/// Reads the current world set. A missing file means an empty set.
pub fn read_world(root: &Path) -> std::io::Result<BTreeSet<String>> {
    match std::fs::read_to_string(world_path(root)) {
        Ok(text) => Ok(text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(e) => Err(e),
    }
}

/// The world file for the config: one package per line, sorted.
pub fn render_world(config: &Config, root: &Path) -> String {
    let mut out = String::new();
    for atom in wanted_world(config, root) {
        out.push_str(&atom);
        out.push('\n');
    }
    out
}

/// The contents bunny's package.use file should have. `[hardware]`'s `*/*`
/// lines come first, so a package's own flags still win over them.
pub fn render_package_use(config: &Config) -> String {
    let mut out = String::from("# Managed by bunny. Edit bunny.toml instead of this file.\n");
    out.push_str(&config.hardware.render());
    for (atom, flags) in config.use_flags() {
        out.push_str(&format!("{atom} {}\n", flags.join(" ")));
    }
    out
}

/// The contents bunny's package.accept_keywords file should have.
pub fn render_keywords(config: &Config) -> String {
    let mut out = String::from("# Managed by bunny. Edit bunny.toml ([pkg] keywords) instead.\n");
    for (atom, keywords) in config.keywords() {
        out.push_str(&format!("{atom} {}\n", keywords.join(" ")));
    }
    out
}

/// What bunny's package.accept_keywords file says now, per package.
pub fn read_bunny_keywords(root: &Path) -> std::collections::BTreeMap<String, Vec<String>> {
    let path = root.join(KEYWORDS_DIR).join(BUNNY_KEYWORDS_FILE);
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            let atom = words.next()?.to_string();
            Some((atom, words.map(String::from).collect()))
        })
        .collect()
}

/// Repositories Portage knows about, from every repos.conf file: the name,
/// and whether bunny manages it (one of its `bunny-<name>.conf` files).
/// Gentoo's own (`[gentoo]`) and `[DEFAULT]` aren't included.
pub fn configured_repositories(root: &Path) -> std::collections::BTreeMap<String, bool> {
    let mut found = std::collections::BTreeMap::new();
    let dir = root.join(REPOS_CONF_DIR);
    let files: Vec<PathBuf> = if dir.is_file() {
        vec![dir]
    } else {
        std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect()
    };
    for file in files {
        let ours = file
            .file_name()
            .map(|n| n.to_string_lossy().starts_with(BUNNY_REPO_PREFIX))
            .unwrap_or(false);
        for line in std::fs::read_to_string(&file).unwrap_or_default().lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                if name != "DEFAULT" && name != "gentoo" {
                    let entry = found.entry(name.to_string()).or_insert(false);
                    *entry |= ours;
                }
            }
        }
    }
    found
}

/// bunny's repos.conf file for one repository.
pub fn render_repo_conf(name: &str, sync_uri: &str) -> String {
    format!(
        "# Managed by bunny: `repositories` in bunny.toml. Edit bunny.toml instead.\n\
         [{name}]\n\
         location = /var/db/repos/{name}\n\
         sync-type = git\n\
         sync-uri = {sync_uri}\n\
         auto-sync = yes\n"
    )
}

pub fn repo_conf_path(root: &Path, name: &str) -> PathBuf {
    root.join(REPOS_CONF_DIR)
        .join(format!("{BUNNY_REPO_PREFIX}{name}.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_keywords_and_reads_them_back() {
        let config = Config::parse(
            "[pkg.\"gui-wm/dwl\"]\nkeywords = [\"~amd64\"]\n[pkg.\"app-misc/tmux\"]\nuse = [\"x\"]\n",
        )
        .unwrap();
        let text = render_keywords(&config);
        assert!(text.contains("gui-wm/dwl ~amd64\n"));
        assert!(!text.contains("tmux"));
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(KEYWORDS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(BUNNY_KEYWORDS_FILE), &text).unwrap();
        assert_eq!(read_bunny_keywords(root.path())["gui-wm/dwl"], ["~amd64"]);
    }

    #[test]
    fn finds_the_repositories_portage_knows() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(REPOS_CONF_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("gentoo.conf"),
            "[DEFAULT]\nmain-repo = gentoo\n[gentoo]\nlocation = /var/db/repos/gentoo\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("eselect-repo.conf"),
            "[guru]\nlocation = /var/db/repos/guru\n",
        )
        .unwrap();
        std::fs::write(
            repo_conf_path(root.path(), "hyproverlay"),
            render_repo_conf("hyproverlay", "https://example.org/h.git"),
        )
        .unwrap();
        let found = configured_repositories(root.path());
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(!found["guru"]);
        assert!(found["hyproverlay"]);
    }

    #[test]
    fn renders_package_use() {
        let config = Config::parse(
            r#"
            [pkg."www-client/firefox"]
            use = ["pgo", "-telemetry"]
            [pkg."app-misc/empty"]
            "#,
        )
        .unwrap();
        let text = render_package_use(&config);
        assert!(text.contains("www-client/firefox pgo -telemetry\n"));
        assert!(!text.contains("app-misc/empty"));
    }

    #[test]
    fn keeps_bunny_itself_in_the_world_set() {
        let config = Config::parse("packages = [\"app-misc/tmux\"]").unwrap();
        let root = tempfile::tempdir().unwrap();
        // Not installed from the overlay: nothing extra.
        assert_eq!(render_world(&config, root.path()), "app-misc/tmux\n");

        let pkg = root.path().join("var/db/pkg/app-admin");
        // A different package whose name starts the same way doesn't count.
        std::fs::create_dir_all(pkg.join("bunnyfoo-1.0")).unwrap();
        assert_eq!(bunny_itself(root.path()), None);

        std::fs::create_dir_all(pkg.join("bunny-bin-0.1.0")).unwrap();
        assert_eq!(bunny_itself(root.path()), Some("app-admin/bunny-bin"));
        assert_eq!(
            render_world(&config, root.path()),
            "app-admin/bunny-bin\napp-misc/tmux\n"
        );

        std::fs::remove_dir(pkg.join("bunny-bin-0.1.0")).unwrap();
        std::fs::create_dir_all(pkg.join("bunny-9999")).unwrap();
        assert_eq!(bunny_itself(root.path()), Some("app-admin/bunny"));
    }

    #[test]
    fn reads_world_ignoring_blanks_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = world_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "app-editors/neovim\n\n# note\nwww-client/firefox\n").unwrap();
        let world = read_world(dir.path()).unwrap();
        assert_eq!(world.len(), 2);
    }

    #[test]
    fn missing_world_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_world(dir.path()).unwrap().is_empty());
    }
}
