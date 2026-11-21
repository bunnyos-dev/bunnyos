//! Taking over `/etc/portage/package.use`, and putting it back.

use std::path::Path;

use anyhow::{Context, Result};

use crate::config::Config;
use crate::import;
use crate::portage;

/// The copy of package.use taken before the takeover, inside the backup folder.
pub(super) const USE_BEFORE: &str = "package.use.before";
/// The same for package.accept_keywords.
pub(super) const KEYWORDS_BEFORE: &str = "package.accept_keywords.before";

/// package.use files that aren't bunny's (they'll be moved to the backup).
pub fn foreign_package_use_files(root: &Path) -> Vec<String> {
    let dir = portage::package_use_dir(root);
    if dir.is_file() {
        return vec![portage::PACKAGE_USE_DIR.to_string()];
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != portage::BUNNY_USE_FILE && n != portage::KEPT_USE_FILE)
        .collect();
    names.sort();
    names
}

/// Moves the user's package.use files into the backup folder and replaces
/// them with bunny's file. Nothing is lost: the flags are in the config, and
/// lines bunny can't manage yet are kept word for word in their own file.
pub(super) fn adopt_package_use(config: &Config, root: &Path, backup: &Path) -> Result<()> {
    let kept = import::read_package_use(root)?.skipped;
    let dir = portage::package_use_dir(root);
    // Lines already in bunny's kept file remember where they first came from.
    let origins = kept_origins(&dir.join(portage::KEPT_USE_FILE));
    let backup_dir = backup.join("package.use");

    if dir.is_file() {
        std::fs::create_dir_all(backup)?;
        std::fs::rename(&dir, &backup_dir).context("Couldn't back up package.use")?;
    } else if dir.is_dir() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == portage::BUNNY_USE_FILE || name == portage::KEPT_USE_FILE {
                continue;
            }
            std::fs::create_dir_all(&backup_dir)?;
            std::fs::rename(entry.path(), backup_dir.join(&name)).with_context(|| {
                format!("Couldn't move {} to the backup", entry.path().display())
            })?;
        }
    }
    std::fs::create_dir_all(&dir)?;

    let kept_path = dir.join(portage::KEPT_USE_FILE);
    if kept.is_empty() {
        let _ = std::fs::remove_file(&kept_path);
    } else {
        let mut text = String::from(
            "# Kept by bunny: lines bunny can't manage yet (version ranges, wildcards).\n\
             # They work as before. Edit them here.\n",
        );
        for (file, line) in &kept {
            let from = if file
                .file_name()
                .is_some_and(|n| n == portage::KEPT_USE_FILE)
            {
                origins
                    .get(line.as_str())
                    .cloned()
                    .unwrap_or_else(|| file.display().to_string())
            } else {
                file.display().to_string()
            };
            text.push_str(&format!("# from {from}\n{line}\n"));
        }
        std::fs::write(&kept_path, text)?;
    }

    std::fs::write(
        dir.join(portage::BUNNY_USE_FILE),
        portage::render_package_use(config),
    )
    .context("Couldn't write bunny's package.use file")?;
    Ok(())
}

/// In bunny's kept file, each line follows a "# from <file>" comment: which
/// file it first came from.
pub(super) fn kept_origins(path: &Path) -> std::collections::HashMap<String, String> {
    let mut origins = std::collections::HashMap::new();
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut from: Option<&str> = None;
    for line in text.lines() {
        if let Some(f) = line.strip_prefix("# from ") {
            from = Some(f);
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            if let Some(f) = from.take() {
                origins.insert(line.to_string(), f.to_string());
            }
        }
    }
    origins
}

/// Copies a file or a folder (recursively).
pub(super) fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to).with_context(|| format!("Couldn't copy {}", from.display()))?;
    }
    Ok(())
}

/// Puts the world file and package.use back as they were before this switch.
/// Used when emerge refuses or the user cancels before anything is installed.
pub(super) fn restore_files(root: &Path, backup: &Path) -> Result<()> {
    let world = portage::world_path(root);
    let saved = backup.join("world");
    if saved.exists() {
        std::fs::copy(&saved, &world).context("Couldn't put the world file back")?;
    } else {
        let _ = std::fs::remove_file(&world);
    }
    let use_dir = portage::package_use_dir(root);
    if use_dir.is_dir() {
        std::fs::remove_dir_all(&use_dir)?;
    } else if use_dir.exists() {
        std::fs::remove_file(&use_dir)?;
    }
    let before = backup.join(USE_BEFORE);
    if before.exists() {
        copy_tree(&before, &use_dir).context("Couldn't put package.use back")?;
    }
    let keywords = root.join(portage::KEYWORDS_DIR);
    if keywords.is_dir() {
        std::fs::remove_dir_all(&keywords)?;
    } else if keywords.exists() {
        std::fs::remove_file(&keywords)?;
    }
    let before = backup.join(KEYWORDS_BEFORE);
    if before.exists() {
        copy_tree(&before, &keywords).context("Couldn't put package.accept_keywords back")?;
    }
    Ok(())
}

/// Writes bunny's package.accept_keywords file from `[pkg] keywords`. Other
/// files there are left alone; a package.accept_keywords that's a single
/// file moves into the folder unchanged (`00-yours`), so bunny's file can sit
/// next to it.
pub(super) fn write_keywords(config: &Config, root: &Path, backup: &Path) -> Result<()> {
    let dir = root.join(portage::KEYWORDS_DIR);
    let saved = backup.join(KEYWORDS_BEFORE);
    if saved.is_dir() {
        std::fs::remove_dir_all(&saved)?;
    } else if saved.exists() {
        std::fs::remove_file(&saved)?;
    }
    if dir.exists() {
        copy_tree(&dir, &saved)?;
    }
    let ours = dir.join(portage::BUNNY_KEYWORDS_FILE);
    if config.keywords().is_empty() {
        if ours.is_file() {
            std::fs::remove_file(&ours)?;
        }
        return Ok(());
    }
    if dir.is_file() {
        let tmp = root.join("etc/portage/.accept_keywords.bunny-move");
        std::fs::rename(&dir, &tmp)?;
        std::fs::create_dir_all(&dir)?;
        std::fs::rename(&tmp, dir.join(portage::KEPT_KEYWORDS_FILE))?;
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&ours, portage::render_keywords(config))
        .context("Couldn't write package.accept_keywords")?;
    Ok(())
}
