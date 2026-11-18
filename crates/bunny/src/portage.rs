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

pub fn world_path(root: &Path) -> PathBuf {
    root.join(WORLD_FILE)
}

pub fn package_use_dir(root: &Path) -> PathBuf {
    root.join(PACKAGE_USE_DIR)
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
/// bunny keeps by itself: the admin tool from `[security]`, and bunny's own
/// package, so a switch never removes bunny.
pub fn wanted_world(config: &Config, root: &Path) -> BTreeSet<String> {
    let mut world: BTreeSet<String> = config.packages.iter().cloned().collect();
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

/// The contents bunny's package.use file should have.
pub fn render_package_use(config: &Config) -> String {
    let mut out = String::from("# Managed by bunny. Edit bunny.toml instead of this file.\n");
    for (atom, flags) in config.use_flags() {
        out.push_str(&format!("{atom} {}\n", flags.join(" ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
