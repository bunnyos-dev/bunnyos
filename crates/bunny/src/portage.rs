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

/// The packages the config asks for, sorted and de-duplicated.
pub fn wanted_world(config: &Config) -> BTreeSet<String> {
    let mut world: BTreeSet<String> = config.packages.iter().cloned().collect();
    if let Some(sec) = &config.security {
        world.insert(sec.privilege.package().to_string());
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
pub fn render_world(config: &Config) -> String {
    let mut out = String::new();
    for atom in wanted_world(config) {
        out.push_str(&atom);
        out.push('\n');
    }
    out
}

/// The contents bunny's package.use file should have.
pub fn render_package_use(config: &Config) -> String {
    let mut out = String::from("# Managed by bunny. Edit bunny.toml instead of this file.\n");
    for (atom, settings) in &config.pkg {
        if !settings.use_flags.is_empty() {
            out.push_str(&format!("{atom} {}\n", settings.use_flags.join(" ")));
        }
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
